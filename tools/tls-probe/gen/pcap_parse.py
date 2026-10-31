"""从 pcap 里抽取 ClientHello 并打印可读指纹。

不依赖 scapy / tshark，只用标准库，便于在 WSL 里直接跑。

用法：python3 pcap_parse.py <pcap> [--sni host]
"""
import struct
import sys
import collections

GT = {
    0x0000: "server_name",
    0x0005: "status_request",
    0x000a: "supported_groups",
    0x000b: "ec_point_formats",
    0x000d: "signature_algorithms",
    0x0010: "alpn",
    0x0015: "padding",
    0x0016: "encrypt_then_mac",
    0x0017: "extended_master_secret",
    0x001b: "compression_certificate",
    0x0023: "session_ticket",
    0x002b: "supported_versions",
    0x002d: "psk_key_exchange_modes",
    0x0033: "key_share",
    0x0029: "pre_shared_key",
    0x0002: "client_certificate_type",
    0x11ec: "application_settings(ALPS)",
    0xfe0d: "encrypted_client_hello(ECH)",
    0x7540: "delegated_credentials",
    0x4469: "next_protocol_negotiation",
    0x0012: "signed_certificate_timestamp",
    0x755f: "renegotiation_info",
    0xff01: "renegotiation_info",
    0x3374: "channel_id",
    0x1c02: "psk_dhe_ke?",
}
CS = {
    0x1301: "TLS_AES_128_GCM_SHA256",
    0x1302: "TLS_AES_256_GCM_SHA384",
    0x1303: "TLS_CHACHA20_POLY1305_SHA256",
    0xc02b: "TLS_ECDHE_ECDSA_AES128_GCM_SHA256",
    0xc02c: "TLS_ECDHE_ECDSA_AES256_GCM_SHA384",
    0xc02f: "TLS_ECDHE_RSA_AES128_GCM_SHA256",
    0xc030: "TLS_ECDHE_RSA_AES256_GCM_SHA384",
    0xcca9: "TLS_ECDHE_ECDSA_CHACHA20_POLY1305",
    0xcca8: "TLS_ECDHE_RSA_CHACHA20_POLY1305",
    0xccaa: "TLS_DHE_RSA_CHACHA20_POLY1305",
    0x009c: "TLS_RSA_AES128_GCM_SHA256",
    0x009d: "TLS_RSA_AES256_GCM_SHA384",
    0xc013: "TLS_ECDHE_RSA_AES128_CBC_SHA",
    0xc014: "TLS_ECDHE_RSA_AES256_CBC_SHA",
    0xc009: "TLS_ECDHE_ECDSA_AES128_CBC_SHA",
    0xc00a: "TLS_ECDHE_ECDSA_AES256_CBC_SHA",
    0x009e: "TLS_DHE_RSA_AES128_GCM_SHA256",
    0x009f: "TLS_DHE_RSA_AES256_GCM_SHA384",
    0x0033: "TLS_DHE_RSA_AES128_CBC_SHA",
    0x0067: "TLS_DHE_RSA_AES128_CBC_SHA256",
    0x006b: "TLS_DHE_RSA_AES256_CBC_SHA256",
    0x0045: "TLS_DHE_RSA_AES128_CBC_SHA",
    0xc00f: "TLS_ECDH_RSA_AES128_CBC_SHA",
    0xc00e: "TLS_ECDH_ECDSA_AES128_CBC_SHA",
    0xc012: "TLS_ECDH_RSA_AES256_CBC_SHA",
    0xc011: "TLS_ECDH_ECDSA_AES256_CBC_SHA",
    0x0009: "TLS_RSA_DES_CBC_SHA",
    0x000a: "TLS_RSA_3DES_CBC_SHA",
    0x0016: "TLS_DHE_RSA_3DES_CBC_SHA",
    0x0013: "TLS_DHE_DSS_3DES_CBC_SHA",
    0xc00d: "TLS_ECDH_ECDSA_3DES_CBC_SHA",
    0xc007: "TLS_ECDHE_ECDSA_3DES_CBC_SHA",
    0xc00c: "TLS_ECDH_RSA_3DES_CBC_SHA",
    0xc008: "TLS_ECDHE_RSA_3DES_CBC_SHA",
}
GROUPS = {
    0x0017: "secp256r1",
    0x0018: "secp384r1",
    0x0019: "secp521r1",
    0x001d: "x25519",
    0x001e: "x448",
    0x0100: "ffdhe2048",
    0x0101: "ffdhe3072",
    0x0102: "ffdhe4096",
    0x0103: "ffdhe6144",
    0x0104: "ffdhe8192",
    0x11ec: "X25519MLKEM768",
}


def read_pcap(path):
    with open(path, "rb") as f:
        data = f.read()
    magic = data[:4]
    if magic in (b"\xd4\xc3\xb2\xa1", b"\x4d\x3c\xb2\xa1"):
        endian = "<"
        ver = 2
    elif magic in (b"\xa1\xb2\xc3\xd4", b"\xa1\xb2\x3c\x4d"):
        endian = ">"
        ver = 2
    else:
        raise SystemExit("不是 libpcap 文件（可能是 pcapng）")
    _magic, _vmaj, _vmin, _tz, _sig, _snap, linktype = struct.unpack(
        endian + "IHHiIII", data[:24])
    off = 24
    out = []
    while off + 16 <= len(data):
        ts, tus, incl, orig = struct.unpack(endian + "IIII", data[off:off + 16])
        off += 16
        out.append((linktype, data[off:off + incl]))
        off += incl
    return out


def l3_payload(linktype, pkt):
    """剥掉链路层，返回 (ip_payload, proto, sport, dport, direction_hint)"""
    if linktype == 113:  # LINUX_SLL2
        # 头部 20 字节：proto(2) reserved(2) ifindex(4) hatype(2) pkttype(1) halen(1) addr(8)
        proto = struct.unpack(">H", pkt[0:2])[0]
        return pkt[20:], proto
    if linktype == 1:  # EN10MB
        et = struct.unpack(">H", pkt[12:14])[0]
        return pkt[14:], et
    if linktype == 276:  # LINUX_SLL (v1)，由于抓包端 colinfo 偏移为 20，头实际为 20 字节
        proto = struct.unpack(">H", pkt[0:2])[0]
        return pkt[20:], proto
    if linktype == 101:  # RAW IP
        return pkt, 0x0800
    return b"", 0


def tcp_segments(path):
    pk = {}
    for linktype, pkt in read_pcap(path):
        ip, _proto = l3_payload(linktype, pkt)
        if not ip:
            continue
        vihl = ip[0]
        if vihl >> 4 != 4:
            continue
        ihl = (vihl & 0xF) * 4
        if ip[9] != 6:
            continue
        src = ".".join(str(b) for b in ip[12:16])
        dst = ".".join(str(b) for b in ip[16:20])
        tot = struct.unpack(">H", ip[2:4])[0]
        tcp = ip[ihl:tot]
        if len(tcp) < 20:
            continue
        sport, dport, seq = struct.unpack(">HHI", tcp[0:8])
        doff = (tcp[12] >> 4) * 4
        flags = tcp[13]
        payload = tcp[doff:]
        if not payload:
            continue
        key = (src, sport, dst, dport)
        pk.setdefault(key, []).append((seq, payload))
    return pk


def tls_client_hellos(path):
    res = []
    for key, segs in tcp_segments(path).items():
        # 只看发往 443 的方向
        if key[3] != 443:
            continue
        segs.sort(key=lambda x: x[0])
        buf = b"".join(s[1] for s in segs)
        # 可能被重传打乱，做贪心扫描找 record 0x16 03 01
        i = 0
        while True:
            i = buf.find(b"\x16\x03\x01", i)
            if i < 0:
                break
            try:
                rec_len = struct.unpack(">H", buf[i + 3:i + 5])[0]
                hs = buf[i + 5:i + 5 + rec_len]
                if len(hs) < 4 or hs[0] != 1:
                    i += 1
                    continue
                hs_len = int.from_bytes(hs[1:4], "big")
                body = hs[4:4 + hs_len]
                if len(body) < hs_len:
                    i += 1
                    continue
                res.append((key, parse_client_hello(body)))
                break
            except Exception:
                i += 1
    return res


def parse_client_hello(b):
    o = 0
    legacy_ver = struct.unpack(">H", b[o:o + 2])[0]
    o += 2
    rand = b[o:o + 32]
    o += 32
    sid_len = b[o]
    o += 1 + sid_len
    cs_len = struct.unpack(">H", b[o:o + 2])[0]
    o += 2
    suites = [struct.unpack(">H", b[o + k:o + k + 2])[0] for k in range(0, cs_len, 2)]
    o += cs_len
    comp_len = b[o]
    o += 1
    comps = list(b[o:o + comp_len])
    o += comp_len
    exts = []
    if o + 2 <= len(b):
        ext_total = struct.unpack(">H", b[o:o + 2])[0]
        o += 2
        end = o + ext_total
        while o + 4 <= min(end, len(b)):
            et = struct.unpack(">H", b[o:o + 2])[0]
            el = struct.unpack(">H", b[o + 2:o + 4])[0]
            ed = b[o + 4:o + 4 + el]
            exts.append((et, ed))
            o += 4 + el
    return {
        "legacy_version": legacy_ver,
        "random": rand,
        "session_id_len": sid_len,
        "suites": suites,
        "compressions": comps,
        "extensions": exts,
    }


def show(ch, label):
    print("=" * 78)
    print(f"# {label}")
    print(f"legacy_version = 0x{ch['legacy_version']:04x}")
    print(f"session_id_len = {ch['session_id_len']}")
    print(f"compressions   = {ch['compressions']}")
    print(f"套件数 = {len(ch['suites'])}")
    for k, s in enumerate(ch["suites"]):
        print(f"  [{k:2}] 0x{s:04x} {CS.get(s, '?')}")
    print(f"扩展数 = {len(ch['extensions'])}")
    print("扩展顺序:")
    names = []
    for et, ed in ch["extensions"]:
        nm = GT.get(et, "?")
        detail = ""
        if et == 0x000a:  # supported_groups
            n = struct.unpack(">H", ed[0:2])[0]
            gs = [struct.unpack(">H", ed[2 + i:4 + i])[0] for i in range(0, n, 2)]
            detail = "[" + ", ".join(GROUPS.get(g, f"0x{g:04x}") for g in gs) + "]"
        elif et == 0x0010:  # alpn
            detail = repr(ed[2:].decode("latin1"))
        elif et == 0x0000:  # sni
            detail = repr(ed[5:].decode("latin1"))
        elif et == 0x000d:  # sigalgs
            n = struct.unpack(">H", ed[0:2])[0]
            detail = f"({n // 2}个)"
        elif et == 0x002b:  # supported_versions
            detail = repr(list(ed[1:]))
        elif et == 0x000b:
            detail = repr(list(ed[1:]))
        names.append(f"{et}({nm})")
        print(f"  {et:5d} 0x{et:04x} {nm:32} len={len(ed):3d} {detail}")
    print("扩展 id 序列: " + ", ".join(str(e) for e, _ in ch["extensions"]))


def ja3(ch):
    import hashlib
    s = ",".join(
        [str(ch["legacy_version"]), "-".join(str(x) for x in ch["suites"]),
         "-".join(str(x) for x in ch["extensions"])]
    )
    gs = ""
    sa = ""
    for et, ed in ch["extensions"]:
        if et == 0x000a:
            n = struct.unpack(">H", ed[0:2])[0]
            gs = "-".join(str(struct.unpack(">H", ed[2 + i:4 + i])[0]) for i in range(0, n, 2))
        if et == 0x000d:
            n = struct.unpack(">H", ed[0:2])[0]
            sa = "-".join(str(struct.unpack(">H", ed[2 + i:4 + i])[0]) for i in range(0, n, 2))
    full = f"{s},{gs},{sa}"
    return full, hashlib.md5(full.encode()).hexdigest()


if __name__ == "__main__":
    path = sys.argv[1]
    sni_filter = None
    if "--sni" in sys.argv:
        sni_filter = sys.argv[sys.argv.index("--sni") + 1]
    seen = collections.Counter()
    for key, ch in tls_client_hellos(path):
        sni = ""
        for et, ed in ch["extensions"]:
            if et == 0x0000:
                sni = ed[5:].decode("latin1")
        fp, h = ja3(ch)
        if sni_filter and sni_filter not in sni:
            continue
        if seen[h]:
            seen[h] += 1
            continue
        seen[h] += 1
        show(ch, f"{key[0]}:{key[1]} -> {key[2]}:{key[3]}  sni={sni!r}  ja3={h}")
    print("=" * 78)
    print("去重统计:", dict(seen))
