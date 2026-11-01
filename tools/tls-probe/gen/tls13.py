"""TLS 1.3 最小客户端：给定任意 ClientHello 字节，完成握手并发出 HTTP 请求。

用途：验证「把 TLS 指纹改成和酷安 App 完全一致之后，服务端还会不会回 1004」。

设计：
  * ClientHello 完全由外部提供（模板字节 + 替换 key_share），本模块不参与构造，
    所以套件顺序 / 扩展顺序 / padding 全部原样继承。
  * 哈希按协商出的套件选择（0x1301/0x1303 → SHA-256，0x1302 → SHA-384），
    这正是之前 InvalidTag 的根因。
  * 只支持 AES-GCM 套件；CHACHA 需要额外依赖，本项目场景用不到。
"""

import hashlib
import hmac
import socket
import struct

from cryptography.hazmat.primitives.ciphers.aead import AESGCM

LABEL_PREFIX = b"tls13 "


# ------------------------------------------------------------------ 哈希 / HKDF

def suite_params(suite: int) -> tuple[int, str]:
    """返回 (key 长度, hashlib 名)。"""
    if suite == 0x1301:
        return 16, "sha256"
    if suite == 0x1302:
        return 32, "sha384"
    if suite == 0x1303:
        return 32, "sha256"
    raise ValueError(f"不支持的套件 0x{suite:04x}")


class Suite:
    def __init__(self, suite: int):
        self.suite = suite
        self.key_len, self.hash_name = suite_params(suite)
        self.hash_len = hashlib.new(self.hash_name).digest_size

    def H(self, data: bytes) -> bytes:
        return hashlib.new(self.hash_name, data).digest()

    def extract(self, salt: bytes, ikm: bytes) -> bytes:
        return hmac.new(salt or b"\x00" * self.hash_len, ikm, self.hash_name).digest()

    def expand(self, prk: bytes, info: bytes, length: int) -> bytes:
        out, t, i = b"", b"", 1
        while len(out) < length:
            t = hmac.new(prk, t + info + bytes([i]), self.hash_name).digest()
            out += t
            i += 1
        return out[:length]

    def expand_label(self, secret: bytes, label: bytes, context: bytes, length: int) -> bytes:
        full = LABEL_PREFIX + label
        info = (length.to_bytes(2, "big") + bytes([len(full)]) + full
                + bytes([len(context)]) + context)
        return self.expand(secret, info, length)

    def key_iv(self, secret: bytes) -> tuple[bytes, bytes]:
        return (self.expand_label(secret, b"key", b"", self.key_len),
                self.expand_label(secret, b"iv", b"", 12))


class KeySchedule:
    """RFC 8446 §7.1，无 PSK。"""

    def __init__(self, s: Suite):
        self.s = s
        self.zero = b"\x00" * s.hash_len
        self.early = s.extract(b"\x00" * s.hash_len, self.zero)
        self.derived_early = s.expand_label(self.early, b"derived", s.H(b""), s.hash_len)
        self.handshake = None
        self.master = None

    def set_shared(self, shared: bytes) -> None:
        self.handshake = self.s.extract(self.derived_early, shared)

    def derive_master(self) -> None:
        derived = self.s.expand_label(self.handshake, b"derived", self.s.H(b""), self.s.hash_len)
        self.master = self.s.extract(derived, self.zero)

    def hs_secret(self, who: bytes, transcript_hash: bytes) -> bytes:
        return self.s.expand_label(self.handshake, who + b" hs traffic", transcript_hash, self.s.hash_len)

    def ap_secret(self, who: bytes, transcript_hash: bytes) -> bytes:
        return self.s.expand_label(self.master, who + b" ap traffic", transcript_hash, self.s.hash_len)


# ------------------------------------------------------------------ 记录层

class RecordCrypto:
    """TLS 1.3 记录层 AES-GCM。

    关键点（踩过的坑）：TLS 1.3 的 AEAD **AAD 是记录头 5 字节**
    `type(0x17) || 0x0303 || length`，不是空字符串。
    用 AAD=None 会稳定地得到 InvalidTag，而且看起来像是密钥算错了，
    实际上密钥调度完全正确。
    """

    def __init__(self, s: Suite, secret: bytes):
        self.key, self.iv = s.key_iv(secret)
        self.aead = AESGCM(self.key)
        self.seq = 0

    def _nonce(self, seq: int) -> bytes:
        pad = bytes(len(self.iv) - 8) + seq.to_bytes(8, "big")
        return bytes(a ^ b for a, b in zip(self.iv, pad))

    @staticmethod
    def _aad(payload_len: int) -> bytes:
        return b"\x17\x03\x03" + payload_len.to_bytes(2, "big")

    def seal(self, plaintext: bytes, inner_type: int) -> bytes:
        body_len = len(plaintext) + 1 + 16
        ct = self.aead.encrypt(self._nonce(self.seq), plaintext + bytes([inner_type]),
                               self._aad(body_len))
        self.seq += 1
        return ct

    def open(self, ciphertext: bytes, seq: int) -> tuple[bytes, int]:
        plain = self.aead.decrypt(self._nonce(seq), ciphertext,
                                  self._aad(len(ciphertext)))
        # 去掉 TLS 1.3 零填充，最后一个非零字节就是 inner type
        i = len(plain) - 1
        while i >= 0 and plain[i] == 0:
            i -= 1
        if i < 0:
            raise ValueError("内容全 0")
        return plain[:i], plain[i]


# ------------------------------------------------------------------ 握手

class Conn:
    def __init__(self, sock, suite_obj, c_hs, s_hs, c_ap, s_ap, alpn):
        self.sock = sock
        self.s = suite_obj
        self.c_hs = c_hs
        self.s_hs = s_hs
        self.c_ap = c_ap
        self.s_ap = s_ap
        self.alpn = alpn
        self.buf = b""
        self.s_ap_seq = 0

    def _record(self) -> bytes:
        while True:
            if len(self.buf) >= 5:
                ln = struct.unpack(">H", self.buf[3:5])[0]
                if len(self.buf) >= 5 + ln:
                    r, self.buf = self.buf[:5 + ln], self.buf[5 + ln:]
                    return r
            d = self.sock.recv(16384)
            if not d:
                raise EOFError("对端关闭")
            self.buf += d

    def read_app(self) -> bytes:
        """读一条应用数据；连接结束或 alert 时返回空串。"""
        while True:
            try:
                rec = self._record()
            except (EOFError, socket.timeout, OSError):
                return b""
            if rec[0] == 0x14:      # 哑 CCS
                continue
            if rec[0] == 0x15:      # alert
                return b""
            if rec[0] != 0x17:
                return b""
            plain, _t = self.s_ap.open(rec[5:], self.s_ap_seq)
            self.s_ap_seq += 1
            return plain

    def send(self, data: bytes) -> None:
        ct = self.c_ap.seal(data, 23)
        self.sock.sendall(b"\x17\x03\x03" + struct.pack(">H", len(ct)) + ct)

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


def default_alpn(server_alpn):
    return server_alpn


def connect(ip: str, hello: bytes, priv, server_name: str,
            timeout: float = 20.0, verbose: bool = True) -> Conn:
    """用给定的完整 ClientHello 记录做握手。

    `priv` 是与 hello 里 key_share 配对、且长度为 32 的私钥字节。
    """
    from cryptography.hazmat.primitives.asymmetric.x25519 import (
        X25519PrivateKey, X25519PublicKey)

    sock = socket.create_connection((ip, 443), timeout=timeout)
    sock.sendall(hello)
    transcript = hello[5:]  # 去掉 5 字节记录头

    buf = b""

    def record() -> bytes:
        nonlocal buf
        while True:
            if len(buf) >= 5:
                ln = struct.unpack(">H", buf[3:5])[0]
                if len(buf) >= 5 + ln:
                    r, buf = buf[:5 + ln], buf[5 + ln:]
                    return r
            d = sock.recv(16384)
            if not d:
                raise EOFError("对端在 ServerHello 前关闭")
            buf += d

    sh_rec = record()
    if sh_rec[0] == 0x15:
        desc = sh_rec[6] if len(sh_rec) > 6 else 0
        raise SystemExit(f"❌ 服务端 Alert 0x{desc:02x}")
    if sh_rec[0] != 0x16 or sh_rec[5] != 0x02:
        raise SystemExit(f"❌ 不是 ServerHello: {sh_rec[:8].hex()}")
    transcript += sh_rec[5:]

    # ---- 解析 ServerHello
    b = sh_rec[9:]
    o = 2 + 32
    sid_len = b[o]
    o += 1 + sid_len
    suite = struct.unpack(">H", b[o:o + 2])[0]
    o += 2
    o += 1
    ext_len = struct.unpack(">H", b[o:o + 2])[0]
    o += 2
    end = o + ext_len
    server_pub = None
    alpn = None
    while o + 4 <= end:
        et = struct.unpack(">H", b[o:o + 2])[0]
        el = struct.unpack(">H", b[o + 2:o + 4])[0]
        v = b[o + 4:o + 4 + el]
        if et == 51:
            n = struct.unpack(">H", v[2:4])[0]
            server_pub = v[4:4 + n]
        elif et == 16:
            alpn = v[3:3 + v[2]].decode()
        o += 4 + el

    s = Suite(suite)
    if verbose:
        print(f"  ServerHello: suite=0x{suite:04x} hash={s.hash_name} "
              f"key={s.key_len}B alpn={alpn}")

    shared = X25519PrivateKey.from_private_bytes(priv).exchange(
        X25519PublicKey.from_public_bytes(server_pub))

    ks = KeySchedule(s)
    ks.set_shared(shared)
    th_sh = s.H(transcript)

    c_hs = RecordCrypto(s, ks.hs_secret(b"c", th_sh))
    s_hs = RecordCrypto(s, ks.hs_secret(b"s", th_sh))

    # ---- 收加密握手消息直到 Finished
    s_seq = 0
    srv_finished = None
    cert_len = 0
    pending = b""   # 跨记录可能被切开，未消费完的明文
    while True:
        rec = record()
        if rec[0] == 0x14:
            continue
        if rec[0] == 0x15:
            desc = rec[6] if len(rec) > 6 else 0
            raise SystemExit(f"❌ 握手过程中收到 Alert 0x{desc:02x}")
        plain, _t = s_hs.open(rec[5:], s_seq)
        s_seq += 1
        pending += plain
        # 只有「完整」的握手消息才能进 transcript
        while len(pending) >= 4:
            htype = pending[0]
            hlen = (pending[1] << 16) | (pending[2] << 8) | pending[3]
            if len(pending) < 4 + hlen:
                break
            msg = pending[:4 + hlen]
            pending = pending[4 + hlen:]
            transcript += msg
            if htype == 11:
                cert_len = hlen
            elif htype == 20:
                srv_finished = msg[4:]
        if srv_finished is not None:
            break

    # ---- 校验服务端 Finished：密钥调度正确性的强证据
    srv_fin_key = s.expand_label(ks.hs_secret(b"s", th_sh), b"finished", b"", s.hash_len)
    th_before = s.H(transcript[:-4 - len(srv_finished)])
    ok = hmac.new(srv_fin_key, th_before, s.hash_name).digest() == srv_finished
    if verbose:
        print(f"  ✅ 收到 Finished (cert={cert_len}B)  服务端 Finished 校验: "
              f"{'通过' if ok else '不匹配'}")

    # ---- 应用流量密钥
    ks.derive_master()
    th_fin = s.H(transcript)
    c_ap = RecordCrypto(s, ks.ap_secret(b"c", th_fin))
    s_ap = RecordCrypto(s, ks.ap_secret(b"s", th_fin))

    # ---- 客户端 Finished
    c_fin_key = s.expand_label(ks.hs_secret(b"c", th_sh), b"finished", b"", s.hash_len)
    verify = hmac.new(c_fin_key, s.H(transcript), s.hash_name).digest()
    fin = b"\x14" + len(verify).to_bytes(3, "big") + verify
    ct = c_hs.seal(fin, 22)
    sock.sendall(b"\x17\x03\x03" + struct.pack(">H", len(ct)) + ct)

    return Conn(sock, s, c_hs, s_hs, c_ap, s_ap, alpn)


def http_request(conn: Conn, host: str, path: str, headers: dict,
                 limit: int = 200000) -> bytes:
    lines = [f"GET {path} HTTP/1.1", f"Host: {host}"]
    lines += [f"{k}: {v}" for k, v in headers.items()]
    lines.append("Connection: close")
    conn.send(("\r\n".join(lines) + "\r\n\r\n").encode())

    out = b""
    while len(out) < limit:
        chunk = conn.read_app()
        if not chunk:
            break
        out += chunk
    return out
