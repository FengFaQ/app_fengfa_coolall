#!/usr/bin/env python3
"""
诊断 InvalidTag：检查 TLS 1.3 握手的记录层细节。

常见原因：
1. transcript hash 范围错误（应含 ClientHello + ServerHello）
2. record 解密时的 additional_data 构造错误
3. 密钥派生顺序错误（derived -> early_secret）
4. 记录分片/重组问题
"""

import hashlib
import socket
import struct

from cryptography.hazmat.primitives.asymmetric.x25519 import (
    X25519PrivateKey,
    X25519PublicKey,
)
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

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


def hkdf_extract(salt, ikm, hash_name):
    import hmac as hm

    if salt is None:
        salt = b"\x00" * hashlib.new(hash_name).digest_size
    return hm.new(salt, ikm, hash_name).digest()


def hkdf_expand(prk, info, length, hash_name):
    import hmac as hm

    out = b""
    t = b""
    i = 1
    while len(out) < length:
        t = hm.new(prk, t + info + bytes([i]), hash_name).digest()
        out += t
        i += 1
    return out[:length]


def expand_label(secret, label, context, length, hash_name):
    full = b"tls13 " + label
    info = (
        struct.pack(">H", length)
        + struct.pack(">B", len(full))
        + full
        + struct.pack(">B", len(context))
        + context
    )
    return hkdf_expand(secret, info, length, hash_name)


def main():
    hello = extract_client_hello(PCAP)
    if not hello:
        print("no hello")
        return

    priv = X25519PrivateKey.generate()
    pub = priv.public_key().public_bytes_raw()

    new_hello = bytearray(hello)
    # key_share 公钥偏移：从 handshake 起点算 229（含 4 字节 hs header）
    new_hello[5 + 229 : 5 + 229 + 32] = pub

    sock = socket.create_connection(("119.188.175.46", 443), timeout=20)
    sock.send(bytes(new_hello))

    data = sock.recv(32768)
    print(f"收到 {len(data)} 字节")

    # 解析所有 record
    p = 0
    records = []
    while p + 5 <= len(data):
        rt = data[p]
        ver = data[p + 1 : p + 3]
        rl = struct.unpack(">H", data[p + 3 : p + 5])[0]
        payload = data[p + 5 : p + 5 + rl]
        records.append((rt, ver, payload))
        p += 5 + rl

    print(f"record 数: {len(records)}")
    for i, (rt, ver, pl) in enumerate(records):
        print(f"  [{i}] type=0x{rt:02x} ver={ver.hex()} len={len(pl)}")
        if rt == 0x16 and pl[0] == 2:
            print(f"       ServerHello")

    # 取 ServerHello
    sh = records[0][2]
    hs_len = (sh[1] << 16) | (sh[2] << 8) | sh[3]
    server_hello = sh[4 : 4 + hs_len]

    q = 2 + 32
    sid_len = server_hello[q]
    q += 1 + sid_len
    suite = struct.unpack(">H", server_hello[q : q + 2])[0]
    q += 2 + 1
    ext_len = struct.unpack(">H", server_hello[q : q + 2])[0]
    q += 2
    end = q + ext_len
    server_pub = None
    while q + 4 <= end:
        et = struct.unpack(">H", server_hello[q : q + 2])[0]
        el = struct.unpack(">H", server_hello[q + 2 : q + 4])[0]
        ed = server_hello[q + 4 : q + 4 + el]
        if et == 51:
            klen = struct.unpack(">H", ed[2:4])[0]
            server_pub = ed[4 : 4 + klen]
        q += 4 + el

    hash_name = "sha384" if suite == 0x1302 else "sha256"
    hash_len = hashlib.new(hash_name).digest_size
    key_len = 32 if suite in (0x1301, 0x1302) else 16

    print(f"\n套件 0x{suite:04x}  hash={hash_name}  key_len={key_len}")

    peer = X25519PublicKey.from_public_bytes(server_pub)
    shared = priv.exchange(peer)

    # TLS 1.3 密钥调度
    early_secret = hkdf_extract(None, b"\x00" * hash_len, hash_name)
    derived = expand_label(early_secret, "derived", hashlib.new(hash_name, b"").digest(), hash_len, hash_name)
    handshake_secret = hkdf_extract(derived, shared, hash_name)

    # transcript：ClientHello + ServerHello
    th = hashlib.new(hash_name)
    th.update(bytes(new_hello[5:]))  # 完整 handshake 消息（不含 record header）
    th.update(sh[: 4 + hs_len])       # ServerHello 完整
    transcript = th.digest()

    c_hs = expand_label(handshake_secret, "c hs traffic", transcript, hash_len, hash_name)
    s_hs = expand_label(handshake_secret, "s hs traffic", transcript, hash_len, hash_name)

    print(f"transcript: {transcript.hex()[:40]}...")
    print(f"s_hs_traffic: {s_hs.hex()[:40]}...")

    # 派生服务端握手密钥
    s_key = expand_label(s_hs, "key", b"", key_len, hash_name)
    s_iv = expand_label(s_hs, "iv", b"", 12, hash_name)

    print(f"s_key: {s_key.hex()}")
    print(f"s_iv : {s_iv.hex()}")

    # 解密后续记录
    for i, (rt, ver, pl) in enumerate(records):
        if rt != 0x17:
            continue
        print(f"\n解密 record[{i}] len={len(pl)}")
        # 跳过 inner content type
        nonce = bytes(a ^ b for a, b in zip(s_iv, (i).to_bytes(12, "big")))
        aad = bytes([rt]) + ver + struct.pack(">H", len(pl))
        try:
            pt = AESGCM(s_key).decrypt(nonce, pl, aad)
            print(f"  ✅ 解密成功: {pt[:20].hex()}")
            print(f"     内容类型: 0x{pt[-1]:02x}")
        except Exception as e:
            print(f"  ❌ {type(e).__name__}: {e}")

    sock.close()


if __name__ == "__main__":
    main()
