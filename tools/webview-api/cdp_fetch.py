#!/usr/bin/env python3
"""
路线 B 前提验证：用 Edge（Chromium 内核）通过 CDP 发酷安 API 请求。

判据：
  - 返回 data        → 浏览器 TLS 指纹可通过，路线 B 成立
  - 仍返回 1004      → 路线 B 不通，如实报告

关键点：
  1. 必须先 Navigate 到 https://api2.coolapk.com/再 fetch —— 同源，绕开 CORS。
     用 about:blank 或 www.coolapk.com 作为 origin 会被 CORS 拦掉。
  2. Page.navigate 会带上浏览器自己的 Cookie 罐（真实的 Chromium 会话）。
  3. 请求头通过 fetch() 的 headers 传入，Origin/Referer 由 Chromium 自己填。

用法：
  python cdp_fetch.py            # 完整实验：A 组原始请求 / B 组带 Cookie / C 组对照
  python cdp_fetch.py --keep     # 实验后不关浏览器
"""

import json
import subprocess
import sys
import time
from pathlib import Path

import requests
import websocket  # websocket-client

from kuan_headers import TARGET_URL, build_headers

EDGE = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
CDP_PORT = 9222
USER_DATA = Path(__file__).resolve().parent / ".edge-profile"


# --------------------------------------------------------------------------
# CDP 最小客户端
# --------------------------------------------------------------------------
class Cdp:
    def __init__(self, ws_url: str):
        self.ws = websocket.create_connection(ws_url, timeout=30, max_size=None)
        self.next_id = 0

    def send(self, method: str, params: dict | None = None) -> dict:
        self.next_id += 1
        mid = self.next_id
        self.ws.send(json.dumps({"id": mid, "method": method, "params": params or {}}))

        for _ in range(400):  # 跳过事件帧，直到拿到同 id 的响应
            raw = self.ws.recv()
            if not raw:
                continue
            msg = json.loads(raw)
            if msg.get("id") == mid:
                if "error" in msg:
                    raise RuntimeError(f"{method} 失败: {msg['error']}")
                return msg.get("result", {})
        raise RuntimeError(f"{method} 无响应")

    def eval(self, expression: str, await_promise: bool = False, timeout: float = 45):
        res = self.send(
            "Runtime.evaluate",
            {
                "expression": expression,
                "returnByValue": True,
                "awaitPromise": await_promise,
                "timeout": int(timeout * 1000),
            },
        )
        if res.get("exceptionDetails"):
            d = res["exceptionDetails"]
            raise RuntimeError(f"JS 异常: {d.get('text')} {d.get('exception', {}).get('description', '')}")
        return res.get("result", {}).get("value")

    def close(self):
        try:
            self.ws.close()
        except Exception:
            pass


def launch_edge(port: int, user_data: Path) -> subprocess.Popen:
    user_data.mkdir(parents=True, exist_ok=True)
    args = [
        EDGE,
        f"--remote-debugging-port={port}",
        f"--user-data-dir={user_data}",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-features=msEdgeFirstRunExperience",
        "about:blank",
    ]
    return subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def wait_for_cdp(port: int, timeout: float = 30.0) -> dict:
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        try:
            targets = requests.get(f"http://127.0.0.1:{port}/json", timeout=2).json()
            for t in targets:
                if t.get("type") == "page" and t.get("webSocketDebuggerUrl"):
                    return t
        except Exception as e:  # noqa: BLE001
            last = str(e)
        time.sleep(0.5)
    raise RuntimeError(f"CDP 端口 {port} 未就绪: {last}")


# --------------------------------------------------------------------------
# 实验
# --------------------------------------------------------------------------
FETCH_JS = """
(async () => {
  const url = %URL%;
  const headers = %HEADERS%;
  const t0 = performance.now();
  try {
    const r = await fetch(url, { method: 'GET', headers, credentials: 'include', cache: 'no-store' });
    const text = await r.text();
    return {
      ok: true,
      status: r.status,
      ms: Math.round(performance.now() - t0),
      len: text.length,
      body: text.slice(0, 1200),
    };
  } catch (e) {
    return { ok: false, status: 0, ms: Math.round(performance.now() - t0), len: 0,
             body: 'FETCH THREW: ' + e.name + ': ' + e.message };
  }
})()
"""


def run_case(cdp: Cdp, name: str, headers: dict[str, str], navigate_to: str, label: str) -> dict:
    print(f"\n=== {name} ===")
    print(f"  origin  : {navigate_to}")
    for k in ("User-Agent", "X-App-Device", "X-App-Token"):
        v = headers.get(k, "")
        print(f"  {k:14s}: {(v[:60] + '...') if len(v) > 60 else v}")

    nav = cdp.send("Page.navigate", {"url": navigate_to})
    # 等导航 settle：轮询 document.readyState
    for _ in range(60):
        try:
            state = cdp.eval("document.readyState")
            if state in ("interactive", "complete"):
                break
        except Exception:  # noqa: BLE001
            pass
        time.sleep(0.2)
    time.sleep(0.6)

    js = FETCH_JS.replace("%URL%", json.dumps(TARGET_URL)).replace(
        "%HEADERS%", json.dumps(headers, ensure_ascii=False)
    )
    res = cdp.eval(js, await_promise=True)
    if not res:
        res = {"ok": False, "status": 0, "body": "<no value>", "len": 0, "ms": 0}

    body = res.get("body") or ""
    verdict = "?"
    try:
        parsed = json.loads(body)
        if isinstance(parsed, dict):
            if parsed.get("status") == 1004 or parsed.get("status") == "1004":
                verdict = "1004"
            elif "data" in parsed:
                verdict = "DATA"
            elif "message" in parsed:
                verdict = f"other(status={parsed.get('status')})"
    except Exception:  # noqa: BLE001
        verdict = "NON-JSON"

    print(f"  HTTP    : {res.get('status')}  ({res.get('ms')}ms, {res.get('len')} bytes)")
    print(f"  判定    : {verdict}")
    print(f"  body    : {body[:400]}")

    # 顺手记录浏览器自己的 UA，确认是 Chromium 而非伪装
    if name.endswith("#1"):
        ua = cdp.eval("navigator.userAgent")
        print(f"  浏览器UA: {ua}")

    return {"name": label, "verdict": verdict, "status": res.get("status"), "body": body}


def main() -> int:
    keep = "--keep" in sys.argv
    proc = launch_edge(CDP_PORT, USER_DATA)
    results = []
    try:
        target = wait_for_cdp(CDP_PORT)
        print(f"CDP 就绪: {target['url']}")
        cdp = Cdp(target["webSocketDebuggerUrl"])
        try:
            cdp.send("Page.enable")
            cdp.send("Runtime.enable")

            # A 组：同源导航到 api2 根，带完整 App 头
            h_full = build_headers()
            results.append(
                run_case(cdp, "A-1 完整 App 头 @ api2 同源", h_full,
                         "https://api2.coolapk.com/", "A1: 完整头 + 同源 api2")
            )

            # A 组对照：只带 Accept，看裸请求返回什么（区分「门禁」与「参数错」）
            results.append(
                run_case(cdp, "A-2 仅 Accept @ api2 同源", {"Accept": "application/json"},
                         "https://api2.coolapk.com/", "A2: 裸请求 + 同源 api2")
            )

            # B 组：先访问 web 站建立浏览器会话，再打 API
            results.append(
                run_case(cdp, "B-1 完整 App 头 @ web 会话后", h_full,
                         "https://www.coolapk.com/", "B1: 完整头 + 先逛 www")
            )

            # C 组：完全默认（不传任何自定义头），纯浏览器指纹
            results.append(
                run_case(cdp, "C-1 零自定义头 @ api2 同源", {},
                         "https://api2.coolapk.com/", "C1: 零自定义头")
            )
        finally:
            cdp.close()
    finally:
        if not keep:
            try:
                proc.terminate()
                proc.wait(timeout=10)
            except Exception:  # noqa: BLE001
                pass

    print("\n" + "=" * 70)
    print("汇总")
    print("=" * 70)
    for r in results:
        print(f"  {r['name']:32s} HTTP {r['status']}  →  {r['verdict']}")

    any_data = any(r["verdict"] == "DATA" for r in results)
    print("\n结论: " + ("✅ 浏览器指纹可通过（拿到 data）" if any_data
                        else "❌ 浏览器指纹同样被拒（1004）"))
    return 0 if any_data else 1


if __name__ == "__main__":
    sys.exit(main())
