#!/usr/bin/env python3
"""
路线 B 前提验证：用 Edge 的 TLS 指纹，通过 CDP Fetch 域发请求。

为什么不用裸 fetch()：
  页面 JS 的 fetch 会被 Chromium 强制补上 Origin / Referer / Sec-Fetch-*，
  且响应受 CORS 约束。上一版直接 fetch 拿到的是 TencentEdgeOne 的 403。

本版用 CDP Fetch 拦截：
  1. 导航到 https://api2.coolapk.com/ 建立同域上下文与 cookie 罐
  2. Fetch.enable(requestStage=Request) 拦截
  3. 页面发起 fetch（被拦）
  4. Fetch.continueRequest 精确改写请求头为酷安 App 头
  5. 开第二个 Fetch 拦截在 Response 阶段读原始 body

TLS 握手 / h2 / TLS 扩展全部由 Edge(Chromium) 产生，
请求头完全由我们控制 —— 这正是「浏览器指纹 + App 头」的实验条件。

用法: python cdp_intercept.py
"""

import base64
import json
import sys
import threading
import time

import websocket

from cdp_fetch import CDP_PORT, USER_DATA, launch_edge, wait_for_cdp
from kuan_headers import TARGET_URL, build_headers


class Cdp4:
    """后台线程收流的 CDP 客户端，事件进队列，send 同步等待响应。"""

    def __init__(self, ws_url):
        self.ws = websocket.create_connection(ws_url, timeout=60, max_size=None)
        self.next_id = 0
        self.lock = threading.Condition()
        self.responses = {}
        self.events = []
        self.running = True
        self.t = threading.Thread(target=self._loop, daemon=True)
        self.t.start()

    def _loop(self):
        while self.running:
            try:
                raw = self.ws.recv()
            except Exception:  # noqa: BLE001
                if self.running:
                    time.sleep(0.05)
                continue
            if not raw:
                continue
            try:
                msg = json.loads(raw)
            except Exception:  # noqa: BLE001
                continue
            with self.lock:
                if "id" in msg:
                    self.responses[msg["id"]] = msg
                elif "method" in msg:
                    self.events.append(msg)
                self.lock.notify_all()

    def send(self, method, params=None, timeout=30.0):
        self.next_id += 1
        mid = self.next_id
        self.ws.send(json.dumps({"id": mid, "method": method, "params": params or {}}))
        deadline = time.time() + timeout
        with self.lock:
            while mid not in self.responses:
                if time.time() > deadline:
                    raise RuntimeError(f"{method} 超时")
                self.lock.wait(0.1)
            msg = self.responses.pop(mid)
        if "error" in msg:
            raise RuntimeError(f"{method}: {msg['error'].get('message')}")
        return msg.get("result", {})

    def wait_event(self, method, timeout=30.0, pred=None):
        deadline = time.time() + timeout
        with self.lock:
            while True:
                for i, ev in enumerate(self.events):
                    if ev["method"] == method and (pred is None or pred(ev)):
                        return self.events.pop(i)
                left = deadline - time.time()
                if left <= 0:
                    return None
                self.lock.wait(min(left, 0.2))

    def pop_events(self, method):
        with self.lock:
            hits = [e for e in self.events if e["method"] == method]
            self.events = [e for e in self.events if e["method"] != method]
            return hits

    def close(self):
        self.running = False
        try:
            self.ws.close()
        except Exception:  # noqa: BLE001
            pass


BROWSER_ONLY = (
    "origin", "referer", "sec-fetch-site", "sec-fetch-mode", "sec-fetch-dest",
    "sec-fetch-user", "sec-ch-ua", "sec-ch-ua-mobile", "sec-ch-ua-platform",
    "upgrade-insecure-requests", "sec-purpose", "priority",
)


def run_case(name, url, app_headers, navigate_to, strip_browser=True):
    proc = launch_edge(CDP_PORT, USER_DATA)
    try:
        target = wait_for_cdp(CDP_PORT)
        cdp = Cdp4(target["webSocketDebuggerUrl"])
        try:
            cdp.send("Network.enable")
            cdp.send("Page.enable")
            cdp.send("Runtime.enable")

            print(f"\n=== {name}")
            print(f"    origin       : {navigate_to}")
            print(f"    App 头数量   : {len(app_headers)}")
            print(f"    剥离浏览器头 : {strip_browser}")

            cdp.send("Page.navigate", {"url": navigate_to})
            time.sleep(2.5)

            # Request 阶段拦截
            cdp.send("Fetch.enable", {
                "patterns": [{"urlPattern": "*://api2.coolapk.com/v6/*", "requestStage": "Request"}]
            })

            expr = (f"fetch({json.dumps(url)},{{cache:'no-store',credentials:'include'}})"
                    ".then(r=>r.text()).then(t=>t.slice(0,50)).catch(e=>'ERR '+e.message)")
            cdp.send("Runtime.evaluate",
                     {"expression": expr, "awaitPromise": False, "returnByValue": True})

            paused = cdp.wait_event("Fetch.requestPaused", timeout=20)
            if not paused:
                print("    ❌ 未捕获到请求")
                return {"name": name, "verdict": "NO-INTERCEPT", "status": 0, "body": ""}

            rid = paused["params"]["requestId"]
            orig = paused["params"]["request"]

            merged = dict(orig.get("headers") or {})
            drop = {h.lower() for h in app_headers}
            if strip_browser:
                drop |= set(BROWSER_ONLY)
            merged = {k: v for k, v in merged.items() if k.lower() not in drop}
            merged.update(app_headers)

            print(f"    实际发出头数 : {len(merged)}")
            for k in ("User-Agent", "X-App-Device", "X-App-Token", "Accept"):
                if k in merged:
                    v = merged[k]
                    print(f"      {k:14s}: {(v[:55] + '...') if len(v) > 55 else v}")

            cdp.send("Fetch.continueRequest", {
                "requestId": rid,
                "headers": [{"name": k, "value": str(v)} for k, v in merged.items()],
            })

            # 等 fetch promise 结算（页面拿到 body 或 CORS 错误）
            res = None
            deadline = time.time() + 20
            while time.time() < deadline:
                try:
                    r = cdp.send("Runtime.evaluate", {
                        "expression": "window.__r === undefined ? 'pending' : window.__r",
                        "returnByValue": True}, timeout=5)
                    val = r.get("result", {}).get("value")
                    break
                except Exception:  # noqa: BLE001
                    time.sleep(0.2)

            # 从 Network 事件读状态
            status, server, body = 0, "?", ""
            for _ in range(40):
                hits = cdp.pop_events("Network.responseReceived")
                for h in hits:
                    p = h["params"]
                    if "api2.coolapk.com" in p["response"].get("url", ""):
                        status = p["response"].get("status", 0)
                        server = next((v for k, v in (p["response"].get("headers") or {}).items()
                                       if k.lower() == "server"), "?")
                        try:
                            b = cdp.send("Network.getResponseBody", {"requestId": p["requestId"]})
                            body = b.get("body", "")
                            if b.get("base64Encoded"):
                                body = base64.b64decode(body).decode("utf-8", "replace")
                        except Exception as e:  # noqa: BLE001
                            body = f"<body unavailable: {e}>"
                        break
                if status:
                    break
                time.sleep(0.3)

            print(f"    HTTP {status}  server={server}  len={len(body)}")
            print(f"    body: {body[:300]}")

            verdict = "?"
            try:
                p = json.loads(body)
                if isinstance(p, dict):
                    st = str(p.get("status"))
                    if st == "1004":
                        verdict = "1004"
                    elif "data" in p:
                        verdict = "DATA"
                    else:
                        verdict = f"other(status={st})"
            except Exception:  # noqa: BLE001
                verdict = "NON-JSON"
            print(f"    判定: {verdict}")
            return {"name": name, "verdict": verdict, "status": status, "body": body[:600]}
        finally:
            cdp.close()
    finally:
        try:
            proc.terminate()
            proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            pass


def main() -> int:
    results = []
    results.append(run_case("A. Chromium TLS + 完整 App 头（剥浏览器头）",
                            TARGET_URL, build_headers(),
                            "https://api2.coolapk.com/", strip_browser=True))
    print("\n" + "=" * 72)
    for r in results:
        print(f"  {r['name']}: HTTP {r['status']} → {r['verdict']}")
    return 0 if any(r["verdict"] == "DATA" for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
