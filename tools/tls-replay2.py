#!/usr/bin/env python3
"""
关键实验 2：替换 key_share，保留真机其他所有字节。

原理：
- 真机 ClientHello 除 key_share 外都是静态的（套件、扩展、顺序、padding）
- key_share 里的公钥必须配对我们持有的私钥才能完成密钥交换
- 所以：保留真机的前缀结构，只把 key_share 换成我们自己生成的 x25519 公钥

如果服务器接受，就能完成完整握手。
"""

import os
import socket
import struct

from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

PCAP = "/mnt/c/Users/Administrator/AppData/Local/Temp/both.pcap"


def extract_client_hello(path: str, sni_filter: str = "api2.coolapk.com") -> bytes | None:
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


def find_key_share(hello: bytes):
    """定位 key_share 扩展的位置与内容。返回 (ext_start, ext_len, key_offset, key_len)。"""
    hs = hello[5:]
    p = 6 + 32
    p += 1 + hs[p]
    cs_len = struct.unpack(">H", hs[p : p + 2])[0]
    p += 2 + cs_len
    p += 1 + hs[p]
    ext_len = struct.unpack(">H", hs[p : p + 2])[0]
    ext_start = p + 2
    p = ext_start
    end = ext_start + ext_len
    while p + 4 <= end:
        et = struct.unpack(">H", hs[p : p + 2])[0]
        el = struct.unpack(">H", hs[p + 2 : p + 4])[0]
        if et == 51:  # key_share
            # 结构：list_len(2) + entry: group(2) + key_len(2) + key(n)
            list_len = struct.unpack(">H", hs[p + 4 : p + 6])[0]
            group = struct.unpack(">H", hs[p + 6 : p + 8])[0]
            key_len = struct.unpack(">H", hs[p + 8 : p + 10])[0]
            return (p, el, p + 10, key_len, group)
        p += 4 + el
    return None


def check_response(data: bytes) -> str:
    if len(data) < 6:
        return f"太短: {data.hex()}"
    rt = data[0]
    if rt == 0x16:
        ht = data[5]
        return "✅✅ ServerHello" if ht == 2 else f"Handshake type={ht}"
    if rt == 0x15:
        desc = data[6] if len(data) > 6 else 0
        names = {0x28: "handshake_failure", 0x32: "illegal_parameter", 0x2F: "insufficient_security"}
        return f"❌ Alert 0x{desc:02x} ({names.get(desc, '?')})"
    return f"记录类型 0x{rt:02x}"


def main():
    print("=" * 70)
    print("替换 key_share，保留真机其他字节")
    print("=" * 70)

    hello = extract_client_hello(PCAP)
    if not hello:
        print("❌ 未找到 ClientHello")
        return

    print(f"原始 ClientHello: {len(hello)} 字节")

    ks = find_key_share(hello)
    if not ks:
        print("❌ 未找到 key_share 扩展")
        return

    ext_start, ext_len, key_off, key_len, group = ks
    print(f"key_share 位置: 扩展起点 {ext_start}, 公钥偏移 {key_off}, 长度 {key_len}, 组 0x{group:04x}")
    print(f"原公钥: {hello[5 + key_off : 5 + key_off + key_len].hex()}")
    print()

    # 生成新的 x25519 密钥对
    priv = X25519PrivateKey.generate()
    pub = priv.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    print(f"新公钥: {pub.hex()}")
    print()

    # 替换（注意 hello 前 5 字节是 record header，hs 从 index 5 开始）
    new_hello = bytearray(hello)
    hs_key_off = 5 + key_off
    new_hello[hs_key_off : hs_key_off + key_len] = pub

    print(f"替换后 ClientHello: {len(new_hello)} 字节")
    print()

    for ip in ["119.188.175.46", "221.204.163.76"]:
        print(f"--- 发送到 {ip}:443 ---")
        try:
            sock = socket.create_connection((ip, 443), timeout=15)
            sock.send(bytes(new_hello))
            data = sock.recv(16384)
            print(f"  {check_response(data)}")
            if len(data) > 5 and data[0] == 0x16 and data[5] == 2:
                # 解析 ServerHello 里的 key_share
                print(f"  ServerHello 长度: {len(data)}")
                print(f"  前48字节: {data[:48].hex()}")
            sock.close()
        except Exception as e:
            print(f"  失败: {type(e).__name__}: {e}")
        print()


if __name__ == "__main__":
    main()
