#!/usr/bin/env python3
"""
关键验证 2：用真实的 x25519 密钥对，看握手能否推进。

上一轮用随机字节当 key_share，服务器回 Alert 0x32 (illegal_parameter)。
现在改用真实的 x25519 公钥，看是否能收到 ServerHello。

如果能收到 ServerHello，说明「手工构造 ClientHello」这条路可行，
剩下的就是实现完整握手状态机。
"""

import os
import socket
import struct

# 尝试用 cryptography 库（如果有）
try:
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

    HAS_CRYPTO = True
except ImportError:
    HAS_CRYPTO = False


def gen_x25519():
    """生成 x25519 密钥对，返回 (私钥, 公钥字节)。"""
    if HAS_CRYPTO:
        priv = X25519PrivateKey.generate()
        pub = priv.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        return priv, pub
    # 回退：用固定的测试公钥（RFC 7748 的测试向量基点的公钥）
    # 这是合法的 x25519 公钥，只是私钥未知
    pub = bytes.fromhex(
        "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
    )
    return None, pub


def build_client_hello(sni: str, pub: bytes) -> bytes:
    suites = [
        0x1301, 0x1302, 0x1303,
        0xC02B, 0xC02C, 0xCCA9, 0xC02F,
        0xC030, 0xCCA8, 0xC013, 0xC014,
        0x009C, 0x009D, 0x002F, 0x0035,
    ]
    random_bytes = os.urandom(32)
    session_id = os.urandom(32)

    exts = b""

    # 0: server_name
    sni_bytes = sni.encode()
    exts += struct.pack(">HHH", 0x0000, len(sni_bytes) + 5, len(sni_bytes) + 3)
    exts += struct.pack(">BH", 0, len(sni_bytes)) + sni_bytes

    # 23: extended_master_secret
    exts += struct.pack(">HH", 23, 0)

    # 65281: renegotiation_info
    exts += struct.pack(">HHB", 0xFF01, 1, 0)

    # 10: supported_groups
    groups = [0x001D, 0x0017, 0x0018]
    g = struct.pack(">H", len(groups) * 2) + b"".join(struct.pack(">H", x) for x in groups)
    exts += struct.pack(">HH", 10, len(g)) + g

    # 11: ec_point_formats
    exts += struct.pack(">HHBB", 11, 2, 1, 0)

    # 35: session_ticket
    exts += struct.pack(">HH", 35, 0)

    # 16: ALPN
    alpns = [b"h2", b"http/1.1"]
    al = b"".join(struct.pack(">B", len(a)) + a for a in alpns)
    exts += struct.pack(">HH", 16, len(al) + 2) + struct.pack(">H", len(al)) + al

    # 5: status_request
    sr = struct.pack(">BHB", 1, 0, 0) + struct.pack(">H", 0)
    exts += struct.pack(">HH", 5, len(sr)) + sr

    # 13: signature_algorithms
    sigalgs = [0x0403, 0x0804, 0x0401, 0x0503, 0x0805, 0x0501, 0x0806, 0x0601]
    sa = struct.pack(">H", len(sigalgs) * 2) + b"".join(struct.pack(">H", s) for s in sigalgs)
    exts += struct.pack(">HH", 13, len(sa)) + sa

    # 51: key_share —— 用真实公钥
    ks_entry = struct.pack(">H", 0x001D) + struct.pack(">H", len(pub)) + pub
    ks = struct.pack(">H", len(ks_entry)) + ks_entry
    exts += struct.pack(">HH", 51, len(ks)) + ks

    # 45: psk_key_exchange_modes
    exts += struct.pack(">HHBB", 45, 2, 1, 1)

    # 43: supported_versions
    sv = struct.pack(">B", 4) + struct.pack(">HH", 0x0304, 0x0303)
    exts += struct.pack(">HH", 43, len(sv)) + sv

    # 21: padding 补齐到 512
    body_wo_pad = (
        struct.pack(">H", 0x0303) + random_bytes
        + struct.pack(">B", len(session_id)) + session_id
        + struct.pack(">H", len(suites) * 2)
        + b"".join(struct.pack(">H", s) for s in suites)
        + struct.pack(">BB", 1, 0)
        + struct.pack(">H", len(exts)) + exts
    )
    total = 4 + len(body_wo_pad)
    pad_needed = 512 - total - 4
    if pad_needed > 0:
        exts += struct.pack(">HH", 21, pad_needed) + b"\x00" * pad_needed

    body = (
        struct.pack(">H", 0x0303) + random_bytes
        + struct.pack(">B", len(session_id)) + session_id
        + struct.pack(">H", len(suites) * 2)
        + b"".join(struct.pack(">H", s) for s in suites)
        + struct.pack(">BB", 1, 0)
        + struct.pack(">H", len(exts)) + exts
    )
    hs = struct.pack(">B", 1) + struct.pack(">I", len(body))[1:] + body
    return struct.pack(">BH", 0x16, 0x0301) + struct.pack(">H", len(hs)) + hs


def check_hello_response(data: bytes) -> str:
    """解析服务器响应的类型。"""
    if len(data) < 6:
        return f"响应太短: {data.hex()}"

    rectype = data[0]
    if rectype == 0x16:  # Handshake
        hs_type = data[5]
        if hs_type == 2:
            return "✅ ServerHello（握手可以继续！）"
        if hs_type == 11:
            return "Certificate"
        return f"✅ Handshake type={hs_type}"
    if rectype == 0x15:  # Alert
        lvl = data[5]
        desc = data[6] if len(data) > 6 else 0
        names = {
            0x28: "handshake_failure",
            0x2F: "insufficient_security",
            0x32: "illegal_parameter",
            0x33: "unknown_ca",
            0x47: "protocol_version",
        }
        return f"❌ Alert level={lvl} desc=0x{desc:02x} ({names.get(desc, 'unknown')})"
    if rectype == 0x17:  # Application Data
        return "Application Data（会话已建立）"
    return f"未知记录类型 0x{rectype:02x}"


def main():
    print("=" * 70)
    print("用真实 x25519 公钥测试握手")
    print("=" * 70)

    if not HAS_CRYPTO:
        print("⚠️  没有 cryptography 库，使用固定测试公钥")
        print("   安装：pip install cryptography --break-system-packages")
    print()

    _, pub = gen_x25519()
    print(f"x25519 公钥: {pub.hex()}")
    print()

    hello = build_client_hello("api2.coolapk.com", pub)
    print(f"ClientHello 长度: {len(hello)} 字节")
    print()

    for ip in ["119.188.175.46", "221.204.163.76"]:
        print(f"--- 连接 {ip}:443 ---")
        try:
            sock = socket.create_connection((ip, 443), timeout=15)
            sock.send(hello)
            data = sock.recv(8192)
            print(f"  {check_hello_response(data)}")
            print(f"  原始: {data[:20].hex()}")
            sock.close()
        except Exception as e:
            print(f"  失败: {type(e).__name__}: {e}")
        print()


if __name__ == "__main__":
    main()
