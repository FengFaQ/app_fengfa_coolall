#!/usr/bin/env python3
"""
精确模拟 OkHttp 的 TLS 指纹。

真机抓包确认 App (OkHttp) 与我方差异：

App:
  套件 15 个: 1301 1302 1303 c02b c02c cca9 c02f c030 cca8 c013 c014 009c 009d 002f 0035
  扩展顺序:   0, 23, 65281, 10, 11, 35, 16, 5, 13, 51, 45, 43, 21

我方 curl (BoringSSL 默认):
  套件 17 个: 1301 1302 1303 c02b c02f c02c c030 cca9 cca8 c009 c013 c00a c014 009c 009d 002f 0035
  扩展顺序:   0, 23, 65281, 10, 11, 16, 13, 51, 45, 43, 21

目标：让 curl_cffi 发出与 App 一致的指纹。
"""

import base64
import importlib.util

from curl_cffi import requests
from curl_cffi.requests.impersonate import ExtraFingerprints

spec = importlib.util.spec_from_file_location(
    "tok", "/mnt/c/Users/Administrator/Documents/dsh/Kuan/tools/verify_token_v3.py"
)
tok = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tok)

DEVICE_PLAIN = "; ; ; ; meizu; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; null"
UA = (
    "Dalvik/2.1.0 (Linux; U; Android 16; MEIZU 21 BQ2A.251110.001-BP2A.250605.031.A3) "
    "(#Build; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; 16) "
    "+CoolMarket/16.7.0-2610291-universal"
)

URL = "https://api2.coolapk.com/v6/main/indexV8?page=1&firstLaunch=1&installTime=1761926400000"

# App 的套件列表（按顺序）
APP_CIPHERS = [
    "TLS_AES_128_GCM_SHA256",
    "TLS_AES_256_GCM_SHA384",
    "TLS_CHACHA20_POLY1305_SHA256",
    "ECDHE-ECDSA-AES128-GCM-SHA256",
    "ECDHE-RSA-AES128-GCM-SHA256",
    "ECDHE-ECDSA-CHACHA20-POLY1305",
    "ECDHE-RSA-CHACHA20-POLY1305",
    "ECDHE-ECDSA-AES256-GCM-SHA384",
    "ECDHE-RSA-AES256-GCM-SHA384",
    "ECDHE-ECDSA-AES128-SHA",
    "ECDHE-RSA-AES128-SHA",
    "AES128-GCM-SHA256",
    "AES256-GCM-SHA384",
    "AES128-SHA",
    "AES256-SHA",
]


def encode_device(plain: str) -> str:
    b = base64.b64encode(plain.encode()).decode().rstrip("=")
    return b[::-1].replace("\r", "").replace("\n", "").replace("=", "")


def build_headers() -> dict:
    dev = encode_device(DEVICE_PLAIN)
    return {
        "User-Agent": UA,
        "X-Requested-With": "XMLHttpRequest",
        "X-Sdk-Int": "36",
        "X-Sdk-Locale": "zh-CN",
        "X-App-Id": "com.coolapk.market",
        "X-App-Token": tok.create(dev, 2610291),
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


def probe(label, impersonate, headers, extra_fp=None):
    try:
        kwargs = {"headers": headers, "timeout": 20}
        if impersonate:
            kwargs["impersonate"] = impersonate
        if extra_fp:
            kwargs["extra_fp"] = extra_fp
        r = requests.get(URL, **kwargs)
        body = r.text
        if '"data"' in body:
            tag = f"✅✅ 成功！长度 {len(body)}"
        elif "1004" in body:
            tag = "1004"
        elif not body:
            tag = "空"
        else:
            tag = body[:80]
        print(f"[{label:<32}] {r.status_code} -> {tag}")
        return '"data"' in body
    except Exception as e:
        print(f"[{label:<32}] 失败: {type(e).__name__}: {str(e)[:80]}")
        return False


def main():
    print("=" * 72)
    print("精确模拟 OkHttp TLS 指纹")
    print("=" * 72)
    h = build_headers()

    # 1. 基线：chrome110
    probe("chrome110（基线）", "chrome110", h)

    # 2. 自定义套件列表
    try:
        fp = ExtraFingerprints(tls_ciphers=APP_CIPHERS)
        probe("自定义套件列表", "chrome110", h, fp)
    except Exception as e:
        print(f"ExtraFingerprints 构造失败: {e}")

    # 3. 看看 ExtraFingerprints 支持哪些字段
    print()
    print("=== ExtraFingerprints 支持的字段 ===")
    import inspect
    sig = inspect.signature(ExtraFingerprints.__init__)
    for name in sig.parameters:
        if name != "self":
            print(f"  {name}")


if __name__ == "__main__":
    main()
