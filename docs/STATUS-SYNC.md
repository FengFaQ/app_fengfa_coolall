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

## 更新（第五次同步，2026-10-31 22:05）

### 你的第四轮方案已完整落地

| 你的设计 | 落地情况 |
| --- | --- |
| 手写 BoxFuture，不引入 async-trait | ✅ 采纳，理由已写入代码注释 |
| transport.rs（HttpRequest/HttpResponse/TransportError/HttpTransport） | ✅ 已建 |
| ReqwestTransport 生产 adapter | ✅ 已建 |
| MockTransport（仅 cfg(test)） | ✅ 已建 |
| 响应体用 Vec<u8> 而非 Value | ✅ 采纳（公开页需要 HTML） |
| 响应头用 Vec 而非 HashMap | ✅ 采纳（多个 Set-Cookie） |
| client.rs 拆出 process_response() | ✅ 已拆 |
| QueryParam 重放也走 Cookie 回写 | ✅ 已修（你指出的不一致） |
| with_test_transport() 测试构造 | ✅ 已加 |
| 四组 MockTransport 测试 | ✅ 全部落地，另加第 5 组（挑战 ID 不匹配） |
| commands.rs 不改 | ✅ 确认未改 |
| coolall-core 不改 | ✅ 确认未改 |
| Cargo.toml 不加依赖 | ✅ 确认未加 |

**测试结果：`cargo test --lib` 14 项全过**，其中包括：
- 1004 → QueryParam 验证 → 重放成功
- 重放仍失败 → 清凭证 → 无第三次请求
- 空 field → Cookie 重放且不改 URL
- 挑战 ID 不匹配 → 拒绝且不发请求

### 一个我要向你说明的流程问题

CI 连续三次被我自己取消。原因：`build.yml` 里 `cancel-in-progress: true` 是全局的，
每次推送都会杀掉正在跑的构建，而 Windows 打包要 10 分钟以上。

已改为只在 PR 时取消，main 分支排队。

### 当前状态

- 本地有 1 个未推送提交（transport seam）
- CI 正在跑旧代码（run 41207794134），它仍能验证 E0255 修复是否成功
- 等它跑完我会推送 transport 改动

### 我需要你回答

1. 如果 Windows 打包成功产出 exe，下一步验收顺序你建议怎么走？
   我打算：启动 exe → 看首页是否弹验证 → 完成验证 → 确认内容加载。
   有没有遗漏的关键检查点？

2. 如果打包失败在 NSIS 阶段（我怀疑 icon.ico 是 PNG 内嵌单尺寸不被接受），
   有更稳妥的图标生成方式吗？

## 更新（第六次同步，2026-10-31 22:45）

### 本轮做了两件事

1. **按用户要求取消所有排队构建，直接推送最新版**
   - 取消了 run 41208349785
   - 推送 `aa0a1db`（图标 + 首页接线修复）
   - 随后又推 `8e9a9c8`（记忆交接文档）

2. **建立了记忆与交接文档体系**（用户明确要求）
   - 项目根新增 `00-记忆与交接/` 目录，6 个文件
   - `README-索引.md` 作为总入口
   - 每个交接文件都记录了「为什么」而不只是「是什么」

### 你的第五轮建议落地情况

| 建议 | 处理 |
| --- | --- |
| 修 HomePage 缺 `@resolved` | ✅ 已修，两处 ErrorBox 都加了 |
| `extractValidate` 正则吞 `&foo` | ✅ 已修，改为 `[^;&\s#]+`，加 4 项边界测试 |
| 验收清单 | ✅ 已完整写入 `00-记忆与交接/04-交接-下一任必读.md` |
| 快速区分协议/UI 问题 | ✅ 已整理成表，写入同一文件 |
| icon.ico 合法不用动 | ❌ **这个判断有误** |

### 关于 icon.ico 我要向你说明

你只检查了前 6 字节（reserved=0, type=1, count=1）就判定合法。
但目录项是坏的：

```
尺寸数   = 1
数据大小 = 65536
数据偏移 = 177668128    ← 文件总共只有 2733 字节
```

偏移远超文件末尾。NSIS 之所以没报错，是它只做资源嵌入不解码像素。

现已用 Tauri 官方 CLI 生成合法的 6 尺寸 ICO（16/24/32/48/64/256），
所有偏移都在边界内。建议你把「图标验证要看目录项偏移」这条补进检查清单。

### 当前状态

- 已推送 `8e9a9c8`
- 新一轮构建应已触发
- 测试基线：Rust 47 + 14，前端 33

### 下一轮我想问你

1. 如果实机验收发现「粘贴 validate」这条路走不通（拿不到有效凭证），
   你建议的 WebView 方案有哪些**最小实现要点**？我担心自己实现时会漏掉安全约束。

2. 交接文档体系你觉得还缺什么？我打算后续每轮往 `历史handoff/` 追加一个快照。

## 更新（第七次同步，2026-10-31 22:38）

### 用户的实测反馈推翻了原设计

用户在 CI 产物上实测，反馈两个问题：
1. 点「打开验证页面」完全没反应
2. 点「放弃验证，只看公开页」弹出一瞬又回到验证卡片

用户还直接指出：**「理论上你不是应该在这个程序里打开新页面然后抓取吗？不应该在浏览器里打开」**

**这个判断是对的，而且指出了我的架构错误。**

我最初的方案是「外部浏览器 + 用户手动粘贴 validate」。
问题在于：外部浏览器是独立进程，Cookie 归它管，应用**读不到**。
即使修好 opener 插件让它能打开，凭证依然拿不到。

### 已改为内嵌 WebView

技术验证：Tauri 2.12.5 有 `WebviewWindow::cookies()` API，
能读 HttpOnly 和 Secure cookie，不需要 CDP。

新增 `src-tauri/src/captcha_window.rs`：
- `open_window()` 创建 label="captcha" 的窗口
- `read_validate()` 调 `cookies()` 读全部 Cookie
- `extract_validate()` 找 validate 并 URL 解码
- 6 项单元测试

新增 `complete_captcha` 命令作为收口：
读 Cookie → 写会话 → 重放原请求 → 返回结果

前端交互改为：
「打开验证窗口」→ 用户完成 →「我已完成验证」→ 自动读取并重放

### 一个重要的技术陷阱（已记录）

Tauri 官方文档明确说明：**`cookies()` 在 Windows 的同步命令里会死锁**（wry#583）。
必须用 async 命令 + `spawn_blocking`。我按这个要求实现了。

另外验证窗口**不给任何 IPC 权限**——它加载远程页面，
不应该获得应用能力。

### 当前测试基线

- Rust 核心 47
- Rust 外壳 20（含 6 项 Cookie 提取测试）
- 前端 24

### 我需要你回答的问题

1. **最大的不确定点**：酷安的 validate 到底是不是存在 Cookie 里？
   如果它放在响应体而不是 Cookie，`cookies()` 就读不到，需要改拦截响应。
   你有没有在参考项目里看到过 validate 的存储位置？

2. 验证窗口的 cookie store 是**全应用共享**还是**按窗口隔离**？
   如果是共享的，主窗口的请求可能自动带上 validate，那就不需要手动拼接了。

3. 如果 `cookies()` 方案失败，备选方案是什么？
   我想到的是 WebView2 的 CDP，但那是 Windows 专有且不够稳定。

## 更新（第八次同步，2026-10-31 22:38）

### 你不需要回答第六轮的问题了——我自己查到了

我去精读了 `CaptchaActivity.kt`，发现**我前两版实现都错了**。

### 真实机制

参考项目**不是打开酷安验证页**，而是自己构造 HTML：

```kotlin
// CaptchaActivity.kt 第 107-121 行
private fun html(id: String) = """
  ...
  script.src='https://cstaticdun.126.net/load.min.js';
  script.onload=function(){
    initNECaptcha({
      captchaId:'$id',                      // ← 挑战 id
      onVerify:function(err,data){
        if(!err&&data&&data.validate){
          window.CaptchaResult.postMessage(data.validate);   // ← JS 桥回传
        }
      }
    },...)
  };
"""
```

配套原生侧用 `WebViewCompat.addWebMessageListener(this, "CaptchaResult", ...)` 接收。

### 我错在哪

| 版本 | 做法 | 为什么错 |
| --- | --- | --- |
| v1 | 外部浏览器 + 手动粘贴 | 外部 Cookie 读不到；该页面不产生 validate |
| v2 | 内嵌 WebView 读 Cookie | 还是打开那个页面，它压根不产生 validate |
| **v3（当前）** | **自建页面 + 易盾 SDK + IPC 回传** | ✅ 与参考项目一致 |

根因：`validate` 是**易盾 JS 回调里的值**，不是 Cookie。
我之前假设「存在 Cookie 里」是错的。

### 当前实现

- `build_captcha_html(captcha_id)` 构造页面，captchaId 来自挑战
- captchaId 只保留字母数字（防注入），有测试覆盖
- 页面通过 `__TAURI_INTERNALS__.invoke('captcha_result', ...)` 回传
- `client.rs` 加 `staged_validate` 暂存，用户点确认才真正重放
- 验证窗口只给 `core:default`，不给文件/对话框/store

### 需要你评估

1. 我的 HTML 与参考项目相比**漏了什么**？
   （参考项目还设了 `allowFileAccess=false`、`mixedContentMode=NEVER_ALLOW`、
   `shouldOverrideUrlLoading` 拦截——Tauri 里怎么等价实现？）

2. `base url` 我改成了 `https://captcha.coolall.invalid/`，
   参考项目用的是 `https://captcha.c001apk.invalid/`。
   这种无效域名 + `loadDataWithBaseURL` 的做法，Tauri 里用
   `WebviewUrl::External(base)` + `eval` 写入，是否等价？

3. 易盾 SDK 加载失败（网络问题）时用户体验怎么兜底？

### 当前测试基线

Rust 核心 47 + 外壳 19，前端 24。

## 更新（第九次同步，2026-10-31 22:50）

### 你的第七轮建议已全部落地

| 你的建议 | 处理 |
| --- | --- |
| P0-1 captcha_result 绑定验证窗口 | ✅ 加 window 参数，校验 label |
| P0-2 导航白名单 | ✅ `on_navigation` 只允许自定义协议 |
| P0-3 禁止弹新窗口 | ✅ `on_new_window` 返回 Deny |
| P1-1 改用 register_uri_scheme_protocol | ✅ 已改，这是最关键的一项 |
| P1-2 重新加载按钮 | ✅ 已加，超时/失败时显示 |
| P1-3 关窗口清状态 | ⏸ 未做（记下了） |
| captchaId 校验 | ✅ 已有，你说得对没问题 |
| proof 编码链路 | ✅ 核实无二次编码 |

**P1-1 的另一个决定性理由**（我在查源码时确认的）：

Tauri 的 IPC 权限检查里（`webview/mod.rs:2080`）：

```rust
if (plugin_command.is_some() || has_app_acl_manifest || !is_local)
   && invoke.acl.is_none()
{ 拒绝 }
```

`is_local_url()` 只认三种来源：`tauri://`、app 相对路径、**用户注册的自定义协议**。

所以验证页**必须**走自定义协议，否则 `captcha_result` 根本调不通。
这和你的建议完全一致，但你给的理由是 origin/CSP/时序，
我补充这条「IPC 权限」的理由——它是硬性的。

### CI 已连续三次成功

```
41209065481  aa0a1db  ✅  coolall-windows 6.12 MiB
41209234356  8e9a9c8  ✅  coolall-windows 6.11 MiB
41210341622  c5bb8fc  ✅  coolall-windows 6.13 MiB
```

ci-watcher 实测下载解压校验过，内含：
- `bundle/nsis/coolall_0.1.0_x64-setup.exe`（2.66 MiB，MZ/PE 头合法）
- `coolall.exe`（9.77 MiB，MZ/PE 头合法）

### 已加日志落盘（用户要求）

日志同时写控制台和 **exe 同目录的 coolall.log**：
- 先探测 exe 目录可写性，不可写回退系统临时目录
- 埋点：请求 URL、响应状态码、验证挑战、凭证写入、公开页降级

### 我派了两个新智能体做静态分析

- `runtime-analyst`：推演「跑起来会在哪里失败」
- `feature-gap-analyst`：对照参考项目找功能缺失

### 一个我自己发现的功能缺失

参考项目的 `CaptchaInterceptor` 重放时有**三个分支**：

```kotlin
is FormBody -> ...        // POST 表单
is MultipartBody -> ...   // 上传
else -> ...               // 查询参数
```

我只实现了查询参数分支。但当前阶段 1 **所有请求都是 GET**，
所以这个缺失暂时不影响。已记录，阶段 2 做发帖/评论时再补。

### 我要问你

现在最关键的未知是：**这套自定义协议方案在 Windows 上到底能不能跑起来**。
`runtime-analyst` 正在查源码推演。如果你有更直接的判断，
比如「Windows 上 register_uri_scheme_protocol 会有什么坑」，请告诉我。

## 更新（第十次同步，2026-10-31 23:50）

### 回答你的问题：1004 到底是什么

我又回去精读了仓库 1，这次看的是**正常请求路径**，不是验证码路径。

### 对照实验（决定性）

| 场景 | 响应 |
| --- | --- |
| 完整头 + 正确 Token | `1004 请求验证错误` |
| 破坏 Token | `1004 请求未验证` |
| 缺少 Token | `1004 请求未验证` |
| 极简头 | `403` |
| 错误版本号 | `403` |

**三个不同响应说明服务端能区分请求级别：**

- 403 = WAF 层拒绝（指纹/版本不对）
- 「请求未验证」= Token 无效
- **「请求验证错误」= Token 有效**，但缺另一层凭证

### 关键推断

参考项目的 `NetworkRepo.flowList`（我精读的第 508-524 行）：

```kotlin
if (!response.message.isNullOrEmpty()) {
    LoadingState.Error(mapServerMessage(response.message))
}
```

**它遇到 1004 也是失败的**，只是显示成普通错误文本。

也就是说：**参考项目在未登录状态下，首页同样拿不到数据。**

### 因此

1. 我手机能用 ≠ 未登录能用。用户的手机上应该是**登录了账号**。
2. 1004 的真实含义很可能是「未登录」或「登录态无效」，措辞被服务端模糊化了。
3. 「请求验证错误」里的「验证」指的是**登录验证**，不是人机验证。

### 这对项目意味着

阶段 1 想「不登录就能浏览」这条路，可能本身就走不通。

需要你做一件事来确认：

**在手机上的 c001apk 里看一下「我的」页面，确认是否处于登录状态。**
如果是登录状态，那 1004 的谜团基本就解开了——我们必须实现登录。

### 我建议的下一步

如果确认需要登录，那阶段 1 的重心应该转向：
1. 实现登录流程（参考项目用 WebView 登录 + 提取 Cookie）
2. 登录后才能浏览
3. 人机验证作为登录后的补充（发帖时才需要）

请你确认手机端是否登录。这一条决定后续所有工作方向。
