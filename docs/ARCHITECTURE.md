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
| `detail(id)` | 详情，CAPTCHA/AUTH 失败时降级公开页 |
| `session_info()` | 当前身份与待完成挑战 |
| `submit_validation(...)` | 写入验证凭证（`lock().await`，不丢弃） |
| `import_shuzlm_id(text)` | 手动导入数盟 ID |

#### `lib.rs`

7 个 `#[tauri::command]`，全部 `async`，统一返回
`Result<InvokeResult<T>, String>`（`session_info` 已统一包装）。

`AppState` 使用 `tokio::sync::Mutex` —— **这是刻意的**：
命令处理里需要在持锁期间 `.await` 网络请求，`std::sync::MutexGuard` 不满足 `Send`。

---

## 4. Seam 现状（诚实版）

上一版文档声称存在 4 个 seam，但**代码里一个都不存在**。这是文档先行的错误，现已更正。

| 声称的 seam | 实际状态 |
| --- | --- |
| `HttpTransport` trait | ❌ 不存在，`client.rs` 直接用 `reqwest::Client` |
| `SessionStore` trait | ❌ 不存在，`Session` 是具体结构体 + 文件持久化 |
| `DeviceProfileSource` trait | ❌ 不存在，`DeviceProfile` 直接构造 |
| `CaptchaFlow` trait | ❌ **不存在，而且这是当前最大的功能缺口** |

**判断依据**（codebase-design 技能）：一个 adapter 是假想 seam，两个 adapter 才是真 seam。
目前每个点只有一种实现，所以不预留接口是正确的——但代价是**无法用 Mock 测试 `client.rs`**。

验证码链路落地时，必然需要至少两个 adapter（真实 WebView / 测试替身），
届时 `CaptchaFlow` 才会成为真 seam。

---

## 5. 测试策略（现状）

| 层 | 方式 | 数量 |
| --- | --- | --- |
| 核心 crate | `cargo test`，纯单元测试 | 41 项 |
| 前端 | `vitest`，纯函数测试 | 8 项 |
| `client.rs` | ❌ **无覆盖** | 0 |
| 端到端 | ❌ 无 | 0 |

**已知短板**：41 项测试集中在纯函数，**最关键的 1004 主流程没有任何自动化覆盖**。
补 MockTransport 集成测试是下一步重点。

---

## 6. 已知功能缺口

按严重程度排列：

1. **验证码重放链路缺失** —— 游客态请求稳定返回 `status: 1004`，
   目前只实现了「存下 validate」，重放 / 单次限制 / 失败清理 / 验证窗口 UI 全无。
   这是应用能否真正使用的唯一主通路。
2. **无登录命令** —— `is_logged_in()` 恒为假。
3. **全局锁串行** —— 单锁包住整个 client，20 秒超时会拖住所有其他命令。
4. **无请求取消** —— 仅靠前端请求序号丢弃过期结果，Rust 侧请求仍会跑完。

---

## 7. 演进路径

**阶段 1（当前）**：协议层打通、首页/搜索/详情/评论、验证码链路。
**阶段 2**：用户主页、话题、应用、数码、文章。
**阶段 3**：消息、私信、发帖、收藏、历史。
**阶段 4**：剩余页面与桌面端专属能力。

每个阶段结束都是**可编译、可运行**的完整状态。
