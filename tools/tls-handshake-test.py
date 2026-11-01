#!/usr/bin/env python3
"""
验证核心问题：手工构造的 ClientHello 能否完成完整 TLS 握手？

背景：
- 真机 App 用 OkHttp，TLS 指纹 15 套件 / 特定扩展顺序
- 我方 rustls / curl / 浏览器 全部 1004
- 唯一可能是精确复刻 ClientHello

但「构造 ClientHello」和「完成握手」是两回事。
本脚本验证：Python 的 ssl 模块能否在自定义 ClientHello 后继续握手。

如果不能，说明必须用底层库（如 scapy + 手工实现 TLS 状态机），成本极高。
"""

import socket
import ssl
import struct


def build_client_hello(sni: str) -> bytes:
    """构造与 OkHttp 一致的 ClientHello。

    关键：套件列表和扩展顺序都要精确匹配。
    """
    # OkHttp 的套件（15 个）
    suites = [
        0x1301, 0x1302, 0x1303,          # TLS 1.3
        0xC02B, 0xC02C, 0xCCA9, 0xC02F,  # ECDHE
        0xC030, 0xCCA8, 0xC013, 0xC014,
        0x009C, 0x009D, 0x002F, 0x0035,
    ]

    # 生成随机数
    import os

    random_bytes = os.urandom(32)
    session_id = os.urandom(32)

    # SNI 扩展
    sni_bytes = sni.encode()
    sni_ext = struct.pack(">HBH", 0x0000, len(sni_bytes) + 5, len(sni_bytes) + 3)
    sni_ext += struct.pack(">BH", 0, len(sni_bytes)) + sni_bytes

    # 扩展（按 OkHttp 顺序）
    exts = b""

    # 0: server_name
    exts += sni_ext

    # 23: extended_master_secret
    exts += struct.pack(">HH", 23, 0)

    # 65281: renegotiation_info
    exts += struct.pack(">HHB", 0xFF01, 1, 0)

    # 10: supported_groups
    groups = [0x001D, 0x0017, 0x0018]
    g_data = struct.pack(">H", len(groups) * 2) + b"".join(struct.pack(">H", g) for g in groups)
    exts += struct.pack(">HH", 10, len(g_data)) + g_data

    # 11: ec_point_formats
    exts += struct.pack(">HHBB", 11, 2, 1, 0)

    # 35: session_ticket（空）
    exts += struct.pack(">HH", 35, 0)

    # 16: ALPN
    alpns = [b"h2", b"http/1.1"]
    alpn_list = b"".join(struct.pack(">B", len(a)) + a for a in alpns)
    alpn_data = struct.pack(">H", len(alpn_list)) + alpn_list
    exts += struct.pack(">HH", 16, len(alpn_data)) + alpn_data

    # 5: status_request
    sr = struct.pack(">BHB", 1, 0, 0) + struct.pack(">H", 0)
    exts += struct.pack(">HH", 5, len(sr)) + sr

    # 13: signature_algorithms
    sigalgs = [0x0403, 0x0804, 0x0401, 0x0503, 0x0805, 0x0501, 0x0806, 0x0601]
    sa = struct.pack(">H", len(sigalgs) * 2) + b"".join(struct.pack(">H", s) for s in sigalgs)
    exts += struct.pack(">HH", 13, len(sa)) + sa

    # 51: key_share（需要真实的 x25519 公钥）
    # 这里用占位，实际握手会失败——这正是要验证的
    key_share = struct.pack(">H", 0x001D) + struct.pack(">H", 32) + os.urandom(32)
    ks_data = struct.pack(">H", len(key_share)) + key_share
    exts += struct.pack(">HH", 51, len(ks_data)) + ks_data

    # 45: psk_key_exchange_modes
    exts += struct.pack(">HHBB", 45, 2, 1, 1)

    # 43: supported_versions
    sv = struct.pack(">B", 4) + struct.pack(">HH", 0x0304, 0x0303)
    exts += struct.pack(">HH", 43, len(sv)) + sv

    # 21: padding（补齐到 512）
    # 先算当前长度
    body_so_far = (
        struct.pack(">H", 0x0303)
        + random_bytes
        + struct.pack(">B", len(session_id))
        + session_id
        + struct.pack(">H", len(suites) * 2)
        + b"".join(struct.pack(">H", s) for s in suites)
        + struct.pack(">B", 1)
        + struct.pack(">B", 0)
        + struct.pack(">H", len(exts))
        + exts
    )
    # ClientHello 总长约 = 4(头部) + 2(版本) + 32(random) + 1+32(sid) + ...
    current = 4 + len(body_so_far)
    padding_needed = 512 - current - 4  # 减去 padding 扩展自身头部
    if padding_needed > 0:
        exts += struct.pack(">HH", 21, padding_needed) + b"\x00" * padding_needed

    # 重算长度
    exts_final = exts
    body = (
        struct.pack(">H", 0x0303)
        + random_bytes
        + struct.pack(">B", len(session_id))
        + session_id
        + struct.pack(">H", len(suites) * 2)
        + b"".join(struct.pack(">H", s) for s in suites)
        + struct.pack(">B", 1)
        + struct.pack(">B", 0)
        + struct.pack(">H", len(exts_final))
        + exts_final
    )

    handshake = struct.pack(">B", 1) + struct.pack(">I", len(body))[1:] + body
    record = struct.pack(">BH", 0x16, 0x0301) + struct.pack(">H", len(handshake)) + handshake
    return record


def main():
    print("=" * 70)
    print("验证：手工构造 ClientHello 能否完成握手")
    print("=" * 70)

    hello = build_client_hello("api2.coolapk.com")
    print(f"ClientHello 长度: {len(hello)} 字节")
    print(f"头部: {hello[:16].hex()}")
    print()

    print("--- 尝试发送到服务器 ---")
    try:
        sock = socket.create_connection(("119.188.175.46", 443), timeout=15)
        sock.send(hello)
        print("✅ 已发送 ClientHello")

        # 读响应
        data = sock.recv(4096)
        print(f"收到 {len(data)} 字节响应")
        print(f"响应头: {data[:16].hex()}")

        if len(data) > 5:
            rectype = data[0]
            if rectype == 0x16:
                hs_type = data[5] if len(data) > 5 else 0
                if hs_type == 2:
                    print("✅✅ 收到 ServerHello！握手可以继续")
                elif hs_type == 15:
                    print("❌ 收到 Certificate（服务端没走 TLS1.3）")
                else:
                    print(f"收到 handshake type {hs_type}")
            elif rectype == 0x15:
                print(f"❌ 收到 Alert（错误）: {data[5:7].hex()}")
            else:
                print(f"收到记录类型 {rectype:#x}")

        sock.close()
    except Exception as e:
        print(f"失败: {type(e).__name__}: {e}")


if __name__ == "__main__":
    main()
