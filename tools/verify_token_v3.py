#!/usr/bin/env python3
"""
酷安 TokenV3 签名算法 —— 独立实现与自校验

用途：
  在**不依赖任何第三方客户端源码**的前提下，从零实现酷安的请求签名算法，
  并用纯 Python（含 bcrypt）算出结果，与 Kotlin/Rust 实现做交叉比对。

背景：
  参考项目 KiritoXDone/c001apk 的 TokenV3.kt 与 daimiaopeng/coolapk-desktop
  实现了同一套算法。本脚本只把它当作**协议契约的参考**，算法本体独立重写，
  一旦发现与参考实现有偏差，以本脚本的验证结果为准。

算法结构（共 6 步）：
  1. 用固定查找表 lookup（930 字节），按 (时间戳 + appCode) % 100 取出
     起始位置 index*4+0x80 处的 0x80 字节片段，base64 解码成 segment
  2. 拼 plain = "com.coolapk.market&{segment}&{md5(device)}&{time}&{appCode}"
  3. password = md5(base64(plain))            —— 作为 bcrypt 的输入密码
  4. source   = base64("{time:x}/{md5(plain)}")[:22]
  5. salt     = source[:21] + 第 22 位字母 -5
  6. hash     = bcrypt(password, "$2a$04$" + salt)，前缀改成 $2y
  最终 token = "v3" + base64(hash)

注意：
  第 1 步的切片作用于**字节数组**，切片结果本身还要再做一次 base64 解码，
  得到 96 个字符的 segment 字符串。这两层解码极易搞混。
"""

import base64
import hashlib
import re
import sys
import time

import bcrypt

# 查找表（算法常量，与业务逻辑无关）。共 930 字节。
LOOKUP_B64 = (
    "VFRCVU9GUXNRMEVsTFVNa1dERWpRU0VzVXlFbUxETkFVeTFEUEZZdUl5MGlN"
    "VU01SXpCVUpGY3RNeWtoTVVRMUpTMURPU1V4TXpoUUxWTXdWMDBzSXloU01U"
    "UXNXVEZEUkZFd1EwUlZNU1FrVVMwaktTTXRKRGhZTFZRa1VqRXpLU1FzTTBC"
    "VkxUUXdVaTAwTlNNc00yQlhMVk1sSkMxRE5GbE5MQ1EwV1RFekxTUXhRMEVo"
    "TEVNbElqRXpLRmN3TXowbExpTkVWekVqUFNZdU5Ea2tMRk13Vml4VEtGa3RR"
    "emhYTEZNc1dDNGpMU1l0VTJCVFRUQTBMRkV1TXpoVExqTWhJakZETUZNc0kw"
    "QlZMQ1FrVUN3ekpTSXNJekJTTUVNOFVDeEVMU0l0STJCUUxVUW9VeTBqTEZN"
    "c0l5VWtNRlF3VmswdFJDeFlNVE1sSkM0ME5TRXdVMFJRTEZNaElpMGtOU013"
    "VTJCUUxqTmdVeXdqT0ZFc05DeFRMRU13V0N4VUxTRXRJeVJUTEROQkl5MDBK"
    "RlpOTVVNc1VEQkVKRmd0TkMwa0xEUWtXQzFETUZNeFJDa2pMVk0wVml4VE1G"
    "Z3hJeTBtTFZNeEpTMHpKRlF4TTJCUUxTUTRXVEJETUZNdUkwVWhUU3d6UEZn"
    "dFJEQlJNRE5FVVMwak9GWXVKRGtsTVNRMFV6QXpOU013UkRra01UTWtWU3hV"
    "TUZRdFJDeFlNVE5GSXl4RUxGWXNJelVoTGlRMFYwMHhOQ3hWTENNMFZ5NHpK"
    "RlV4TkN4VExGUTRWeTRqWUZZeEpEVWpMVU13VXpGRFBGSXNNMEVsTERNc1ZE"
    "QTBPU011TkNrbUxFUWtWaXhFTVNKTkxUTW9VaXd6TEZjdFUwUlFMaVF3VlRG"
    "REpTVXdVeWtqTVVNaEl5MHpJU1VzSTBFak1UTXdVUzFVTkZjeFJERWxMVVF0"
    "SlM0a05GSXNJemhaVFRCVFlGRXdReVJZTFNNbElTNDBMRmdzVkRSWkxVUW9X"
    "VEJEUUZZdFJDeFdMak5FVkRCRE1Ga3RORFVsTEZRNFZTMVVKRmd3TXpSV0xE"
    "UTRVVTB3UXkwbUxVUTRXUzFUTVNNc1V5VWlNVU5BVVN4VUxGWXVJMFJVTGlN"
    "c1dERkVMRk14STJCUUxWTTBXUzRrT0Zjd00wUlhMVk1vV0MwaktGRXhMQ00w"
    "Vml4VE5GQXRSRFJXTFVRbEpDMVRKRkF3TkN4Z1lB"
)
LOOKUP = base64.b64decode(LOOKUP_B64)

ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"

# bcrypt 接受的盐字符集：比 ALPHABET 少了 '+'，这正是重试逻辑存在的原因之一
BCRYPT_SALT_RE = re.compile(r"[./A-Za-z0-9]{22}")

# bcrypt 专用字母表，与标准 Base64 不同（以 './' 开头）
BCRYPT_ALPHABET = b"./ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"

APP_ID = "com.coolapk.market"
DEFAULT_APP_CODE = 2604201


def md5_hex(value: str) -> str:
    return hashlib.md5(value.encode("utf-8")).hexdigest()


def b64_nopad(value: str) -> str:
    return base64.b64encode(value.encode("utf-8")).decode("ascii").rstrip("=")


def segment_for(remainder: int) -> str:
    """取出第 remainder 段（0..99）的 segment 字符串。"""
    index = remainder * 4 + 0x80
    chunk = LOOKUP[index : index + 0x80]
    if len(chunk) < 0x80:
        raise RuntimeError(f"查找表切片越界：余数 {remainder} 需要 {index + 0x80} 字节，实际仅 {len(LOOKUP)}")
    return base64.b64decode(chunk).decode("utf-8")


def salt_is_valid(salt: str) -> bool:
    """
    判断 22 位盐是否合法。

    正则只能过滤 '$' 等非法字符；还有一条更隐蔽的硬性规则：
    22 个字符 × 6 位 = 132 位，而 16 字节只要 128 位，
    末尾多出的 4 位必须为 0，否则无法还原成合法 16 字节盐。

    早期实现靠捕获 bcrypt 的 "Invalid salt" 异常来跳过，
    那是结果而非规则。这里显式检查，行为才与 Rust 实现一致。
    """
    if not BCRYPT_SALT_RE.fullmatch(salt):
        return False
    bits = 0
    for ch in salt.encode("ascii"):
        bits = (bits << 6) | BCRYPT_ALPHABET.index(ch)
    # 132 位中最后 4 位必须为 0
    return (bits & 0b1111) == 0


def create(device: str, app_code: int = DEFAULT_APP_CODE, timestamp: int | None = None) -> str:
    """
    生成签名 token。device 是设备码（32 位十六进制）。

    盐有两个可能被拒的原因：
    1. 末位落到 '+' —— 不在 bcrypt 盐字符集
    2. 末尾 4 位残余不为 0 —— 无法还原成 16 字节
    两种情况都必须跳过并换下一个时间戳重试，这就是外层 16 次循环的意义。
    """
    if timestamp is None:
        timestamp = int(time.time())

    for offset in range(16):
        t = timestamp + offset
        segment = segment_for((t + app_code) % 100)

        plain = f"{APP_ID}&{segment}&{md5_hex(device)}&{t}&{app_code}"
        # 密码输入保留 Base64 padding；盐来源串才去掉 padding。
        password = md5_hex(base64.b64encode(plain.encode("utf-8")).decode("ascii"))
        source = b64_nopad(f"{t:x}/{md5_hex(plain)}")[:22]

        pos = ALPHABET.index(source[21])
        salt = source[:21] + ALPHABET[(pos - 5) % 64]

        if not salt_is_valid(salt):
            continue

        hashed = bcrypt.hashpw(password.encode("utf-8"), f"$2a$04${salt}".encode("ascii"))
        hashed = b"$2y" + hashed[3:]
        return "v3" + base64.b64encode(hashed).decode("ascii")

    raise RuntimeError("16 次尝试均失败，请检查系统时间")


def verify_shape(token: str) -> list[tuple[str, bool, str]]:
    """校验 token 的结构，不依赖网络。"""
    checks: list[tuple[str, bool, str]] = []

    def check(name: str, ok: bool, detail: str = "") -> None:
        checks.append((name, ok, detail))

    check("以 v3 开头", token.startswith("v3"), token[:2])
    body = token[2:]
    check("主体是合法 base64", bool(re.fullmatch(r"[A-Za-z0-9+/]+=*", body)), "")

    raw = base64.b64decode(body)
    check("解码后长度 60", len(raw) == 60, str(len(raw)))
    check("是 $2y 变体", raw.startswith(b"$2y$"), raw[:4].decode("ascii", "replace"))

    m = re.match(rb"^\$2y\$(\d{2})\$(.{22})(.{31})$", raw, re.S)
    check("bcrypt 结构完整", m is not None, "")
    if m:
        cost = int(m.group(1))
        check("cost = 4", cost == 4, str(cost))
        salt = m.group(2).decode("ascii")
        check("盐表字符集合法（不含 +）", bool(BCRYPT_SALT_RE.fullmatch(salt)), salt)
        check("盐表尾位在字母表内", salt[21] in ALPHABET, salt[21])

    return checks


def main() -> int:
    device = sys.argv[1] if len(sys.argv) > 1 else "00000000000000000000000000000000"

    print("=" * 64)
    print("酷安 TokenV3 签名算法 —— 独立实现自校验")
    print("=" * 64)
    print(f"设备码  : {device}")
    print(f"查找表  : {len(LOOKUP)} 字节")

    if len(LOOKUP) != 930:
        print(f"❌ 查找表长度异常，期望 930，实际 {len(LOOKUP)}")
        return 1
    print("✅ 查找表长度正确")

    fixed_ts = 1735689600

    # 1. 查表覆盖
    print("\n[1] 查表覆盖：100 个余数全部可解码")
    bad = []
    for r in range(100):
        try:
            s = segment_for(r)
            if len(s) != 96:
                bad.append((r, f"长度 {len(s)}"))
        except Exception as e:
            bad.append((r, str(e)))
    print(f"  失败项: {len(bad)}  {'✅ 通过' if not bad else '❌ ' + str(bad[:3])}")

    # 2. 确定性
    t1 = create(device, timestamp=fixed_ts)
    t2 = create(device, timestamp=fixed_ts)
    print("\n[2] 确定性：同一输入复现同一签名")
    print(f"  签名: {t1}")
    print(f"  {'✅ 通过' if t1 == t2 else '❌ 失败'}")

    # 3. 结构
    print("\n[3] 结构校验")
    checks = verify_shape(t1)
    for name, ok, detail in checks:
        print(f"  {'✅' if ok else '❌'} {name}" + (f"  ({detail})" if detail else ""))

    # 4. 时间敏感
    t3 = create(device, timestamp=fixed_ts + 1)
    print("\n[4] 时间敏感：时间戳 +1 签名变化")
    print(f"  {'✅ 通过' if t3 != t1 else '❌ 失败'}")

    # 5. 设备敏感
    print("\n[5] 设备敏感：换设备码签名变化")
    try:
        t4 = create("ffffffffffffffffffffffffffffffff", timestamp=fixed_ts)
        dev_ok = t4 != t1
        print(f"  {'✅ 通过' if dev_ok else '❌ 失败'}")
    except RuntimeError as e:
        dev_ok = False
        print(f"  ❌ 失败：{e}")

    # 6. 盐表重试路径：确认会跳过含 '+' 的时间戳而不是崩溃
    print("\n[6] 重试路径：含 '+' 的盐被正确跳过")
    skipped = 0
    for off in range(16):
        t = fixed_ts + off
        seg = segment_for((t + DEFAULT_APP_CODE) % 100)
        plain = f"{APP_ID}&{seg}&{md5_hex(device)}&{t}&{DEFAULT_APP_CODE}"
        src = b64_nopad(f"{t:x}/{md5_hex(plain)}")[:22]
        salt = src[:21] + ALPHABET[(ALPHABET.index(src[21]) - 5) % 64]
        if not BCRYPT_SALT_RE.fullmatch(salt):
            skipped += 1
    retry_ok = skipped > 0
    print(f"  被跳过的时间戳数: {skipped}/16  {'✅ 重试机制生效' if retry_ok else '⚠ 本轮未触发'}")

    # 7. 实时签名
    now_token = create(device)
    print("\n[7] 实时签名（当前时间）")
    print(f"  {now_token}")
    now_shape = verify_shape(now_token)
    print(f"  {'✅ 通过' if all(ok for _, ok, _ in now_shape) else '❌ 失败'}")

    all_ok = (
        not bad
        and t1 == t2
        and all(ok for _, ok, _ in checks)
        and t3 != t1
        and dev_ok
        and retry_ok
        and all(ok for _, ok, _ in now_shape)
    )
    print("\n" + "=" * 64)
    print(f"总体结果: {'✅ 全部通过' if all_ok else '❌ 存在失败项'}")
    print("=" * 64)
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
