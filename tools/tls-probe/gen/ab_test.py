"""对照实验：固定 HTTP 头，只改 TLS 指纹，看响应差异。

这是判断「TLS 指纹是不是 1004 的原因」的严格实验：
  组 A —— ClientHello 用真机 OkHttp 模板（15 套件 / 13 扩展 / BoringSSL padding）
  组 B —— ClientHello 用 rustls 风格（10 套件 / 11 扩展 / 无 padding）
  两组用**完全相同**的 HTTP 头、同一个进程、同一个出口 IP，只隔几秒。

如果 A 组 200、B 组 1004 → TLS 指纹是门禁条件。
如果两组都是 1004 → TLS 指纹不是（唯一）条件，1004 另有来源。
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe  # noqa: E402
import tls13  # noqa: E402

from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey  # noqa: E402
from cryptography.hazmat.primitives.serialization import (  # noqa: E402
    Encoding, NoEncryption, PrivateFormat, PublicFormat)


def forge(pcap_path: str, sni: str, mode: str):
    """用一个最小的 forger 生成指定风格的 ClientHello。

    mode: "okhttp" | "rustls"
    """
    import struct

    if mode == "okhttp":
        rec = probe.load_template(pcap_path, sni)
        if rec is None:
            raise SystemExit("找不到真机模板")
        return rec

    # rustls 风格：10 套件、11 扩展、无 padding、ALPN 只有 http/1.1
    suites = [0x1302, 0x1301, 0x1303, 0xc02c, 0xc02b, 0xcca9, 0xc030, 0xc02f, 0xcca8, 0x00ff]
    priv = X25519PrivateKey.generate()
    pub = priv.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)

    def u16(v):
        return struct.pack(">H", v)

    def ext(eid, body):
        return u16(eid) + u16(len(body)) + body

    sni_b = sni.encode()
    # groups 顺序与 rustls 实际一致：x25519, secp256r1, secp384r1
    groups = b"\x00\x06\x00\x1d\x00\x17\x00\x18"
    key_share_entry = b"\x00\x1d" + u16(32) + pub
    ext_list = [
        ext(45, b"\x01\x01"),
        # status_request：TLS1.3 下必须是 status_type + responder_id_list<0> +
        # request_extensions<0>，即 01 00 00 00 00。
        # 写成 01 00 00 01 00（多一个空条目）会被 BoringSSL 判 decode_error(0x32)。
        ext(5, b"\x01\x00\x00\x00\x00"),
        ext(13, b"\x00\x12\x04\x03\x08\x04\x04\x01\x05\x03\x08\x05\x05\x01\x08\x06\x06\x01\x06\x03"),
        ext(11, b"\x01\x00"),
        ext(23, b""),
        ext(10, groups),
        # ALPN 的列表长度要按实际条目算：08 "http/1.1" = 1+8 = 9 字节。
        ext(16, b"\x00\x09\x08http/1.1"),
        ext(43, b"\x04\x03\x04\x03\x03"),
        ext(0, u16(3 + len(sni_b)) + b"\x00" + u16(len(sni_b)) + sni_b),
        # key_share 的结构是：client_shares<长度> -> entry(group, key_len, key)
        ext(51, u16(len(key_share_entry)) + key_share_entry),
        ext(35, b""),
    ]
    exts = b"".join(ext_list)

    sid = os.urandom(32)
    rnd = os.urandom(32)
    body = (b"\x03\x03" + rnd + bytes([len(sid)]) + sid
            + u16(len(suites) * 2) + b"".join(u16(s) for s in suites)
            + b"\x01\x00" + u16(len(exts)) + exts)
    hs = b"\x01" + len(body).to_bytes(3, "big") + body
    rec = b"\x16\x03\x01" + u16(len(hs)) + hs
    return rec, priv


def dechunk(data: bytes) -> bytes:
    """解 HTTP/1.1 chunked 编码。"""
    out = b""
    while True:
        line_end = data.find(b"\r\n")
        if line_end < 0:
            break
        try:
            size = int(data[:line_end].split(b";")[0], 16)
        except ValueError:
            break
        if size == 0:
            break
        start = line_end + 2
        out += data[start:start + size]
        data = data[start + size + 2:]
    return out


def run_group(label, ip, rec, priv, sni):
    priv_b = priv.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption())
    print(f"--- {label} @ {ip} ({len(rec)} 字节 hello) ---")
    try:
        conn = tls13.connect(ip, rec, priv_b, sni)
        body = tls13.http_request(conn, sni, probe.INDEX_PATH, probe.make_headers())
        conn.close()
        # 只取 HTTP 状态行与 JSON 体，避免把二进制噪声打进日志
        idx = body.find(b"HTTP/1.")
        if idx < 0:
            print(f"  未拿到 HTTP 响应，原始前48字节: {body[:48].hex()}")
            print()
            return
        head, _, payload = body[idx:].partition(b"\r\n\r\n")
        status = head.split(b"\r\n")[0].decode("latin1")
        # 服务器用 chunked 编码，不解码的话看到的只是长度前缀（例如 "38"）
        if b"transfer-encoding: chunked" in head.lower():
            payload = dechunk(payload)
        print(f"  HTTP {status}")
        print(f"  响应: {payload.decode('utf-8', 'replace').strip()[:200]}")
    except SystemExit as e:
        print(f"  {e}")
    except Exception as e:
        print(f"  失败: {type(e).__name__}: {e}")
    print()


if __name__ == "__main__":
    pcap = "/tmp/both.pcap"
    if "--pcap" in sys.argv:
        pcap = sys.argv[sys.argv.index("--pcap") + 1]
    sni = "api2.coolapk.com"
    ip = sys.argv[sys.argv.index("--ip") + 1] if "--ip" in sys.argv else "221.204.163.76"

    print("=" * 66)
    print("固定 HTTP 头，只改 TLS ClientHello 指纹")
    print("=" * 66)

    # A 组：真机 OkHttp 模板（用模板自带的 key_share 位置替换公钥）
    okhttp_rec = probe.load_template(pcap, sni)
    off, klen, grp = probe.find_key_share(okhttp_rec)
    priv_a = X25519PrivateKey.generate()
    pub_a = priv_a.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    a = bytearray(okhttp_rec)
    a[off:off + klen] = pub_a
    run_group("A 组: OkHttp 指纹", ip, bytes(a), priv_a, sni)

    # B 组：rustls 风格指纹
    rust_rec, priv_b = forge(pcap, sni, "rustls")
    run_group("B 组: rustls 指纹", ip, rust_rec, priv_b, sni)
