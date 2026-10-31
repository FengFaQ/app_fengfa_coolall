# 功能缺失分析

> 视角：**功能完整性**（不是安全、不是运行时）。
> 基准：参考项目 `_upstream/c001apk`（实战验证过的安卓客户端）。
> 我方：`src-tauri/src/{client,commands,captcha_window}.rs` + `crates/coolall-core/src/{captcha,error,request}.rs` + `src/components/CaptchaBox.vue`。
> 日期：2026-10-31

---

## 结论（一句话）

**"拿 validate → 拼 proof → 重放一次"这条主干我方已经闭合，但只覆盖了"单线程、GET、一次性"这一种形态；参考项目赖以稳定工作的另外三块——①挑战携带的 `captchaField` 决定凭证落在查询参数还是表单体、②验证成功后凭证的"绑定会话 + 缓存 + 复用"、③并发/取消/超时/放弃时的流程收敛——我方几乎全空，阶段 1 不补就是"看着能过、实际一碰就碎"。**

具体量化（对照参考项目 5 个关键文件）：

| 能力 | 参考项目 | 我方 | 状态 |
|---|---|---|---|
| 请求体形式覆盖（Query / FormBody / Multipart） | `CaptchaInterceptor.kt:41-70` | `captcha.rs:65-74` 只拼 URL | ❌ 缺 2/3 |
| 凭证绑定会话（SESSID hash） | `ApiSessionStore.kt:122-137` | `client.rs:489-492` 裸写 | ❌ |
| 凭证有效期 / 持久化 / 跨重启 | `ApiSessionStore.kt:34-41, 122-137` | 全内存、无 TTL | ❌ |
| 成功后自动复用 | `AddCookiesInterceptor.kt:62-74` | `request.rs:58-60`（仅 Cookie 模式正确） | ⚠️ 半 |
| 重放失败后的收敛 | `CaptchaInterceptor.kt:75-96` | `client.rs:327-334` 清凭证即止 | ⚠️ 半 |
| 并发去重（单飞） | `CaptchaCoordinator.kt:38-40` | 无 | ❌ |
| epoch 时效校验 | `CaptchaCoordinator.kt:13, 29, 33` | `client.rs:147` 无版本号 | ⚠️ 弱等价 |
| 用户取消 / 5 分钟超时 / 页面关闭 | `CaptchaCoordinator.kt:38-62`、`CaptchaActivity.kt:35, 104` | 窗口关了后端不知道 | ❌ |
| 过期验证不被缓存为"已通过" | `CaptchaInterceptor.kt:75-96`（字段体不落库） | **会**落库 | ❌ |
| 单测覆盖 | `CaptchaChallengeTest.kt`、`ApiSessionStoreTest.kt` | 无并发 / 无 TTL / 无 POST 用例 | ❌ |

**完成度估计：参考项目验证链路的功能点约 20 项，我方等价实现 7 项、部分实现 5 项、缺失 8 项（见文末矩阵）。要达到"参考项目完整度"，阶段 1 必须补 5 项（下节），阶段 2 再补 4 项。**

---

## 逐条对比

### 1. 重放失败后的完整处理流程

**参考做法**

`CaptchaInterceptor.kt:71-96`，重放**不是**"发出去就结束"，而是一条带后置判定的状态机：

```kotlin
// :73-74  关掉旧响应，只重放一次
response.close()
val verified = chain.proceed(retry.build())
if (challenge.field.isEmpty()) {                    // :75  仅 Cookie 模式进入后置判定
    val body = json(verified)                        // :76
    val acceptedSession = verified.request.tag(AcceptedApiSession::class.java)?.hash  // :77-83
    if (verified.isSuccessful &&
        runCatching { accepted(body) }.getOrDefault(false) &&   // :85-86 真·成功判定
        session.isCurrent() &&
        acceptedSession != null
    )
        ApiSessions.store.acceptValidation(CaptchaProof(proof).encoded, context, acceptedSession)
    else ApiSessions.store.clearValidation()          // :95  失败一律清凭证
}
```

要点四处，缺一条功能就不完整：

1. `:84-86` **HTTP 200 ≠ 成功**。还要过 `accepted()`（`:105-114`）——`has("data")`、`messageStatus` 不以 `err_` 开头、`status ∈ {0,1}`、`code ∈ {0,1,200}`、`error ∈ {0,false,""}`。我方有等价函数 `error.rs:135-168 is_success`，**但重放路径没调用它**（`client.rs:327-334` 只看 `decode_response` 的 `Result`）。
2. `:77-83` 凭证必须绑定到**"服务端接受的这次会话"**（`AcceptedApiSession.hash`）。我方 `client.rs:489-492` 是直接 `guard.accept_validation(proof)`，**不校验 SESSID 是否在重放过程中被服务端换了**。若重放响应同时 `Set-Cookie: SESSID=new`，我方 `process_response`（`client.rs:219-240`）会先执行 `update_from_set_cookie` → `request.rs:93-95` 清掉 validation，再判定成功——**用户刚验证成功，凭证却丢了**（见第 5 条）。
3. `:75` **只有 `field` 为空（Cookie 模式）才落库凭证**。`field` 非空（表单/查询模式）时凭证不进 store，因为它是"这次 POST 的一次性票据"。我方相反：`client.rs:491` 两种模式都写进 `Session.validation`，于是 `request.rs:58-60` 会把它**永久拼进之后所有请求的 Cookie**。
4. `:95` 失败清凭证——我方有（`client.rs:329-333`），但清的时机是"任何 `Err`"，包括 `Network`。参考项目 `:71-72` 明确注释 "transport errors never trigger write retries"，重放本身不因传输错误重试，但**凭证仍保留**（因为服务端没说它无效）。

另外一个隐含分支：`:16-20` + `:22-23` + `:33` 三道**跳过闸门**——非 API 域（`CoolapkRequestPolicy.isApi`，`CoolapkRequestPolicy.kt:16-17`）、打了 `SkipCaptcha` 标记（`ApiService.kt:49, 63`，用于"先探测、允许降级公开页"的请求）、请求所属会话已失效（`RequestSession.isCurrent()`，`RequestSession.kt:8-9`）、epoch 已变——任一命中就**不弹验证、原样返回**。

**我方现状**

`client.rs:470-500 submit_validation` → `client.rs:275-314 retry_after_validation` → `client.rs:317-334 finish_response*`：

```rust
// client.rs:496-499
match self.retry_after_validation(&pending).await {
    Ok(v) => Ok(v),
    Err(e) => Err(format!("{e}")),
}
```

- `client.rs:279-285`：重放守卫耗尽 → 返回 `Business{status:1004, "本次验证已重放过一次，请重新发起请求"}`。**这是个死胡同**：`pending_captcha` 还在（`client.rs:442` 只有新挑战才覆盖），`replay_guard` 已 `consume`（`:285`），`staged_validate` 已被 `take`（`commands.rs:189`）。用户只能去点"打开验证窗口"，但窗口用的还是**同一个旧 captchaId**。
- `client.rs:317-334`：失败只 `clear_validation()`，**不向 UI 报告"验证被拒"与"网络失败"的区别**；`commands.rs:202-206` 统一包成 `Business{1004, message}`，前端 `CaptchaBox.vue:64-66` 只能显示 `e.message`。
- 没有 `SkipCaptcha` 等价物：我方 `detail()`（`client.rs:370-407`）也必须先真发一次 API 才会知道要验证，无法"先跳过验证直接走公开页"。参考项目 `NetworkRepo.kt:56-90` 用 `SkipCaptcha()` 一次拿到 CAPTCHA 失败后**立刻**降级公开页，失败才提示用户去验证（`:83-86`）。我方 `allow_public_fallback` 必须先 `set_public_fallback(true)`（`commands.rs:215-222`，用户点"放弃验证"，`CaptchaBox.vue:73-86`）才降级——功能上少了一条"不打扰用户"的路径。

**缺失影响**

- 用户在"重放被拒"之后**无法在同一挑战上再试一次**——这正是"验证码过了但服务端说不对"（validate 过期、网络抖动后重放）最常见的场景，参考项目允许 `track_challenge` 重置守卫再走一轮（我方 `client.rs:447` 也有 `replay_guard.reset()`，但只在**新挑战**到达时才调用，重放失败这条路径永远不会重置）。
- `detail()` 首次访问必然触发一次 `CaptchaNeeded`，`CaptchaBox` 弹出，用户点"放弃"才看得到公开页——比参考项目多两次交互。

**补充方案**

```rust
// client.rs —— 把"重放结果"从 Result 提升为显式结局，供命令层区分文案与下一步
#[derive(Debug)]
pub enum ReplayOutcome {
    /// 服务端接受了凭证，已按会话绑定落库
    Accepted(Value),
    /// 服务端明确拒绝（仍是挑战 / err_* / status≠0,1），凭证已清
    Rejected(Error),
    /// 传输层失败，凭证按参考项目语义保留
    Transport(Error),
}

pub async fn retry_after_validation(&mut self, p: &PendingCaptcha)
    -> ReplayOutcome
{
    if !self.replay_guard.may_replay() {
        // 关键：不再返回死胡同，而是允许用户重新走一轮
        self.replay_guard.reset();
    }
    self.replay_guard.consume();

    let sessid_before = self.session.lock().await.sessid.clone();
    let response = /* 同现有 :287-313，按 placement 构造 */ ;
    let set_cookies = response.set_cookies();

    // 1) 先判 Cookie 回写（但不要动 validation —— 由下面的绑定逻辑决定）
    let result = decode_response(&response);

    match result {
        Ok(body) => {
            let sessid_after = self.session.lock().await.sessid.clone();
            // 2) 参考 :77-94：只有 Cookie 模式、且 SESSID 未被替换，才落库
            if p.placement() == ProofPlacement::Cookie
                && sessid_after == sessid_before
                && coolall_core::error::is_success(&body) {
                self.session.lock().await.accept_validation(proof.clone());
            }
            // 3) 查询参数模式：凭证是一次性票据，绝不进 Cookie（参考 :75 的反向分支）
            ReplayOutcome::Accepted(body)
        }
        Err(e @ Error::Network { .. }) => ReplayOutcome::Transport(e), // 保留凭证
        Err(e) => {
            self.session.lock().await.clear_validation();
            ReplayOutcome::Rejected(e)
        }
    }
}
```

对应命令层（`commands.rs:196-207`）按三种结局分别返回 `error.kind`：`captcha_needed` 时前端 `CaptchaBox` 才能显示"请重新验证"而不是通用错误。

---

### 2. `CaptchaCoordinator` 的 epoch 机制

**参考做法**

`CaptchaCoordinator.kt`：

```kotlin
val epoch = AtomicLong()                       // :29

fun request(challenge, valid: () -> Boolean): String? {
    val pending = Pending(challenge)           // :39
    if (!current.compareAndSet(null, pending)) return null   // :40  ← 单飞：已有验证在跑就放弃
    epoch.incrementAndGet()                    // :41  ← 新流程使一切旧结论作废
    main.post { /* 无 Activity → complete(null) */ }          // :42-49
    return try {
        val deadline = System.nanoTime() + 5min                // :51
        while (valid() && System.nanoTime() < deadline) {       // :52
            try { return pending.result.get(250, MILLISECONDS) } // :54
            catch (_: TimeoutException) {}
        }
        null                                                    // :57 超时 → 放弃
    } finally {
        pending.result.complete(null)          // :59
        current.compareAndSet(pending, null)   // :60  ← 无论成败都释放单飞锁
    }
}
```

`epoch` 的两个消费点在 `CaptchaInterceptor.kt`：

- `:13` 进入拦截器时快照 `val epoch = CaptchaCoordinator.epoch.get()`
- `:33` **等待用户验证之前**再读一次，变了就当作"这个结果已经过时"，不复用
- `:24-32` **更重要的用法**：若本次响应要求验证，但 `ApiSessions.store.validation()` 里已经出现了一个**新的**凭证（`currentValidation != sentValidation`），说明并发请求里已经有人验过了 → `response.close(); return chain.proceed(request)`，**直接重放一次，完全不弹窗**。

**我方现状**

- **无单飞**：`open_window`（`captcha_window.rs:191-235`）每次调用都会 `old.close()` 后重建窗口（`:203-205`）。快速点两次"打开验证窗口"（`CaptchaBox.vue:107`）会**先关后开**，把用户已经滑完的滑块面板直接销毁，易盾 SDK 从头加载。
- **无会话版本**：`PendingCaptcha`（`captcha.rs:39-46`）只有 `{id, field, original_url}`。`client.rs:147 staged_validate` 是裸 `Option<String>`，**没有"属于哪个挑战"**。因此：
  - 旧窗口如果延迟回传（易盾的 `onVerify` 在窗口关掉后仍可能触发一次），`captcha_result`（`commands.rs:158-174`）只校验 `window.label() == "captcha"`，**会覆盖当前 challenge 的 staged validate**。
  - 用户先遇到挑战 A，放弃、再浏览遇到挑战 B，然后 A 的旧 `complete_captcha` 仍可能被前端触发——`commands.rs:186` 取的是**当前** pending（B），却把 A 的 validate 配 B 的 captchaId 重放 → 必然被拒 → 又消耗一次重放机会。
- **`ReplayGuard::reset()` 语义错位**：设计文档说"用户重新验证成功后调用"（`captcha.rs:105-108`），但实际只在 `track_challenge`（`client.rs:447`）里调用，即**只在新挑战出现时**。"同一挑战下重新验证"这条路（第 1 条的死胡同）没有被复位。

**缺失影响**

- 并发场景（列表页 + 详情页同时 1004，或用户双击）→ 两个窗口层叠、相互关闭、`staged_validate` 互相覆盖；
- 已有人验证成功时，其它在途请求**不会**像参考项目 `:24-32` 那样"捡现成凭证直接重放"，而是照旧弹窗，用户被迫重复验证；
- 旧挑战的迟到结果可以污染新挑战。

**补充方案**

```rust
// coolall-core/src/captcha.rs —— 给 pending 加上代次与一次性回传槽位
#[derive(Debug, Clone)]
pub struct PendingCaptcha {
    pub id: String,
    pub field: String,
    pub original_url: String,
    /// 每次 track_challenge 生成新值；只有它的回传才被接受
    pub epoch: u64,
}

// client.rs
pub struct CoolapkClient {
    epoch: u64,                      // 单调递增
    staged: Option<(u64, String)>,   // (epoch, validate)
}

pub fn stage_validate(&mut self, epoch: u64, validate: String) -> bool {
    if Some(epoch) != self.pending_captcha.as_ref().map(|p| p.epoch) {
        return false;            // 旧挑战的迟到结果，丢弃
    }
    self.staged = Some((epoch, validate));
    true
}

// 并发请求进来时，先"捡现成凭证"再决定是否弹窗 —— 对应参考 :24-32
async fn maybe_use_existing_validation(&self) -> bool {
    self.session.lock().await.validation.is_some()
}
```

配合"单飞"的等价物（Tauri 侧用一个 `AtomicBool`）：

```rust
// captcha_window.rs
static CAPTCHA_OPEN: AtomicBool = AtomicBool::new(false);

pub fn open_window<R: tauri::Runtime>(app: &AppHandle<R>, id: &str, epoch: u64)
    -> Result<bool, String>      // Ok(false) = 已有一个在进行中，本次不重复打开
{
    if CAPTCHA_OPEN.swap(true, Ordering::SeqCst) {
        return Ok(false);        // 对应 CaptchaCoordinator.kt:40
    }
    // 不再 close 旧窗口，而是复用同一窗口、换 captchaId
    ...
}
// 窗口 closed / 5 分钟超时 / complete_captcha 调用后统一置回 false
```

超时与取消（`CaptchaCoordinator.kt:51-58`、`CaptchaActivity.kt:35, 104`）：Tauri 侧用 `.on_window_event(|e| if let Destroyed = e.event() { cancel_pending() })`，取消时 `pending_captcha = None` 且 `staged = None`——**这一条我现在完全没有**，窗口一关，后端仍认为挑战待办（`client.rs:413-415`）。

---

### 3. `validate` 的两种携带方式 vs POST 表单

**参考做法**

参考项目**实际有三种**携带位置，由 `challenge.field` 是否为空 + 原请求是否有 body 共同决定（`CaptchaInterceptor.kt:41-70`）：

```kotlin
val retry = request.newBuilder()
if (challenge.field.isEmpty())
    retry.tag(CaptchaProof::class.java, CaptchaProof(proof))   // :42  Cookie 路线（由 AddCookiesInterceptor:62-74 落到 Cookie 头）
else when (val body = request.body) {                          // :44
    is FormBody -> { ... }                                     // :45-50  application/x-www-form-urlencoded 里加字段
    is MultipartBody -> { ... }                                // :51-65  multipart 里加 form-data part
    else -> retry.url(request.url.newBuilder()                     // :66-69  无 body（GET）→ 查询参数
        .setQueryParameter(challenge.field, proof).build())
}
```

两处细节必须照抄：

- `:47-48` **先删同名旧字段再加**（`encodedName/encodedValue` 保留原编码，不二次转义）。`FormBody` 若残留旧的 `_v2_post_token`，服务端可能取第一个。
- `:51-52` `body.isOneShot()` → **直接放弃重放**（一次性流无法重建），返回原响应。这是"不能重放就老实说不能"的功能分支。
- `:59` multipart 里同样**先按 `Content-Disposition: name="<field>"` 过滤掉旧 part**。

Cookie 路线在 `AddCookiesInterceptor.kt:62-74`：

```kotlin
if (api) {
    val validation = request.tag(CaptchaProof::class.java)?.encoded   // :64 重放时用请求级 proof
        ?: ApiSessions.store.validation()                             //    否则用已缓存凭证
    if (validation != null) {
        val cookie = builder.build().header("Cookie").orEmpty()
        builder.header("Cookie", listOf(cookie, "validate=$validation").filter{...}.joinToString("; "))
    }                                                                    // :69 validate 永远追加在最后
}
```

注意两点：① `:64` 有**请求级优先**——重放请求用它自己那份 proof，而不是 store 里可能已被清除的；② `:50-60` 只有 `isApi(url)` 为真才带这些头，公开页（`isPublicApi` 之外）一律 `publicRequest`（`CoolapkRequestPolicy.kt:13-14, 29-46`）剥掉所有私有头。

**我方现状**

- `captcha.rs:49-55`：`field` 空 → `Cookie`，非空 → `QueryParam`。**只有两条路，缺 `FormBody` 与 `MultipartBody`**。
- `captcha.rs:65-74 url_with_proof`：无条件拼查询串，**不判断原请求是不是 POST、有没有 body**。对 `@FormUrlEncoded @POST` 的 14 个接口（`ApiService.kt:107, 152-153, 168-170, 176-177, 216-217, 230, 236-237, 242-243, 291-292, 316-317, 329-330, 383-384`），以及 `@Multipart` 的 4 个（`:114-115, 300-301, 360-361`），服务端期待的是**表单字段**，塞在 query 上等于没带。
- `transport.rs:44-63 HttpRequest` 有 `method` 与 `body` 字段，但**只有 `HttpRequest::get` 一个构造函数**（`:53-63`），`HttpMethod::Post`（`:32`）在整个 `src-tauri` 里没有任何调用点。也就是说：**我方现在根本发不出 POST**，`client.rs` 的 6 个业务方法全是 GET（`:197-213, 337-367`）。

**缺失影响**

- 这是"它能做但我们没做"里**最大的功能性鸿沟**：参考项目所有写操作（点赞 `postLike`、回复 `postReply`、关注、发动态、上传）都走 POST，都可能在 `err_request_captcha_v2` 之后需要带 `_v2_post_token`。我方阶段 1 只有只读 GET，**表面看没坏，一旦加写接口就必然踩空**。
- 更隐蔽的是：即便是 GET，参考项目 `:42` 的 Cookie 分支用的是 **request tag** 而不是全局 store——因为 GET 重放时 store 里的凭证可能已被并发请求清掉（第 1 条第 2 点的场景）。
- `:66-69` 的 `setQueryParameter` 会**替换**同名参数；我方 `captcha.rs:67` 是 `&` 直接追加，若原 URL 已带 `_v2_post_token`（用户重试）会出现重复参数。

**补充方案**

```rust
// coolall-core/src/captcha.rs —— 把 proof 的落点从"2 选 1"扩成"3 选 1 + 不可重放"
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofPlacement {
    /// field 为空 → Cookie: validate=...
    Cookie,
    /// 无 body 的请求（GET/DELETE）→ 查询参数
    QueryParam,
    /// application/x-www-form-urlencoded → 表单字段（先删同名再加）
    FormField,
    /// multipart/form-data → form-data part（先过滤同名 part 再加）
    MultipartPart,
    /// one-shot body → 无法构造重放，调用方必须放弃重放
    NotReplayable,
}

impl PendingCaptcha {
    pub fn placement_for(&self, req: &HttpRequest) -> ProofPlacement {
        if self.field.trim().is_empty() { return ProofPlacement::Cookie; }
        match (&req.method, req.body.as_ref(), req.content_type()) {
            (HttpMethod::Get, _, _) => ProofPlacement::QueryParam,
            (_, Some(_), Some(ct)) if ct.starts_with("application/x-www-form-urlencoded")
                => ProofPlacement::FormField,
            (_, Some(_), Some(ct)) if ct.starts_with("multipart/form-data")
                => ProofPlacement::MultipartPart,
            (_, Some(_), _) => ProofPlacement::NotReplayable,   // 对齐 :52 isOneShot
            (_, None, _) => ProofPlacement::QueryParam,
        }
    }

    /// 表单重写：删同名、保编码、再加新值 —— 对齐 CaptchaInterceptor.kt:45-50
    pub fn rewrite_form(&self, body: &str) -> String {
        let kept: Vec<&str> = body.split('&')
            .filter(|kv| kv.split('=').next() != Some(&*urlencoding::encode(&self.field)))
            .collect();
        let mut out = kept.join("&");
        if !out.is_empty() { out.push('&'); }
        out.push_str(&format!("{}={}", urlencoding::encode(&self.field), proof));
        out
    }

    /// 查询参数：替换而非追加 —— 对齐 :68 setQueryParameter
    pub fn url_with_proof(&self, proof: &str) -> String {
        replace_query_param(&self.original_url, &self.field, proof)
    }
}
```

`client.rs:287-313` 的重放分支改成按 `placement_for` 分派，`NotReplayable` 时返回明确错误"该请求无法重放，请重新发起"（而非现在的静默走查询参数）。

---

### 4. `ApiFailure.CAPTCHA` 的消费方

**参考做法**

`ApiFailure.kt:10-19` 定义 8 类，`:47-48` 把 `captcha` 归为 `CAPTCHA`。真正有意思的是**谁消费它**：

1. `ApiFailureInterceptor.kt(=ApiFailure.kt):89-94` **放行 `err_request_captcha`**（不带 `_v2`），让下游 `CaptchaInterceptor` 先处理；只有 `CaptchaInterceptor` 判定"处理不了 / 用户放弃"时，这个响应才以 `ApiFailure(CAPTCHA)` 的形态抛给 Repository。
2. `NetworkRepo.kt:56-89` **分类处理路径不同**：
   - `verifyCaptcha = false`（默认，首次加载）→ 请求带 `SkipCaptcha()`（`:62-63`），**不弹验证**；
   - 捕获到 `CAPTCHA` 或 `AUTHENTICATION` 且 id 是纯数字（`:67-75`）→ **立刻**降级 `https://www.coolapk.com/feed/{id}`（`:79`）；
   - 公开页也失败才把原失败抛出去（`:83-86`）。
3. `FeedViewModel.kt:339` 用户点重试时 `fetchFeedData(verifyCaptcha = true)`——这是一条**显式升格**：只有用户主动要求时才允许弹验证。
4. `:59-66 safeMessage` 做人话映射；`ApiFailure.expiredSession`（`:21`）再区分"登录信息有误"。

也就是说 `CAPTCHA` 类型在参考项目里触发三件事：**（a）一次静默降级尝试；（b）把"是否打扰用户"的决策权交给调用点（SkipCaptcha 有无）；（c）用户显式重试时才开始真体验证流程。**

**我方现状**

- `error.rs:92-119` 的 `Error` 有 `CaptchaNeeded{challenge}`，`error.rs:200-221` 的分类与参考项目基本对齐（还多挡了 `status==1004` 的默认挑战，`:205-209`）。
- **消费方只有 2 个**：
  - `client.rs:439-449 track_challenge`——记录 pending；
  - `client.rs:384-387 detail` 的公开页降级判据，且**必须 `allow_public_fallback` 为真**。
- **其它 5 个方法都不看它**：`browse`（`:336-345`）、`search`（`:347-356`）、`replies`（`:358-367`）只 `track_challenge` 就原样返回 `Err`，前端拿到 `captcha_needed` 后 `ErrorBox.vue:29-35` 弹 `CaptchaBox`——**没有"静默降级"这一层**。
- **没有 `SkipCaptcha` 等价物**：无法表达"这次请求允许不弹验证"。
- **前端缺"显式升格"语义**：`CaptchaBox.vue:107` 的"打开验证窗口"是唯一的入口，任何 `captcha_needed` 都会直接弹它。
- `commands.rs:96-101 submit_validation` 把所有 Err 都包成 `Business{1004}`，把 `AuthRequired` / `Network` / `Parse` 的区分**抹平**了（对比 `client.rs:482-488` 本来是有 `Error` 类型的）。

**缺失影响**

- 首页/搜索/评论遇到 1004 时，用户体验是"打开就弹验证框"，而不是参考项目的"先给你看能看的内容，只有你要求时才验证"；
- 分类信息在命令层被压扁成 1004，前端无法区分"验证被拒"、"登录失效"、"网络错误"，`ErrorBox` 的 `retryable` 只能靠 `message` 字符串猜；
- `Error::retryable()`（`error.rs:122-128`）不含 `CaptchaNeeded`，前端 `ApiError.retryable`（`src/api/index.ts:43-45`）也不含 `captcha_needed`——这是对的，但因为没有别的消费方，`captcha_needed` 实际上只剩"弹框"一个出口。

**补充方案**

```rust
// 1) 请求级"允许跳过验证"，对齐 SkipCaptcha
pub struct RequestOptions {
    /// true = 遇到 1004 不要弹验证，让调用方走降级
    pub skip_captcha: bool,
}

pub async fn detail_with(&mut self, id: &str, opt: RequestOptions) -> Result<Value, Error> {
    let result = self.send(base, &path).await;
    if opt.skip_captcha {
        return result;          // 不 track_challenge，也就不弹窗
    }
    self.track_challenge(&result, base, &path);
    result
}

// 2) 命令层不再压扁错误种类
#[tauri::command]
pub async fn submit_validation(...) -> Result<InvokeResult<Value>, String> {
    match client.submit_validation(&challenge_id, &field, &validate).await {
        Ok(data) => Ok(InvokeResult::success(data)),
        Err(outcome) => Ok(InvokeResult::failure(outcome.into_error())), // 保留 kind
    }
}
```

前端相应地在 `ErrorBox.vue:29` 之前加一层：`if (captcha_needed && canSkip) { 自动 setPublicFallback → retry }`。

---

### 5. 验证成功后凭证的缓存与复用

**参考做法**——这是参考项目**最完整**的一块，四个维度：

**(a) 绑定到"服务端接受的会话"**
`ApiSessionStore.kt:122-137`：

```kotlin
fun acceptValidation(proof: String, context: Context, acceptedSession: String) {
    if (context.identity != active() || acceptedSession != hash()) return   // :123
    state = state.copy(validation = Validation(
        identity = active().hash,
        session = acceptedSession,          // SESSID 的 SHA-256
        proof = proof,
        expires = now() + 86400000L,        // :131  1 天
        process = process, generation = active().generation,
    ))
    persist()                                // :136  落盘
}
```

**(b) 读取时有五道闸门**
`ApiSessionStore.kt:74-82`：身份 hash 一致 + 会话 hash 一致 + 未过期 + （同进程 或 设备代次一致）。

**(c) 请求时自动附加**
`AddCookiesInterceptor.kt:62-74`：**每个 API 请求**都读 store 里的 validation 并追加到 Cookie 末尾；重放时用请求级 tag 覆盖（`:64`）。这就是"验证一次，之后所有请求都带"。

**(d) 失效即清**
`ApiSessionStore.kt:113`（SESSID 改变）、`CaptchaInterceptor.kt:95`（重放被拒）、`ApiSessionStoreTest.kt:46-55`（会话被替换 / 登出）。另外 `CaptchaInterceptor.kt:35` 在**发起验证前**就先 `clearValidation()`——避免用旧凭证去验新挑战。

**(e) 跨重启**
`persist()`（`:152-154`）写 SharedPreferences；`ApiSessionStoreTest.kt:41-42` 断言"重建 store 后凭证仍在"。

**我方现状**

- **有复用（Cookie 模式）**：`request.rs:58-60` 每个请求都拼 `validate=`，`request.rs:249-257` 的测试也覆盖了。**但这是错的复用**：`QueryParam`/表单模式的凭证也被无条件塞进 Cookie（第 1 条第 3 点），而参考项目只在 `field` 为空时才用 Cookie。
- **无会话绑定**：`client.rs:489-492` 直接 `accept_validation(proof)`；`Session`（`request.rs:24-34`）里没有 `sessid_hash` 字段，无法判断"这个凭证是不是针对当前 SESSID 的"。
- **无有效期**：`Session.validation: Option<String>`（`request.rs:29`），永不超时。参考项目 1 天。
- **不持久化（但有理由）**：`client.rs:150-169 persist_session` 显式把 `validation` 置 `None`，注释写明"短时效、不落盘"。这点**与参考项目不同但是安全的**——因为参考项目把凭证存进了 SharedPreferences，我方选择只存内存。功能上的代价是：**应用重启后用户必须重新验证**。这是一个可以接受的取舍（下面列入阶段 2），但至少要**在 UI 上说清楚**。
- **没有"发起验证前先清旧凭证"**：`open_captcha_window`（`commands.rs:121-134`）不清 `Session.validation`，旧凭证会和新 proof 同时存在于 Cookie（`request.rs:58-60` 只拼一次 `validate=`，不会被覆盖，但会带着一个已知无效的值去重放）。

**缺失影响**

- **最严重的行为差异**：参考项目里用户验证一次，后续 1 天内的**所有** API 请求自动带 `validate` → 不再 1004。我方 `QueryParam` 模式的凭证只活在那一次重放的 URL 里，**下一个请求（分页、点详情）又会 1004**，用户会陷入"每翻一页验一次"的循环。
- 凭证过期后我方无感知，仍继续带一个失效的 `validate=` Cookie，可能导致服务端**持续拒绝**而不是"重新下发挑战"——用户会看到反复的 `err_*` 而非可操作的验证入口。

**补充方案**

```rust
// request.rs —— Session 增加凭证生命周期，但保持"不落盘"
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    pub sessid: Option<String>,
    #[serde(skip)]                     // 继续不落盘
    pub validation: Option<Validation>,
    pub uid: Option<String>, pub username: Option<String>, pub token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Validation {
    proof: String,
    /// 绑定写入时的 SESSID（没有 SESSID 时绑定到空串）
    sessid: Option<String>,
    /// 设备身份，切换 shuzlmId 后凭证作废
    identity: String,
    expires_at: std::time::Instant,
    /// 仅 Cookie 模式落库；QueryParam/表单模式不落库
    placement: ProofPlacement,
}

impl Session {
    pub fn accept_validation(&mut self, proof: String, placement: ProofPlacement) {
        if placement != ProofPlacement::Cookie { return; }   // 对齐 CaptchaInterceptor.kt:75
        self.validation = Some(Validation {
            proof, sessid: self.sessid.clone(),
            identity: String::new(),                     // 由调用方填
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(86_400),
            placement,
        });
    }

    /// 读取时五道闸门 —— 对齐 ApiSessionStore.kt:74-82
    pub fn active_validation(&self) -> Option<&str> {
        self.validation.as_ref()
            .filter(|v| v.sessid == self.sessid)
            .filter(|v| v.expires_at > std::time::Instant::now())
            .map(|v| v.proof.as_str())
    }
}
```

`cookie_header()`（`request.rs:45-66`）改用 `active_validation()`；`client.rs` 在 `open_captcha_window` 之前先 `clear_validation()`（对齐 `CaptchaInterceptor.kt:35`）。

---

### 6.（补充发现）并发点击"刷新"会不会弹两个窗口

用户提的第 3 问，单独确认：

- **后端**：`open_window`（`captcha_window.rs:202-205`）**有**旧窗口关闭，所以不会真的层叠两个窗口。但这是"先关后开"，代价是**丢失用户已完成的那一步**。
- **前端**：`CaptchaBox.vue:107` 的按钮不会禁用，`openWindow()`（`:41-52`）没有进行中守卫；`confirmDone()`（`:55-70`）只有 `submitting` 防重入，但它只挡"确认"，不挡"打开"。快速点两次"我已完成验证" → 第二次进入时 `staged_validate` 已被 `take`（`commands.rs:189`），返回"还没有收到验证结果"——文案与事实不符（用户明明验证过了）。
- **并发请求**：`AppState.client` 是单个 `Mutex`（`lib.rs:35`），所有请求串行。所以**参考项目 `CaptchaInterceptor.kt:24-32` 的"有人已验过，我捡现成"这个优化在我方永远不会触发**（因为不存在真正的并发请求）。功能上这算"少一个优化"，但一旦未来放开并发（例如分页预取），缺它就会造成重复弹窗。

**补充方案**：见第 2 条——`CAPTCHA_OPEN: AtomicBool` 单飞 + `epoch` 校验迟到回传；前端 `CaptchaBox.vue:107` 的按钮在 `submitting || opening` 时 `disabled`。

---

### 7.（补充发现）"窗口关了/用户放弃"后端不知道

参考项目 `CaptchaActivity.kt:35, 104`：

```kotlin
pending.result.whenComplete { _, _ -> runOnUiThread { finish() } }      // :35 结果一到就关页面
override fun onDestroy() {
    if (!isChangingConfigurations) requestId?.let { CaptchaCoordinator.complete(it, null) }  // :104  ← 关窗 = 取消
}
```

`CaptchaCoordinator.kt:44` 还会在"没有可用的 Activity"时立即 `complete(null)`。

我方：`captcha_window.rs:237-242 close_window` 只关窗口，**不通知 `CoolapkClient`**。后果：用户关掉验证窗口后，`pending_captcha` 仍非 `None`（`client.rs:413-415`），`session_info` 仍报告 `challengeId`，前端下次进详情页仍会弹 `CaptchaBox` 且显示"打开验证窗口"。5 分钟超时（参考 `:51`）我方也完全没有——一个卡死的窗口可以让整个验证流程永远挂起。

**补充方案**：

```rust
// captcha_window.rs
pub fn close_window<R: tauri::Runtime>(app: &AppHandle<R>, state: &AppState) {
    CAPTCHA_OPEN.store(false, Ordering::SeqCst);
    if let Some(win) = app.get_webview_window(CAPTCHA_WINDOW_LABEL) { let _ = win.close(); }
    tauri::async_runtime::block_on(async {
        let mut c = state.client.lock().await;
        c.cancel_pending();      // pending_captcha=None, staged=None, replay_guard.reset()
    });
}
```

外加 `.on_window_event` 捕获 `Destroyed`，以及命令层记录 `opened_at`，超过 5 分钟未完成即视为取消（下次 `browse/detail` 时清理）。

---

## 阶段 1 必须补的

按"不补就功能不成立"排序，5 项：

| # | 项目 | 参考依据 | 工作量 | 为什么是阶段 1 |
|---|---|---|---|---|
| **P1** | **`ProofPlacement` 扩到 4 种 + `NotReplayable`**：`FormField` / `MultipartPart` / 查询参数**替换而非追加** | `CaptchaInterceptor.kt:41-70` | 小（纯 core 函数 + 单测） | 写操作只要上线就会踩；GET 的重复参数也是现成 bug。且**不需要先有 POST 接口就能先写好并测好** |
| **P2** | **凭证绑定会话 + 有效期 + 只在 Cookie 模式落库** | `ApiSessionStore.kt:74-82, 122-137`；`CaptchaInterceptor.kt:75` | 中 | 决定"验证一次能管多久"；也是修 `request.rs:58-60` 无条件塞 Cookie 的正确姿势 |
| **P3** | **重放结局三态化（Accepted / Rejected / Transport）+ 守卫可复位** | `CaptchaInterceptor.kt:71-96` | 小 | 直接消灭"验证被拒后死胡同"；让前端能区分文案 |
| **P4** | **取消 / 超时 / 关窗收敛 + 迟到回传 epoch 校验** | `CaptchaCoordinator.kt:38-62`；`CaptchaActivity.kt:104` | 小-中 | 没有它，窗口关了后端还在等，用户只能重启 |
| **P5** | **命令层不再把错误压扁成 1004**（`commands.rs:96-101, 202-206`） | `ApiFailure.kt:10-19, 59-66` | 极小 | 一行级别，直接决定前端能不能显示正确提示 |

> 注意 P5 是实现 P3 的前置：不先把 `Error` 的 `kind` 透出去，P3 的三态化在前端看不见。

---

## 可以推到阶段 2 的

| # | 项目 | 参考依据 | 推迟理由 |
|---|---|---|---|
| S1 | **`SkipCaptcha` 请求级标记 + "先降级公开页、用户要求才验证"** | `ApiService.kt:49, 63`；`NetworkRepo.kt:56-89`；`FeedViewModel.kt:339` | 属体验优化；阶段 1 的"用户点放弃才降级"（`client.rs:384-387`）已经能用，只是多两次点击 |
| S2 | **并发单飞（`CAPTCHA_OPEN`）+ 捡现成凭证** | `CaptchaCoordinator.kt:40`；`CaptchaInterceptor.kt:24-32` | 当前 `AppState.client` 单锁串行（`lib.rs:35`），并发弹窗实际不可达；放开并发前必须补 |
| S3 | **凭证跨重启持久化（1 天）** | `ApiSessionStore.kt:136, 152-154`；`ApiSessionStoreTest.kt:41-44` | 我方刻意不落盘（`client.rs:150-169`）是安全取舍。若要在阶段 2 支持，至少用 OS 凭据库而非明文 JSON |
| S4 | **按域/路径裁剪 SESSID 与私有头（`publicRequest`）** | `CoolapkRequestPolicy.kt:13-46`；`ApiSessionStore.kt:60-71` | 我方暂无公开页以外的主机调用；一旦接入图片/CDN 或 `www.coolapk.com` 才需要 |
| S5 | **页面关闭时的 `isChangingConfigurations` 语义、5 分钟倒计时 UI** | `CaptchaActivity.kt:104`；`CaptchaCoordinator.kt:51` | 属 P4 的打磨项 |

---

## 附：完整度矩阵（逐文件、逐行为）

| 参考行为 | 位置 | 我方等价物 | 状态 |
|---|---|---|---|
| 非 API 域不参与验证 | `CaptchaInterceptor.kt:16-17` | 无（我方只调 api/api2） | ✅ 天然满足 |
| `SkipCaptcha` 跳过 | `:18`；`ApiService.kt:49,63` | 无 | ❌ S1 |
| 识别 `err_request_captcha_v2` + 兜底 id/field | `:21`；`CaptchaChallenge.kt:24-48` | `error.rs:46-86` | ✅ |
| 挑战绑定请求会话 | `:22-23`；`RequestSession.kt:8-9` | 无 | ❌ P4（epoch 部分） |
| **已有人验过 → 直接重放不弹窗** | `:24-32` | 无 | ❌ S2 |
| epoch 快照 + 时效校验 | `:13, :33`；`CaptchaCoordinator.kt:29,41` | 无（`pending.id` 匹配是弱等价，`client.rs:479-481`） | ⚠️ 部分 |
| 发起验证前清旧凭证 | `:35` | 无 | ❌ P2 |
| Cookie 路线（请求级 proof 优先） | `:42`；`AddCookiesInterceptor.kt:62-74` | `request.rs:58-60`（只有 store 级，且无条件） | ⚠️ 半 |
| **FormBody 加字段（删同名、保编码）** | `:45-50` | 无 | ❌ P1 |
| **Multipart 加 part / one-shot 放弃** | `:51-65` | 无 | ❌ P1 |
| 无 body → 查询参数（替换语义） | `:66-69` | `captcha.rs:65-74`（追加语义） | ⚠️ 半-P1 |
| 只重放一次 | `:71-74` | `captcha.rs:89-109 ReplayGuard` + `client.rs:279-285` | ✅ |
| 成功后用 `accepted()` 真判成功 | `:84-86, :105-114` | `error.rs:135-168`（**重放路径未调用**） | ⚠️ P3 |
| 凭证绑定 AcceptedApiSession | `:77-94`；`ApiSessionStore.kt:122-137` | 无 | ❌ P2 |
| 失败清凭证 | `:95` | `client.rs:327-334` | ✅（时机偏宽，P3 修正） |
| 凭证 1 天有效 + 五道闸门 | `ApiSessionStore.kt:34-41, 74-82` | 无 | ❌ P2 |
| 凭证跨重启 | `ApiSessionStore.kt:136` | 刻意不落盘 | ⏭ S3 |
| SESSID 变更清凭证 | `ApiSessionStore.kt:113` | `request.rs:93-95` | ✅ |
| 登出/清空会话清凭证 | `ApiSessionStore.kt:145-150` | `request.rs:85-89` | ✅ |
| 单飞（已有验证在跑就放弃） | `CaptchaCoordinator.kt:40` | `captcha_window.rs:203-205`（先关后开，非单飞） | ❌ S2 |
| 5 分钟超时 | `:51-58` | 无 | ❌ P4 |
| 关窗 = 取消 | `CaptchaActivity.kt:104` | 无 | ❌ P4 |
| 8 类错误 + `expiredSession` | `ApiFailure.kt:10-21` | `error.rs:92-119`（无 `expiredSession`） | ⚠️ 部分 P5 |
| `err_request_captcha` 放行给下游 | `ApiFailure.kt:89-94` | 无等价分层（`classify` 一步到底） | ⚠️ 部分 |
| 公开页降级（CAPTCHA/AUTH + 纯数字 id） | `NetworkRepo.kt:66-89` | `client.rs:384-403`（需 `allow_public_fallback`） | ⚠️ 半-S1 |
| 单测覆盖挑战解析 / 凭证 / 会话绑定 | `CaptchaChallengeTest.kt`；`ApiSessionStoreTest.kt` | `error.rs:243-355`、`captcha.rs:111-169`、`client.rs:561-867` | ✅（但**缺并发、TTL、POST 三类用例**，随 P1-P4 补） |

**统计**：✅ 9 项 / ⚠️ 部分 8 项 / ❌ 缺 9 项（含 5 项阶段 1、4 项阶段 2、其中 1 项为刻意取舍）。
