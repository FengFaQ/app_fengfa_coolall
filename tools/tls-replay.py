#!/usr/bin/env python3
"""
关键实验：用真机抓到的原始 ClientHello 字节重放。

思路：
1. 从 both.pcap 提取真机 App 发出的完整 ClientHello 原始字节
2. 修改其中可变的字段（SNI 可保留、random/session_id 可保留）
3. 直接发送给服务器
4. 看服务器响应

如果原始字节能收到 ServerHello，说明「重放真机字节」这条路可行，
后续只需要解决密钥交换（key_share 需要重新生成）。
"""

import os
import socket
import struct
import sys


def extract_client_hello(pcap_path: str, sni_filter: str = "api2.coolapk.com"):
    """从 pcap 中提取指定 SNI 的 ClientHello 原始字节。"""
    data = open(pcap_path, "rb").read()

    # 判断字节序
    magic = data[:4]
    endian = "<" if magic == b"\xd4\xc3\xb2\xa1" else ">"

    off = 24
    while off + 16 <= len(data):
        ts_sec, ts_usec, incl, orig = struct.unpack(endian + "IIII", data[off : off + 16])
        off += 16
        pkt = data[off : off + incl]
        off += incl

        idx = pkt.find(b"\x16\x03\x01")
        if idx < 0 or idx + 6 > len(pkt):
            continue

        rec_len = struct.unpack(">H", pkt[idx + 3 : idx + 5])[0]
        raw_record = pkt[idx : idx + 5 + rec_len]
        hs = pkt[idx + 5 : idx + 5 + rec_len]
        if len(hs) < 4 or hs[0] != 0x01:
            continue

        # 找 SNI
        p = 6 + 32
        sid_len = hs[p]
        p += 1 + sid_len
        cs_len = struct.unpack(">H", hs[p : p + 2])[0]
        p += 2 + cs_len
        comp_len = hs[p]
        p += 1 + comp_len
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
            return raw_record
    return None


def check_response(data: bytes) -> str:
    if len(data) < 6:
        return f"响应太短({len(data)}B): {data.hex()}"
    rt = data[0]
    if rt == 0x16:
        ht = data[5]
        if ht == 2:
            return "✅✅ ServerHello！握手可以继续"
        return f"✅ Handshake type={ht}"
    if rt == 0x15:
        desc = data[6] if len(data) > 6 else 0
        names = {
            0x28: "handshake_failure",
            0x2F: "insufficient_security",
            0x32: "illegal_parameter",
            0x33: "unknown_ca",
            0x47: "protocol_version",
            0x70: "protocol_version(70)",
        }
        return f"❌ Alert 0x{desc:02x} ({names.get(desc, 'unknown')})"
    return f"记录类型 0x{rt:02x}"


def main():
    pcap = "/mnt/c/Users/Administrator/AppData/Local/Temp/both.pcap"
    if not os.path.exists(pcap):
        print(f"pcap 不存在: {pcap}")
        print("需要重新抓包")
        return

    print("=" * 70)
    print("重放真机原始 ClientHello")
    print("=" * 70)

    hello = extract_client_hello(pcap)
    if not hello:
        print("❌ 未找到 api2.coolapk.com 的 ClientHello")
        return

    print(f"提取到 ClientHello: {len(hello)} 字节")
    print(f"头部: {hello[:24].hex()}")
    print()

    # 解析确认
    hs = hello[5:]
    p = 6 + 32
    sid_len = hs[p]
    p += 1 + sid_len
    cs_len = struct.unpack(">H", hs[p : p + 2])[0]
    ciphers = [hs[p + 2 + i : p + 4 + i].hex() for i in range(0, cs_len, 2)]
    print(f"套件数: {len(ciphers)}")
    print(f"套件: {ciphers}")
    print()

    # 直接发送
    for ip in ["119.188.175.46", "221.204.163.76"]:
        print(f"--- 发送到 {ip}:443 ---")
        try:
            sock = socket.create_connection((ip, 443), timeout=15)
            sock.send(hello)
            data = sock.recv(16384)
            print(f"  {check_response(data)}")
            print(f"  前32字节: {data[:32].hex()}")
            sock.close()
        except Exception as e:
            print(f"  失败: {type(e).__name__}: {e}")
        print()


if __name__ == "__main__":
    main()
