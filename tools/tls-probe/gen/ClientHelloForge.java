// ClientHelloForge.java —— 逐字节复现 Android/OkHttp 的 TLS ClientHello
//
// 为什么是 Java：JDK 17 自带 X25519 / AES-GCM / HKDF / X.509，零第三方依赖，
// 因此可以在不联网拉 crates 的前提下，把「服务端看到的那串字节」完全等价地重放出来。
// 生成出来的 ClientHello 交给 Rust 侧（tools/tls-probe）当字节流过 TLS 引擎使用，
// 用于回答一个问题：把 TLS 指纹改成和酷安 App 一样，1004 会不会消失。
//
// 设计要点：
//   1. 所有「可变」字段（random / session_id / key_share / 填充）都可从命令行注入，
//      这样能把真机抓包里的原始 ClientHello 原样重放（重放实验）。
//   2. 默认模式自己生成随机数，产出一条结构上与真机完全一致的 ClientHello。

import java.io.ByteArrayOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyFactory;
import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.security.spec.X509EncodedKeySpec;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

public final class ClientHelloForge {

    // ------------------------------------------------------------------ 常量

    /** 真机（c001apk / OkHttp + Conscrypt）的完整套件列表，顺序来自 pcap。 */
    static final int[] REAL_SUITES = {
            0x1301, 0x1302, 0x1303,
            0xc02b, 0xc02c, 0xcca9,
            0xc02f, 0xc030, 0xcca8,
            0xc013, 0xc014, 0x009c, 0x009d,
            0x002f, 0x0035,
    };

    /** 真机的分组顺序。 */
    static final int[] REAL_GROUPS = {0x001d, 0x0017, 0x0018}; // x25519, secp256r1, secp384r1

    /** 真机的签名算法（9 个）。 */
    static final int[] REAL_SIGALGS = {
            0x0403, 0x0804, 0x0401, 0x0503, 0x0805,
            0x0501, 0x0806, 0x0601, 0x0603,
    };

    /** 真机 ALPN 顺序：h2 在前。 */
    static final String[] REAL_ALPN = {"h2", "http/1.1"};

    static final int EXT_SERVER_NAME = 0;
    static final int EXT_STATUS_REQUEST = 5;
    static final int EXT_SUPPORTED_GROUPS = 10;
    static final int EXT_EC_POINT_FORMATS = 11;
    static final int EXT_SIGNATURE_ALGORITHMS = 13;
    static final int EXT_ALPN = 16;
    static final int EXT_SESSION_TICKET = 35;
    static final int EXT_SUPPORTED_VERSIONS = 43;
    static final int EXT_PSK_KEY_EXCHANGE_MODES = 45;
    static final int EXT_KEY_SHARE = 51;
    static final int EXT_PADDING = 21;
    static final int EXT_EXTENDED_MASTER_SECRET = 23;
    static final int EXT_RENEGOTIATION_INFO = 0xff01;

    /** 真机扩展顺序（13 个），逐字来自抓包。 */
    static final int[] REAL_EXT_ORDER = {
            EXT_SERVER_NAME,             // 0
            EXT_EXTENDED_MASTER_SECRET,  // 23
            EXT_RENEGOTIATION_INFO,      // 65281
            EXT_SUPPORTED_GROUPS,        // 10
            EXT_EC_POINT_FORMATS,        // 11
            EXT_SESSION_TICKET,          // 35
            EXT_ALPN,                    // 16
            EXT_STATUS_REQUEST,          // 5
            EXT_SIGNATURE_ALGORITHMS,    // 13
            EXT_KEY_SHARE,               // 51
            EXT_PSK_KEY_EXCHANGE_MODES,  // 45
            EXT_SUPPORTED_VERSIONS,      // 43
            EXT_PADDING,                 // 21
    };

    /**
     * 真机的 ClientHello 长度（含 4 字节 handshake 头，不含 5 字节记录头）。
     *
     * 实测反推（见 pcap_parse.py）：13 个扩展那条 hello 恒为 512，多出的空间由 padding 扩展吃掉。
     *
     *   SNI api2.coolapk.com  (16) → padding 237
     *   SNI api.coolapk.com   (15) → padding 238
     *   SNI image.coolapk.com (17) → padding 236
     *   SNI avatar.coolapk.com(18) → padding 235
     *
     * 也就是「padding 让 ClientHello 恰好 512 字节」的经典防探测做法，这里按同一规则复现。
     */
    static final int HELLO_PADDING_TARGET = 512;

    // ------------------------------------------------------------------ 编码工具

    static void u8(ByteArrayOutputStream o, int v) {
        o.write(v & 0xFF);
    }

    static void u16(ByteArrayOutputStream o, int v) {
        o.write((v >>> 8) & 0xFF);
        o.write(v & 0xFF);
    }

    static void u24(ByteArrayOutputStream o, int v) {
        o.write((v >>> 16) & 0xFF);
        o.write((v >>> 8) & 0xFF);
        o.write(v & 0xFF);
    }

    static void vec8(ByteArrayOutputStream o, byte[] b) {
        u8(o, b.length);
        o.write(b, 0, b.length);
    }

    static void vec16(ByteArrayOutputStream o, byte[] b) {
        u16(o, b.length);
        o.write(b, 0, b.length);
    }

    static void u16List(ByteArrayOutputStream o, int[] vals) {
        u16(o, vals.length * 2);
        for (int v : vals) {
            u16(o, v);
        }
    }

    // ------------------------------------------------------------------ 注入参数

    static final class Inject {
        byte[] random;
        byte[] sessionId;
        int[] suites;
        int[] extOrder;
        String[] alpn;
        byte[] keySharePub;
        byte[] privKey;
        Integer paddingPad;
    }

    // ------------------------------------------------------------------ 主流程

    public static void main(String[] args) throws Exception {
        Map<String, String> opt = new TreeMap<>();
        for (int i = 0; i < args.length; i++) {
            if (args[i].startsWith("--")) {
                String k = args[i].substring(2);
                String v = (i + 1 < args.length && !args[i + 1].startsWith("--"))
                        ? args[++i] : "1";
                opt.put(k, v);
            }
        }

        if (opt.containsKey("inspect")) {
            inspect(opt.get("inspect"));
            return;
        }

        String sni = opt.getOrDefault("sni", "api2.coolapk.com");
        String out = opt.get("out");
        String keyOut = opt.get("key-out");
        String suiteOrder = opt.getOrDefault("suites", "okhttp");
        String extOrder = opt.getOrDefault("ext-order", "okhttp");
        String padMode = opt.getOrDefault("padding", "boring");

        Inject inj = new Inject();
        if (opt.containsKey("random-hex")) {
            inj.random = HexFormat.of().parseHex(opt.get("random-hex"));
        }
        if (opt.containsKey("session-id-hex")) {
            inj.sessionId = HexFormat.of().parseHex(opt.get("session-id-hex"));
        }
        if (opt.containsKey("key-share-hex")) {
            inj.keySharePub = HexFormat.of().parseHex(opt.get("key-share-hex"));
        }
        if (opt.containsKey("priv-key-hex")) {
            inj.privKey = HexFormat.of().parseHex(opt.get("priv-key-hex"));
        }

        switch (suiteOrder) {
            case "okhttp":
                inj.suites = REAL_SUITES;
                break;
            case "none":
                inj.suites = new int[0];
                break;
            case "rustls":
                inj.suites = new int[]{
                        0x1302, 0x1301, 0x1303, 0xc02c, 0xc02b, 0xcca9,
                        0xc030, 0xc02f, 0xcca8, 0x00ff};
                break;
            default:
                throw new IllegalArgumentException("未知 --suites " + suiteOrder);
        }

        switch (extOrder) {
            case "okhttp":
                inj.extOrder = REAL_EXT_ORDER;
                break;
            case "rustls":
                inj.extOrder = new int[]{45, 5, 13, 11, 23, 10, 16, 43, 0, 51, 35};
                break;
            case "none":
                inj.extOrder = new int[0];
                break;
            default:
                throw new IllegalArgumentException("未知 --ext-order " + extOrder);
        }

        boolean offerH2 = !opt.containsKey("no-h2");
        inj.alpn = offerH2 ? REAL_ALPN : new String[]{"http/1.1"};

        switch (padMode) {
            case "boring":
                inj.paddingPad = HELLO_PADDING_TARGET;
                break;
            case "none":
                inj.paddingPad = -1;
                break;
            default:
                inj.paddingPad = Integer.parseInt(padMode);
                break;
        }

        SecureRandom rng = new SecureRandom();
        KeyPair kp = generateX25519();
        byte[] pub = rawX25519Public(kp.getPublic());
        byte[] priv = x25519PrivateScalar(kp.getPrivate());

        if (inj.random == null) {
            inj.random = new byte[32];
            rng.nextBytes(inj.random);
        }
        if (inj.sessionId == null) {
            inj.sessionId = new byte[32];
            rng.nextBytes(inj.sessionId);
        }
        if (inj.keySharePub == null) {
            inj.keySharePub = pub;
            inj.privKey = priv;
        }

        byte[] hello = buildClientHello(inj, sni);

        System.out.println("sni          = " + sni);
        System.out.println("suites       = " + inj.suites.length);
        System.out.println("ext order    = " + Arrays.toString(inj.extOrder));
        System.out.println("wire length  = " + hello.length);
        System.out.println("hello length = " + (hello.length - 5));
        System.out.println("hello sha256 = " + sha256Hex(hello));
        System.out.println("key_share    = " + HexFormat.of().formatHex(inj.keySharePub));

        if (out != null) {
            Files.write(Path.of(out), hello);
            System.out.println("已写入 hello: " + out);
        }
        if (keyOut != null) {
            Files.write(Path.of(keyOut), inj.privKey);
            System.out.println("已写入私钥: " + keyOut);
        }
    }

    static void inspect(String path) throws Exception {
        byte[] hello = Files.readAllBytes(Path.of(path));
        Parsed p = parseClientHello(hello);
        System.out.println("文件           = " + path);
        System.out.println("wire length    = " + hello.length);
        System.out.println("handshake len  = " + (hello.length - 5));
        System.out.println("random         = " + HexFormat.of().formatHex(p.random));
        System.out.println("session_id     = " + HexFormat.of().formatHex(p.sessionId));
        System.out.println("key_share      = " + HexFormat.of().formatHex(p.keyShare));
        System.out.println("suites         = " + Arrays.toString(p.suites));
        System.out.println("ext_order      = " + Arrays.toString(p.extOrder));
        System.out.println("padding        = " + p.paddingLen);
        System.out.println("sni            = " + p.sni);
        System.out.println("sha256         = " + sha256Hex(hello));
    }

    // ------------------------------------------------------------------ X25519

    static KeyPair generateX25519() throws Exception {
        KeyPairGenerator g = KeyPairGenerator.getInstance("X25519");
        return g.generateKeyPair();
    }

    /** 取出 X.509 SubjectPublicKeyInfo 末尾的 32 字节原始公钥。 */
    static byte[] rawX25519Public(java.security.PublicKey pk) {
        byte[] enc = pk.getEncoded();
        return Arrays.copyOfRange(enc, enc.length - 32, enc.length);
    }

    /** 取出 PKCS#8 末尾的 32 字节私钥标量。 */
    static byte[] x25519PrivateScalar(java.security.PrivateKey sk) {
        byte[] enc = sk.getEncoded();
        return Arrays.copyOfRange(enc, enc.length - 32, enc.length);
    }

    /** 用注入的 32 字节原始公钥重建 X25519 公钥对象。 */
    static java.security.PublicKey importX25519Public(byte[] raw) throws Exception {
        byte[] prefix = HexFormat.of().parseHex("302a300506032b656e032100");
        byte[] spki = new byte[prefix.length + raw.length];
        System.arraycopy(prefix, 0, spki, 0, prefix.length);
        System.arraycopy(raw, 0, spki, prefix.length, raw.length);
        return KeyFactory.getInstance("X25519")
                .generatePublic(new X509EncodedKeySpec(spki));
    }

    // ------------------------------------------------------------------ 组装 ClientHello

    static byte[] buildClientHello(Inject inj, String sni) throws Exception {
        byte[] extsNoPad = encodeExtensions(inj, sni, -1);
        int paddingLen = computePadding(inj, extsNoPad.length);
        byte[] exts = encodeExtensions(inj, sni, paddingLen);

        ByteArrayOutputStream body = new ByteArrayOutputStream();
        u16(body, 0x0303);
        body.write(inj.random, 0, 32);
        vec8(body, inj.sessionId);
        u16List(body, inj.suites);
        u8(body, 1);
        u8(body, 0);
        u16(body, exts.length);
        body.write(exts, 0, exts.length);

        byte[] b = body.toByteArray();

        ByteArrayOutputStream out = new ByteArrayOutputStream();
        u8(out, 0x16);
        u16(out, 0x0301);
        u16(out, b.length + 4);
        u8(out, 0x01);
        u24(out, b.length);
        out.write(b, 0, b.length);
        return out.toByteArray();
    }

    /**
     * 计算 padding 长度。
     *
     * `inj.paddingPad` 的语义：
     *   = -1   → 不加 padding 扩展
     *   >= 512 → 视为「目标 ClientHello 总长」，padding 取差值（真机行为）
     *   < 512  → 视为「padding 本身的字节数」（手工指定）
     */
    static int computePadding(Inject inj, int extsNoPadLen) {
        if (inj.paddingPad == null || inj.paddingPad < 0) {
            return -1;
        }
        if (inj.paddingPad < HELLO_PADDING_TARGET) {
            return inj.paddingPad;
        }
        int withPadHeader = extsNoPadLen + 4;
        int helloLen = 4 + 2 + 32 + 1 + inj.sessionId.length
                + 2 + inj.suites.length * 2 + 2 + 1 + 2 + withPadHeader;
        return Math.max(0, inj.paddingPad - helloLen);
    }

    /**
     * status_request 扩展体。
     *
     * 真机抓包是 `01 00 00 00 00`：
     *   status_type = 1 (ocsp)
     *   responder_id_list<0..2^16-1> 长度 = 0
     *   request_extensions<0..2^16-1> 长度 = 0
     *
     * 注意不是 Chromium 常见的 `01 00 00 01 00`（那会带一个空的
     * CertificateStatusRequestItemV2 条目）。少这 1 个字节，整条 hello 就只有 511，
     * 与真机的 512 对不上。
     */
    static final byte[] REAL_STATUS_REQUEST = {1, 0, 0, 0, 0};

    static byte[] encodeExtensions(Inject inj, String sni, int paddingLen)
            throws Exception {
        Map<Integer, byte[]> byId = new TreeMap<>();
        byId.put(EXT_SERVER_NAME, encodeSni(sni));
        byId.put(EXT_EXTENDED_MASTER_SECRET, new byte[0]);
        byId.put(EXT_RENEGOTIATION_INFO, new byte[]{0});
        byId.put(EXT_SUPPORTED_GROUPS, encodeSupportedGroups());
        byId.put(EXT_EC_POINT_FORMATS, new byte[]{1, 0});
        byId.put(EXT_SESSION_TICKET, new byte[0]);
        byId.put(EXT_ALPN, encodeAlpn(inj.alpn));
        byId.put(EXT_STATUS_REQUEST, REAL_STATUS_REQUEST);
        byId.put(EXT_SIGNATURE_ALGORITHMS, encodeSigAlgs());
        byId.put(EXT_KEY_SHARE, encodeKeyShare(inj.keySharePub));
        byId.put(EXT_PSK_KEY_EXCHANGE_MODES, new byte[]{1, 1});
        byId.put(EXT_SUPPORTED_VERSIONS, new byte[]{4, 3, 4, 3, 3});
        if (paddingLen >= 0) {
            byId.put(EXT_PADDING, new byte[paddingLen]);
        }

        ByteArrayOutputStream o = new ByteArrayOutputStream();
        for (int id : inj.extOrder) {
            byte[] v = byId.remove(id);
            if (v == null) {
                continue;
            }
            u16(o, id);
            vec16(o, v);
        }
        // 顺序表里没提到的扩展按 id 升序补在后面
        for (Map.Entry<Integer, byte[]> e : byId.entrySet()) {
            u16(o, e.getKey());
            vec16(o, e.getValue());
        }
        return o.toByteArray();
    }

    static byte[] encodeSni(String host) {
        byte[] h = host.getBytes(StandardCharsets.US_ASCII);
        ByteArrayOutputStream entry = new ByteArrayOutputStream();
        u8(entry, 0);
        vec16(entry, h);
        ByteArrayOutputStream o = new ByteArrayOutputStream();
        vec16(o, entry.toByteArray());
        return o.toByteArray();
    }

    static byte[] encodeSupportedGroups() {
        ByteArrayOutputStream o = new ByteArrayOutputStream();
        u16List(o, REAL_GROUPS);
        return o.toByteArray();
    }

    static byte[] encodeSigAlgs() {
        ByteArrayOutputStream o = new ByteArrayOutputStream();
        u16List(o, REAL_SIGALGS);
        return o.toByteArray();
    }

    static byte[] encodeAlpn(String[] protos) {
        ByteArrayOutputStream list = new ByteArrayOutputStream();
        for (String p : protos) {
            vec8(list, p.getBytes(StandardCharsets.US_ASCII));
        }
        ByteArrayOutputStream o = new ByteArrayOutputStream();
        vec16(o, list.toByteArray());
        return o.toByteArray();
    }

    static byte[] encodeKeyShare(byte[] pub) {
        ByteArrayOutputStream entry = new ByteArrayOutputStream();
        u16(entry, 0x001d);
        vec16(entry, pub);
        ByteArrayOutputStream o = new ByteArrayOutputStream();
        vec16(o, entry.toByteArray());
        return o.toByteArray();
    }

    // ------------------------------------------------------------------ 解析（自检 + 重放）

    static final class Parsed {
        byte[] random;
        byte[] sessionId;
        int[] suites;
        int[] extOrder;
        byte[] keyShare;
        int paddingLen = -1;
        String sni;
    }

    static Parsed parseClientHello(byte[] rec) {
        Parsed p = new Parsed();
        int o = 5;
        o += 4;
        o += 2;
        p.random = Arrays.copyOfRange(rec, o, o + 32);
        o += 32;
        int sidLen = rec[o] & 0xFF;
        o += 1;
        p.sessionId = Arrays.copyOfRange(rec, o, o + sidLen);
        o += sidLen;
        int csLen = ((rec[o] & 0xFF) << 8) | (rec[o + 1] & 0xFF);
        o += 2;
        p.suites = new int[csLen / 2];
        for (int i = 0; i < csLen / 2; i++) {
            p.suites[i] = ((rec[o + i * 2] & 0xFF) << 8) | (rec[o + i * 2 + 1] & 0xFF);
        }
        o += csLen;
        int compLen = rec[o] & 0xFF;
        o += 1 + compLen;
        int extTotal = ((rec[o] & 0xFF) << 8) | (rec[o + 1] & 0xFF);
        o += 2;
        int end = o + extTotal;
        List<Integer> order = new ArrayList<>();
        while (o + 4 <= end) {
            int id = ((rec[o] & 0xFF) << 8) | (rec[o + 1] & 0xFF);
            int len = ((rec[o + 2] & 0xFF) << 8) | (rec[o + 3] & 0xFF);
            byte[] v = Arrays.copyOfRange(rec, o + 4, o + 4 + len);
            if (id == EXT_SERVER_NAME) {
                int n = ((v[1] & 0xFF) << 8) | (v[2] & 0xFF);
                p.sni = new String(v, 3, n, StandardCharsets.US_ASCII);
            } else if (id == EXT_KEY_SHARE) {
                int n = ((v[3] & 0xFF) << 8) | (v[4] & 0xFF);
                p.keyShare = Arrays.copyOfRange(v, 5, 5 + n);
            } else if (id == EXT_PADDING) {
                p.paddingLen = len;
            }
            order.add(id);
            o += 4 + len;
        }
        p.extOrder = order.stream().mapToInt(Integer::intValue).toArray();
        return p;
    }

    static String sha256Hex(byte[] b) throws Exception {
        byte[] d = MessageDigest.getInstance("SHA-256").digest(b);
        return HexFormat.of().formatHex(d);
    }
}
