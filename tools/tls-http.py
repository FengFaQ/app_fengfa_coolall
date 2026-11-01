#!/usr/bin/env python3
"""
终极验证：完成 TLS 1.3 握手，发出真实 HTTP 请求。

如果这一步成功并拿到 data，就证明整条技术路线可行。
如果仍是 1004，说明除 TLS 指纹外还有别的门禁。
"""

import base64
import hashlib
import hmac
import importlib.util
import socket
import struct

from cryptography.hazmat.primitives.asymmetric.x25519 import (
    X25519PrivateKey,
    X25519PublicKey,
)
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

PCAP = "/mnt/c/Users/Administrator/AppData/Local/Temp/both.pcap"
KS_OFF, KS_LEN = 229, 32

REAL_UA = (
    "Dalvik/2.1.0 (Linux; U; Android 16; MEIZU 21 BQ2A.251110.001-BP2A.250605.031.A3) "
    "(#Build; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; 16) "
    "+CoolMarket/16.7.0-2610291-universal"
)
DEVICE_PLAIN = "; ; ; ; meizu; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; null"

spec = importlib.util.spec_from_file_location(
    "tok", "/mnt/c/Users/Administrator/Documents/dsh/Kuan/tools/verify_token_v3.py"
)
tok_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tok_mod)


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
        ep, end = p + 2, p + 2 + el
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
    raise RuntimeError("no hello")


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


def suite_params(s):
    return (32, "sha384") if s == 0x1302 else (32, "sha256") if s == 0x1303 else (16, "sha256")


def parse_records(d):
    p, out = 0, []
    while p + 5 <= len(d):
        rl = struct.unpack(">H", d[p + 3 : p + 5])[0]
        if p + 5 + rl > len(d):
            break
        out.append((d[p], d[p + 1 : p + 3], d[p + 5 : p + 5 + rl]))
        p += 5 + rl
    return out


def enc_record(key, iv, seq, content_type, data):
    """加密一条 TLS 1.3 记录。"""
    inner = data + bytes([content_type])
    nonce = bytes(a ^ b for a, b in zip(iv, seq.to_bytes(12, "big")))
    aad = bytes([0x17]) + b"\x03\x03" + struct.pack(">H", len(inner) + 16)
    ct = AESGCM(key).encrypt(nonce, inner, aad)
    return b"\x17\x03\x03" + struct.pack(">H", len(ct)) + ct


def build_http_request(dev: str) -> bytes:
    token = tok_mod.create(dev, 2610291)
    path = "/v6/main/indexV8?page=1&firstLaunch=1&installTime=1761926400000"
    headers = [
        ("User-Agent", REAL_UA),
        ("X-Requested-With", "XMLHttpRequest"),
        ("X-Sdk-Int", "36"),
        ("X-Sdk-Locale", "zh-CN"),
        ("X-App-Id", "com.coolapk.market"),
        ("X-App-Token", token),
        ("X-App-Version", "16.7.0"),
        ("X-App-Code", "2610291"),
        ("X-Api-Version", "16"),
        ("X-App-Device", dev),
        ("X-Dark-Mode", "0"),
        ("X-App-Channel", "coolapk"),
        ("X-App-Mode", "universal"),
        ("X-App-Supported", "2610291"),
        ("Accept", "application/json"),
        ("Host", "api2.coolapk.com"),
        ("Connection", "close"),
    ]
    req = f"GET {path} HTTP/1.1\r\n"
    for k, v in headers:
        req += f"{k}: {v}\r\n"
    req += "\r\n"
    return req.encode()


def main():
    hello = load_hello()
    priv = X25519PrivateKey.generate()
    pub = priv.public_key().public_bytes_raw()
    new_hello = bytearray(hello)
    new_hello[KS_OFF : KS_OFF + KS_LEN] = pub
    print(f"ClientHello {len(new_hello)}B, 公钥 {pub.hex()[:32]}...")

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
            if len(parse_records(buf)) >= 4:
                break
        except socket.timeout:
            break

    recs = parse_records(buf)
    print(f"收到 {len(buf)}B, {len(recs)} records")

    sh_rec = recs[0][2]
    hs_len = (sh_rec[1] << 16) | (sh_rec[2] << 8) | sh_rec[3]
    sh = sh_rec[: 4 + hs_len]

    q = 4 + 2 + 32
    q += 1 + sh[q]
    suite = struct.unpack(">H", sh[q : q + 2])[0]
    q += 3
    ext_tot = struct.unpack(">H", sh[q : q + 2])[0]
    q += 2
    end = q + ext_tot
    server_pub = None
    while q + 4 <= end:
        et = struct.unpack(">H", sh[q : q + 2])[0]
        el = struct.unpack(">H", sh[q + 2 : q + 4])[0]
        ed = sh[q + 4 : q + 4 + el]
        if et == 51:
            kl = struct.unpack(">H", ed[2:4])[0]
            server_pub = ed[4 : 4 + kl]
        q += 4 + el

    key_len, hn = suite_params(suite)
    hl = hashlib.new(hn).digest_size
    shared = priv.exchange(X25519PublicKey.from_public_bytes(server_pub))

    early = hkdf_extract(None, b"\x00" * hl, hn)
    derived = exp_label(early, "derived", hashlib.new(hn, b"").digest(), hl, hn)
    hs_secret = hkdf_extract(derived, shared, hn)

    th1 = hashlib.new(hn, bytes(new_hello[5:]) + sh).digest()
    c_hs = exp_label(hs_secret, "c hs traffic", th1, hl, hn)
    s_hs = exp_label(hs_secret, "s hs traffic", th1, hl, hn)
    s_key = exp_label(s_hs, "key", b"", key_len, hn)
    s_iv = exp_label(s_hs, "iv", b"", 12, hn)

    # 解密服务端握手消息
    seq = 0
    plain = b""
    for rt, ver, pl in recs[1:]:
        if rt != 0x17:
            continue
        nonce = bytes(a ^ b for a, b in zip(s_iv, seq.to_bytes(12, "big")))
        aad = bytes([rt]) + ver + struct.pack(">H", len(pl))
        try:
            pt = AESGCM(s_key).decrypt(nonce, pl, aad)
            plain += pt[:-1]
            seq += 1
        except Exception:
            break
    print(f"解密服务端握手 {len(plain)}B")

    # 算 master secret
    # 注意：derived2 的 context 是 transcript 到 server Finished 的哈希，
    # 也就是 th1 + 服务端握手消息（不含 client Finished）
    th_sfin = hashlib.new(hn, bytes(new_hello[5:]) + sh + plain).digest()
    derived2 = exp_label(hs_secret, "derived", th_sfin, hl, hn)
    master = hkdf_extract(derived2, b"\x00" * hl, hn)

    # 客户端 Finished
    th2 = th_sfin
    c_fin_key = exp_label(c_hs, "finished", b"", hl, hn)
    c_verify = hmac.new(c_fin_key, th2, hn).digest()
    fin_msg = bytes([20]) + (len(c_verify)).to_bytes(3, "big") + c_verify

    c_key = exp_label(c_hs, "key", b"", key_len, hn)
    c_iv = exp_label(c_hs, "iv", b"", 12, hn)
    fin_rec = enc_record(c_key, c_iv, 0, 22, fin_msg)
    sock.send(fin_rec)
    print("已发送 client Finished")

    # 应用数据密钥：transcript 包含 client Finished
    th3 = hashlib.new(hn, bytes(new_hello[5:]) + sh + plain + fin_msg).digest()
    c_ap = exp_label(master, "c ap traffic", th3, hl, hn)
    s_ap = exp_label(master, "s ap traffic", th3, hl, hn)
    c_ap_key = exp_label(c_ap, "key", b"", key_len, hn)
    c_ap_iv = exp_label(c_ap, "iv", b"", 12, hn)

    # 发 HTTP 请求
    dev = base64.b64encode(DEVICE_PLAIN.encode()).decode().rstrip("=")[::-1]
    http = build_http_request(dev)
    req_rec = enc_record(c_ap_key, c_ap_iv, 0, 23, http)
    sock.send(req_rec)
    print(f"已发送 HTTP 请求 ({len(http)}B)")

    # 收响应
    resp = b""
    try:
        while True:
            c = sock.recv(65536)
            if not c:
                break
            resp += c
    except socket.timeout:
        pass
    sock.close()

    print(f"收到响应 {len(resp)}B")

    # 解密响应
    s_ap_key = exp_label(s_ap, "key", b"", key_len, hn)
    s_ap_iv = exp_label(s_ap, "iv", b"", 12, hn)
    body = b""
    sseq = 0
    for rt, ver, pl in parse_records(resp):
        if rt != 0x17:
            continue
        nonce = bytes(a ^ b for a, b in zip(s_ap_iv, sseq.to_bytes(12, "big")))
        aad = bytes([rt]) + ver + struct.pack(">H", len(pl))
        try:
            pt = AESGCM(s_ap_key).decrypt(nonce, pl, aad)
            body += pt[:-1]
            sseq += 1
        except Exception as e:
            print(f"  响应解密失败: {type(e).__name__}")
            break

    print()
    print("=" * 70)
    print("HTTP 响应内容")
    print("=" * 70)
    print(body.decode("utf-8", "replace")[:1500])


if __name__ == "__main__":
    main()
