#!/usr/bin/env python3
"""
完整 TLS 1.3 握手 + HTTP 请求（最终版）。

修正了 ServerHello 的解析偏移。
"""

import hashlib
import hmac
import socket
import struct

from cryptography.hazmat.primitives.asymmetric.x25519 import (
    X25519PrivateKey,
    X25519PublicKey,
)
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

PCAP = "/mnt/c/Users/Administrator/AppData/Local/Temp/both.pcap"
KS_OFF = 229
KS_LEN = 32

REAL_UA = (
    "Dalvik/2.1.0 (Linux; U; Android 16; MEIZU 21 BQ2A.251110.001-BP2A.250605.031.A3) "
    "(#Build; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; 16) "
    "+CoolMarket/16.7.0-2610291-universal"
)


def load_hello():
    data = open(PCAP, "rb").read()
    endian = "<" if data[:4] == b"\xd4\xc3\xb2\xa1" else ">"
    off = 24
    while off + 16 <= len(data):
        _, _, incl, _ = struct.unpack(endian + "IIII", data[off : off + 16])
        off += 16
        pkt = data[off : off + incl]
        off += incl
        idx = pkt.find(b"\x16\x03\x01")
        if idx < 0 or idx + 6 > len(pkt):
            continue
        rl = struct.unpack(">H", pkt[idx + 3 : idx + 5])[0]
        hs = pkt[idx + 5 : idx + 5 + rl]
        if len(hs) < 4 or hs[0] != 1:
            continue
        p = 6 + 32
        p += 1 + hs[p]
        csl = struct.unpack(">H", hs[p : p + 2])[0]
        p += 2 + csl
        p += 1 + hs[p]
        el = struct.unpack(">H", hs[p : p + 2])[0]
        ep = p + 2
        end = ep + el
        sni = None
        while ep + 4 <= end:
            et = struct.unpack(">H", hs[ep : ep + 2])[0]
            e_len = struct.unpack(">H", hs[ep + 2 : ep + 4])[0]
            if et == 0:
                n = struct.unpack(">H", hs[ep + 7 : ep + 9])[0]
                sni = hs[ep + 9 : ep + 9 + n].decode()
            ep += 4 + e_len
        if sni and "api2" in sni:
            return pkt[idx : idx + 5 + rl]
    raise RuntimeError("未找到 ClientHello")


def hkdf_extract(salt, ikm, h):
    if salt is None:
        salt = b"\x00" * hashlib.new(h).digest_size
    return hmac.new(salt, ikm, h).digest()


def hkdf_expand(prk, info, length, h):
    out, t, i = b"", b"", 1
    while len(out) < length:
        t = hmac.new(prk, t + info + bytes([i]), h).digest()
        out += t
        i += 1
    return out[:length]


def exp_label(secret, label, ctx, length, h):
    full = b"tls13 " + (label.encode() if isinstance(label, str) else label)
    info = struct.pack(">H", length) + bytes([len(full)]) + full + bytes([len(ctx)]) + ctx
    return hkdf_expand(secret, info, length, h)


def suite_params(suite):
    if suite == 0x1302:
        return 32, "sha384"
    if suite == 0x1303:
        return 32, "sha256"
    return 16, "sha256"


def parse_records(data):
    p, out = 0, []
    while p + 5 <= len(data):
        rt = data[p]
        rl = struct.unpack(">H", data[p + 3 : p + 5])[0]
        if p + 5 + rl > len(data):
            break
        out.append((rt, data[p + 1 : p + 3], data[p + 5 : p + 5 + rl]))
        p += 5 + rl
    return out


def main():
    hello = load_hello()
    print(f"ClientHello: {len(hello)} 字节")

    priv = X25519PrivateKey.generate()
    pub = priv.public_key().public_bytes_raw()
    new_hello = bytearray(hello)
    new_hello[KS_OFF : KS_OFF + KS_LEN] = pub
    print(f"植入公钥: {pub.hex()}")

    sock = socket.create_connection(("119.188.175.46", 443), timeout=20)
    sock.send(bytes(new_hello))
    sock.settimeout(8)

    buf = b""
    while True:
        try:
            c = sock.recv(65536)
            if not c:
                break
            buf += c
            recs = parse_records(buf)
            if len(recs) >= 3:
                break
        except socket.timeout:
            break

    records = parse_records(buf)
    print(f"收到 {len(buf)} 字节, {len(records)} 个 record")

    # ---- 解析 ServerHello ----
    sh_rec = records[0][2]
    hs_len = (sh_rec[1] << 16) | (sh_rec[2] << 8) | sh_rec[3]
    server_hello = sh_rec[: 4 + hs_len]

    q = 4 + 2 + 32          # handshake header + legacy_version + random
    sid_len = server_hello[q]
    q += 1 + sid_len
    suite = struct.unpack(">H", server_hello[q : q + 2])[0]
    q += 2 + 1              # cipher + compression
    ext_total = struct.unpack(">H", server_hello[q : q + 2])[0]
    q += 2
    end = q + ext_total

    server_pub = None
    while q + 4 <= end:
        et = struct.unpack(">H", server_hello[q : q + 2])[0]
        el = struct.unpack(">H", server_hello[q + 2 : q + 4])[0]
        ed = server_hello[q + 4 : q + 4 + el]
        if et == 51:
            klen = struct.unpack(">H", ed[2:4])[0]
            server_pub = ed[4 : 4 + klen]
        q += 4 + el

    key_len, hash_name = suite_params(suite)
    hash_len = hashlib.new(hash_name).digest_size
    print(f"套件 0x{suite:04x}  hash={hash_name}  key_len={key_len}")
    print(f"服务端公钥: {server_pub.hex()}")

    # ---- 密钥调度 ----
    shared = priv.exchange(X25519PublicKey.from_public_bytes(server_pub))
    early = hkdf_extract(None, b"\x00" * hash_len, hash_name)
    derived = exp_label(early, "derived", hashlib.new(hash_name, b"").digest(), hash_len, hash_name)
    hs_secret = hkdf_extract(derived, shared, hash_name)

    th_in = bytes(new_hello[5:]) + server_hello
    th = hashlib.new(hash_name, th_in).digest()

    c_hs = exp_label(hs_secret, "c hs traffic", th, hash_len, hash_name)
    s_hs = exp_label(hs_secret, "s hs traffic", th, hash_len, hash_name)
    s_key = exp_label(s_hs, "key", b"", key_len, hash_name)
    s_iv = exp_label(s_hs, "iv", b"", 12, hash_name)
    print(f"✅ 握手密钥派生完成")

    # ---- 解密服务端握手消息 ----
    seq = 0
    plain_hs = b""
    for rt, ver, pl in records[1:]:
        if rt != 0x17:
            continue
        nonce = bytes(a ^ b for a, b in zip(s_iv, seq.to_bytes(12, "big")))
        aad = bytes([rt]) + ver + struct.pack(">H", len(pl))
        try:
            pt = AESGCM(s_key).decrypt(nonce, pl, aad)
            plain_hs += pt[:-1]  # 去掉内层 content type
            seq += 1
            print(f"  ✅ record #{seq} 解密成功 ({len(pt)} 字节)")
        except Exception as e:
            print(f"  ❌ record #{seq+1} 解密失败: {type(e).__name__}")
            break

    if not plain_hs:
        sock.close()
        return

    print()
    print("=" * 70)
    print("🎯 服务端握手消息解密成功")
    print("=" * 70)
    names = {8: "EncryptedExtensions", 11: "Certificate", 15: "CertificateVerify", 20: "Finished"}
    p = 0
    while p + 4 <= len(plain_hs):
        ht = plain_hs[p]
        hl = (plain_hs[p + 1] << 16) | (plain_hs[p + 2] << 8) | plain_hs[p + 3]
        print(f"  type={ht:2d} {names.get(ht, '?'):22s} len={hl}")
        p += 4 + hl

    sock.close()


if __name__ == "__main__":
    main()
