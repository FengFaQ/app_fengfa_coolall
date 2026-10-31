#!/usr/bin/env python3
"""
TLS 指纹路线 C：用 curl_cffi 模拟浏览器指纹实测。

目的：确认 1004 是否与 TLS 指纹有关。
- 若模拟浏览器指纹能拿到 data → 服务端不严格校验，方案多
- 若仍 1004 → 需要精确模拟 OkHttp

curl_cffi 的 impersonate 支持 chrome / safari / firefox 等，
不支持直接模拟 OkHttp，但能验证「指纹是否重要」这个前提。
"""

import base64
import importlib.util
import sys

from curl_cffi import requests

# 加载 TokenV3 实现
spec = importlib.util.spec_from_file_location(
    "tok", "/mnt/c/Users/Administrator/Documents/dsh/Kuan/tools/verify_token_v3.py"
)
tok = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tok)

# 真机数据（从手机 c001apk datastore 导出）
DEVICE_PLAIN = "; ; ; ; meizu; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; null"
UA = (
    "Dalvik/2.1.0 (Linux; U; Android 16; MEIZU 21 BQ2A.251110.001-BP2A.250605.031.A3) "
    "(#Build; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; 16) "
    "+CoolMarket/16.7.0-2610291-universal"
)
SDK = "36"
INSTALL_TIME = "1761926400000"

URL = (
    "https://api2.coolapk.com/v6/main/indexV8"
    f"?page=1&firstLaunch=1&installTime={INSTALL_TIME}"
)


def encode_device(plain: str) -> str:
    b = base64.b64encode(plain.encode()).decode().rstrip("=")
    return b[::-1].replace("\r", "").replace("\n", "").replace("=", "")


def build_headers() -> dict:
    dev = encode_device(DEVICE_PLAIN)
    token = tok.create(dev, 2610291)
    return {
        "User-Agent": UA,
        "X-Requested-With": "XMLHttpRequest",
        "X-Sdk-Int": SDK,
        "X-Sdk-Locale": "zh-CN",
        "X-App-Id": "com.coolapk.market",
        "X-App-Token": token,
        "X-App-Version": "16.7.0",
        "X-App-Code": "2610291",
        "X-Api-Version": "16",
        "X-App-Device": dev,
        "X-Dark-Mode": "0",
        "X-App-Channel": "coolapk",
        "X-App-Mode": "universal",
        "X-App-Supported": "2610291",
        "Accept": "application/json",
    }


def probe(label: str, impersonate, headers: dict):
    try:
        r = requests.get(URL, headers=headers, impersonate=impersonate, timeout=20)
        body = r.text
        if '"data"' in body:
            tag = f"✅ 成功！长度 {len(body)}"
        elif "1004" in body:
            tag = "1004 请求验证错误"
        elif "1000" in body:
            tag = "1000 请求方式错误"
        elif not body:
            tag = "空响应"
        else:
            tag = body[:100]
        print(f"[{label:<24}] HTTP {r.status_code} -> {tag}")
    except Exception as e:
        print(f"[{label:<24}] 失败: {type(e).__name__}: {str(e)[:100]}")


def main():
    print("=" * 70)
    print("curl_cffi 模拟不同浏览器指纹")
    print("=" * 70)

    headers = build_headers()
    print(f"设备串: {headers['X-App-Device'][:50]}...")
    print(f"Token : {headers['X-App-Token'][:50]}...")
    print()

    # curl_cffi 支持的 impersonate 目标
    targets = [
        "chrome110",
        "chrome120",
        "chrome124",
        "chrome131",
        "safari15_5",
        "safari17_0",
        "firefox133",
    ]

    for t in targets:
        probe(t, t, headers)

    print()
    print("--- 对照：不指定指纹（curl 默认）---")
    probe("默认（无模拟）", None, headers)


if __name__ == "__main__":
    main()
