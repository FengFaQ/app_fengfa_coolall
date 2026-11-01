#!/usr/bin/env python3
"""
关键判别：把「同一份 Token」分别用 rustls 与 Chromium 发出，对比响应。

已知 rustls 完整头 → 1004「请求验证错误」，Chromium 完整头 → 403「API unsupported.」。
若 Chromium 恒定 403 且与头无关，说明 EdgeOne 对 Chromium 的 Session/协商层
直接判为「不支持」，请求根本没进到酷安业务层，Token 对错都看不到。

本脚本额外尝试：给 Chromium 补上它 403 时可能缺失的东西——
  X1 完整 App 头 + Cookie（真实 w1- SESSID）
  X2 完整 App 头 + Accept: application/json + Accept-Language
  X3 完整 App 头 + 二进制协商：带 Connection: keep-alive
"""

import json
import sys

from cdp_intercept import run_case
from kuan_headers import TARGET_URL, build_headers

API2 = "https://api2.coolapk.com/"


def main() -> int:
    base = build_headers()
    results = []

    x1 = dict(base)
    x1["Cookie"] = "SESSID=w1-4f577be931ba19b25b2cd23dc124af31-1793490360-58d4aae46ca9d6d3"
    results.append(run_case("X1 完整头 + 真实 w1- SESSID", TARGET_URL, x1, API2))

    x2 = dict(base)
    x2["Accept-Language"] = "zh-CN,zh;q=0.9,en;q=0.8"
    x2["Accept-Encoding"] = "gzip, deflate, br"
    results.append(run_case("X2 完整头 + Accept-Language/Encoding", TARGET_URL, x2, API2))

    x3 = dict(base)
    x3["Connection"] = "keep-alive"
    results.append(run_case("X3 完整头 + Connection: keep-alive", TARGET_URL, x3, API2))

    print("\n" + "=" * 72)
    print("补充判别汇总")
    print("=" * 72)
    for r in results:
        print(f"  {r['name']:36s} HTTP {r['status']} → {r['verdict']}")
        print(f"      {r['body'][:160]}")
    return 0 if any(r["verdict"] == "DATA" for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
