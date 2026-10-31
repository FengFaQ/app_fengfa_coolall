#!/usr/bin/env python3
"""
关键对照：用 curl_cffi 精确抓取我方 ClientHello，与真机比对。

目的：确认 curl_cffi 实际发出的指纹是什么，
以及它与 OkHttp 差在哪。
"""

import base64

from curl_cffi import requests

DEVICE_PLAIN = "; ; ; ; meizu; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; null"


def encode_device(plain: str) -> str:
    b = base64.b64encode(plain.encode()).decode().rstrip("=")
    return b[::-1].replace("\r", "").replace("\n", "").replace("=", "")


def main():
    print("设备串:", encode_device(DEVICE_PLAIN)[:60], "...")
    print()

    # 用 curl_cffi 的 verbose 看 TLS 细节
    print("=" * 70)
    print("测试 1：chrome110 指纹的 TLS 信息")
    print("=" * 70)
    try:
        r = requests.get(
            "https://api2.coolapk.com/v6/main/indexV8?page=1",
            impersonate="chrome110",
            timeout=20,
            verbose=True,
        )
        print(f"HTTP {r.status_code}")
        print(r.text[:150])
    except Exception as e:
        print(f"失败: {e}")


if __name__ == "__main__":
    main()
