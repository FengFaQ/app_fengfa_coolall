#!/usr/bin/env python3
"""
对照实验：同一 URL，rustls(requests) vs Chromium(Edge CDP)。

目的：确定 `{"status":403,"message":"API unsupported."}` 的含义。
  - 如果 rustls 的「裸请求」也返回同样的 403 → 说明这是「缺少 App 头」的通用响应，
    与 TLS 指纹无关，说明 Chromium 那组其实走通了门禁、只差头部。
  - 如果 rustls 裸请求返回 1004 → 403 是 Chromium 特有，需要另外解释。
"""

import json
import sys

import requests

from cdp_intercept import run_case
from kuan_headers import TARGET_URL, build_headers

CASES = [
    ("裸请求（无头）", {}),
    ("仅 Accept", {"Accept": "application/json"}),
    ("仅 UA(真机)", {"User-Agent": build_headers()["User-Agent"]}),
    ("完整 App 头", build_headers()),
    ("App 头但 Token 为空", {k: v for k, v in build_headers().items() if k != "X-App-Token"}),
]


def main() -> int:
    print("=" * 72)
    print("rustls (requests) 对照组")
    print("=" * 72)
    rustls = []
    for name, h in CASES:
        try:
            r = requests.get(TARGET_URL, headers=h, timeout=20)
            body = r.text[:200]
            server = r.headers.get("Server", "?")
            print(f"  {name:22s} HTTP {r.status_code} server={server}")
            print(f"      {body[:150]}")
            rustls.append((name, r.status_code, body))
        except Exception as e:  # noqa: BLE001
            print(f"  {name:22s} 异常 {e}")
            rustls.append((name, 0, str(e)))

    print("\n" + "=" * 72)
    print("Chromium (Edge CDP) 对照组")
    print("=" * 72)
    chromium = []
    for name, h in CASES:
        r = run_case(f"Chromium {name}", TARGET_URL, h, "https://api2.coolapk.com/",
                     strip_browser=True)
        chromium.append((name, r["status"], r["body"]))

    print("\n" + "=" * 72)
    print("对照矩阵")
    print("=" * 72)
    print(f"  {'用例':22s} {'rustls':>28s}   {'Chromium':>28s}")
    for (n1, s1, b1), (n2, s2, b2) in zip(rustls, chromium):
        def short(b):
            b = (b or "").replace("\n", " ")[:40]
            return b
        print(f"  {n1:22s} {str(s1) + ' ' + short(b1):>28s}   {str(s2) + ' ' + short(b2):>28s}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
