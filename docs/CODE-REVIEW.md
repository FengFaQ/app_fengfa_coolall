# 协议层代码审查

> 审查对象：`src-tauri/src/` 下 `token_v3.rs` / `device.rs` / `request.rs` / `error.rs` / `client.rs`
> 审查基准：`docs/PROTOCOL.md`（唯一事实来源）、`docs/ARCHITECTURE.md`
> 审查方式：逐行对照文档 + 位运算穷举验证（与 Kotlin/Python 参考实现逐位对齐）
> 日期：2026-10-31

---

## 结论（一句话：是否可发布）

**不可发布。** 41 项单元测试全绿，但 `token_v3.rs` 的盐「末尾 4 位残余为 0」判断写错了一处移位，
导致**每一个合法盐都被判为非法**，`create()` 在任何时间戳下都会返回 `Err(Exhausted)`——
全绿的原因是这个错误被「收敛」测试断言成了正确行为。另外 `X-App-Device` 的段数与文档存在偏差、
`X-App-Session` 头缺失且无登录入口、客户端在网络请求期间持有全局锁。

---

## 严重问题（必须修）

### S1. `token_v3.rs:105` — 盐残余位检查永远为假，签名功能 100% 不可用

```rust
// 第 101 行：每凑满 8 位就清掉已消费的位
bits &= (1u64 << nbits) - 1;
// 第 105 行：要求「剩余位为 0」——但上面的掩码已经保证剩余位恒为 0
if written == 16 && bits == 0 {
```

**问题描述。** 22 个字符 × 6 位 = 132 位。前 16 个字符产出 12 字节、余下 `nbits = 32` 位；
第 17 个字符进来后 `nbits = 38`，**只消费一个字节**（`nbits -= 8` 是 `if` 不是 `while`），
留下 `nbits = 30`；第 18–22 个字符各消费 1 字节且各留下 1 位，最终
`written = 16, nbits = 35, bits` 只保存了最近 35 位。

真正需要验证的「末尾 4 位残余」位于位置 `128..132`，在第 5 个字符进入之后就被后续字符挤出
了 64 位窗口。`written == 16` 时窗口里**不可能**存在这 4 位，于是：

- 残余位非法（如文档 §3.2 的例子 `...jI4OR`）→ `bits != 0` → 返回 `None`（**结论对，理由错**）；
- 残余位合法（末位 ∈ `./ABCDEFGHIJKLMNOP`）→ `bits` 仍非 0 → 返回 `None`（**误杀**）。

穷举验证（64 个末位字符 × 4 组不同前缀，共 256 组）：

| 末位字符 | 文档/参考实现 | 本实现 |
| --- | --- | --- |
| `./ABCDEFGHIJKLMNOP`（16 个） | 合法 | **全部误拒** |
| `QRSTUVWXYZ` 及之后（48 个） | 非法 | 拒绝（碰巧正确） |

配合第 144–149 行的 `'+'` 守卫（见 G1），`create()` 对**任意**输入都只会落到
`Err(TokenError::Exhausted)`，`X-App-Token` 无法生成，`build_headers()` 在第 123 行
直接失败，所有网络接口不可用。

**为什么 41 项测试没发现。** 相关测试把错误固化成了「正确行为」：

- `token_v3.rs:194-203` `deterministic_for_same_input` / `matches_python_reference` 调用 `.unwrap()`，
  按上表应当**失败**；
- `token_v3.rs:226-232` `adjacent_timestamps_may_converge` 断言 `t` 与 `t+1` 输出相同——
  该断言只在 `create()` 永不成功时成立。

因此文档 §13 里的「TokenV3 7/7 通过」与 Rust 现状**必然不同源**：要么这 41 项测试是在
`tools/rs-probe` 的副本上跑过的，要么 `EXPECTED` 常量从未被真正比对成功。请先跑一次
`cargo test` 确认，再动手修——**测试全绿这个前提本身就是不成立的**。

**修复方案。** 按参考实现的定点写法直接检查第 22 个字符，不要依赖剩余位：

```rust
fn salt_residual_is_zero(salt: &str) -> bool {
    // 132 位中最后 4 位来自第 22 个字符的低 4 位，必须为 0
    salt.as_bytes()
        .get(21)
        .and_then(|&c| BCRYPT_ALPHABET.iter().position(|&x| x == c))
        .map(|v| v < 16)
        .unwrap_or(false)
}
```

`decode_bcrypt_salt()` 保留（它返回的 16 字节仍然要喂给 `hash_with_salt`），但合法性判定改用上面的函数；
或者把 `if nbits >= 8` 改成 `while nbits >= 8` 并在 `written == 16` 时立刻校验「当前 6 位的高 4 位为 0」。

**修复后必须补的回归测试**（现在的测试全部漏掉这一类）：
1. `decode_bcrypt_salt("Njc3NDg1ODIvMmRjMTc4Me")` 必须 `is_some()`（末位 `e`，参考实现判合法；
   当前测试第 240 行断言它合法，但按现在的实现会得到 `None`——这条测试应当已经失败）；
2. 末位为 `+` 的盐必须被拒；
3. 末位为 `.` `O` `u` `e` 的盐必须被接受（覆盖 bcrypt 字母表与标准字母表的**错位**）；
4. 用固定时间戳断言 `create()` 返回 `Ok`，且 16 个偏移中至少 3 个被跳过（对齐 §13 第 3 项）。

---

### S2. `device.rs:131-135` — 设备串段数与文档 §4.1 不一致（9 段 vs 10 段）

```rust
format!("{}; ; ; ; {}; {}; {}; {}; null", self.szlm_id, p.manufacturer, p.brand, p.model, p.build)
// 实际产出：id; ; ; ; Xiaomi; Xiaomi; 2211133C; TKQ1.221114.001; null   ← 9 段
```

**问题描述。** 文档 §4.1 的格式是 `{szlmId};{?};{?};{?};{manufacturer};{brand};{model};{build};null`，
即占位段编号 1–4 共 **4 个**，加机型 4 段 + 结尾 `null`，合计 **10 段**。`k = 9` 无法同时满足这三者。
本实现写了 3 个空格占位，恰好 9 段。

`device.rs:196` 的 `assert_eq!(decoded.split(';').count(), 9)` 把这个选择固化了下来，
而文档 §4.2 的校验方法只要求「能还原 9 段明文」，没写段数——**测试与文档在这里互相矛盾**，
并且测试是照实现写的，所以它不构成证据。

风险等级说明：若服务端只按「第 5–8 段」取值，3 个占位会让 `manufacturer` 落到第 5 段，
两种写法都能被解析；若按 4 个占位硬编码，则本实现整体错位一段，UA 与设备串会互相矛盾（§5.3 交叉校验会拒）。
**这是我能给出的最大不确定项，需要一次真实 200 响应来定案**，但当前代码与文档不一致是确定的事实。

**修复方案。** 以文档为准写 4 个占位段，并去掉每个字段前的空格（保证「解码后按 `;` 切分、再 `trim`」
两种读法都得到同一组值）：

```rust
format!("{};;;;;{};{};{};{};null", self.szlm_id, p.manufacturer, p.brand, p.model, p.build)
```

然后同步把 `device.rs:196` 的断言改成 `10`，并在文档 §4.2 明确写出「10 段」。

---

### S3. `request.rs:121-166` / `client.rs:121-180` — 缺 `X-App-Session` / `X-App-Token` 之外的会话头，且登录态无法建立

**问题描述。** 文档 §6.1 的 15 个必需头**确实齐全**（见核对表），这一项通过。但：

- `client.rs:92-95` 给 `reqwest::Client` 设了 `.user_agent("coolall/0.1.0")`，随后 `client.rs:140-142`
  逐个头 `header()` 覆盖。reqwest 允许这样覆盖，**这是可行的**（与任务提示里的怀疑相反），
  但它让「UA 必须严格照抄」这条铁律依赖覆盖顺序，任何人把 `build_headers` 改成不返回 UA 就会静默退回
  `coolall/0.1.0` → 按 §5.3 直接 403。应当**删掉第 94 行的默认 UA**，让 UA 只有一个来源。
- `lib.rs` 注册的 7 个命令里没有任何登录命令，`Session.uid/username/token` 永远是 `None`，
  `is_logged_in()` 恒为假。阶段 1 声称「登录」是能力之一（ARCHITECTURE §8），实际不存在。
  这不是本次协议层的缺陷，但意味着 `request.rs:47-54` 的登录态 Cookie 分支**从未被执行过**，
  属于未经运行验证的代码路径。

**修复方案。** 删除 `client.rs:94` 的 `.user_agent(...)`；在文档里把「登录」标为阶段 2，
或在核对表中注明「登录态 Cookie 拼接已实现但未接入调用方」。

---

### S4. `client.rs:182-284` + `lib.rs:61-72` — 全局锁跨越网络请求，`try_lock` 失败静默丢弃用户凭证

**问题描述。** `lib.rs` 用 `std::sync::Mutex<CoolapkClient>` 包住整个客户端，`browse/detail/replies/search`
在**持有这把 std 互斥锁的情况下 await 网络请求**（`client.rs:144`）。后果：

1. 首页、详情、评论、搜索**完全串行**，任何一次 20 秒超时都会阻塞所有其他请求；
2. `session_info`、`submit_validation` 这两个**同步命令**会阻塞 Tauri 主线程等待网络锁——
   验证码弹窗期间「提交验证结果」可能长时间无响应；
3. 更糟的是 `client.rs:269`：`submit_validation` 内部对**同一个已经被外层持有的锁**再 `try_lock()`。
   外层已经持有该 `tokio::Mutex`，`try_lock` 必然返回 `Err(TryLockError::WouldBlock)`，
   函数返回 `false`，用户辛苦完成的 `validate` 被**静默丢弃**，且 `lib.rs:155` 把它当成正常的 `Ok(false)`
   传给前端。`client.rs:268` 的注释「用阻塞方式拿到锁」与代码不符（`try_lock` 不阻塞）。

**修复方案。**
- 把 `AppState.client` 改为 `tokio::sync::Mutex<CoolapkClient>` 并在 `lib.rs` 用 `.await` 加锁；
  但更好的是消除锁：`CoolapkClient` 内部只需 `Arc<Mutex<Session>>` 共享，`identity` 用 `ArcSwap` 或
  `RwLock`，命令层不再需要包住整个客户端 → 天然并发。
- `submit_validation` 不应自己再拿锁：让它接收一个已经可用的 `&Mutex<Session>`，
  或者干脆由 `lib.rs` 负责加锁、`CoolapkClient` 只暴露 `Session` 层的窄接口。
- 即使保留 `try_lock`，也**绝对不能**把失败映射成 `false`——返回 `Result` 让前端重试。

---

## 一般问题（建议修）

### G1. `token_v3.rs:144-149` — `'+'` 守卫实际恒真，但与 S1 联动后语义已错乱

```rust
if salt.bytes().any(|c| !(c.is_ascii_alphanumeric() || c == b'.' || c == b'/')) { continue; }
```

**验证结果。** `is_ascii_alphanumeric() || '.' || '/'` 这个集合**恰好等于** bcrypt 字母表
`./A–Za–z0–9`（64 个字符，无遗漏、无多余）。所以它拒绝的只有 `+`，与文档 §3.2 第①条一致，
**作为 `+` 过滤器是对的**。但注释说的是「`+` 在 ALPHABET 内但不是合法 bcrypt 盐字符」，
真正的判据应当是「字符不在 `BCRYPT_ALPHABET` 内」，写成字面量集合会在未来字母表调整时悄悄失效。

**修复方案。** 改为 `BCRYPT_ALPHABET.contains(&c)`，把「合法字符集」收敛成一个常量。

### G2. `token_v3.rs:129-131` — `source_full.len() < 22` 的 `continue` 是死代码

`source_full` 是 `base64_nopad("{t:x}/{32位hex}")`，长度至少 36，永远 ≥ 22。这段分支不可达，
会误导后续维护者以为存在长度不足的情况。**修复方案**：删除，或改成 `debug_assert!`。

### G3. `token_v3.rs:63-74` — `segment_for` 每次调用都重新 base64 解码

16 次重试最多解码 16 次，每次 128 字节。性能上无所谓，但让 `create()` 的失败路径难以推理。
**修复方案**：`LOOKUP` 初始化时一次性解出 100 个 segment 放进 `OnceLock<Vec<String>>`，
`segment_for` 退化为索引；`create()` 的失败原因就只剩「盐非法」一种。

### G4. `error.rs:169-239` — `classify()` 的 `failed` 判定与 `is_success()` 不同源

两处各写一份「成功/失败」判定，且**不一致**：

| 场景 | `is_success()` | `classify()` | 结果 |
| --- | --- | --- | --- |
| `{"status": 0}` 无 `data` | `true` | `failed == false`（`message` 为空） | 一致 |
| `{"code": 0, "message": "x"}` | `true` | `failed == true` | **矛盾** |
| `{"status": 1008, "data": {...}}`（§9 的验证码族） | `false` | `failed == true` → 落到 `CaptchaNeeded` | 一致但靠巧合 |

`classify()` 第 191 行「无 `data` 且有 `message` 且无 `code`/`status`」这条启发式在 `is_success()` 里没有，
目前 `client.rs` 只用 `classify`，所以还没炸；但 `is_success` 是 `pub` 的，谁先用谁出错。
**修复方案**：`classify()` 开头直接复用 `if is_success(body) { return None; }`，删除第 186–195 行的重复逻辑。

### G5. `error.rs:215` — `clues.contains("验证")` 会吞掉所有含「验证」的业务错误

按文档 §12 这正是要求（`CAPTCHA: 含 captcha / 「验证」`），**与文档一致**。
但顺序上它排在 `AUTHENTICATION` 之前，于是「请先验证登录信息」这类消息会被判成验证码而非登录失效。
文档的判定顺序也如此，所以**不算偏离文档**，仅记录为已知取舍。**建议**：在核对表里显式标注该顺序是有意为之。

### G6. `error.rs:186-191` + `client.rs:176` — `status`/`code` 为字符串数字时判定失效

`text("status").parse::<i64>()` 会处理 `"1004"`，但 `is_success()` 用的是 `s.as_i64()`，
对 `"0"` 返回 `None` → 走 `_ => return false`。同一份 JSON 两个函数结论相反（又一个 G4 的变体）。
**修复方案**：统一用一个 `fn as_int(v: &Value) -> Option<i64>` 同时接受 `Value::Number` 与 `Value::String`。

### G7. `error.rs:44-84` — `captchaField` 的过滤条件过严，且与 §9.3 语义脱节

第 71-78 行的过滤器允许「空字符串」或「`_` 开头且 ≤64 且 `[A-Za-z0-9_]`」，否则静默回落到
`_v2_post_token`。文档 §9.3 只要求「非空 → 作为表单字段」。一个合法的 `captchaField`（比如未来变成
`postToken`，没有下划线）会被**静默替换成默认值**，用户验证必然失败且无从排查。
**修复方案**：放宽为「非空即接受」，仅在为空时记为空串；把「`_` 开头」降级为一个 `debug_assert`。

### G8. `error.rs:32-41` — `proof()` 未按 §9.3 做 URL 编码

文档 §9.3：`proof = URLEncode("NEC:" + captchaId[:8] + ":" + validate, "UTF-8")`。
`proof()` 只做了字符串拼接；`urlencoding::encode` 在整个 crate 里只出现在 `client.rs:192/203/215`（搜索与 id）。
`validate` 里带 `+` `/` `=` 的概率很高，拼进 Cookie 后 `;` 或空格会直接破坏 Cookie 解析。
另外第 35 行只挡了 `< 32` 的控制字符，**没挡 `0x7F` 与 `;`**，一个含 `;` 的 validate 能注入额外的 Cookie 对。
**修复方案**：`proof()` 返回 `urlencoding::encode(&format!("NEC:{short}:{validate}")).into_owned()`，
并显式拒绝 `;`、`,`、空白与 `0x7F`。

### G9. `request.rs:71-98` — `Set-Cookie` 解析与 §8.1 的四条保存条件只对齐了两条

文档 §8.1 要求：名称 `SESSID`、域必须匹配（主机或 `coolapk.com`）、值匹配正则、过期上限 7 天。
实现只检查了名称与值，**没有校验 `Domain`**，也**没有处理 `Expires`/`Max-Age`（除 `max-age=0`）**。
一个来自其他域的 `SESSID`（重定向、CDN 恶意 Header）会被无条件接受。
第 73 行 `raw.split_once(';').or(Some((raw.as_str(), "")))` 中 `.or(Some(..))` 恒为 `Some`，
写法冗余且掩盖了「没有属性」这一情况的处理。

**修复方案**：解析 `Domain`，要求为空或后缀匹配 `coolapk.com`；解析 `Max-Age` 并截断到 604800 秒；
把第 73 行的 `.or(...)` 换成 `raw.split_once(';').map_or((raw.as_str(), ""), |x| x)`。

### G10. `request.rs:45-66` — `cookie_header()` 里 `validate` 无条件附加

第 58-60 行不看当前情境，只要有 `validation` 就拼 `validate=`。§9.3 规定：
`captchaField` **非空**应作为表单/查询参数附加，**为空**才用 `validate` Cookie。
`Session` 只存了一个字符串，没有保存 `captchaField`，所以这条分支**永远走 Cookie**。
叠加 G8 的未编码问题，这条路径很难成功。

**修复方案**：`Session.validation` 改为 `Option<Validation { proof: String, field: String }>`，
在 `build_headers` 里按 `field` 决定进 Cookie 还是进 query（或由 `send()` 追加到 path）。

### G11. `client.rs:182/191/202/214` — 四个方法都取了 `&mut self`，但只为了写一个 `pending_challenge`

`track_challenge` 只需要在 `Error::CaptchaNeeded` 时记一下挑战。这让四个读方法变成 `&mut`，
正是它迫使 `lib.rs` 用「包住整个客户端」的锁（S4）。**修复方案**：把 `pending_challenge`
换成 `Arc<Mutex<Option<CaptchaChallenge>>>` 或 `RwLock`，方法签名回到 `&self`，`lib.rs` 的锁即可去掉。

### G12. `client.rs:214-238` — `detail()` 的公开页兜底会吃掉真正的错误

`Err(primary) => ... 兜底成功就返回 Ok`。按 §10 这是设计意图，但兜底会把
`Error::AuthRequired` / `CaptchaNeeded` 一并吞掉：前端再也拿不到「需要验证」的信号，
只能看到一个没有结构的 HTML，`pending_challenge` 也不会被记录（`detail` 没有调用 `track_challenge`）。
**修复方案**：只在 `Http{403}` / `CaptchaNeeded` 时兜底，`Network` 与 `Parse` 直接上抛；
兜底成功时把原始错误塞进返回体（如 `"primaryError": {...}`）供前端区分。

### G13. `client.rs:163-165` — 403 的特殊分支与下面重复

`if status == FORBIDDEN { return Err(Http{403}) }` 紧接 `if !status.is_success()`，
二者对 403 产生完全相同的结果。**修复方案**：删除第 163-165 行，只保留统一分支（或反过来，
让 403 携带 `Error::ClientUpgrade` 的提示，才配得上这个特殊分支）。

### G14. `device.rs:111-112` — `trimmed[8..]` 依赖字节偏移，且前缀判断不严谨

`to_lowercase().starts_with("shuzlmid")` 通过后用 `trimmed[8..]` 切**原串**。这里 `ShuzlmID` 是 8 个 ASCII 字节，
目前不会 panic；但 `starts_with` 允许 `ShuzlmIDxxx` 这种行也被识别，且 `to_lowercase()` 对某些 Unicode
输入（如 `İ`）长度会变化，两者组合存在理论上的切片 panic。
**修复方案**：用 `trimmed.get(..8)` 做大小写不敏感的 `eq_ignore_ascii_case("ShuzlmID")`，
再用 `strip_prefix` 取余下部分，全程不按下标切片。

### G15. `device.rs:77-83` — `random_szlm_id()` 用 32 次 `format!` 拼字符串

每次分配一个 `String`，语义上也看不出「32 位小写 hex」。**修复方案**：
`hex::encode(rand::thread_rng().gen::<[u8; 16]>())`，一行且更明确。

---

## 建议（可选）

### N1. 深模块视角：`client.rs` 的接口泄漏了实现

`browse/detail/replies/search` 全部返回 `serde_json::Value`——这是**最浅**的一种接口：
调用者必须自己知道 `data` 在哪、如何分页、`isPublic` 怎么判断。ARCHITECTURE §2.1 承诺
`Result<FeedPage>` / `Result<Content>`，实现与此不符，导致 7 项实现里「错误分类」「公开页兜底」
这两项对调用者是不可见的。建议为 `FeedPage` / `Content` / `ReplyPage` 落类型，`client` 只返回它们。

### N2. `session_info()` 返回裸 `Value`，且声称的内容与实现不符

`client.rs:246-254` 返回的 JSON 只有 `challengeId/challengeField/szlmId/userAgent`，
ARCHITECTURE §2.1 承诺的 `SessionState`（登录/游客/验证状态）并不存在——**没有任何字段告诉调用者
当前是否已登录或是否持有 validate**。建议落一个 `SessionState { logged_in: bool, has_validation: bool, pending_challenge: Option<..> }`。

### N3. `Session` 的字段全 `pub`，`SessionStore` 这个 seam 形同虚设

ARCHITECTURE §2.4 声称 `SessionStore` trait 有 `FileSessionStore` / `MemorySessionStore` 两个 adapter，
是「真 seam」。实际代码里**没有这个 trait**：`request.rs` 的 `Session` 是一个全字段 `pub` 的结构体，
`client.rs` 直接 `Session::default()`。这不是「只有一个 adapter 的假 seam」，而是连 seam 都还没建立，
文档先行了。建议要么实现该 trait（`client.rs` 注入 `Box<dyn SessionStore>`，测试用内存实现），
要么把 ARCHITECTURE §3 的 seam 清单改成「阶段 2 目标」。

### N4. `HttpTransport` / `DeviceProfileSource` / `CaptchaFlow` 三个 seam 均未存在

同样是文档先于实现。特别值得注意的是 `CaptchaFlow`——它是「唯一能解决 1004 的路径」，
但代码里只有 `pending_challenge` 一个字段和一个 `submit_validation`，**没有超时、没有窗口、没有取消**。
建议优先落地 `CaptchaFlow`，否则 S4 里那条被丢弃的 validate 连重试入口都没有。

### N5. `request.rs:121` 返回 `HashMap` 而非 `HeaderMap`

`HashMap` 的迭代顺序随机，`for (k, v) in &headers` 下发的头顺序每次不同。协议上无所谓，
但**排查抓包时极其难受**（同一份代码两次抓包头顺序不同）。且 `HashMap` 无法表达「同名头多次出现」。
建议返回 `reqwest::header::HeaderMap` 或 `Vec<(HeaderName, HeaderValue)>`，顺便把
「值里含非法字符」从运行期 panic 变成编译期/构造期错误。

### N6. `dark_mode` 恒为 `false`

`client.rs:130` 写死 `dark_mode: false`，§6.2 明确说「参考实现里 `X-Dark-Mode` 是跟主题联动的变量」。
不影响 WAF，但用户切深色模式时请求头不变。建议从设置里读。

### N7. `browse` 只支持首页与同域动态列表，`page.max(1)` 掩盖了非法页码

`page = 0` 被静默改成 1，前端传错也不会报错。建议显式校验并返回 `Err(Parse)`。

### N8. 测试策略与 ARCHITECTURE §4 不符

§4.1 要求测「编码可逆（解码回 9 段）」「SESSID 保存/失效」「各类响应映射」——这些都有。
§4.2 要求的 `MockTransport` 集成测试**一个都没有**，`1004 → CaptchaNeeded`、
`err_request_captcha_v2` 识别、公开页兜底时机全部只在 `classify()` 单测层面验证过，
`client.rs` 的 284 行逻辑（含 S4 的锁问题）**测试覆盖为零**。这正是 S1 能蒙混过关的根本原因：
最危险的接线层没有任何测试。

---

## 与文档的一致性核对表

| 文档条款 | 要求 | 实现位置 | 结论 |
| --- | --- | --- | --- |
| §2 `APP_ID` | `com.coolapk.market` | `device.rs:16`、`token_v3.rs:24` | ✅ 一致（但**定义了两份**，见下） |
| §2 `CHANNEL`/`MODE`/`LOCALE`/`REQUEST_WITH` | 定值 | `device.rs:17-20` | ✅ |
| §2 `VERSION_NAME`/`CODE`/`API_VERSION` | `16.7.0`/`2610291`/`16` | `device.rs:23-25` | ✅ 三者自洽 |
| §2 `BASE_URL`/`BASE_URL_2`/`FOOTER_URL` | 三个域 | `device.rs:30-32`（`FOOTER_URL` 名为 `PUBLIC_URL`） | ✅ 值一致，⚠️ 命名偏离 |
| §2 常量集中管理 | 「禁止散落」 | `token_v3.rs:24` 又写了一遍 `APP_ID` | ⚠️ 轻微违反 |
| §3.1 两层解码 | 切片后二次 base64 | `token_v3.rs:63-74` | ✅ |
| §3.1 起始索引 `%100*4+0x80` | — | `token_v3.rs:65` | ✅ |
| §3.1 `plain` 拼接顺序 | 5 段 | `token_v3.rs:125` | ✅ 与 Python 参考逐字对齐 |
| §3.1 `source` 取前 22 位 | — | `token_v3.rs:128-132` | ✅ |
| §3.1 末位 −5 mod 64 | — | `token_v3.rs:141` | ✅ |
| §3.1 `$2a` → `$2y` | — | `token_v3.rs:160` | ✅ |
| §3.2① 盐含 `+` 必须跳过 | — | `token_v3.rs:144-149` | ✅（集合恰等于 bcrypt 字母表，见 G1） |
| §3.2② **末尾 4 位残余为 0** | 显式检查 | `token_v3.rs:105` | ❌ **S1：恒判非法** |
| §3.2③ 16 次重试 | `0..16` | `token_v3.rs:120` | ✅ 边界正确（`0..16` 即 16 次） |
| §3.2④ `device` 是编码值 | 不是 szlmId | `client.rs:122-127` | ✅ 传的是 `device_header` |
| §3.3 输出 `v3` + base64(60 字节) | — | `token_v3.rs:161` | ✅ 结构正确 |
| §3.4 LOOKUP 930 字节、不手抄 | 资源文件 | `assets/token_lookup.b64`（1240 字符） | ✅ 与文档 §3.4 的 1240 字符吻合 |
| §4.1 9 段 / 4 个空占位 | `id; ; ; ; maker;…` | `device.rs:132` | ❌ **S2：段数与文档矛盾** |
| §4.1 占位段分号间有空格 | — | `device.rs:132` | ✅ 与字面示例一致 |
| §4.2 Base64 无填充 → 反转 → 去 `=` | — | `device.rs:138-146` | ✅ |
| §4.2 解码可逆 | — | `device.rs:149-161` | ✅（测试覆盖） |
| §4.3 长度 8..4096 + 字符集 + 排除 null/undefined | — | `device.rs:86-95` | ✅ |
| §4.3 导入正则 MULTILINE+IGNORECASE、唯一匹配 | — | `device.rs:100-126` | ⚠️ 手写解析而非正则，语义近似（见 G14） |
| §5.1 UA 三段式 | — | `device.rs:164-176` | ✅ 与 §5.2 示例逐字一致 |
| §6.1 15 个必需头 | 齐全 | `request.rs:130-159` | ✅ **15/15 齐全** |
| §6.1 `Accept: application/json` | 建议 | `request.rs:159` | ✅ |
| §6.2 `X-App-Code` ≠ `X-App-Supported` | 两个都发 | `request.rs:146,155-158` | ✅ |
| §6.2 `X-Dark-Mode` 跟随主题 | 变量 | `client.rs:130` 恒 `false` | ⚠️ **N6** |
| §7 禁止降级协议版本 | — | `device.rs:23` 常量不可配 | ✅ |
| §8.1 名称须为 `SESSID` | — | `request.rs:79` | ✅ |
| §8.1 域须为请求主机或 `coolapk.com` | — | `request.rs:71-98` | ❌ **G9：未校验 Domain** |
| §8.1 值正则 `{1,512}` | — | `request.rs:110-116` | ✅ 字符集与长度逐字对齐 |
| §8.1 过期上限 7 天 | — | `request.rs:85` 只处理 `max-age=0` | ❌ **G9** |
| §8.1 **SESSID 变化时清 validate** | — | `request.rs:93-95` | ✅ 有，且有测试 |
| §8.2 Cookie 顺序 `uid;username;token;SESSID` | — | `request.rs:47-57` | ✅ 顺序正确 |
| §8.2 游客态只带 SESSID | — | `request.rs:55-57` | ✅ |
| §9.2 `err_request_captcha_v2` 识别 | — | `error.rs:44-48` | ✅ |
| §9.2 兜底值 id/field | — | `error.rs:20-25` | ✅ 逐字一致 |
| §9.2 captchaId 长度 16..64 字母数字 | — | `error.rs:63-65` | ✅ |
| §9.3 `proof = NEC:{id[:8]}:{validate}` | — | `error.rs:39-40` | ✅ 格式正确 |
| §9.3 **URLEncode(proof)** | — | `error.rs:32-41` | ❌ **G8：未编码** |
| §9.3 validate ≤8192、无空白控制符 | — | `error.rs:33-35` | ⚠️ 未拦 `0x7F` 与 `;` |
| §9.3 `captchaField` 非空 → 表单参数 | — | `error.rs:71-78`、`request.rs:58-60` | ❌ **G7/G10** |
| §9.3 `captchaField` 为空 → validate Cookie | — | `request.rs:58-60` | ✅ |
| §9.4 只重放一次，传输错误不重放 | — | 无重放实现 | ⚠️ 未实现（见 N4） |
| §10 公开页兜底、id 须为纯数字 | — | `client.rs:221-234` | ✅ 判定正确（但见 G12） |
| §11 接口清单 8 个 | — | 实现 6 个（缺 `user/space`、`user/feedList`、`checkLoginInfo`、`feed/detail` 之外的 4 个） | ⚠️ 阶段 1 未全实现，路径与域均正确 |
| §12 `-10` 或含「网络环境」→ RISK_CONTROL | — | `error.rs:209-211` | ✅ |
| §12 `err_request_need_upgrade_new_version` → CLIENT_UPGRADE | — | `error.rs:212-214` | ✅ |
| §12 含 captcha/「验证」→ CAPTCHA | — | `error.rs:215-219` | ✅（顺序亦与文档一致） |
| §12 含 login/auth/「登录」或 401 → AUTH | — | `error.rs:220-226` | ✅ |
| §12 含 rate_limit/「频繁」或 429 → RATE_LIMIT | — | `error.rs:227-229` | ✅ |
| §12 其余 → BUSINESS | — | `error.rs:231-238` | ✅ |
| §12 `HTTP` 用于 200..399 之外 | — | `client.rs:166-170` | ✅（`is_success()` 是 2xx，`200..399` 差异只影响 3xx，reqwest 默认已跟随重定向） |
| §12 1004 识别 | — | `error.rs:203-207` | ✅ 且排在挑战识别之后（顺序正确，不会误判为 BUSINESS） |
| §12 成功判定四条件 | — | `error.rs:133-166` | ⚠️ 与 `classify()` 不同源（G4/G6） |
| §13 #3 「16 个中 3 个被跳过」 | — | — | ❌ 当前实现 16 个**全部**被跳过（S1） |
| §13 #1 「TokenV3 7/7 通过」 | — | — | ❌ 与源码当前状态不相容（S1） |

**核对表汇总：** 必需项 15/15 请求头齐全、§3 算法步骤全部正确、§8.1 的 validate 联动正确、§12 判定顺序正确。
**偏离项 3 个严重 + 9 个一般**，其中 S1 使整个协议层不可用。

---

## 附：本次审查的验证方法（可复现）

1. 精读 `docs/PROTOCOL.md` 全文，逐条提取可断言的契约。
2. 以仓库自带的 `tools/verify_token_v3.py`（独立参考实现）为对照，用 Python 按**相同的位运算**
   镜像 `token_v3.rs:85-110` 的 `decode_bcrypt_salt`，对其做穷举：
   - 64 个末位字符 × 4 组不同 21 字符前缀 = 256 组；
   - 结果：与参考规则**恰好相反**——参考接受则实现拒绝，参考拒绝则实现接受或拒绝（碰巧）。
3. 逐位追踪 `nbits/bits` 在 22 个字符上的演化，确认第 132 位区域在第 5 个字符后即离开窗口，
   从而证明 `written == 16 && bits == 0` 只在 `bits` 恰好为 0 时成立，而此时的 0 与残余位无关。
4. 对 `BCRYPT_ALPHABET` 与 Rust 的 `is_ascii_alphanumeric() || '.' || '/'` 做集合比对，确认二者等价（G1）。
5. 对设备串做 `split(';')` 计数，确认 9 vs 10 的差异并给出最小修复（S2）。
6. 逐条走查 `lib.rs` 的命令层，确认 S4 的锁重入路径。

**未完成的验证：** 本机 cargo 缓存中没有 `bcrypt` crate，`tools/rs-probe` 无法离线构建，
因此**没有运行 `cargo test`**，S1 的结论来自等价位运算镜像而非 Rust 运行输出。
建议接手者第一步就跑一次 `cargo test`——按 S1 的分析，`matches_python_reference` 与
`salt_with_trailing_nonzero_bits_is_rejected` 中至少一条**应当是失败的**。
