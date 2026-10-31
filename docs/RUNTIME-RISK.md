# 运行时风险分析

> 视角：**这套东西一跑起来会死在哪**。不是代码规范，不是安全审计（另有文档），
> 而是逐步推演 WebView2 里实际会发生什么。
>
> 证据来源（全部读过源码，不是猜）：
> - `tauri-2.12.5` / `tauri-runtime-2.12.2` / `tauri-runtime-wry-2.12.3` / `tauri-utils-2.10.2`
> - `wry-0.57.1`
> - 本仓库 `src-tauri/gen/schemas/{acl-manifests,capabilities}.json`（`tauri-build` 的实际产物）
> - 参考项目 `_upstream/c001apk/.../CaptchaActivity.kt`、`CaptchaChallenge.kt`
>
> 路径统一简写：`tauri::` = `tauri-2.12.5/src/`，`wry::` = `wry-0.57.1/src/`。

---

## 结论

**约 40–50%：窗口能打开、CSP 没问题、IPC 权限也没问题，但「重新加载」后页面拿不到 `captchaId`，
而这恰好是「易盾 SDK 加载失败」时用户唯一天然会点的按钮。**

拆开说，五个必答问题的实测答案是：

| # | 问题 | 结论 | 风险 |
|---|------|------|------|
| 1 | Windows 上 URL 是什么形式 | `coolall-captcha://localhost/?x` → **`http://coolall-captcha.localhost/?x`**，路由正常、query 保留 | **低（已证实可用）** |
| 2 | CSP 头是否生效 | **生效**，`'unsafe-inline'` 放行内联脚本，易盾脚本不会被拦 | **低（已证实可用）** |
| 3 | `on_navigation` 会不会误伤子资源 | **不会**。子资源走 `WebResourceRequested`，不走 `NavigationStarting` | **低** |
| 4 | `core:default` 能否调自定义命令 | **能**。拦截条件里的 `has_app_acl_manifest == false`，该分支不触发 | **低（已证实可用）** |
| 5 | `tauri::WebviewWindow` 参数能否拿到调用者 | **能**，且 label 判定是可靠的 | **低** |

**真正的坑不在任务单列的五条里。** 三种高于「低」的风险都在别处：

| 等级 | 风险 | 一句话 |
|------|------|--------|
| **中高** | R1 | `location.reload()` 让子资源请求不再带 query，`captchaId` 丢失 → 重载后必然失败 |
| **中** | R2 | CSP 是「拦得住，但拦完不吱声」：被拦的请求没有提示，页面上看不出发生了什么 |
| **中** | R3 | 多窗口共享同一 `WebContext`，换 `captchaId` 时存在竞态（推断，**不确定**） |

外加一条「不是 bug 但会拖死排查」的：验证窗没有 devtools 入口，出问题时几乎无法取证。

---

## 逐条风险

### R1【中高】点「重新加载」后 captchaId 必然丢失

**现象**

用户第一次打开验证窗，易盾组件加载失败（断网、公司网络拦 `cstaticdun.126.net`、超时…），
页面显示「验证组件加载失败，请检查网络后重试」+「重新加载」按钮。
用户修好网络 → 点「重新加载」→ **页面提示「缺少验证标识，请关闭窗口后重试」**，
之后再点多少次都一样。表现为「重试永远无效，只能关窗重开」。

**原因**

HTML 里的重载按钮走的是原生 `location.reload()`（`captcha_window.rs:115`）：

```js
document.getElementById('reload').onclick = function () { location.reload(); };
```

而 `captchaId` **只存在于首页地址的 query 里**（`captcha_window.rs:209`）：

```text
http://coolall-captcha.localhost/?captchaId=<id>
 └─ 文档请求：handler 从 request.uri().query() 取 id → 正确返回带 id 的 HTML
 └─ 重载后的子资源请求：http://coolall-captcha.localhost/   ← 没有 ?captchaId
```

首页是「重新加载后文档地址」，但 WebView2 重载时新的**资源请求**不再携带 query；
handler 只用 query 判断内容（`lib.rs:129-136`），拿不到 `captchaId=` 就返回 `CAPTCHA_ID` 为空串的 HTML。
页面里 `if (!CAPTCHA_ID) { statusText('缺少验证标识…'); return; }`（`captcha_window.rs:129-132`）
于是直接短路。

注意 HTML 会**重新构造**：`build_captcha_html("")` 里 `template.replace("__ID__", "")`
得到一个语法完全正常、但没 id 的页面——所以用户看到的不是「白屏」，而是一句看起来很合理的
「缺少验证标识，请关闭窗口后重试」。**这句话会把排查方向直接带偏**
（会让人以为是 Rust 侧没传 id，其实是重载丢了 id）。

**证据**

- `lib.rs:129-136`：`captcha_id` 唯一来源是 `request.uri().query()`，无 query 时 `unwrap_or("")`
- `captcha_window.rs:129-132`：`CAPTCHA_ID` 空则直接 `return`，SDK 脚本**不会被插入**
- `captcha_window.rs:184`：`build_captcha_html("")` 不 panic，静默产出无 id 页面
- `wry::webview2/mod.rs:1187-1197`：handler 收到的 `request.uri()` 是
  `revert_uri_work_around(webview_request_uri)`，即**本次资源请求本身**的 URI，而不是文档 URL

> 置信度：**高**。这是 `location.reload()` + 靠 query 驱动的协议处理器的经典组合。
> 仍建议按下面的验证方法实测一次（我无法在本环境运行 GUI）。

**验证方法**（5 分钟内可做完，不需要走完验证码）

1. `npm run tauri:dev`，主窗口 F12 → Console 执行
   `window.__TAURI_INTERNALS__.invoke('open_captcha_window')`
   （需先有一次 1004 让 `pending_captcha` 非空；否则用下面「桩」的办法）。
2. 验证窗出现后，**不断网**，直接点「重新加载」。
3. 若页面变为「缺少验证标识」→ **R1 复现**。
4. 对照实验：在 handler 里临时 `eprintln!("URI={}", request.uri())`，
   观察首次进入与重载后的 URI 差异（这一步就是全部证据）。

**修复方案**（按推荐度排序）

1. **把 id 编码进路径，而不是 query。** 重载时路径一定保留：

   ```text
   coolall-captcha://localhost/c/<captcha_id>
   ```

   handler 里改从 `request.uri().path()` 的最后一段取 id，仍然过 `valid_captcha_id`。
   同时保留 query 解析作为兼容分支。
2. **让 HTML 与 `captchaId` 解耦。** 页面不再把 id 写死进 HTML，改为启动时自己从
   `location.pathname` / `location.search` 读；这样重载天然正确，也顺带解决「静态 HTML」的诉求。
3. **兜底**：handler 在拿不到 id 时返回 400 + 明确文案，而不是返回一个看起来正常的空 id 页面；
   至少让失败可诊断。

> 建议 1+2 一起做，改动很小，且不引入任何新的时序依赖。

---

### R2【中】CSP 是「静默拦截」：拦住了也不告诉你

**现象**

页面一直停在「正在加载验证组件…」，20 秒后变「加载超时，请重新加载」。
DevTools 里只看到 `Refused to …`，但**应用内没有任何可看的地方**。

**原因 / 风险点**

CSP（`lib.rs:144-153`）确实生效（见下方「已证实无问题」），但它和易盾 SDK 的真实行为是
**靠清单式放行对上的**，只要 SDK 有任意一种没被列进来的加载方式，就会**静默失败**。
当前清单里的薄弱处：

| 指令 | 现状 | 潜在冲突 |
|------|------|----------|
| `connect-src` | 只放行 `*.dun.163.com` / `*.163yun.com` / `cstaticdun.126.net` | 若 SDK 走 XHR 回 126.net 其它子域、或带埋点的第三方域名 → 被拦 |
| `img-src` | `data: https:` | 宽松，基本不会拦（反向风险：过宽） |
| `frame-src` | 仅 `*.dun.163.com` | 若易盾改用 iframe 承载在某 CDN 域名 → 被拦 |
| `form-action 'none'` | 全禁 | 若某条验证分支走表单提交 → 被拦 |
| `script-src` | `'unsafe-inline'` + `cstaticdun.126.net` | 不覆盖「SDK 再动态 pull 一个子脚本」的情况 |

**证据**

- `wry::webview2/mod.rs:1215-1222`：response header 被**原样**拼进
  `CreateWebResourceResponse` 的 headers 串——自定义协议返回的 CSP **确实**会进 WebView2 引擎。
- `tauri::manager/mod.rs:53-90`（`set_csp`）与 `tauri::protocol/tauri.rs:183`：
  Tauri 自己的 CSP 注入**只服务于 `tauri://` 协议与 isolation**，对用户自定义协议不做任何加工。
  → 即：这里这条 CSP 是**唯一**的 CSP，没有别的兜底，也没有任何「宽松模式」。
- 参考项目 `CaptchaActivity.kt` 用的是 `loadDataWithBaseURL("https://captcha.c001apk.invalid/", …)`，
  Android WebView **没有任何 CSP**。也就是说我们这条链路是**参考项目从未走过的路径**，
  放行清单没有经过任何实战校验。

**验证方法**

1. 开发模式打开验证窗，右键 → 检查（或 `Ctrl+Shift+I`，见 R6）→ 看 Console 的 `Refused to` 报错。
2. 断网后再打开，确认超时路径的文案正常。
3. 用一个临时探针把「CSP 问题」和「外网问题」分离：把 `NECAPTCHA_SDK_URL` 换成本机可控的地址，
   看「脚本能加载 + 内联脚本能跑」基线是否成立。

**修复方案**

- **首选**：保留 CSP，但把 `connect-src` 放宽到 `https:`（仅这一条），因为该页面是
  我们自己构造、内容完全可控、且**没有任何本地能力**（见 R5 的权限面），
  `https:` 的收益远大于风险。若坚持收紧，至少补上易盾常见域名。
- 在页面里挂
  `window.addEventListener('securitypolicyviolation', e => statusText('被 CSP 拦截：' + e.violatedDirective + ' ' + e.blockedURI))`，
  让拦截**可见**。这一行对排查的价值极高，成本几乎为零。
- 二十秒超时文案里带出 `navigator.onLine`，区分「没网」和「被拦」。

---

### R3【中｜不确定】换 captchaId 重开时，旧页面可能读到新 id

**现象**

两次验证连续触发（同一会话里先后遇到两个挑战，或用户「放弃」后又「重试」），
第二次打开的页面里 `captcha_id` 可能是**上一个**的值，导致易盾拿着旧 id 初始化失败。

**原因**

`open_window` 是「先 `close` 再 `new`」（`captcha_window.rs:203-205`），而
`wry` 的 WebView2 后端把所有 webview 挂在**同一个 `WebContext`** 上：

```rust
// tauri-runtime-wry#lib.rs:5115-5142
for (scheme, protocol) in uri_scheme_protocols { … with_asynchronous_custom_protocol(…) }
```

`uri_scheme_protocols` 是按 webview 注册的，但 handler 闭包共享同一个 `AppManager`；
而运行时的 `registered_custom_protocols` 去重**只在 Linux 系生效**
（`lib.rs:5117-5131` 的 `#[cfg(any(target_os = "linux", …))]`），Windows 上每个 webview 各自注册。

理论上 `close()` 是同步的、旧 webview 已销毁，不该串。但：

- 关闭是否**同步完成**取决于运行时的消息队列处理时机；
- 若 `close()` 因为窗口还没建完而失败（`let _ = old.close();` **吞掉了错误**，`captcha_window.rs:204`），
  旧窗口会**继续存在**，随后 `WebviewWindowBuilder::new` 因 label 重复直接报错
  ——用户看到的是「创建验证窗口失败」，而不是「重开成功」。

**证据**

- `captcha_window.rs:203-205`：`let _ = old.close();` —— 错误被丢弃
- `tauri::manager/webview.rs:463-466`：`if self.webviews_lock().contains_key(&pending.label)
  { return Err(WebviewLabelAlreadyExists) }` → label 冲突是一条**会真的返回**的错误路径
- `tauri-runtime-wry#lib.rs:5117-5131`：跨 webview 的协议去重仅 Linux

> 置信度：**不确定**。时序竞态，无法静态证明。但它**几乎零成本就能消除**，建议直接做。

**验证方法**

1. 先开验证窗 → 不关闭，再调一次 `open_captcha_window`（Console 连点两次）。
   观察：是报「创建验证窗口失败」，还是正常重开。
2. 若报错 → 说明 `close()` 在「窗口尚未完全建立」时不可靠，R3 的一半已复现。

**修复方案**

- 不用「关旧再建新」，改成 **复用同一个窗口 + 重新导航**：
  拿到已存在的 `WebviewWindow` 就 `win.navigate(url)`；不存在才 `build()`。
  既消除 label 冲突，也消除共享 `WebContext` 的时序问题。
- 无论如何，把 `let _ = old.close();` 的返回值记到日志里
  （项目里已有 `tauri_plugin_log`，一行 `log::warn!` 即可）。

---

### R4【低中】失败路径上的文案会把排查带偏，且几乎无取证手段

**现象（多个小问题叠加，单个都不致命，合起来让「跑不通」变成「查不动」）**

| 场景 | 用户看到的 | 实际情况 | 问题 |
|------|-----------|---------|------|
| 没有待完成挑战就点「打开验证窗口」 | `当前没有待完成的验证`（`commands.rs:131`） | 正确 | OK |
| 挑战 id 不合规 | `验证标识格式不正确（期望 16-64 位…）`（`captcha_window.rs:196-200`） | 易盾 id 长度/字符不符预期 | OK，但可附实际长度 |
| validate 格式非法 | `验证结果格式不正确：{message}`（`commands.rs:169`） | **`message` 恒为 `'ok'`**（`captcha_window.rs:121`） | 报错里没有任何有用信息 |
| 页面还没回传就点确认 | `还没有收到验证结果…`（`commands.rs:160-165`） | 正确 | OK（注意：**已由 `Err` 改为 `Ok(InvokeResult::failure)`**，前端拿到的错误文案一致，行为不变） |
| `captcha_result` 被拒 | JS 里 `catch` 后 `statusText('回传失败：'+e)` | `invoke` 返回的是 **Promise**，同步 `try/catch` **抓不到** | 见下 |

**关键点：`report()` 的 `try/catch` 是无效的**

```js
try {
  window.__TAURI_INTERNALS__.invoke('captcha_result', {...});   // 返回 Promise
  statusText('验证通过，请回到主窗口点「我已完成验证」');
} catch (e) { /* 永远不会进 */ }
```

`invoke` 是异步的，`reject` 落在 Promise 上，**同步 try/catch 抓不到**。
后果：即使 `captcha_result` 被 ACL 拒绝、参数名不匹配、或 Rust 侧返回 `Err`，
页面也会**平静地显示「验证通过」**，而主窗口点「我已完成验证」时提示「还没有收到验证结果」。
→ 用户陷入「两边说法互相矛盾」的状态，且**没有任何日志**。

**证据**

- `captcha_window.rs:117-127`：`invoke` 未 `await`、未 `.catch()`，`try/catch` 包了个 Promise
- `commands.rs:165-170`：`captcha_result` 的两处 `Err` 都不会在页面显示
- 项目已接入 `tauri_plugin_log`（`lib.rs:100-113`），但 `captcha_result` 的
  `stage_validate` 路径**没有任何 `log::info!`**，出问题后 `coolall.log` 里没有痕迹

> 行号时效性提醒：审查期间 `commands.rs` 在 22:49 被其他人改过
> （`complete_captcha` 的两个早退分支从 `Err(String)` 改为 `Ok(InvokeResult::failure)`）。
> 这不改变本节的结论——IPC 层的失败仍然是 Promise rejection，`try/catch` 依然抓不到；
> 但 **Rust 侧返回 `Err` 现在也会以 `ok:false` 的形式「成功」返回**，页面更不可能察觉回传失败。
> `captcha_window.rs` / `lib.rs` / `capabilities/` 未被改动，其余行号仍然有效。

**验证方法**

1. 故意把 `captcha_result` 的参数名改错（如 `validate` → `v`）跑一次：
   若页面仍显示「验证通过」→ 证明 `try/catch` 无效。（改完记得还原；只在本地临时改。）
2. 看 exe 同目录 `coolall.log` 是否有验证链路任何记录 —— 预期**没有**。

**修复方案**

- 改成 `await` / `.catch`：

  ```js
  window.__TAURI_INTERNALS__.invoke('captcha_result', { validate, message: 'ok' })
    .then(() => statusText('验证通过，请回到主窗口点「我已完成验证」'))
    .catch(e => statusText('回传失败：' + e));
  ```

- `captcha_result` 收到 validate 时 `log::info!("captcha_result 已暂存（len={}）", validate.len())`，
  失败时 `log::warn!`。**只记长度不记内容**，不违反「凭证不落盘」的既定约束。

---

### R5【低】权限面比文档声称的宽

**现象**：不导致崩溃，但和 `captcha_window.rs` 模块注释（第 40 行
「窗口只授予 `captcha_result` 一个命令的权限」）不一致。

**事实**

`captcha_window.rs:40` 写「只授予 `captcha_result` 一个命令」，而
`capabilities/captcha.json` 实际给的是 `["core:default"]`，展开后是
（`gen/schemas/acl-manifests.json` → `core.default_permission.permissions`）：

```text
core:path:default, core:event:default, core:window:default, core:webview:default,
core:app:default, core:image:default, core:resources:default, core:menu:default, core:tray:default
```

**证据**：`tauri::ipc/authority.rs:462-489`（`resolve_access` 只按 `allowed_commands` 过滤），
加上 `acl-manifests.json` 里 `core` 的 `default_permission` 清单。

**验证方法**：验证窗里 `invoke('plugin:event|emit', …)` 或任意 `core:event`
只读项，看是否成功。

**修复方案**：把 `captcha.json` 的 `permissions` 收窄到实际需要的只读集合
（至少去掉 `menu` / `tray` / `app`），或者干脆去掉 `core:default` 换成最小项；
同时把模块注释改成与 capability 一致的措辞。

> 注：这属于安全审查范畴，这里只标注「运行时确实生效」这一点，不与安全同事的结论冲突。

---

### R6【中｜影响排查效率，不影响功能】验证窗没有 devtools 入口

**现象**：出了 R1/R2/R4 中任何一个，你**看不到任何一方的证据**：
没有 Console、没有 Network、日志里也没有验证链路。

**证据**

- `tauri-runtime-wry#lib.rs:5148-5152`：`#[cfg(any(debug_assertions, feature = "devtools"))]`
  才 `with_devtools(…unwrap_or(true))` → **debug 构建默认开**，release **默认关**。
- `captcha_window.rs:211-231` 的 builder 没调用 `.devtools()`，也没设
  `additional_browser_args`，所以打包版**无法**打开 F12。
- `tauri.conf.json` 里没有 `"devtools"` 相关配置。

**验证方法**：`npm run tauri:build` 出来的安装版，验证窗按 F12 / 右键「检查」→ 没反应。

**修复方案（只改开发体验，不改行为）**

```rust
let mut builder = WebviewWindowBuilder::new(…);
#[cfg(debug_assertions)]
{
    builder = builder.devtools(true);
}
```

release 保持关闭。这样 `tauri dev` 下可以直接看 Console，R2 的 `Refused to` 一眼可见。

---

## 已证实「没问题」的点（附源码证据，避免同事重复踩）

这一节同样重要：任务单里列的五个怀疑点，**四个是可以排除的**。

### ✅ 问题 1：Windows 上的 URL 形式

`coolall-captcha://localhost/?captchaId=x` **能**正确路由，并且**确实被改写成** `http://` 形式。

```text
wry::webview2/mod.rs:495            let http_or_https = if pl_attrs.use_https { "https" } else { "http" };
wry::lib.rs:1771                    use_https: false,          // 默认值
tauri-runtime-2.12.2/webview.rs:536 use_https_scheme: false,   // 默认值
tauri-runtime-wry#lib.rs:4754       builder.with_https_scheme(webview_attributes.use_https_scheme)
wry::custom_protocol_workaround.rs:21-27
                                    coolall-captcha://…  →  http://coolall-captcha.
wry::webview2/mod.rs:594-600        attributes.url 命中 custom_protocols → apply_uri_work_around(...)
wry::webview2/mod.rs:1072-1074      请求侧用 is_work_around_uri(uri, http_or_https, protocol) 反向匹配
wry::webview2/mod.rs:1190-1197      revert_uri_work_around → request.uri(&path)
```

**结论**：用户可见地址是 `http://coolall-captcha.localhost/?captchaId=<id>`；
handler 里 `request.uri().query()` 能正常拿到 `captchaId=<id>`（`prepare_request` 保留完整 URI，
`wry::webview2/mod.rs:1197`）。

—— **`WebviewUrl::CustomProtocol` 本身也是对的**，因为在
`WebviewUrl::deserialize`（`tauri-utils-2.10.2/config.rs:88-109`）里非 http/https 的 URL 会自动归为
`CustomProtocol`，不会被误判成 `External`。

> 顺带提醒一个**日志/断言陷阱**：任何在 `on_navigation` 或页面里断言
> `url.scheme() == "coolall-captcha"` 的代码，在 Windows 上**永远为假**，
> 因为拿到的是改写后的 `http` URL。见下一条。

### ✅ 问题 3：`on_navigation` 不会误伤子资源

- `wry::webview2/mod.rs:754-774`：`navigation_handler` 挂在 `add_NavigationStarting` 上。
  WebView2 的 `NavigationStarting` **只对主框架导航触发**，子资源请求走
  `WebResourceRequested`（`wry::webview2/mod.rs:1050`）——两者是不同的事件通道。
  → 加载 `https://cstaticdun.126.net/load.min.js` **不会**进入 `on_navigation`。
- `captcha_window.rs:228` 的 `url.scheme() == CAPTCHA_PROTOCOL` 对**主框架导航**来说是
  「一个都不放行」（因为 Windows 上它是 `http`）。但窗口**初始 URL 不是通过导航回调决定的**
  （`wry::webview2/mod.rs:590-607` 直接 `webview.Navigate(&url)`，且 `NavigationStarting`
  是在 `build` 之后才注册的），所以首页仍能正常显示。
- 副作用有两处，都不影响验证主流程的**首次**加载：
  1. 页面内若执行 `location.reload()`，主框架导航会被**拒绝**（scheme 是 `http`）。
     —— 这其实是**另一个独立的隐患**，与 R1 叠加：就算把 id 问题修了，
     重载按钮在当前判定下**仍然不会真的重载**。修复时必须一并处理。
  2. 页面内若点了外链，`on_navigation` 拒绝 → WebView2 表现为「静默无反应」；
     且默认 `allow_link_preview` 在 Windows 上不生效
     （`tauri-runtime-wry#lib.rs:5046` 被 `#[cfg(any(macos, ios))]` 包住，Windows 不编译），
     → **不存在**「点了链接结果用系统浏览器打开」的意外跳转。这一点可以放心。

**修复建议（与 R1 合并）**：

```rust
.on_navigation(|url| {
    url.scheme() == CAPTCHA_PROTOCOL                         // macOS/Linux
        || url.domain() == Some("coolall-captcha.localhost") // Windows 改写后的形态
})
```

### ✅ 问题 2：CSP 确实生效，`'unsafe-inline'` 确实放行内联脚本

- `wry::webview2/mod.rs:1215-1229`：自定义协议 response 的 header 被原样写进
  `CreateWebResourceResponse` → **CSP 进入 WebView2**，由 Chromium 内核正常执行。
- `tauri::manager/mod.rs:429` 的 `set_csp` 只在 `tauri://`（`protocol/tauri.rs:183`）和
  isolation（`protocol/isolation.rs:61`）里被调用 → 自定义协议**不会被 Tauri 二次改写**，
  我们写什么就是什么。
- `script-src 'unsafe-inline' https://cstaticdun.126.net` 同时覆盖了
  「页面内联 `<script>`」与「`<script src=…>` 加载易盾」两种用法，**配置本身是自洽的**。

> 唯一的不确定来自易盾 SDK 运行时**还会**请求哪些域名（见 R2）。这与 CSP 是否生效无关。

### ✅ 问题 4：`core:default` 下能调自定义命令

拦截条件是（`tauri::webview/mod.rs:2080-2087`）：

```rust
if (plugin_command.is_some() || has_app_acl_manifest || !is_local)
   && request.cmd != FETCH_CHANNEL_DATA_COMMAND
   && invoke.acl.is_none()
{ reject; return; }
```

对 `captcha_result` 逐项核对：

| 条件 | 实际值 | 是否触发拦截 |
|------|--------|-------------|
| `plugin_command.is_some()` | 命令名无 `plugin:` 前缀 → `None` | 否 |
| `has_app_acl_manifest` | 本仓库**没有** `permissions/*.toml`，`gen/schemas/acl-manifests.json` 里**没有** `__app-acl__` 键 → `false` | 否 |
| `!is_local` | 见下 | 否 |
| `request.cmd != FETCH_CHANNEL_DATA_COMMAND` | 不是该命令 | 联合条件已不成立 |

**`is_local` 的推导链**（这是原设计成立的关键，已逐行核实）：

```rust
// tauri::webview/mod.rs:1961  fn is_local_url
// Windows 分支（:1988-1998）:
let scheme = scheme == self.manager().tauri_protocol_url(uses_https).scheme();
//              uses_https = false  →  "http"           // 当前页正是 http://  → true
let protocol = current_url
    .domain()                                        // "coolall-captcha.localhost"
    .and_then(|d| d.strip_suffix(".localhost"))      // → "coolall-captcha"
    .map(|protocol| protocols.contains_key(protocol)) // 我们的协议已注册 → true
    .unwrap_or_default();
scheme && protocol                                    // → true
```

`protocols` 就是 `register_uri_scheme_protocol(CAPTCHA_PROTOCOL, …)` 写入的映射（`tauri::app.rs:2279-2287`）。
→ **`is_local == true`**，`Origin::Local`，`resolve_access` 对 `core:default` 里的
`ExecutionContext::Local` 条目匹配成功（`tauri::ipc/authority.rs:57-67`）。

**其它支持性事实**：

- `invoke_key` 校验（`tauri::webview/mod.rs:2008-2026`）会通过，
  因为 `__TAURI_INTERNALS__` 的初始化脚本被注入到所有本地 webview，
  且该窗口与主窗口同属一个 `AppManager`。
- `tauri-runtime-wry#lib.rs:5115-5142` 中 `uri_scheme_protocols` 是**逐个 webview** 注册的，
  不存在「只有主窗口有协议」的问题。

**但仍然建议加一条兜底**：若将来引入 `permissions/` 目录（`has_app_acl_manifest` 会变成 `true`），
自定义命令必须在 capability 中显式声明，否则**会立刻被拦**。
当前 `captcha.json` 只有 `core:default` → 届时 `captcha_result` 会失效。

### ✅ 问题 5：`tauri::WebviewWindow` 能拿到调用者

- `tauri::webview/webview_window.rs:1606-1617`：`impl CommandArg for WebviewWindow<R>`
  从 `command.message.webview()` 取，**源码注释原话是 "This will never fail"**，
  唯一失败路径是「current webview is not a WebviewWindow」。
- 我们是 `WebviewWindowBuilder::new(…)` 创建的窗口级 webview（不是 `add_child` 出来的子 webview），
  `is_webview_window()` 判定见 `tauri::window/mod.rs:1257-1259`：
  `self.webviews().iter().all(|w| w.label() == self.label())` → **true**。
- `commands.rs:165` 的 `window.label() != CAPTCHA_WINDOW_LABEL` 判定因此是**可靠且确实生效**的，
  它同时也就是「只有 captcha 窗口能调」的安全边界。

---

## 最可能第一个失败的点

**不是 SDK 加载、不是 CSP、也不是 IPC——是「第一次打开就失败之后，用户点『重新加载』」这一下。**

理由（按发生概率 × 严重度排序）：

1. **第一次打开失败的概率不低**：易盾 `load.min.js` 走公网，
   在受限网络 / 代理 / VPN 下 20 秒超时（`captcha_window.rs:134-136`）是常见结果。
2. **失败后 UI 会主动引导用户点「重新加载」**（`captcha_window.rs:140-144` 的 `showReload()`）。
3. **这一下必然失败**：`location.reload()` 既丢了 query（R1），
   主框架导航又会因为 Windows 的 `http://` 改写被 `on_navigation` 拒绝（问题 3 的副作用）。
   → 结果是「缺少验证标识」或毫无反应，**并且此后再也无法恢复**。
4. 失败信息（「缺少验证标识」）指向**完全错误的方向**（Rust 侧没传 id），
   会浪费下一轮排查大量时间。

换句话说：**实现里唯一「失败后自救」的路径，恰好是唯一一条被证伪的路径。**
而且它发生在用户已经遇到问题之后——所以即使主流程能跑通，用户的一次网络抖动也会把整条链路彻底锁死。

如果网络本身通畅，第一失败点会后移到 **R3**（连续两次打开时的 label 冲突）
或 **易盾 SDK 运行时依赖的某个未放行域名（R2）**。

---

## 建议的验证顺序

原则：**先验证「不需要外网」的，再验证「需要外网」的**；
每步都要能**独立失败**，避免把多个原因混在一起。

| 步 | 验什么 | 怎么验 | 通过的标志 | 失败说明 |
|----|--------|--------|-----------|---------|
| **0** | 先拿到一个挑战 | 见下「桩」段落 | `open_captcha_window` 返回 Ok | 若拿不到，问题在 `track_challenge` / `pending_captcha`，不在本链路 |
| **1** | 窗口能开 + 协议能路由 | 开发模式下开验证窗，看是否出现「正在加载验证组件…」 | 出现该文案 | 说明 handler 没被调用或返回了空页面 |
| **2** | **重载是否保留 id**（R1） | 点「重新加载」 | 仍显示「正在加载…」而不是「缺少验证标识」 | **当前预计失败**；也顺带验证 `on_navigation` 是否拒绝重载 |
| **3** | 内联脚本能否执行（CSP 的 inline 面） | Console 看 `document.getElementById('status').textContent` | 内联脚本已执行 | 失败 → 查 `script-src` |
| **4** | 外网脚本能否加载（CSP 的 src 面） | Network 面板看 `load.min.js` 状态码 | 200 | 403/被拦 → 是**网络**问题；`Refused to` → 是 **CSP** 问题，两者要分清 |
| **5** | 易盾能否初始化 | 看是否变「请按下方提示完成人机验证」，并出现组件 | 组件出现 | 「初始化失败」→ 查 `connect-src` / 域名白名单（R2） |
| **6** | IPC 回传是否通 | 完成一次验证，看页面文案 | 变「验证通过，请回到主窗口…」 | 注意 R4：这句话**不可信**，必须以第 7 步为准 |
| **7** | 回传是否真的到达 Rust | 主窗口点「我已完成验证」 | 加载出内容 | 报「还没有收到验证结果」→ 回传没到达（先修 R4 才有日志） |
| **8** | 连续两次打开（R3） | Console 连点两次 `open_captcha_window` | 第二次也正常 | 报「创建验证窗口失败」→ R3 复现 |

**第 0 步的「桩」办法**（本机没有可用挑战时）：
在 `captcha_window.rs::open_window` 里临时把 `captcha_id` 写死为
`"414e5c9b866a03db03f860a1a9101672"`（易盾公开 demo id，参考项目
`CaptchaChallenge.kt:48` 也把它当 fallback），**仅本地临时改，不提交**。
这样第 1–6 步完全不依赖酷安服务器。

**收尾**：以上任一步复现后，**先修 R1（路径传 id + navigation 双形态放行）再谈其它**——
它同时挡住了「重载」和「on_navigation 误拒重载」两个问题，是投入产出比最高的一个改动。

---

## 附：三条「不确定」的诚实标注

1. **R1 的复现**依赖 WebView2 的 `location.reload()` 是否真的会丢掉文档 URL 的 query。
   结论基于「重载是一次新的文档请求、handler 只看本次请求的 URI」这两点（源码已证），
   但**未在本机实跑**（本环境无法启动 GUI）。建议按第 2 步实测确认。
2. **R3 的竞态**无法静态证明，只是从「`close()` 错误被吞 + label 冲突会硬报错 + 共享 WebContext」推得。
3. **易盾 SDK 实际需要的域名集合**未知（不抓包无法确定）；CSP 清单是按「合理猜测」写的，
   没有实战依据。第 4–5 步就是为了把这件事暴露出来。

除此之外的结论（问题 1/2/3/4/5 的判定、CSP 生效、IPC 放行、`WebviewWindow` 参数、
`use_https_scheme == false`）都有**逐行源码定位**，可以直接当作结论使用。
