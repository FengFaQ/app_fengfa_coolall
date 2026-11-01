"""决定性实验：用「真机 ClientHello 模板 + 自有 key_share」完成完整 TLS 1.3 握手，
然后真的发出 HTTP 请求，回答「TLS 指纹对齐后 1004 会不会消失」。

与 lead 的 tools/tls-replay2.py 的本质区别：
  replay2 只 recv 一次，看到 0x16/0x02 就报「ServerHello 成功」。
  那只能证明服务端**没有在 TLS 层拒绝**，并不能证明业务层放行。
  本脚本走完握手、发出真实请求、打印业务响应体。

用法：
    python3 probe.py                        # 用 both.pcap 里的模板
    python3 probe.py --hello /tmp/h.bin --key /tmp/k.bin
"""

import glob
import os
import socket
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import tls13

REAL_UA = ("Dalvik/2.1.0 (Linux; U; Android 16; MEIZU 21 "
           "BQ2A.251110.001-BP2A.250605.031.A3) (#Build; meizu; MEIZU 21; "
           "BQ2A.251110.001-BP2A.250605.031.A3; 16) +CoolMarket/16.7.0-2610291-universal")

BASE_HEADERS = {
    "User-Agent": REAL_UA,
    "X-Requested-With": "XMLHttpRequest",
    "X-Sdk-Int": "36",
    "X-Sdk-Locale": "zh-CN",
    "X-App-Id": "com.coolapk.market",
    "X-App-Version": "16.7.0",
    "X-App-Code": "2610291",
    "X-Api-Version": "16",
    "X-Dark-Mode": "0",
    "X-App-Channel": "coolapk",
    "X-App-Mode": "universal",
    "X-App-Supported": "2610291",
    "Accept": "application/json",
}

INDEX_PATH = "/v6/main/indexV8?page=1&firstLaunch=1&installTime=1761926400000"


def find_key_share(rec: bytes):
    """在 ClientHello 记录里定位 key_share 公钥的绝对偏移。"""
    hs = rec[5:]
    p = 4 + 2 + 32
    sid = hs[p]
    p += 1 + sid
    cs = struct.unpack(">H", hs[p:p + 2])[0]
    p += 2 + cs
    comps = hs[p]
    p += 1 + comps
    p += 2
    end = len(hs)
    while p + 4 <= end:
        et = struct.unpack(">H", hs[p:p + 2])[0]
        el = struct.unpack(">H", hs[p + 2:p + 4])[0]
        if et == 51:
            grp = struct.unpack(">H", hs[p + 6:p + 8])[0]
            klen = struct.unpack(">H", hs[p + 8:p + 10])[0]
            return 5 + p + 10, klen, grp
        p += 4 + el
    raise SystemExit("模板里没有 key_share")


def load_template(pcap: str, sni: str):
    import pcap_parse
    for key, segs in pcap_parse.tcp_segments(pcap).items():
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
        got = ""
        for et, ed in ch["extensions"]:
            if et == 0:
                got = ed[5:].decode()
        if got == sni:
            return rec
    return None


def main():
    args = sys.argv[1:]
    sni = "api2.coolapk.com"
    if "--sni" in args:
        sni = args[args.index("--sni") + 1]

    if "--hello" in args:
        rec = open(args[args.index("--hello") + 1], "rb").read()
        print(f"模板来自文件: {len(rec)} 字节")
    else:
        pcap = "/tmp/both.pcap"
        if "--pcap" in args:
            pcap = args[args.index("--pcap") + 1]
        rec = load_template(pcap, sni)
        if not rec:
            raise SystemExit(f"{pcap} 里找不到 sni={sni} 的 ClientHello")
        print(f"模板来自 {pcap}: {len(rec)} 字节")

    off, klen, grp = find_key_share(rec)
    print(f"key_share: 偏移 {off} 长度 {klen} 组 0x{grp:04x}")

    # 生成配套 x25519 密钥对
    from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
    from cryptography.hazmat.primitives.serialization import (
        Encoding, PrivateFormat, PublicFormat, NoEncryption)
    priv = X25519PrivateKey.generate()
    priv_bytes = priv.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption())
    pub = priv.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)

    hello = bytearray(rec)
    hello[off:off + klen] = pub
    hello = bytes(hello)
    print(f"植入自有公钥: {pub.hex()}")
    print()

    for ip in ("221.204.163.76", "119.188.175.46"):
        print(f"=== {ip}:443 ===")
        try:
            conn = tls13.connect(ip, hello, priv_bytes, sni)
            body = tls13.http_request(conn, sni, INDEX_PATH, BASE_HEADERS)
            conn.close()
            head, _, payload = body.partition(b"\r\n\r\n")
            status = head.split(b"\r\n")[0].decode("latin1") if head else "?"
            print(f"  HTTP: {status}")
            print(f"  响应体: {payload[:220].decode('utf-8', 'replace')}")
        except SystemExit as e:
            print(f"  {e}")
        except Exception as e:
            print(f"  失败: {type(e).__name__}: {e}")
        print()


def make_headers():
    """真机 15 个请求头，含 TokenV3 生成的 X-App-Token。"""
    import base64, importlib.util
    spec = importlib.util.spec_from_file_location(
        "tok", "/mnt/c/Users/Administrator/Documents/dsh/Kuan/tools/verify_token_v3.py")
    tm = importlib.util.module_from_spec(spec); spec.loader.exec_module(tm)
    dev = BASE_HEADERS.get("X-App-Device", "")
    h = dict(BASE_HEADERS)
    h["X-App-Token"] = tm.create(dev, 2610291)
    return h


def _old_make_headers():
    """真机 15 个请求头。需要 token 时由调用方补 X-App-Token。"""
    return dict(BASE_HEADERS)


if __name__ == "__main__":
    main()
