#!/usr/bin/env python3
"""
路线 B 变体矩阵：Chromium TLS 下，找出哪个头部组合能拿到 data。

已知基线（本次实测）：
  - rustls + 完整 App 头                 → 1004
  - Chromium + 完整 App 头（剥浏览器头） → 200 {"status":403,"message":"API unsupported."}

变体矩阵（全部固定 Chromium TLS，只改请求头）：
  V1 完整 App 头 + 保留浏览器头（Origin/Referer/Sec-Fetch 等）
  V2 完整 App 头，但 UA 换成 Chromium 的
  V3 完整 App 头 + X-Requested-With 去掉
  V4 只带 TalkBack/App 最小头集
  V5 完整 App 头 + Accept-Encoding: gzip（浏览器默认已带）
  V6 打 api.coolapk.com（而不是 api2）
"""

import json
import sys

from cdp_intercept import run_case
from kuan_headers import TARGET_URL, build_headers

API2_ROOT = "https://api2.coolapk.com/"
ALT_URL = "https://api.coolapk.com/v6/main/indexV8?page=1&firstLaunch=1&installTime=1761926400000"


def main() -> int:
    base = build_headers()
    results = []

    results.append(run_case("V1 完整头 + 保留浏览器头", TARGET_URL, base, API2_ROOT,
                            strip_browser=False))

    ua_browser = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
                  "(KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36 Edg/156.0.0.0")
    v2 = dict(base)
    v2["User-Agent"] = ua_browser
    results.append(run_case("V2 完整头 + Chromium UA", TARGET_URL, v2, API2_ROOT,
                            strip_browser=True))

    v3 = {k: v for k, v in base.items() if k != "X-Requested-With"}
    results.append(run_case("V3 去掉 X-Requested-With", TARGET_URL, v3, API2_ROOT,
                            strip_browser=True))

    minimal = {k: base[k] for k in
               ("User-Agent", "X-App-Id", "X-App-Version", "X-App-Code",
                "X-Api-Version", "X-App-Device", "X-App-Token", "X-Sdk-Int")}
    results.append(run_case("V4 最小 App 头集", TARGET_URL, minimal, API2_ROOT,
                            strip_browser=True))

    results.append(run_case("V6 api.coolapk.com 域名", ALT_URL, base,
                            "https://api.coolapk.com/", strip_browser=True))

    print("\n" + "=" * 72)
    print("变体矩阵汇总")
    print("=" * 72)
    for r in results:
        print(f"  {r['name']:34s} HTTP {r['status']}  →  {r['verdict']}")
        if r["body"]:
            print(f"      {r['body'][:160]}")
    return 0 if any(r["verdict"] == "DATA" for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
