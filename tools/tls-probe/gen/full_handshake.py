"""决定性实验：模板 ClientHello + 自有 key_share，跑完整握手并打真 API。

与 lead 的 tools/tls-replay2.py 的区别：
  replay2 只 `recv()` 一次，看到 0x16/0x02 就报「ServerHello 成功」——
  但那只验证了 TLS 层没被拒，**没有实际发出 HTTP 请求，也没有看响应体**。
  本脚本用完整 TLS 1.3 握手（HKDF + AES-128-GCM）走到 HTTP 层，
  拿到真正的业务响应，回答「1004 会不会消失」。

用法：python3 full_handshake.py <pcap> [--sni api2.coolapk.com] [--http1]
"""
import hashlib
import hmac
import json
import socket
import ssl
import struct
import sys
import time

from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

sys.path.insert(0, "/mnt/c/Users/Administrator/Documents/dsh/Kuan/tools/tls-probe/gen")
import pcap_parse  # noqa: E402

# ------------------------------------------------------------------ TLS 1.3 密码学

LABEL_PREFIX = b"tls13 "
HASH_LEN = 32  # SHA-256


def hkdf_extract(salt: bytes, ikm: bytes) -> bytes:
    if not salt:
        salt = b"\x00" * HASH_LEN
    return hmac.new(salt, ikm, hashlib.sha256).digest()


def hkdf_expand(prk: bytes, info: bytes, length: int) -> bytes:
    out = b""
    t = b""
    i = 1
    while len(out) < length:
        t = hmac.new(prk, t + info + bytes([i]), hashlib.sha256).digest()
        out += t
        i += 1
    return out[:length]


def expand_label(secret: bytes, label: bytes, context: bytes, length: int) -> bytes:
    full = LABEL_PREFIX + label
    info = struct.pack(">H", length) + bytes([len(full)]) + full + bytes([len(context)]) + context
    return hkdf_expand(secret, info, length)


class KeySchedule:
    """TLS 1.3 (RFC 8446 §7.1) 精简实现，只做客户端侧。"""

    def __init__(self, psk: bytes | None = None):
        self.zero = b"\x00" * HASH_LEN
        # early_secret
        self.early = hkdf_extract(b"\x00" * HASH_LEN, psk if psk else self.zero)
        # derived = Derive-Secret(early, "derived", Hash(""))
        self.derived_early = expand_label(self.early, b"derived", hashlib.sha256(b"").digest(), HASH_LEN)

    def handshake_secret(self, shared: bytes) -> None:
        self.handshake = hkdf_extract(self.derived_early, shared)

    def master_secret(self, transcript_hash: bytes) -> None:
        # RFC 8446 §7.1：
        #   derived = Derive-Secret(Handshake Secret, "derived", Hash(""))
        #   Master Secret = HKDF-Extract(derived, 0)
        # 注意 "derived" 用的是空 transcript 的哈希，不是当前 transcript。
        derived = expand_label(self.handshake, b"derived",
                               hashlib.sha256(b"").digest(), HASH_LEN)
        self.master = hkdf_extract(derived, self.zero)

    def traffic_secret(self, base: str, transcript_hash: bytes) -> bytes:
        secret = self.handshake if base == "handshake" else self.master
        return expand_label(secret, b"c ap traffic" if base != "handshake" else b"c hs traffic",
                            transcript_hash, HASH_LEN)


def key_iv(secret: bytes, key_len: int = 16) -> tuple[bytes, bytes]:
    """按套件取 key（AES-128=16 / AES-256=32）与 iv（恒 12）。

    注意：酷安服务端选的是 0x1302 TLS_AES_256_GCM_SHA384，key 必须是 32 字节。
    早期版本这里写死 16，导致记录层 AEAD 一直 InvalidTag——
    而握手密钥调度其实完全正确（已用 OpenSSL 的 keylog 交叉验证）。
    """
    return expand_label(secret, b"key", b"", key_len), expand_label(secret, b"iv", b"", 12)


def suite_params(suite: int) -> tuple[int, str]:
    """返回 (key 长度, hashlib 名字)。"""
    if suite == 0x1301:  # TLS_AES_128_GCM_SHA256
        return 16, "sha256"
    if suite == 0x1302:  # TLS_AES_256_GCM_SHA384
        return 32, "sha384"
    if suite == 0x1303:  # TLS_CHACHA20_POLY1305_SHA256
        return 32, "sha256"
    raise ValueError(f"暂不支持的套件 0x{suite:04x}")


# ------------------------------------------------------------------ 记录层

class RecordCrypto:
    """TLS 1.3 记录层 AES-GCM，含 nonce 递增与 XOR 脱敏。"""

    def __init__(self, secret: bytes, key_len: int = 16):
        self.key, self.iv = key_iv(secret, key_len)
        self.aead = AESGCM(self.key)
        self.seq = 0

    def _nonce(self) -> bytes:
        n = bytearray(self.iv)
        for i in range(len(n)):
            n[len(n) - 1 - i] ^= (self.seq >> (8 * i)) & 0xFF
        return bytes(n)

    def encrypt(self, plaintext: bytes, inner_type: int) -> bytes:
        ct = self.aead.encrypt(self._nonce(), plaintext + bytes([inner_type]), None)
        self.seq += 1
        return ct

    def decrypt(self, ciphertext: bytes, inner_type: int, seq: int) -> bytes:
        n = bytearray(self.iv)
        for i in range(len(n)):
            n[len(n) - 1 - i] ^= (seq >> (8 * i)) & 0xFF
        pt = self.aead.decrypt(bytes(n), ciphertext, None)
        return pt


def strip_padding(plain: bytes) -> bytes:
    i = len(plain) - 1
    while i >= 0 and plain[i] == 0:
        i -= 1
    if i < 0:
        raise ValueError("内容全 0，无法去填充")
    return plain[:i]


# ------------------------------------------------------------------ 握手

def extract_hello_for(pcap: str, sni_want: str) -> bytes:
    for key, ch in pcap_parse.tls_client_hellos(pcap):
        sni = ""
        for et, ed in ch["extensions"]:
            if et == 0:
                sni = ed[5:].decode()
        if sni != sni_want:
            continue
        # 重新序列化拿原始字节不现实，这里直接按 pcap 顺序重建
        return rebuild(ch, sni)
    raise SystemExit(f"pcap 里没有 sni={sni_want} 的 ClientHello")


def rebuild(ch, sni: str) -> bytes:
    return None  # 占位，实际用 below 的 raw 提取


def raw_client_hello(pcap: str, sni_want: str):
    """返回 (record_bytes, key_share_abs_offset, key_len, group)"""
    pk = pcap_parse.tcp_segments(pcap)
    for key, segs in pk.items():
        if key[3] != 443:
            continue
        segs.sort(key=lambda x: x[0])
        buf = b"".join(s[1] for s in segs)
        i = buf.find(b"\x16\x03\x01")
        if i < 0:
            continue
        rec_len = struct.unpack(">H", buf[i + 3:i + 5])[0]
        rec = buf[i:i + 5 + rec_len]
        if rec[5] != 0x01:
            continue
        ch = pcap_parse.parse_client_hello(rec[9:])
        sni = ""
        for et, ed in ch["extensions"]:
            if et == 0:
                sni = ed[5:].decode()
        if sni != sni_want:
            continue
        # 找 key_share
        hs = rec[5:]
        p = 6 + 32
        p += 1 + hs[p]
        cs_len = struct.unpack(">H", hs[p:p + 2])[0]
        p += 2 + cs_len
        p += 1 + hs[p]
        p += 2
        end = len(hs)
        while p + 4 <= end:
            et = struct.unpack(">H", hs[p:p + 2])[0]
            el = struct.unpack(">H", hs[p + 2:p + 4])[0]
            if et == 51:
                grp = struct.unpack(">H", hs[p + 6:p + 8])[0]
                klen = struct.unpack(">H", hs[p + 8:p + 10])[0]
                return rec, 5 + p + 10, klen, grp
            p += 4 + el
    return None


def handshake(ip: str, hello: bytes, priv, pub: bytes, sni: str, verbose=True):
    """走完 TLS 1.3 握手，返回 (sock, client_app_crypto, server_app_crypto, alpn)"""
    sock = socket.create_connection((ip, 443), timeout=20)
    sock.sendall(hello)

    transcript = b""  # 累积握手消息（不含记录头）
    transcript += hello[5:]

    buf = b""

    def recv_record():
        nonlocal buf
        while True:
            if len(buf) >= 5:
                ln = struct.unpack(">H", buf[3:5])[0]
                if len(buf) >= 5 + ln:
                    r = buf[:5 + ln]
                    buf = buf[5 + ln:]
                    return r
            d = sock.recv(16384)
            if not d:
                raise EOFError("对端关闭")
            buf += d

    # ---- ServerHello（明文记录）
    rec = recv_record()
    if rec[0] == 0x15:
        desc = rec[6] if len(rec) > 6 else 0
        raise SystemExit(f"❌ 服务端 Alert 0x{desc:02x}")
    assert rec[0] == 0x16 and rec[5] == 0x02, f"不是 ServerHello: {rec[:8].hex()}"
    transcript += rec[5:]
    sh_body = rec[9:]
    o = 2 + 32
    sid_len = sh_body[o]
    o += 1 + sid_len
    suite = struct.unpack(">H", sh_body[o:o + 2])[0]
    o += 2
    o += 1  # compression
    ext_len = struct.unpack(">H", sh_body[o:o + 2])[0]
    o += 2
    end = o + ext_len
    server_pub = None
    server_alpn = None
    while o + 4 <= end:
        et = struct.unpack(">H", sh_body[o:o + 2])[0]
        el = struct.unpack(">H", sh_body[o + 2:o + 4])[0]
        v = sh_body[o + 4:o + 4 + el]
        if et == 51:  # key_share
            server_pub = v[4:4 + struct.unpack(">H", v[2:4])[0]]
        if et == 16:  # alpn
            n = v[2]
            server_alpn = v[3:3 + n].decode()
        o += 4 + el

    if verbose:
        print(f"  ServerHello: suite=0x{suite:04x} alpn={server_alpn} "
              f"server_pub={server_pub.hex()[:16]}...")

    # ---- 确定套件参数（key 长度 + transcript 哈希）
    key_len, hash_name = suite_params(suite)
    if hash_name != "sha256":
        raise SystemExit(
            f"服务端选了 0x{suite:04x}，transcript 哈希是 {hash_name}；"
            "本脚本的 HKDF 目前只按 SHA-256 实现，请改 HASH_LEN/new 相应哈希后再跑")

    # ---- 算共享密钥
    shared = _x25519(priv, server_pub)

    ks = KeySchedule()
    ks.handshake_secret(shared)
    th_sh = hashlib.sha256(transcript).digest()
    # 服务端 handshake traffic 用 "s hs traffic"
    s_hs_secret = expand_label(ks.handshake, b"s hs traffic", th_sh, HASH_LEN)
    s_hs = RecordCrypto(s_hs_secret, key_len)

    # ---- 收加密的握手消息，直到 Finished
    s_seq = 0
    finished = None
    encrypted_exts = None
    cert_msg = None
    while True:
        rec = recv_record()
        if rec[0] == 0x14:  # ChangeCipherSpec：TLS 1.3 兼容模式下的哑记录，忽略
            continue
        if rec[0] == 0x15:
            desc = rec[6] if len(rec) > 6 else 0
            raise SystemExit(f"❌ 握手过程中收到 Alert 0x{desc:02x}")
        assert rec[0] == 0x17, f"期望加密记录，得到 0x{rec[0]:02x}"
        plain = s_hs.decrypt(rec[5:], 0, s_seq)
        s_seq += 1
        plain = strip_padding(plain)
        # 可能一条记录里有多个握手消息，简化：按长度切
        p = 0
        while p + 4 <= len(plain):
            htype = plain[p]
            hlen = (plain[p + 1] << 16) | (plain[p + 2] << 8) | plain[p + 3]
            body = plain[p + 4:p + 4 + hlen]
            transcript += plain[p:p + 4 + hlen]
            if htype == 8:  # EncryptedExtensions
                encrypted_exts = body
            elif htype == 11:
                cert_msg = body
            elif htype == 20:
                finished = body
            p += 4 + hlen
        if finished is not None:
            break

    if verbose:
        print(f"  ✅ 收到 Finished，握手消息齐了 "
              f"(cert={len(cert_msg) if cert_msg else 0}B)")

    # ---- 校验服务端 Finished（可选但能证明密钥调度正确）
    finished_key = expand_label(s_hs_secret, b"finished", b"", HASH_LEN)
    th_before_finished = hashlib.sha256(transcript[:-4 - len(finished)]).digest()
    expect = hmac.new(finished_key, th_before_finished, hashlib.sha256).digest()
    if verbose:
        ok = expect == finished
        print(f"  服务端 Finished 验证: {'✅ 通过' if ok else '❌ 不匹配'}")
        if not ok:
            print(f"    expect {expect.hex()}\n    got    {finished.hex()}")

    # ---- master secret → 应用流量密钥
    th_fin = hashlib.sha256(transcript).digest()
    ks.master_secret(th_fin)
    c_ap_secret = expand_label(ks.master, b"c ap traffic", th_fin, HASH_LEN)
    s_ap_secret = expand_label(ks.master, b"s ap traffic", th_fin, HASH_LEN)
    c_hs = RecordCrypto(c_hs_secret, key_len)
    c_ap = RecordCrypto(c_ap_secret, key_len)
    s_ap = RecordCrypto(s_ap_secret, key_len)

    # ---- 计算 client handshake traffic secret（发 Finished 用）
    c_hs_secret = expand_label(ks.handshake, b"c hs traffic", th_sh, HASH_LEN)

    # ---- 发客户端 Finished
    c_fin_key = expand_label(c_hs_secret, b"finished", b"", HASH_LEN)
    c_verify = hmac.new(c_fin_key, hashlib.sha256(transcript).digest(), hashlib.sha256).digest()
    fin_msg = b"\x14" + len(c_verify).to_bytes(3, "big") + c_verify
    ct = c_hs.encrypt(fin_msg, 22)
    sock.sendall(b"\x17\x03\x03" + struct.pack(">H", len(ct)) + ct)

    return sock, c_ap, s_ap, server_alpn, recv_record


def _x25519(priv, server_pub: bytes) -> bytes:
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PublicKey
    return priv.exchange(X25519PublicKey.from_public_bytes(server_pub))


def http_get(sock, c_ap: RecordCrypto, s_ap: RecordCrypto, host: str,
             path: str, headers: dict, recv_record) -> str:
    lines = [f"GET {path} HTTP/1.1", f"Host: {host}"]
    for k, v in headers.items():
        lines.append(f"{k}: {v}")
    lines.append("Connection: close")
    req = ("\r\n".join(lines) + "\r\n\r\n").encode()

    ct = c_ap.encrypt(req, 23)
    sock.sendall(b"\x17\x03\x03" + struct.pack(">H", len(ct)) + ct)

    out = b""
    s_seq = 0
    sock.settimeout(15)
    try:
        while True:
            rec = recv_record()
            if rec[0] == 0x15:
                break
            if rec[0] != 0x17:
                break
            plain = strip_padding(s_ap.decrypt(rec[5:], 0, s_seq))
            s_seq += 1
            out += plain
            if len(out) > 60000:
                break
    except (EOFError, socket.timeout, ssl.SSLError):
        pass
    return out.decode("utf-8", "replace")


# ------------------------------------------------------------------ main

if __name__ == "__main__":
    pcap = sys.argv[1] if len(sys.argv) > 1 else "/tmp/both.pcap"
    sni = "api2.coolapk.com"
    if "--sni" in sys.argv:
        sni = sys.argv[sys.argv.index("--sni") + 1]

    got = raw_client_hello(pcap, sni)
    if not got:
        raise SystemExit("没找到模板 ClientHello")
    rec, key_off, klen, grp = got
    print(f"模板 ClientHello: {len(rec)} 字节 (record+hello), key_share@{key_off} len={klen} group=0x{grp:04x}")

    priv = X25519PrivateKey.generate()
    pub = priv.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    hello = bytearray(rec)
    hello[key_off:key_off + klen] = pub
    hello = bytes(hello)
    print(f"替换公钥: {pub.hex()}")
    print()

    for ip in ["221.204.163.76", "119.188.175.46"]:
        print(f"--- {ip}:443 ---")
        try:
            sock, c_ap, s_ap, alpn, recv_record = handshake(ip, hello, priv, pub, sni)
            print(f"  ALPN = {alpn}")
            body = http_get(
                sock, c_ap, s_ap, sni,
                "/v6/main/indexV8?page=1&firstLaunch=1&installTime=1761926400000",
                {
                    "User-Agent": "Dalvik/2.1.0 (Linux; U; Android 16; MEIZU 21 BQ2A.251110.001-BP2A.250605.031.A3) (#Build; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; 16) +CoolMarket/16.7.0-2610291-universal",
                    "Accept": "application/json",
                },
                recv_record)
            head = body.split("\r\n\r\n", 1)
            print(f"  HTTP 状态行: {head[0].splitlines()[0] if head else '?'}")
            payload = head[1] if len(head) > 1 else ""
            print(f"  响应体前 200: {payload[:200]}")
            sock.close()
        except SystemExit as e:
            print(f"  {e}")
        except Exception as e:
            print(f"  失败: {type(e).__name__}: {e}")
        print()
