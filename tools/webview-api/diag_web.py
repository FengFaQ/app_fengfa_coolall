#!/usr/bin/env python3
"""
最后一个关键实验：Chromium 发「真浏览器」请求打 web 站 / API，看能不能拿到真实内容。

如果浏览器连 www.coolapk.com 都只能拿到 403 API unsupported.，
说明 EdgeOne WAF 对「这台机器的 Edge」本身就拦，SESSID w1-/a1- 之差也就不重要了。

对比项：
  1. Edge 原生 navigate https://www.coolapk.com/  → 页面标题 / 长度
  2. Edge 原生 navigate https://api2.coolapk.com/v6/main/indexV8?... → 响应
  3. rustls 打 https://www.coolapk.com/ → 状态码 / 长度
"""

import base64
import json
import sys
import time

import requests

from cdp_fetch import CDP_PORT, USER_DATA, launch_edge, wait_for_cdp
from cdp_intercept import Cdp4
from kuan_headers import TARGET_URL, build_headers


def main() -> int:
    print("=" * 72)
    print("1) rustls 打 www.coolapk.com（真浏览器之外的对照）")
    print("=" * 72)
    try:
        r = requests.get("https://www.coolapk.com/", headers={
            "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
                          "(KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36 Edg/156.0.0.0",
        }, timeout=20)
        print(f"  HTTP {r.status_code}  len={len(r.content)}  server={r.headers.get('Server')}")
        print(f"  head: {r.text[:200]}")
    except Exception as e:  # noqa: BLE001
        print(f"  异常: {e}")

    proc = launch_edge(CDP_PORT, USER_DATA)
    try:
        target = wait_for_cdp(CDP_PORT)
        cdp = Cdp4(target["webSocketDebuggerUrl"])
        try:
            cdp.send("Page.enable")
            cdp.send("Network.enable")
            cdp.send("Runtime.enable")

            print("\n" + "=" * 72)
            print("2) Edge 原生 navigate https://www.coolapk.com/")
            print("=" * 72)
            cdp.send("Page.navigate", {"url": "https://www.coolapk.com/"})
            time.sleep(5)
            ev = cdp.pop_events("Network.responseReceived")
            for h in ev:
                p = h["params"]["response"]
                if p["url"].rstrip("/") == "https://www.coolapk.com":
                    print(f"  HTTP {p['status']} server={next((v for k,v in (p.get('headers') or {}).items() if k.lower()=='server'),'?')}")
                    try:
                        b = cdp.send("Network.getResponseBody", {"requestId": h["params"]["requestId"]})
                        body = b.get("body", "")
                        if b.get("base64Encoded"):
                            body = base64.b64decode(body).decode("utf-8", "replace")
                        print(f"  len={len(body)}")
                        print(f"  head: {body[:300]}")
                    except Exception as e:  # noqa: BLE001
                        print(f"  body 不可读: {e}")
            info = cdp.send("Runtime.evaluate", {
                "expression": "JSON.stringify({title:document.title, "
                              "len:document.documentElement.outerHTML.length, "
                              "txt:document.body?document.body.innerText.slice(0,200):''})",
                "returnByValue": True})
            print(f"  页面: {info.get('result', {}).get('value')}")

            print("\n" + "=" * 72)
            print("3) Edge 原生 navigate API URL（浏览器原生，无任何自定义头）")
            print("=" * 72)
            cdp.pop_events("Network.responseReceived")
            cdp.send("Page.navigate", {"url": TARGET_URL})
            time.sleep(4)
            got = False
            for h in cdp.pop_events("Network.responseReceived"):
                p = h["params"]["response"]
                if "api2.coolapk.com" in p["url"]:
                    got = True
                    print(f"  HTTP {p['status']} proto={p.get('protocol')}")
                    try:
                        b = cdp.send("Network.getResponseBody", {"requestId": h["params"]["requestId"]})
                        body = b.get("body", "")
                        if b.get("base64Encoded"):
                            body = base64.b64decode(body).decode("utf-8", "replace")
                        print(f"  len={len(body)}")
                        print(f"  head: {body[:400]}")
                    except Exception as e:  # noqa: BLE001
                        print(f"  body 不可读: {e}")
            if not got:
                print("  （没有捕获到 api2 的响应事件）")

            print("\n  罐内 cookie:")
            for c in cdp.send("Network.getAllCookies").get("cookies", []):
                if "coolapk" in c.get("domain", ""):
                    print(f"    {c['domain']:22s} {c['name']} = {c['value'][:40]}")
        finally:
            cdp.close()
    finally:
        try:
            proc.terminate()
            proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
