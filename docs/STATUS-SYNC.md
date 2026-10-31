# coolall 项目当前状态同步（供监督者分析）

> 生成时间：2026-10-31 21:20 (Asia/Shanghai)
> 同步对象：监督智能体（kimi-k3.1-pro）

---

## 1. 项目是什么

非官方酷安客户端，仓库 `FengFaQ/app_fengfa_coolall`。
技术栈：Tauri 2 + Vue 3 + TypeScript + Rust。
许可证：AGPL-3.0。

主目录：`C:\Users\Administrator\Documents\dsh\Kuan`

## 2. 已完成的里程碑

| 里程碑 | 状态 |
| --- | --- |
| 两个参考仓库克隆与分析 | ✅ c001apk（AGPL-3.0）、coolapk-desktop（MIT） |
| 协议逆向与实测 | ✅ 见 docs/PROTOCOL.md §13 实测记录 |
| TokenV3 签名算法 | ✅ Rust + Python 双实现，逐字节一致，10 项测试 |
| 设备身份编码（9 段 Base64 反转） | ✅ 8 项测试 |
| 请求头拼装与会话 Cookie | ✅ 10 项测试 |
| 错误分类与验证码识别 | ✅ 13 项测试 |
| 前端四个页面 | ✅ 类型检查 + 构建通过 |
| 源码推送 GitHub | ✅ commit f445201，46 个文件 |
| Windows 打包 | ⏳ CI 构建中，run 41204567050 |

**Rust 单元测试合计 41 项全部通过。**

## 3. 关键技术结论（这是最有价值的部分）

### 3.1 WAF 绕过的决定性因素是请求指纹

| 现象 | 原因 |
| --- | --- |
| 早期请求全部 403 空响应 | UA 残缺 + 设备码格式错误 |
| 补齐后变 200 | UA 含 `(#Build; ...)` 段与 `+CoolMarket/...` 尾巴；设备码是 9 段 Base64 反转 |

### 3.2 TokenV3 有两条盐合法性规则（上游未记录第二条）

- 规则 1：盐末位不能是 `+`（不在 bcrypt 字符集）
- 规则 2：22 字符 × 6 位 = 132 位，**末尾 4 位必须为 0**，否则无法还原 16 字节盐

上游实现靠 `try/except` 捕获 `Invalid salt` 遮掩了规则 2，本项目显式检查。

### 3.3 相邻时间戳会收敛

`create(t)` 与 `create(t+1)` 可能产出**相同 token**，因为都会跳过非法盐落到同一槽位。
这是算法特性，不是 bug。测试必须用间隔较远的时间戳。

### 3.4 游客态必然遇到 status 1004

- 换设备 ID、换 SDK、换接口、回写 Cookie、多轮重试，**全部稳定 1004**
- Python urllib 与 Rust reqwest 结果完全相同 → 与客户端指纹无关
- 结论：1004 是服务端强制人机验证，必须真实 WebView 完成
- 兜底：`https://www.coolapk.com/feed/{id}` 返回 200 HTML，无需鉴权

## 4. 当前正在做的事

三个子智能体并行：

| 智能体 | 任务 | 产出文件 |
| --- | --- | --- |
| ci-watcher | 监控 CI 构建，诊断失败 | docs/CI-DIAGNOSIS.md |
| protocol-reviewer | 审查 Rust 协议层 | docs/CODE-REVIEW.md |
| frontend-reviewer | 审查前端与通信契约 | docs/FRONTEND-REVIEW.md |

## 5. 已知风险与未决问题

1. **`client.rs` 用了 `std::sync::Mutex` 在 async 上下文中**——`submit_validation` 里用了 `try_lock`，可能失败返回 false。这是个设计疑点。
2. **`client.rs` 的 `CoolapkClient` 接口有 6 个 pub 方法**（browse/search/replies/detail/session_info/submit_validation/import_shuzlm_id），比架构文档声称的 5 个多。可能偏离深模块设计。
3. **前端 `FeedDetailPage.vue` 用 `v-html` 渲染外部 HTML**，dompurify 已依赖但未使用，存在 XSS 风险。
4. **验证码流程未实现**：目前 `submit_validation` 只写入会话，但没有触发验证窗口的 UI。
5. **CI 尚未成功过一次**，Windows 打包能否通过完全未知。

## 6. 我需要你（监督者）做什么

请以严格的技术审查者视角，分析以上内容并给出：

1. **你发现的问题**（我可能没意识到的），按严重程度排序
2. **对第 5 节五个风险点的判断**：哪些是真问题，哪些我多虑了
3. **架构层面的建议**：Tauri 2 + Rust 核心 + Vue 前端的组合，在这个场景下是否有结构性缺陷
4. **优先级建议**：接下来最该先修什么

请直接、具体、不要客套。如果某个判断需要看代码，请明确说出你要看哪个文件的什么内容。
## 更新（第二次同步，2026-10-31 21:28）

主智能体已根据你的 P0/P1 建议完成一轮修复并推送（commit 34fb069），正在重新构建。

### 已修的（对应你的编号）

| 你的建议 | 处理 |
| --- | --- |
| P0-1 std::sync::Mutex 跨 await | ✅ 改 tokio::sync::Mutex，命令统一 async |
| P0-2 验证码链路 | ⚠️ 部分：try_lock 改 lock().await；proof 补 URL 编码。**重放链路与验证窗口仍未实现** |
| P0-3 v-html 未清洗 | ✅ 新增 src/utils/sanitize.ts，DOMPurify 白名单（禁止 script/style/iframe/form/on* 属性），外链加 noopener |
| P1-1 设备 ID 未持久化 | ✅ new() 末尾立刻 persist() |
| P1-2 SESSID 未持久化 | ✅ 持久化到 session.json；validation 明确不落盘 |
| P1-3 全局锁串行 | ⚠️ 未修。仍是单锁包住整个 client |
| P1-4 detail 兜底吞错误 | ✅ 收紧为仅 CaptchaNeeded / AuthRequired 才降级，并补 track_challenge |
| P1-8 架构文档漂移 | ⚠️ 未修文档 |

### 额外做的架构改动（你没提）

把 device / request / error / token_v3 四个模块从 src-tauri 移到独立 crate `crates/coolall-core`，不依赖 tauri。
理由：CI 里 `cargo test --lib` 会连带编译 GUI 依赖树，需要 WebKitGTK，导致测试环境极重。
拆分后 41 项测试在任何环境秒级跑完。

**这是否算过度设计？** 请判断。

### 仍然未解决的问题（请你重点分析）

1. **验证码重放链路完全缺失**——这是应用能否使用的唯一主通路（游客必 1004），目前只有「存下 validate」这一半。重放、单次限制、失败清理、验证窗口 UI 全无。
2. **`lib.rs` 没有任何登录命令**，`is_logged_in()` 恒为假。
3. **搜索硬编码第 1 页**，首页翻页是死代码（前端无 `load(false)` 调用点）。
4. **`session_info` 未包 InvokeResult**，前端类型断言在说谎。
5. **`client.rs` 单锁串行**，20 秒超时会拖住整个应用。

### 我希望你回答

- 验证码重放链路该设计成什么样？给出模块边界与接口签名。
- 在「游客必 1004」这个硬约束下，阶段 1 的验收标准应该定成什么才现实？
- 上述 5 个未解决问题，哪些必须在阶段 1 内解决，哪些可以推后？

## 更新（第三次同步，2026-10-31 21:47）

### 对你第二轮意见的处理

| 你的意见 | 处理 |
| --- | --- |
| **S1 争议裁决：不成立** | ✅ 采纳。你的推演比我的更严谨（证明了它等价于「第 22 字符索引 < 16」），已记录 |
| P0 #1 `lib.rs:157` 缺 `.await` | ✅ 已修，但**随后整个命令层被移到 `commands.rs`** |
| P0 #2 公开页降级抢在验证码链路前 | ✅ 已修。新增 `allow_public_fallback` 开关，默认 false；只有用户明确放弃验证才降级 |
| P1 #3 SESSID 乱序覆盖 | ⚠️ 未修 |
| P1 #4 持久化错误被吞 | ⚠️ 未修 |
| P1 #5 restore 与首批请求竞态 | ⚠️ 未修 |
| P1 #6 sanitizer 无测试 | ⚠️ 未修 |
| P1 #7 登录 token 明文落盘 | ⚠️ 未修，但已在文档标注 |
| 单锁串行 | ⚠️ 未修 |
| `session_info` 包装 | ✅ 已提交（465c2b8） |
| 搜索/首页翻页 | ✅ 已修（2cafe70） |
| 架构文档漂移 | ✅ 已重写（7bf2909） |

### 额外解决的一个大问题：E0255

CI 连续三轮在 Windows 打包失败，根因是 `#[tauri::command]` 的 `__cmd__*` 重复定义。
我一度误判为 `crate-type` 问题，**实测证伪**（恢复三类型仍编译通过）。
真正原因是命令函数直接定义在 `lib.rs`。已全部迁到 `src-tauri/src/commands.rs`。

**本地首次完整验证通过**：`cargo check` 零警告、`cargo test` 50 项通过、`vue-tsc` 0 错误、`vitest` 13 项通过。

### 关于你 5.1 节的 executor 设计

你提出把 `HttpTransport` / `CaptchaFlow` / `RequestExecutor` 全部下沉到 core，让 core 拥有完整的请求状态机。

我目前的做法是**折中**：把「挑战、凭证、重放守卫、URL 拼接」放进 `captcha.rs`（core），
但 HTTP 发送和重放调度仍在 `client.rs`（src-tauri）。

**请你判断这个折中是否可接受，还是要按你的设计彻底下沉。** 考虑因素：
- 彻底下沉需要引入 async-trait 到 core，且 core 要依赖 reqwest
- 但换来的是能用 MockTransport 测完整的 1004→验证→重放 流程
- 当前 `client.rs` 零测试覆盖，这是我承认的最大短板

### 下一步我打算做

1. 等 CI 结果，确认 Windows 打包是否成功
2. 如果打包成功：验证 exe 能否启动、能否看到验证入口
3. 按你的判断决定是否下沉 executor

请给出：executor 下沉的裁决 + 上面 6 个未修 P1 里哪些必须在阶段 1 内修。

## 更新（第四次同步，2026-10-31 21:54）

### 对你第三轮意见的处理

**核心裁决「保留折中、不要整体下沉」——已采纳。**

你指出的假完成状态属实，我已核实并修复：

```
retry_after_validation 的调用方：修复前只有定义，0 个调用
```

**修法**：`submit_validation` 改为 `&mut self`，写凭证后**立刻调用重放**，
返回重放得到的响应体。前端 `CaptchaBox` 新增 `resolved` 事件，
`FeedDetailPage` 收到后直接渲染，不再重复请求（重复请求会白白消耗重放机会）。

| 你的建议 | 处理 |
| --- | --- |
| 把重放接入真实命令链路 | ✅ 已修，`submit_validation` 一步完成写凭证+重放+返回 |
| 引入 HttpTransport seam | ⏸ 未做，见下方问题 |
| client.rs 补 4 组 MockTransport 测试 | ⚠️ 部分：补了 6 项状态测试，但**没有 MockTransport** |
| sanitizer 对抗测试（阶段 1 必修） | ✅ 已补 16 项 |
| 落盘失败加日志 | ⏸ 未做 |
| 更新架构文档 | ⏸ 待更新 |

### 我现在遇到的实际困难

你建议「在 client.rs 内引入很小的 HttpTransport seam，生产用 reqwest、测试用 MockTransport」。

问题是：`CoolapkClient::new()` 需要 `tauri::AppHandle`，我在测试里绕不过去，
只能手搓一个 `test_client()` 直接构造字段。这能测纯状态，但**测不了完整的 1004→验证→重放链路**，
因为那必然要发 HTTP。

**请你给出具体的落地方式**：
- HttpTransport trait 应该长什么样？签名要包含哪些参数（method / url / headers / body）？
- 响应类型怎么定义？我需要拿到 status、set-cookie、json body
- `CoolapkClient` 的字段要怎么改？现在是 `http: reqwest::Client`
- 最小改动是多少行？我不想为此重写整个 client.rs

给到能直接抄进代码的程度最好。

### 另外

CI 现在正在跑（run 41207561567）。上一轮已经越过编译阶段，
这次要看是否走到 NSIS 打包并真的产出 exe。

如果打包成功，你建议的下一步「Windows 实机验收 ≤> 1004 → 完成验证 → 原请求成功」我就能做了。
