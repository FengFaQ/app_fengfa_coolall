# 架构设计

> 本文档描述 `coolall` 的实际结构。**以代码现状为准**，不写未实现的设计目标。
>
> 最后更新：2026-10-31（commit 2cafe70）

---

## 1. 总体形状

```
┌──────────────────────────────────────────────────┐
│                 UI 层（Vue 3）                    │
│  Home / Search / FeedDetail / Settings           │
│  src/                                            │
└─────────────────────┬────────────────────────────┘
                      │ invoke('命令名', {...})
┌─────────────────────▼────────────────────────────┐
│           外壳层（src-tauri，含 GUI 依赖）         │
│  lib.rs    —— 7 个 #[tauri::command]             │
│  client.rs —— CoolapkClient（HTTP + 会话 + 兜底） │
│  依赖 tauri / reqwest / tokio                     │
└─────────────────────┬────────────────────────────┘
                      │ 普通函数调用
┌─────────────────────▼────────────────────────────┐
│      协议核心（crates/coolall-core，无 GUI 依赖）   │
│  token_v3.rs —— 请求签名                          │
│  device.rs   —— 设备身份编码与 UA                  │
│  request.rs  —— 请求头拼装与会话结构               │
│  error.rs    —— 错误分类与验证码识别               │
│  只依赖 base64 / md-5 / bcrypt / serde            │
└─────────────────────┬────────────────────────────┘
                      │ HTTPS
                ┌─────▼─────┐
                │ 酷安服务端 │
                └───────────┘
```

---

## 2. 为什么拆成两个 crate

**这是本项目最重要的一个架构决定。**

```
crates/coolall-core   无 GUI 依赖 → 任何环境秒级编译测试
src-tauri             含 GUI 依赖 → 只在打包时编译
```

拆分的直接理由（来自实测）：

`cargo test --lib` 会编译整个 crate 的依赖树。当协议逻辑与 `tauri` 在同一个 crate 时，
测试必须安装 WebKitGTK、libappindicator 等重型系统库，CI 每次都要多花几分钟装依赖，
本地开发也需要完整桌面环境。

拆分后：

| 指标 | 拆分前 | 拆分后 |
| --- | --- | --- |
| 测试所需系统依赖 | WebKitGTK + GTK3 + libsoup 等 | 无 |
| 首次编译耗时 | 数分钟 | 约 15 秒 |
| 41 项测试耗时 | 取决于环境 | 0.01 秒 |

**代价**：多一层 crate 边界，`src-tauri` 需要 `path` 依赖引入。
**判断**：收益远大于代价，而且将来若要接其他外壳（命令行、移动端）可以直接复用核心。

---

## 3. 模块清单（以代码为准）

### 3.1 `crates/coolall-core`

#### `token_v3.rs`

```rust
pub fn create(device: &str, app_code: i64, timestamp: i64) -> Result<String, TokenError>;
```

纯函数，无状态、无 IO。**最值得测试的模块**，因为输出格式严格、重试有分支、
出错时无法从服务端响应反推原因。

内部关键函数：
- `segment_for(remainder: u32)` —— 查表取片段，含两层 base64 解码
- `decode_bcrypt_salt(salt: &str)` —— 22 字符 → 16 字节，含两条合法性规则

#### `device.rs`

```rust
pub struct DeviceIdentity { pub szlm_id: String, pub profile: DeviceProfile }

impl DeviceIdentity {
    pub fn device_header(&self) -> String;          // X-App-Device
    pub fn user_agent(&self) -> String;             // 三段式 UA
    pub fn random_szlm_id() -> String;
    pub fn valid_szlm_id(id: &str) -> bool;
    pub fn parse_shuzlm_id(text: &str) -> Result<String, String>;
    pub fn decode_device_header(h: &str) -> Option<String>;  // 自检用
}
```

#### `request.rs`

```rust
pub struct Session { sessid, validation, uid, username, token }

impl Session {
    pub fn cookie_header(&self) -> Option<String>;
    pub fn update_from_set_cookie(&mut self, headers: &[String]);
    pub fn accept_validation(&mut self, proof: impl Into<String>);
    pub fn clear_validation(&mut self);
}

pub fn build_headers(ctx: &RequestContext) -> Result<HashMap<String, String>, TokenError>;
```

#### `error.rs`

```rust
pub enum Error { Network, Http, Business, CaptchaNeeded, AuthRequired,
                 RateLimited, ClientUpgrade, RiskControl, Parse }

pub fn is_success(body: &Value) -> bool;
pub fn classify(body: &Value) -> Option<Error>;
pub struct CaptchaChallenge { pub id: String, pub field: String }
```

### 3.2 `src-tauri`

#### `client.rs` —— `CoolapkClient`

| 方法 | 说明 |
| --- | --- |
| `new(app)` | 读配置，生成或恢复设备身份，**立即持久化** |
| `restore_session()` | 启动时恢复 SESSID（异步，由 setup 钩子调度） |
| `browse(target, page)` | 首页 / 动态列表 |
| `search(keyword, page)` | 搜索 |
| `replies(id, page)` | 评论列表 |
| `detail(id)` | 详情；CAPTCHA/AUTH 失败且**显式允许**时才降级公开页 |
| `session_info()` | 当前身份与待完成挑战 |
| `submit_validation(...)` | **写入凭证并立刻重放原请求**，返回重放结果 |
| `retry_after_validation(pending)` | 带凭证重放一次；成功保留凭证，失败清除 |
| `set_public_fallback(allow)` | 开关公开页兜底，默认 `false` |
| `import_shuzlm_id(text)` | 手动导入数盟 ID |

#### `commands.rs` —— Tauri 命令层

**为什么要单独一个文件**：`#[tauri::command]` 会在模块级宏命名空间生成
`__cmd__{name}` 与 `__tauri_command_name_{name}`。当这些命令直接定义在
`lib.rs` 里、而 `lib.rs` 又同时被 `main.rs` 引用时，宏展开会产生
`E0255: defined multiple times`。

> 曾误判为 `crate-type` 多类型导致，**实测证伪**——恢复
> `staticlib/cdylib/rlib` 三种类型后编译照样通过。真正的变量是命令定义位置。

8 个命令，全部 `async`，统一返回 `Result<InvokeResult<T>, String>`：

```
browse / detail / replies / search
session_info / submit_validation / import_shuzlm_id / set_public_fallback
```

`AppState` 使用 `tokio::sync::Mutex` —— **这是刻意的**：
命令处理里需要在持锁期间 `.await` 网络请求，`std::sync::MutexGuard` 不满足 `Send`。

---

## 4. 验证码链路（当前实现）

```
① 任意请求 → 服务端返回 status 1004 / err_request_captcha_v2
② classify() 映射为 Error::CaptchaNeeded { challenge }
③ track_challenge() 记录 PendingCaptcha
   { id, field, original_url }        ← 必须存完整 URL 才能重放
④ 前端 ErrorBox 检测 needsCaptcha → 渲染 CaptchaBox
⑤ 用户完成验证，粘贴 validate
⑥ submit_validation(挑战ID, 字段, validate)
   ├─ 校验 challenge_id 是否匹配当前待办（防旧窗口污染）
   ├─ challenge.proof() 拼 NEC:{id前8位}:{validate} 并 URL 编码
   ├─ 写入 Session.validation
   └─ 立刻调用 retry_after_validation()
⑦ 重放（ReplayGuard 保证只一次）
   ├─ field 为空 → 当作 validate Cookie
   └─ field 非空 → 拼成查询参数
⑧ 成功 → 返回响应体，前端直接渲染（不再重复请求）
   失败 → 清除凭证
```

**关键约束**：

1. 每个验证流程**只重放一次**
2. 重放后的 1004 **不再**触发第二次交互
3. 传输错误不自动重放，但必须清掉本次凭证
4. `validation` 凭证只在内存，**不落盘**
5. 公开页降级是**显式**的，默认关闭

---

## 5. Seam 现状（诚实版）

| 声称的 seam | 实际状态 |
| --- | --- |
| `HttpTransport` trait | ❌ 不存在，`client.rs` 直接用 `reqwest::Client` |
| `SessionStore` trait | ❌ 不存在，`Session` 是具体结构体 + 文件持久化 |
| `DeviceProfileSource` trait | ❌ 不存在，`DeviceProfile` 直接构造 |
| `CaptchaFlow` trait | ❌ 不存在，当前用「粘贴 validate」代替 WebView 自动回传 |

**代价**：`CoolapkClient::new()` 需要 `tauri::AppHandle`，测试无法构造真实实例，
只能用 `test_client()` 手搓字段，因此**测不了完整的 1004→验证→重放链路**。

**下一步**：引入 `HttpTransport` seam（生产 `ReqwestTransport`，测试 `MockTransport`），
让上述链路可被集成测试覆盖。两个 adapter 才是真 seam。

---

## 6. 测试策略（现状）

| 层 | 方式 | 数量 |
| --- | --- | --- |
| `coolall-core` | `cargo test`，纯单元测试 | 47 项 |
| `client.rs` | `cargo test --lib`，状态逻辑 | 9 项 |
| 前端纯函数 | `vitest` | 29 项（含 16 项 sanitizer 对抗测试） |
| 验证码重放链路 | ❌ **无集成测试**（缺 MockTransport） | 0 |
| 端到端 | ❌ 无 | 0 |

**已知短板**：最有价值的「1004 → 验证 → 重放 → 成功」链路没有自动化覆盖。
单元测试只能证明各部件正确，不能证明它们被正确串起来——
上一轮就出现过「重放函数写好了但没有任何调用方」的假完成状态，正是这个短板导致的。

---

## 7. 已知功能缺口

按严重程度排列：

1. **验证码获取方式未经验证** —— 当前用「打开网页 → 用户粘贴 validate」的方式，
   未经真实服务端验证。若行不通，需改为内嵌 WebView 自动回传。
2. **无登录命令** —— `is_logged_in()` 恒为假，登录明确延后到阶段 2。
3. **全局锁串行** —— 单锁包住整个 client，20 秒超时会拖住所有其他命令。
   阶段 1 暂保留（它顺带消除了几处并发竞态），阶段 2 拆分。
4. **无请求取消** —— 仅靠前端请求序号丢弃过期结果，Rust 侧请求仍会跑完。

---

## 7. 演进路径

**阶段 1（当前）**：协议层打通、首页/搜索/详情/评论、验证码链路。
**阶段 2**：用户主页、话题、应用、数码、文章。
**阶段 3**：消息、私信、发帖、收藏、历史。
**阶段 4**：剩余页面与桌面端专属能力。

每个阶段结束都是**可编译、可运行**的完整状态。
