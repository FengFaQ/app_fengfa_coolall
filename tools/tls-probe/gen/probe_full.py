"""用「真机指纹 + 真机完整请求头（含 TokenV3 签名）」打 API。

前面的 ab_test.py 只带了 13 个静态头，缺 X-App-Token / X-App-Device / Cookie，
那本来就是必 1004 的。本脚本补齐，用 coolall-core 的 token_v3 实现签名。

用法：
    python3 probe_full.py                 # 两组都跑
    python3 probe_full.py --only okhttp
"""

import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import probe  # noqa: E402
import tls13  # noqa: E402
from ab_test import dechunk, forge  # noqa: E402

from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey  # noqa: E402
from cryptography.hazmat.primitives.serialization import (  # noqa: E402
    Encoding, NoEncryption, PrivateFormat, PublicFormat)

# 真机设备串明文与 SDK（来自 adb 导出的 datastore）
REAL_PLAIN = "; ; ; ; meizu; meizu; MEIZU 21; BQ2A.251110.001-BP2A.250605.031.A3; null"
REAL_SDK = "36"
APP_CODE = 2610291


def rust_token_create(device: str, app_code: int, ts: int) -> str:
    """调 tools/tls-probe/tokencli —— 底层复用 rs-probe 里已交叉验证的 TokenV3 实现。"""
    exe = "/tmp/tokencli-target/release/tokencli"
    if not os.path.exists(exe):
        raise SystemExit(
            f"找不到 {exe}，先编译：\n"
            "  cd tools/tls-probe/tokencli && "
            "CARGO_TARGET_DIR=/tmp/tokencli-target cargo build --release")
    out = subprocess.run([exe, device, str(app_code), str(ts)],
                         capture_output=True, text=True)
    if out.returncode != 0:
        raise SystemExit(f"token 生成失败: {out.stderr.strip()}")
    return out.stdout.strip()


def encode_device(plain: str) -> str:
    import base64
    b = base64.b64encode(plain.encode()).decode().rstrip("=")
    return b[::-1].replace("\r", "").replace("\n", "").replace("=", "")


def full_headers(device_header: str, token: str) -> dict:
    h = dict(probe.BASE_HEADERS)
    h["X-App-Device"] = device_header
    h["X-App-Token"] = token
    return h


def main():
    pcap = "/tmp/both.pcap"
    sni = "api2.coolapk.com"
    ip = "221.204.163.76"
    only = None
    if "--only" in sys.argv:
        only = sys.argv[sys.argv.index("--only") + 1]

    device = encode_device(REAL_PLAIN)
    print(f"X-App-Device = {device}")

    import time
    token = rust_token_create(device, APP_CODE, int(time.time()))
    print(f"X-App-Token  = {token[:48]}...")
    print()

    if only in (None, "okhttp"):
        rec = probe.load_template(pcap, sni)
        off, klen, _grp = probe.find_key_share(rec)
        p = X25519PrivateKey.generate()
        pub = p.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        a = bytearray(rec)
        a[off:off + klen] = pub
        run("A 组: OkHttp 指纹 + 完整头", ip, bytes(a), p, sni, full_headers(device, token))

    if only in (None, "rustls"):
        rrec, p2 = forge(pcap, sni, "rustls")
        run("B 组: rustls 指纹 + 完整头", ip, rrec, p2, sni, full_headers(device, token))


def run(label, ip, rec, priv, sni, headers):
    priv_b = priv.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption())
    print(f"--- {label} ({len(rec)} 字节 hello) ---")
    try:
        conn = tls13.connect(ip, rec, priv_b, sni)
        body = tls13.http_request(conn, sni, probe.INDEX_PATH, headers)
        conn.close()
        idx = body.find(b"HTTP/1.")
        if idx < 0:
            print(f"  未拿到 HTTP 响应: {body[:48].hex()}")
            return
        head, _, payload = body[idx:].partition(b"\r\n\r\n")
        status = head.split(b"\r\n")[0].decode("latin1")
        # 服务器会下发 Set-Cookie，打出来看会话是否被建立
        cookies = [l for l in head.split(b"\r\n") if l.lower().startswith(b"set-cookie")]
        if b"transfer-encoding: chunked" in head.lower():
            payload = dechunk(payload)
        print(f"  HTTP {status}")
        for c in cookies:
            print(f"  {c.decode('latin1')[:100]}")
        print(f"  响应: {payload.decode('utf-8', 'replace').strip()[:200]}")
    except SystemExit as e:
        print(f"  {e}")
    except Exception as e:
        print(f"  失败: {type(e).__name__}: {e}")
    print()


if __name__ == "__main__":
    main()
