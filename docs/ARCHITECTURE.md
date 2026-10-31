# 架构设计

> 本文档描述 `coolall` 的模块划分与依赖关系，使用「深模块」语言：
> **接口**（调用者需要知道的一切）、**实现**（里面怎么做的）、
> **seam**（接口所在位置）、**adapter**（满足接口的具体实现）。

---

## 1. 总体形状

```
┌─────────────────────────────────────────────────┐
│                 UI 层（Vue 3）                  │
│   Home / Search / FeedDetail / Settings         │
└────────────────────┬────────────────────────────┘
                     │ invoke('coolapk_request', ...)
┌────────────────────▼────────────────────────────┐
│           Rust 核心（src-tauri）                │
│                                                 │
│   ┌───────────────────────────────────────┐     │
│   │  接口：CoolapkClient                   │     │
│   │    browse(target, page) → Page        │     │
│   │    detail(id) → Content               │     │
│   │    replies(id, page) → Page           │     │
│   │    search(keyword, page) → Page       │     │
│   │    session() → SessionState           │     │
│   ├───────────────────────────────────────┤     │
│   │  实现（深）                            │     │
│   │   · TokenV3 签名                       │     │
│   │   · 设备身份编码                        │     │
│   │   · UA 构造                            │     │
│   │   · 请求头拼装                          │     │
│   │   · SESSID 会话管理                     │     │
│   │   · 验证码挑战识别与重放                 │     │
│   │   · 错误分类                            │     │
│   │   · 公开页兜底                          │     │
│   └───────────────────────────────────────┘     │
└─────────────────────────────────────────────────┘
                     │ HTTPS
              ┌──────▼──────┐
              │  酷安服务端  │
              └─────────────┘
```

---

## 2. 深模块划分

### 2.1 `CoolapkClient` —— 核心模块

**接口**（调用者必须知道的一切）：

| 方法 | 签名 | 说明 |
| --- | --- | --- |
| `browse` | `(target: BrowseTarget, page: Page) → Result<FeedPage>` | 首页/动态列表 |
| `detail` | `(id: &str) → Result<Content>` | 动态正文，失败自动降级公开页 |
| `replies` | `(id: &str, page: Page) → Result<ReplyPage>` | 评论列表 |
| `search` | `(keyword: &str, page: Page) → Result<FeedPage>` | 搜索 |
| `session` | `() → SessionState` | 当前登录/游客/验证状态 |

**调用者需要知道的不变量**：

1. 所有方法都是异步的，内部已处理重试与超时
2. 返回值已把 HTTP 错误、业务错误、验证码状态**统一映射**为 `Result`
3. 需要人机验证时返回 `Err(Error::CaptchaNeeded { challenge })`，调用者只需唤起验证窗口
4. 设备身份、Token、Cookie 全部由模块内部维护，**调用者不感知**

**为什么是深模块**：

删除它，上述 7 项实现（签名、编码、UA、头、会话、验证码、错误分类）
会在每个页面里重新长出来。接口只有 5 个方法，实现却有上千行复杂逻辑——
这正是 depth 的定义（leverage 在接口，locality 在实现）。

### 2.2 `DeviceIdentity` —— 内部模块

**接口**：

```rust
trait DeviceIdentity {
    fn device_header(&self) -> String;   // X-App-Device 的编码值
    fn user_agent(&self) -> String;      // 完整 UA
    fn sdk_int(&self) -> String;
    fn profile(&self) -> &DeviceProfile;
}
```

**实现**：从持久化配置读取 `szlmId` 与机型信息，按 §4.2 编码。
`szlmId` 首次随机生成，可被手动导入覆盖。

**内部 seam**：`DeviceProfileSource`，用于测试时注入固定机型。

### 2.3 `TokenV3` —— 叶子模块（纯函数）

```rust
pub fn create(device: &str, app_code: i64, timestamp: i64) -> Result<String, TokenError>;
```

无状态、无 IO、可直接单测。**这是最值得测试的模块**，因为：
- 输出格式严格（60 字节 bcrypt + 前缀）
- 重试路径有分支
- 出错时几乎无法从服务端响应反推原因

**验收**：与 `tools/verify_token_v3.py` 的输出逐字节一致。

### 2.4 `SessionStore` —— 状态模块

```rust
trait SessionStore {
    fn cookie_for(&self, host: &str) -> Option<String>;
    fn save_from(&mut self, headers: &HeaderMap);
    fn validation(&self) -> Option<&str>;
    fn accept_validation(&mut self, proof: &str);
    fn clear(&mut self);
}
```

**实现**：持久化 SESSID 与 validate，SESSID 变化时自动清除 validate。

**为什么需要 seam**：两个 adapter——
`FileSessionStore`（生产）与 `MemorySessionStore`（测试）。
**两个 adapter 意味着是真实的 seam**，不是假想中的。

### 2.5 `CaptchaFlow` —— 跨进程模块

**接口**：

```rust
async fn request(challenge: CaptchaChallenge) -> Result<String, CaptchaError>;
```

**实现**：打开 Tauri 的验证码窗口，等待前端回传 `validate`，超时 5 分钟。
凭证只在内存中流转，不落盘、不进日志。

**注意**：这是唯一能解决 `status: 1004` 的路径。见
[PROTOCOL.md](PROTOCOL.md) §9。

---

## 3. Seam 清单

| Seam | 位置 | Adapter |
| --- | --- | --- |
| HTTP 发送 | `HttpTransport` trait | `ReqwestTransport` / `MockTransport` |
| 会话存储 | `SessionStore` trait | `FileSessionStore` / `MemorySessionStore` |
| 设备画像 | `DeviceProfileSource` trait | `LocalProfile` / `FixedProfile`（测试） |
| 验证码交互 | `CaptchaFlow` trait | `TauriCaptchaFlow` / `AutoRejectCaptcha`（测试） |

**判断依据**：凡是存在「生产实现 + 测试替身」两个 adapter 的，才是真 seam。
不存在第二个 adapter 的地方不预留接口，避免浅模块。

---

## 4. 测试策略

**测试面 = 接口面。** 所有测试穿过接口，不测实现内部。

### 4.1 单元测试（无网络）

| 对象 | 测什么 |
| --- | --- |
| `TokenV3` | 确定性、结构合法、时间/设备敏感、重试路径 |
| `DeviceIdentity` | 编码可逆（解码回 9 段）、UA 格式、ID 校验 |
| `SessionStore` | SESSID 保存/失效、validate 与 SESSID 绑定 |
| 错误分类 | 各类响应体映射到正确错误类型 |

### 4.2 集成测试（Mock 传输）

用 `MockTransport` 喂入真实抓取的响应样本，验证：

- 成功响应正确解析
- `1004` 映射为 `CaptchaNeeded`
- `err_request_captcha_v2` 被正确识别为挑战
- 降级到公开页的时机正确

### 4.3 手工验收（真实环境）

自动化测试覆盖不到真实 TLS 指纹与验证码。最终验收：

1. 构建出 Windows 包
2. 启动，进入首页
3. 若弹出验证码，完成验证
4. 能正常浏览、搜索、看评论

---

## 5. 前端结构

```
src/
├─ api/            # 唯一的 invoke 封装层，前端不直接调 tauri.invoke
├─ stores/         # Pinia：会话、列表、设置
├─ pages/          # Home / Search / Detail / Settings
├─ components/     # 卡片、列表项、加载态、错误态
└─ styles/         # 主题变量
```

**约束**：`api/` 是前端唯一与 Rust 通信的地方。
其他模块只依赖 `api/` 暴露的函数，不直接 `invoke`。

---

## 6. 错误处理约定

Rust 侧统一错误类型：

```rust
pub enum Error {
    Network(String),
    Http { status: u16 },
    Business { status: i64, message: String },
    CaptchaNeeded { challenge: CaptchaChallenge },
    AuthRequired,
    RateLimited,
    ClientUpgrade,
    Parse(String),
}
```

序列化后前端拿到 `{ kind, message, retryable }`，据此决定：
重试按钮、重新登录、唤起验证、展示提示。

---

## 7. 数据流示例

一次首页加载：

```
UI 点击刷新
  → api.browse('home', page=1)
    → invoke('coolapk_browse', { target: 'home', page: 1 })
      → CoolapkClient::browse
        → DeviceIdentity.device_header()     # 取编码后的设备串
        → TokenV3::create(device, 2610291, now)
        → 拼装 15 个请求头
        → HttpTransport::send(api2, /v6/main/indexV8, headers)
          ├─ 200 + data      → 解析为 FeedPage → 返回
          ├─ 200 + 1004      → Err(CaptchaNeeded)
          └─ 403             → Err(Http{403})   # 协议版本/头不全会到这
  ← FeedPage
UI 渲染列表
```

---

## 8. 演进路径

**阶段 1（当前）**：登录、首页、搜索、详情、评论。
**阶段 2**：用户主页、话题、应用、数码、文章。
**阶段 3**：消息、私信、发帖、收藏、历史。
**阶段 4**：剩余页面、桌面端专属能力（多窗口、托盘、快捷键）。

每个阶段结束都是**可编译、可运行**的完整状态。
