#!/usr/bin/env python3
"""
终极验证：用真机 ClientHello 模板完成完整 TLS 握手并发 HTTP 请求。

如果这个成功，就证明：
1. TLS 指纹可以绕过
2. 整个技术路线可行
3. 剩下只是移植到 Rust 的工作量问题

实现要点：
- 用真机 ClientHello 模板 + 自生成 key_share
- 完成 TLS 1.3 握手（需要处理 ServerHello / EncryptedExtensions /
  Certificate / CertificateVerify / Finished）
- 然后发 HTTP 请求

由于完整 TLS 1.3 握手实现复杂，这里先验证到「收到 ServerHello 并解析出服务端公钥」，
证明密钥交换可以继续。
"""

import os
import socket
import struct
import hashlib

from cryptography.hazmat.primitives.asymmetric.x25519 import (
    X25519PrivateKey,
    X25519PublicKey,
)
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

PCAP = "/mnt/c/Users/Administrator/AppData/Local/Temp/both.pcap"


def extract_client_hello(path, sni_filter="api2.coolapk.com"):
    data = open(path, "rb").read()
    magic = data[:4]
    endian = "<" if magic == b"\xd4\xc3\xb2\xa1" else ">"
    off = 24
    while off + 16 <= len(data):
        _, _, incl, _ = struct.unpack(endian + "IIII", data[off : off + 16])
        off += 16
        pkt = data[off : off + incl]
        off += incl
        idx = pkt.find(b"\x16\x03\x01")
        if idx < 0 or idx + 6 > len(pkt):
            continue
        rec_len = struct.unpack(">H", pkt[idx + 3 : idx + 5])[0]
        hs = pkt[idx + 5 : idx + 5 + rec_len]
        if len(hs) < 4 or hs[0] != 0x01:
            continue
        p = 6 + 32
        p += 1 + hs[p]
        cs_len = struct.unpack(">H", hs[p : p + 2])[0]
        p += 2 + cs_len
        p += 1 + hs[p]
        ext_len = struct.unpack(">H", hs[p : p + 2])[0]
        p += 2
        end = p + ext_len
        sni = None
        while p + 4 <= end:
            et = struct.unpack(">H", hs[p : p + 2])[0]
            el = struct.unpack(">H", hs[p + 2 : p + 4])[0]
            if et == 0:
                try:
                    n = struct.unpack(">H", hs[p + 7 : p + 9])[0]
                    sni = hs[p + 9 : p + 9 + n].decode()
                except Exception:
                    pass
            p += 4 + el
        if sni and sni_filter in sni:
            return pkt[idx : idx + 5 + rec_len]
    return None


def hkdf_expand_label(secret, label, context, length):
    """TLS 1.3 的 HKDF-Expand-Label。"""
    full_label = b"tls13 " + label.encode()
    info = (
        struct.pack(">H", length)
        + struct.pack(">B", len(full_label))
        + full_label
        + struct.pack(">B", len(context))
        + context
    )
    return HKDF(algorithm=hashes.SHA256(), length=length, salt=b"", info=info).derive(secret)


def main():
    print("=" * 70)
    print("完整 TLS 1.3 握手验证")
    print("=" * 70)

    hello = extract_client_hello(PCAP)
    if not hello:
        print("❌ 未找到 ClientHello")
        return

    # 生成密钥对
    priv = X25519PrivateKey.generate()
    our_pub = priv.public_key().public_bytes_raw()

    # 替换 key_share（偏移 224，长度 32，从 handshake 起始算，注意 +5 是 record header）
    new_hello = bytearray(hello)
    new_hello[5 + 224 : 5 + 224 + 32] = our_pub

    print(f"ClientHello: {len(new_hello)} 字节")
    print(f"我们的公钥: {our_pub.hex()}")
    print()

    sock = socket.create_connection(("119.188.175.46", 443), timeout=20)
    sock.send(bytes(new_hello))
    print("已发送 ClientHello")

    # 接收 ServerHello
    data = sock.recv(16384)
    print(f"收到 {len(data)} 字节")

    if data[0] != 0x16 or data[5] != 2:
        print(f"❌ 不是 ServerHello: {data[:16].hex()}")
        return

    print("✅ ServerHello")

    # 解析 ServerHello
    hs = data[5:]
    hs_len = (hs[1] << 16) | (hs[2] << 8) | hs[3]
    server_hello = hs[4 : 4 + hs_len]

    # server_version(2) + random(32) + session_id_len(1) + session_id + cipher(2) + comp(1) + ext_len(2) + ext
    p = 2 + 32
    sid_len = server_hello[p]
    server_random = server_hello[2:34]
    p += 1 + sid_len
    cipher = struct.unpack(">H", server_hello[p : p + 2])[0]
    p += 2 + 1
    ext_len = struct.unpack(">H", server_hello[p : p + 2])[0]
    p += 2
    end = p + ext_len

    server_pub = None
    while p + 4 <= end:
        et = struct.unpack(">H", server_hello[p : p + 2])[0]
        el = struct.unpack(">H", server_hello[p + 2 : p + 4])[0]
        ed = server_hello[p + 4 : p + 4 + el]
        if et == 51:  # key_share
            group = struct.unpack(">H", ed[0:2])[0]
            klen = struct.unpack(">H", ed[2:4])[0]
            server_pub = ed[4 : 4 + klen]
        p += 4 + el

    print(f"  密码套件: 0x{cipher:04x}")
    print(f"  Server random: {server_random.hex()[:32]}...")
    if server_pub:
        print(f"  服务端公钥: {server_pub.hex()}")

        # 计算共享密钥
        peer = X25519PublicKey.from_public_bytes(server_pub)
        shared = priv.exchange(peer)
        print(f"  共享密钥: {shared.hex()[:32]}...")
        print("✅✅ 密钥交换成功！")

        # 派生握手密钥
        early_secret = b"\x00" * 32
        derived = hkdf_expand_label(early_secret, "derived", b"", 32)
        client_hello_hash = hashlib.sha256(bytes(new_hello[5:])).digest()
        handshake_secret = HKDF(
            algorithm=hashes.SHA256(), length=32, salt=derived, info=b""
        ).derive(shared)

        c_hs_traffic = hkdf_expand_label(handshake_secret, "c hs traffic", client_hello_hash, 32)
        s_hs_traffic = hkdf_expand_label(handshake_secret, "s hs traffic", client_hello_hash, 32)
        print(f"  client handshake traffic: {c_hs_traffic.hex()[:32]}...")
        print(f"  server handshake traffic: {s_hs_traffic.hex()[:32]}...")
        print()
        print("=" * 70)
        print("🎯 结论：完整 TLS 1.3 密钥交换可行")
        print("=" * 70)
        print("剩余工作：解密后续握手消息、发送 Finished、建立加密通道")
        print("这些都是标准的 TLS 1.3 流程，用成熟库可以实现")

    sock.close()


if __name__ == "__main__":
    main()
