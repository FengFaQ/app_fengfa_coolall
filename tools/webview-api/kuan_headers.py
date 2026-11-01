#!/usr/bin/env python3
"""
酷安请求头构造 —— 路线 B（WebView / 浏览器指纹）专用。

不依赖主项目 Rust 源码，按 docs/PROTOCOL.md 的契约独立复刻：
  - 设备串：Base64 无 padding → 整体反转 → 去 '='
  - Token ：复用 Kuan/tools/verify_token_v3.py 的 create()

真机数据取自上游 c001apk 抓包（MEIZU 21 / Android 16 / SDK 36），
与 crates/coolall-core/src/device.rs 里的 DeviceProfile 默认值不同——
那边是 Xiaomi 2211133C / Android 13 / SDK 33 的旧画像。
"""

import base64
import sys
from pathlib import Path

# 允许以脚本方式直接运行：把 tools/ 挂进 sys.path 以便 import verify_token_v3
_TOOLS = Path(__file__).resolve().parent.parent
if str(_TOOLS) not in sys.path:
    sys.path.insert(0, str(_TOOLS))

import verify_token_v3  # noqa: E402

# ---- 协议常量（对齐 crates/coolall-core/src/device.rs::constants）----
APP_ID = "com.coolapk.market"
CHANNEL = "coolapk"
MODE = "universal"
LOCALE = "zh-CN"
REQUEST_WITH = "XMLHttpRequest"
VERSION_NAME = "16.7.0"
VERSION_CODE = "2610291"
API_VERSION = "16"
APP_CODE = 2_610_291

# ---- 真机画像（上游 c001apk 抓包）----
SZL_MID = "DUP4awltXU__0ABOiBz7NwkxTrv4hbvIeRgd"
MANUFACTURER = "meizu"
BRAND = "meizu"
MODEL = "MEIZU 21"
BUILD = "BQ2A.251110.001-BP2A.250605.031.A3"
ANDROID_VERSION = "16"
SDK_INT = "36"

USER_AGENT = (
    "Dalvik/2.1.0 (Linux; U; Android 16; MEIZU 21 BQ2A.251110.001-BP2A.250605.031.A3) "
    "(#Build; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; 16) "
    "+CoolMarket/16.7.0-2610291-universal"
)

TARGET_URL = (
    "https://api2.coolapk.com/v6/main/indexV8"
    "?page=1&firstLaunch=1&installTime=1761926400000"
)


def device_plaintext() -> str:
    """9 段设备串明文。"""
    return f"{SZL_MID}; ; ; ; {MANUFACTURER}; {BRAND}; {MODEL}; {BUILD}; null"


def device_header() -> str:
    """X-App-Device：Base64 无填充 → 整体反转 → 去 '='。"""
    b64 = base64.b64encode(device_plaintext().encode("utf-8")).decode("ascii")
    trimmed = b64.rstrip("=")
    return trimmed[::-1].replace("=", "")


def build_headers(token: str | None = None) -> dict[str, str]:
    """构造完整请求头。token 为 None 时现算一个。"""
    if token is None:
        token = verify_token_v3.create(device_header(), APP_CODE)

    return {
        "User-Agent": USER_AGENT,
        "X-Requested-With": REQUEST_WITH,
        "X-Sdk-Int": SDK_INT,
        "X-Sdk-Locale": LOCALE,
        "X-App-Id": APP_ID,
        "X-App-Token": token,
        "X-App-Version": VERSION_NAME,
        "X-App-Code": VERSION_CODE,
        "X-Api-Version": API_VERSION,
        "X-App-Device": device_header(),
        "X-Dark-Mode": "0",
        "X-App-Channel": CHANNEL,
        "X-App-Mode": MODE,
        "X-App-Supported": VERSION_CODE,
        "Accept": "application/json",
    }


if __name__ == "__main__":
    h = build_headers()
    plain = device_plaintext()
    print(f"设备串明文 : {plain}")
    print(f"段数       : {len(plain.split(';'))}")
    print(f"X-App-Device: {h['X-App-Device']}")
    print(f"X-App-Token : {h['X-App-Token']}")
    print("请求头:")
    for k, v in h.items():
        shown = v if len(v) <= 70 else v[:70] + "..."
        print(f"  {k:20s} {shown}")
