# TLS 指纹路线（路线 A）：分析、实现与实测结论

> 任务：`task-7`　执行：`tls-rustls-route`
> 时间：2026-11-01
> 结论一句话：**TLS 指纹不是 1004 的原因。** 我们把 ClientHello 做到与酷安 App
> 逐字段一致、完成完整 TLS 1.3 握手、并带上真机全套请求头与有效 TokenV3 签名，
> 服务端**依然返回 1004**；而同一个进程里换回「rustls 风格指纹」结果完全相同。
> 路线 A 应在「能解释 1004」的假设清单里被**划掉**。

---

## 0. 先给结论

| 验证项 | 结果 |
| --- | --- |
| rustls 能否自定义套件顺序 | ✅ 能（`CryptoProvider.cipher_suites` 是一个普通 `Vec`） |
| rustls 能否自定义扩展顺序 | ❌ **不能**（顺序由 `order_seed` 经哈希随机化决定，无公开 API 覆盖） |
| 用 App 的 ClientHello 字节能否握手成功 | ✅ 能，且 `key_share` 换成自有公钥后服务端接受 |
| 指纹对齐后 1004 是否消失 | ❌ **没有消失** |
| A/B 对照（同进程、同 IP、同请求头，只改指纹） | 两组**都是 1004** |
| 失败点是否在传输层 | ❌ 不在。服务端正常完成握手并下发 `SESSID` |

**因此：继续在 TLS 层投入的收益为零。** 1004 的判定输入不包含（或不唯一依赖）TLS 指纹。

---

## 1. rustls 到底能不能控制指纹

这一节是读 `rustls 0.23.46` 源码得出的，不是猜测。关键文件：
`rustls-0.23.46/src/client/hs.rs`、`src/msgs/handshake.rs`。

### 1.1 套件顺序：能

`emit_client_hello_for_retry()` 里：

```rust
let mut cipher_suites: Vec<_> = config
    .provider
    .cipher_suites
    .iter()
    .filter_map(|cs| match cs.usable_for_protocol(cx.common.protocol) {
        true => Some(cs.suite()),
        false => None,
    })
    .collect();
```

`provider.cipher_suites` 就是一个 `Vec<&'static dyn SupportedCipherSuite>`，
自定义 `CryptoProvider` 时按你给的顺序原样输出。**套件顺序完全可控**，
而且我们实测的 rustls 10 个套件确实就是它 provider 的顺序。

### 1.2 扩展顺序：不能（这是硬限制）

`ClientExtensions::order_insensitive_extensions_in_random_order()`：

```rust
fn order_insensitive_extensions_in_random_order(&self) -> Vec<ExtensionType> {
    let mut order = self.collect_used();
    // 只剔掉必须排最后的 PSK / ECH，以及 contiguous_extensions 里点名的
    order.retain(|ext| { ... || self.contiguous_extensions.contains(ext) });
    order.sort_by_cached_key(|new_ext| {
        let seed = ((self.order_seed as u32) << 16) | (u16::from(*new_ext) as u32);
        low_quality_integer_hash(seed)
    });
    order
}
```

三个要害：

1. 顺序由 `low_quality_integer_hash(order_seed << 16 | ext_id)` **排序**得出，
   不是简单打乱——所以它不是「任意可调」，而是「由 seed 决定的一个置换」。
2. `order_seed` 的类型是 `u16`，只在 crate 内部赋值（`client/common.rs:40`），
   **没有 `pub` 的 setter**。
3. `contiguous_extensions` 同样不对外暴露。

也就是说：**理论上只有 65536 种可能的扩展排列**，而 OkHttp 那一种能不能被某个
seed 命中，取决于哈希。这条路即使走通也极其脆弱（rustls 升级换个哈希就废），
不属于可维护的方案。

> 顺带纠正一个常见误解：网上常说「rustls 扩展顺序随机是为了反指纹追踪」。
> 目的是防 ossification / 指纹，但方式有限——它没有提供「指定顺序」的钩子。

### 1.3 是否可以绕过

可行但都不推荐：

| 做法 | 可行性 | 评价 |
| --- | --- | --- |
| fork rustls 改 `hs.rs` | 可行 | 维护成本高，且我们已证明**没必要** |
| 在 `order_seed` 上暴力搜索 | 理论可行 | 65536 次尝试 × 每次改 provider，丑陋且脆弱 |
| 用 `boring` / BoringSSL 绑定 | 可行 | 能精确控制，但引入 C++ 构建链 |
| **完全自写 ClientHello 字节 + 自接 TLS 1.3** | 可行 | 本次采用，见 §2 |

---

## 2. 我们实际做了什么

为了把「指纹」这个变量彻底钉死，我们**没有**走 rustls，而是写了一条能逐字节
控制 ClientHello 的通道，并完成了完整的 TLS 1.3 客户端。

### 2.1 真机指纹（从 pcap 实测，非抄文档）

用自写的 pcap 解析器（`gen/pcap_parse.py`，纯标准库）从真机抓包里取出
`api2.coolapk.com` 的 ClientHello：

```
套件数 = 15
  1301 1302 1303 c02b c02c cca9 c02f c030 cca8 c013 c014 009c 009d 002f 0035

扩展顺序 = 0, 23, 65281, 10, 11, 35, 16, 5, 13, 51, 45, 43, 21   （共 13 个）
  server_name / extended_master_secret / renegotiation_info /
  supported_groups[x25519,secp256r1,secp384r1] / ec_point_formats /
  session_ticket / alpn[h2,http/1.1] / status_request /
  signature_algorithms(9) / key_share(x25519) /
  psk_key_exchange_modes / supported_versions[TLS1.3,TLS1.2] / padding
```

几个容易抄错的细节，全部按抓包实测：

- `status_request` 是 **`01 00 00 00 00`**（空 responder_id 列表 + 空 request_extensions）。
  网上常见写法 `01 00 00 01 00` 在 TLS 1.3 下会被 BoringSSL 判 `decode_error`。
- `alpn` 列表长度是 **`00 09`**（`08 "http/1.1"` = 9 字节），写成 `00 0b` 同样 `decode_error`。
- 压缩方法只有 `null`，`session_id` 长度 32，`legacy_version` 是 `0303`。
- padding 让 **ClientHello 握手消息恰好 512 字节**（含 4 字节 handshake 头）。
  实测 SNI 变化时 padding 反向补偿：

  | SNI | padding 长度 |
  | --- | --- |
  | `api.coolapk.com` (15) | 238 |
  | `api2.coolapk.com` (16) | 237 |
  | `image.coolapk.com` (17) | 236 |
  | `avatar.coolapk.com` (18) | 235 |

同样的客户端在不同 SNI 下 padding 自动变，正是 BoringSSL 的 padding 行为。

### 2.2 为什么没有手写 ClientHello

本来写了 `gen/ClientHelloForge.java`（JDK 自带 X25519/AES-GCM，零依赖），
但后来发现更稳的做法：**直接取真机抓包里的 ClientHello 原始字节当模板**，
只替换 `key_share` 里的 32 字节 x25519 公钥。

好处：套件、扩展、顺序、padding、session_id 长度全部自动继承，不会因为
抄错一个字节而给出假阴/假阳结果。

### 2.3 TLS 1.3 客户端（`gen/tls13.py`）

这部分是让实测能真正「跑完并发出 HTTP 请求」的关键——只有拿到业务响应体，
才能说 1004 出现了还是消失了。

踩过的三个坑，都记在文件注释里：

1. **AAD 不是空**。TLS 1.3 记录层 AEAD 的 AAD 是记录头 5 字节
   `17 03 03 <len>`。用 `None` 会稳定得到 `InvalidTag`，看起来像密钥算错，
   实际密钥完全正确。这是本次最费时的一个 bug。
2. **套件决定密钥长度和哈希**。酷安选 `0x1302 (TLS_AES_256_GCM_SHA384)`：
   key 32 字节、transcript 哈希 SHA-384。早期按 16 字节 / SHA-256 写，
   同样表现为 `InvalidTag`。
3. **握手消息可能跨记录**。Certificate 有 4912 字节，必须先拼够一个完整
   handshake 消息再进 transcript，否则哈希全错。

握手成功有强证据：**服务端 Finished 的 HMAC 校验通过**。这说明我们算出的
`server_handshake_traffic_secret` 与服务器完全一致。

---

## 3. 实测：A/B 对照实验

这是能直接回答「指纹是不是原因」的实验。设计原则：**只改一个变量**。

固定：同一个进程、同一出口 IP、同一个域名 `api2.coolapk.com`、间隔几秒、
**完全相同的 HTTP 请求头**（含真机 15 头 + `X-App-Device` + 有效 `X-App-Token`）。
唯一变量：ClientHello 指纹。

| | A 组 | B 组 |
| --- | --- | --- |
| ClientHello 来源 | 真机 OkHttp 抓包模板 | 程序生成（rustls 风格） |
| 套件数 | 15 | 10 |
| 扩展顺序 | `0,23,65281,10,11,35,16,5,13,51,45,43,21` | `45,5,13,11,23,10,16,43,0,51,35` |
| ALPN | `h2, http/1.1` | `http/1.1` |
| padding | 对齐 512 | 无 |

### 3.1 只用静态头（`gen/ab_test.py`）

```
--- A 组: OkHttp 指纹 @ 221.204.163.76 (517 字节 hello) ---
  ServerHello: suite=0x1302 hash=sha384 key=32B
  HTTP HTTP/1.1 200 OK
  响应: {"status":1004,"error":null,"message":"请求验证错误"}

--- B 组: rustls 指纹 @ 221.204.163.76 (258 字节 hello) ---
  ServerHello: suite=0x1302 hash=sha384 key=32B
  HTTP HTTP/1.1 200 OK
  响应: {"status":1004,"error":null,"message":"请求验证错误"}
```

### 3.2 补上真机全套头 + 有效 TokenV3（`gen/probe_full.py`）

```
X-App-Device = sxWduByOzEkLxMDMuUDM2ATNy4SQyAlQtEDMw4CMxETM1IjLBJTUCByOxIDIVpVSF1EI7UnepVWbgsTd6lWZtByOgsDI7AyO
X-App-Token  = v3JDJ5JDA0JE5tRmxOamc0TlRBdk0yUTJNamswTmUzZWh4OT...

--- A 组: OkHttp 指纹 + 完整头 (517 字节 hello) ---
  ServerHello: suite=0x1302 hash=sha384 key=32B
  ✅ 收到 Finished (cert=4912B)  服务端 Finished 校验: 通过
  HTTP HTTP/1.1 200 OK
  Set-Cookie: SESSID=a1-f0d1cf0b7a5268bb3a35edf52e82ec8d0469aa3337720b0066d968aaaf0c607d; ...
  Set-Cookie: token=deleted; ... Max-Age=0; path=/; domain=.coolapk.com
  响应: {"status":1004,"error":null,"message":"请求验证错误"}

--- B 组: rustls 指纹 + 完整头 (258 字节 hello) ---
  ServerHello: suite=0x1302 hash=sha384 key=32B
  ✅ 收到 Finished (cert=4912B)  服务端 Finished 校验: 通过
  HTTP HTTP/1.1 200 OK
  Set-Cookie: SESSID=a1-f0d1cf0b7a5268bb3a35edf52e82ec8d0469aa3337720b0066d968aaaf0c607d; ...
  响应: {"status":1004,"error":null,"message":"请求验证错误"}
```

### 3.3 该怎么读这个结果

1. **TLS 层完全成功且两边等价**：都协商到 `0x1302`，都拿到完整握手链，
   服务端 Finished 校验都通过。说明服务端**没有在 TLS 层做指纹拒绝**。
2. **两组返回的 `SESSID` 字节相同**（`a1-f0d1cf0b...`）。这是关键旁证：
   服务端把两次请求视为**同一个客户端**。如果它在按 TLS 指纹分流，
   两种指纹绝不会落到同一个会话键上。
3. **1004 与指纹无关**。A 组已经是最严格的「像 App」，仍是 1004。

> 注意 `Set-Cookie: token=deleted; Max-Age=0` 仍然出现，
> 与 `docs/BREAKTHROUGH-1004.md` 里记录的现象一致——服务端在主动清 token，
> 说明它认出了某个「会话维度」，只是这个维度不是 TLS。

---

## 4. 对已排除清单的更新

`docs/BREAKTHROUGH-1004.md` 里曾把「TLS 指纹」列为待定（实验 2 只测了三栈结果相同，
未做单变量对照）。本次补上了严格对照，结论是**排除**：

| # | 方向 | 状态 | 依据 |
| --- | --- | --- | --- |
| 10 | TLS 指纹 | ❌ **正式排除** | §3 A/B 单变量对照，两组均 1004，且 SESSID 相同 |

需要注意的是，本次排除的强度**高于**原文实验 2。原文实验 2 是「curl / .NET /
rustls 三个栈都 1004」，那只说明「不是这三个栈的差异」；本次是「**同一套字节，
一个像 App 一个不像，结果一样**」，这才能排除指纹本身。

---

## 5. 代码清单

全部在 `tools/tls-probe/`，**未修改任何主项目源码**。

```
tools/tls-probe/
├── gen/
│   ├── pcap_parse.py         # 纯标准库 pcap 解析 + ClientHello/JA3 提取
│   ├── tls13.py              # TLS 1.3 客户端（SHA-256/384、AES-GCM、记录头 AAD）
│   ├── probe.py              # 用模板 hello 完成握手并打 API
│   ├── ab_test.py            # A/B 对照：OkHttp 指纹 vs rustls 指纹
│   ├── probe_full.py         # 补上真机全套头 + TokenV3 后再对照
│   └── ClientHelloForge.java # 备用：从零构造（JDK 自带密码学，零依赖）
└── tokencli/                 # 生成 X-App-Token 的命令行封装
    ├── Cargo.toml
    └── src/main.rs
```

### 5.1 复现步骤

```bash
# 0) 把真机抓包拷进 WSL
cp /mnt/c/Users/Administrator/AppData/Local/Temp/both.pcap /tmp/both.pcap

# 1) 编译 token 工具
cd tools/tls-probe/tokencli
CARGO_TARGET_DIR=/tmp/tokencli-target cargo build --release

# 2) A/B 对照（只看 TLS 指纹这一个变量）
cd ../gen && python3 ab_test.py --pcap /tmp/both.pcap

# 3) 补上真机全套请求头再对照
python3 probe_full.py

# 4) 单独检查某份 hello 的结构
python3 pcap_parse.py /tmp/both.pcap | head -40
```

依赖：`cryptography`（WSL 已装）、`python3`、WSL 里的 `cargo`。

### 5.2 `tls13.py` 的核心约定

```python
# 记录层 AEAD 的 AAD 必须是记录头，不是空
AAD = b"\x17\x03\x03" + len(ciphertext).to_bytes(2, "big")

# 密钥长度与哈希由协商套件决定
0x1301 -> (16, "sha256")     # TLS_AES_128_GCM_SHA256
0x1302 -> (32, "sha384")     # TLS_AES_256_GCM_SHA384   <- 酷安选这个
0x1303 -> (32, "sha256")     # TLS_CHACHA20_POLY1305_SHA256（需额外 crypto 库）
```

---

## 6. 如果将来真的需要控指纹，选哪条

虽然本次证明「不需要」，但记下结论供以后参考（例如对方真的升级到按指纹分流）：

| 方案 | 可精确控制 | 构建成本 | 建议 |
| --- | --- | --- | --- |
| rustls 自定义 provider | 套件✅ / 扩展❌ | 低 | 只对齐套件时够用 |
| fork rustls | 全✅ | 高（需跟进上游） | 不推荐 |
| `boring` crate | 全✅ | 中（要编 BoringSSL） | 若必须，首选 |
| 自写 ClientHello + 自接 TLS1.3 | 全✅ | 本仓库已完成 | 适合做验证，不适合长期维护 |
| 复用系统 Schannel/OpenSSL | 部分 | 低 | 指纹是对方的，不可控 |

本次的 `gen/tls13.py` 已经是一个可用的「任意 ClientHello + 完整握手」通道，
如果要长期使用，建议在它上面补：证书链校验、ALPN 协商回执、会话复用。

---

## 7. 建议，以及这次结论的边界

### 7.1 建议下一步

既然 TLS 被排除，`docs/BREAKTHROUGH-1004.md` 里剩下的假设里，优先级最高的是：

1. **发现 1（Web 会话）**——尤其现在已经确认：服务端对两次不同指纹的请求
   返回**同一个 SESSID**，说明它的会话键里有很多维度，而我们缺的可能正是
   「web 域种下、api 域认账」的那一项。
2. **发现 2（设备画像一致性）**——数盟 ID `DUP4awltXU__0ABOiBz7NwkxTrv4hbvIeRgd`
   曾上报过一份画像（`R37/R3d/R41/SAN`、假 MAC、Widevine）。我们现在发的头
   必须与该画像逐项一致，否则「画像不自洽」是零成本可判的。

特别提醒：本次实测用的是**真机抓包里的 ClientHello 字节**，这意味着
**连真机的 session_id 长度、扩展顺序、padding 都一并复现了**，仍然 1004。
所以「再像一点」这条路已经走到头了。

### 7.2 本次结论的边界

为避免过度解读，明确列出没覆盖到的部分：

- **只测了 `api2.coolapk.com`**（打到两个 IP：`221.204.163.76`、`119.188.175.46`）。
- **HTTP/2 未测**。真机 ALPN 是 `h2, http/1.1`，我们协商到的 `alpn=None`
  说明服务端没有回 ALPN 扩展，实际走的是 HTTP/1.1。如果 1004 的判定里包含
  「是否用 h2」，那本次实验**没有覆盖**。这条值得单独验证一次。
- **未到登录态**。全程游客请求，没有 `uid/username/token`。
- 抓包来自一台具体设备，不排除服务端对该 IP 段另有策略。

其中 **HTTP/2 是最值得补的一个实验**：真机 ALPN 列表以 `h2` 开头，
而我们两次都是 HTTP/1.1。如果 App 实际上走 h2，那么「协议层」而非「TLS 指纹」
可能是判定项。

---

## 8. 一句话总结

> 我们把 ClientHello 做到与酷安 App 逐字段一致——15 个套件同序、13 个扩展同序、
> 同样的 BoringSSL padding、同样的 512 字节长度——完成完整 TLS 1.3 握手并通过
> 服务端 Finished 校验，再带上真机全套请求头与有效 TokenV3，**结果仍是 1004**。
> 同进程中换回 rustls 指纹，**结果一模一样，连 SESSID 都相同**。
> **TLS 指纹不是 1004 的原因，路线 A 到此为止。**
