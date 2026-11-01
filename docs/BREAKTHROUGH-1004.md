# 1004 突破口分析（第 8 方向）

> 任务：task-6「找 1004 的新突破口」
> 时间：2026-10-31 23:55 – 2026-11-01 00:10（Asia/Shanghai）
> 方法：**只读**精读两套参考实现 + 我方实现；`docs/1004-排查报告.md` 已排除的 7 个方向**全部不重复**。
> 约束：未改任何源码，未 push，只写本文件。
> 本文新增实测：`/v6` 路由 404 对照组、`Set-Cookie` 下发观查、参考项目真实端点组、TokenV3 盐槽位统计、数盟真机链路核对、方案 A 的 WebView 兜底可行性。

---

## 0. 结论先行

**第 8 个方向：1004 不是「请求构造错误」，而是「无 App 本地状态」被服务端识别。**

| 证据 | 含义 |
| --- | --- |
| 我方请求打到**根本不存在的** `/v6/__nope__` 也返回 1004（本次实测） | 1004 发生在**路由之前**的门禁层，与端点无关（顺便用实验补证已排除的「端点」方向） |
| 连 `checkLoginInfo` 都被拦（既有实测） | 1004 **不是**「需要登录」，是「客户端不被信任」 |
| `Set-Cookie: SESSID=…; domain=.coolapk.com` —— **没有 `Secure`**（本次实测） | 服务端**专门为 `http://` 场景下发会话**，说明它预期客户端会先在非 HTTPS 上下文里持有会话 |
| 探针发出的 `Cookie` 头被**静默忽略**：三次不同 Cookie 组合拿到的 `Set-Cookie` 完全一致（本次实测） | 服务端**不认我们自己拼的 Cookie**，它只认它自己种下的 |
| 两套参考实现都存在**「先在一个 WebView 里打开酷安页面」**的代码路径（见 §1） | 这条路径我们完全没有，是双方**结构性差异** |

因此突破口不是再改请求头/签名，而是补上**「用 WebView 建立并养护会话」**这一步。下面 §1 给出最有价值的方案 A/B/C 及其依据、验证方法与修法。

---

## 1. 发现清单

### 发现 1（★★★ 首推）：两套参考实现都会在 WebView 里访问酷安域，我方完全没有这条路径

**依据**

| 文件:行 | 原文 |
| --- | --- |
| `_upstream/c001apk/.../ui/component/WebView.kt:90-130`（上游 WebView 复用逻辑，节选） | `CookieManager.getInstance().let { cookies -> cookies.setAcceptCookie(true) … val domains = listOf("coolapk.com", "account.coolapk.com", "www.coolapk.com")` |
| 同上 `:121-127` | `cookies.setCookie("https://www.coolapk.com/", "DID=${prefs.szlmId}; Domain=.coolapk.com; Path=/; Secure")` 与 `cookies.setCookie(…, "displayVersion=v16; Domain=.coolapk.com; Path=/; Secure")` |
| `_upstream/c001apk/.../ui/component/WebView.kt:245` | `loadUrl(url, mapOf("X-Requested-With" to APP_ID))` —— 用 WebView 真正加载酷安页面 |
| `_upstream/c001apk/.../util/CoolapkLink.kt:13-20` | 一等公民域名集合：`coolapk.com / www.coolapk.com / m.coolapk.com / coolapk1s.com / www.coolapk1s.com` |
| `_upstream/c001apk/.../ui/main/AppNavigationActions.kt:26-74` | 每个分享链接都会 `navigateToWebView(url)` |
| `_upstream/c001apk/.../ui/settings/SettingsScreen.kt:509` | `if (prefs.szlmId.isBlank()) "尚未生成，点击获取"` —— 用户**没拿到数盟 ID 也能进主页看内容** |
| `_upstream/coolapk-desktop/src-tauri/src/coolapk/client.rs:1042-1055` | 桌面端同样有一套带酷安指纹头的 client，并用 `requirements` 命名空间单独建「公开页」client |
| `src-tauri/src/client.rs:37-48`（我方） | 只有 `BASE_URL`(`api.coolapk.com`) 与 `BASE_URL_2`(`api2.coolapk.com`)；`PUBLIC_URL` 仅用于 `/feed/{id}` HTML 兜底，**从未用 WebView 访问** |

**关键推论**：c001apk 能浏览的前提之一，是它的进程里有一个真实 WebView **真实加载过** `www.coolapk.com` / `coolapk1s.com` 页面。WebView 会执行页面 JS，页面 JS 会种 cookie、打点、拿服务端会话，之后同一进程的 OkHttp 请求才带着这份「有历史的会话」去调 API。

**附带旁证（本次实测）**：`Set-Cookie: SESSID=…; expires=…; Max-Age=8640000; path=/; domain=.coolapk.com` —— **没有 `Secure`**。这是给 `http(s)` 混合上下文准备的 cookie，指向的正是「浏览器语义环境」，而不是「纯 API 脚本」。同时三个不同 `Cookie` 请求头拿到的 `Set-Cookie` 完全一样（`a1-352883bd…`），说明服务端在**忽略我们送的 Cookie**、只按它自己的规则重新下发。

再旁证：上游 `WebView.kt` 亲自往 WebView 里种 `DID=<szlmId>` 与 `displayVersion=v16`——**这两个 cookie 存在于 web 域，却从未出现在任何 API 请求里**（`AddCookiesInterceptor.kt:50-60` 只放 `uid/username/token/SESSID/validate`）。说明 web 域与 api 域共享的是**服务端侧的设备画像**，而不是 cookie 本身。

**为什么可能有效**

1. 1004 在**路由前**发生（见 §2 实测），只能是门禁层判定。门禁层能看的东西不多：TLS 特征、IP、请求头、Cookie、**该设备在该时刻是否已有 web 会话**。
2. 我们已经把前四项都排除（7 个方向），第五项从未被测过。
3. 参考实现「碰巧」都能建立第五项：用户点任意分享链接、进 WebView、登录页都会产生真实页面访问。
4. 若这条成立，`X-App-Device` 只需**与 web 会话里的 DID 一致**（这正是 `ParamsScreen.kt:80-122` 在改机型时同时重算 `X-App-Device` 与随机 MAC 的原因——保持一致是硬要求）。

**验证方法（30 分钟内可完成，不动源码）**

1. 打开一个可用的浏览器内核窗口（Windows 上直接用 WebView2 / Edge / Chrome 均等价于「真 web 环境」），访问 `https://www.coolapk.com/`，**在页面内执行**：
   ```js
   document.cookie = "DID=76c27a71a4e73a9a37c8edc179a1ef84; Domain=.coolapk.com; Path=/";
   document.cookie = "displayVersion=v16; Domain=.coolapk.com; Path=/";
   // 停留 5 秒，让首页 JS 自己初始化
   ```
2. 在**同一个**浏览器里新开标签，直接访问 `https://api2.coolapk.com/v6/main/indexV8?page=1&firstLaunch=1&installTime=1`。
   - 浏览器会带上 web 域 cookie，但**不会**带 `X-App-*` 头——这一条只用于看「浏览器会话本身是否被认可」。
3. 更严格的版本（真正的判定实验）：用 Tauri / Node 起一个带 **WebView2 用户数据目录**的最小程序，在 WebView 里访问 `https://www.coolapk.com/` 并注入上述 cookie；随后**从同一个 user data dir 读出 cookie 列表**，把 `SESSID` 等值喂给我们的 `probe_breakthrough.py`（B 组已实现回带逻辑），看是否不再是 1004。
4. 最快的否决性对照：在手机上装一个 **WebView 开发者工具**（或让用户按 `chrome://inspect` 调试 c001apk 的 WebView），执行一次 `document.cookie`，确认 c001apk 进程里 web 域确实有 `DID` / `SESSID` / `displayVersion`。
   - 如果 c001apk WebView 里**没有**任何 cookie 而 API 依然 200，方案 A 被否决，转到方案 B。

**若成立怎么修（代码级，本次不实施）**

1. **新增原生初始化 seam**：`src-tauri/src/bootstrap.rs`
   ```rust
   /// 用 WebView 建立 web 域会话：访问首页 + 注入 DID/displayVersion，等页面稳定后读回 cookie。
   pub async fn warm_up<R: tauri::Runtime>(app: &tauri::AppHandle<R>, szlm: &str) -> Result<Vec<(String, String)>, String>
   ```
   - 不要复用 `captcha_window.rs` 的 `coolall-captcha://` 自定义协议（那是本地来源），这里必须用 `WebviewUrl::External("https://www.coolapk.com/".parse()?)`，以得到**远程来源 + 真 web cookie 罐**。
   - 窗口建议 `visible(false)`，`.on_navigation(|u| u.domain().map(|d| d.ends_with("coolapk.com")).unwrap_or(false))` 只放行酷安域，`on_new_window` 拒绝。
   - cookie 读取沿用 `docs/PROTOCOL.md §9.6` 已踩过的坑：**必须 async + `spawn_blocking`**（wry#583 在同步命令里死锁）。
2. **把 cookie 灌进 API 会话**：`crates/coolall-core/src/request.rs::Session` 增加 `web_cookies: Vec<(String,String)>`，`cookie_header()` 末尾按 `name=value` 拼上（顺序放在 `validate` 之前，因为 `CaptchaInterceptor.kt:66-73` 要求 `validate` 永远最后）。
3. **会话新鲜度**：`client.rs` 在 `Error::Business{status:1004}` 且 `web_cookies` 为空/超 24h 时，触发一次 `warm_up()` 再重试一次（仅一次，与 `ReplayGuard` 的「只重放一次」语义保持一致）。
4. **能力声明**：`src-tauri/capabilities/main.json` 已有 `core:webview:allow-create-webview-window`；新窗口若需要被主窗口控制，需要在 `main.json` 的 `windows` 里同时包含它，或新建 `bootstrap.json` 并用 `"windows": ["bootstrap"]`。

**风险与不可行边界（必须先说清）**

- WKWebView / WebView2 的 cookie 罐**不与 reqwest 共享**，所以「浏览器里 200」不等于「reqwest 200」；必须真的把 cookie 值取出来塞进 `Cookie` 头（这正是上面第 2 步）。
- 若门禁层真正判定的东西是 **TLS 指纹**（web 与 api 是不同 host，浏览器到 api 域依然是浏览器 TLS）
  → 那这条会失败，但**失败本身也是关键信息**：它把「会话历史」与「TLS 指纹」区分开，直接指向 §发现 6 的方案 C。

---

### 发现 2（★★☆）：**数盟真机链路不可复制 —— 设备画像的「四项物理身份」在桌面端不存在**

**依据**

| 文件:行 | 原文 |
| --- | --- |
| `_upstream/c001apk/.../logic/smid/DuClient.kt:79` | `val androidId = Settings.Secure.getString(ctx.contentResolver, Settings.Secure.ANDROID_ID)` |
| 同 `:93` | `widevineId()?.let { put("6yY", it) }` → `MediaDrm(PROPERTY_DEVICE_UNIQUE_ID)`（`:120-127`） |
| 同 `:95` | `put("dLH", DuCodec.enc(bootUuid(ctx), p))` —— 每安装一次的持久 UUID |
| 同 `:102` | `put("UOv", DuCodec.enc(bootTime(), p))` —— `now - elapsedRealtime` 推算的开机时刻 |
| 同 `:105-108` | `R37/R3d/R41/SAN` = `Build.MANUFACTURER/MODEL/VERSION.RELEASE/SDK_INT` |
| `crates/coolall-core/src/shuzilm.rs:211-231`（我方） | `fake_mac(sha1_upper(android_id))`；`dLH` 恒为 `enc("00000000-0000-4000-8000-000000000000")`；`UOv` 恒为 `enc("0.0")`；**没有 `6yY`(Widevine)**；`R37/R3d/R41/SAN` 硬编码 Xiaomi/2211133C/13/33 |
| `_upstream/c001apk/.../logic/smid/DigitalUnionRepository.kt:23-27` | 已有 `szlmId` 时**不再请求数盟**，直接复用；也就是说**数盟 ID 是每设备一次、长期不变的** |

**关键推论**

既然数盟 ID 是「每设备一次、长期不变」，那么它一定被酷安侧持久登记成一条设备画像，画像里必然包含上报时的 `R37/R3d/R41/SAN` 与 `P1J/AYk`（伪 MAC）。我们当前这套硬编码 `P1J/AYk` 是 `SHA1(随机 android_id)` 派的，`dLH`/`UOv` 是常量——能签发出 ID 不代表画像自洽。
更重要的是：**我们在桌面端永远拿不到 Widevine、真 AndroidID、真开机时刻**。所以「复刻真机链路」这条路最多只能拿到「格式合法」的 ID，而不是「可被酷安当真实设备」的 ID。

**为什么可能有效**：如果 1004 的判定输入包含「数盟 ID ↔ 请求头画像一致性」，那么我们的 ID 与画像必然不一致，任何请求头微调都不会有用——这正好解释了 7 个方向全部失败的表象。

**验证方法**

1. **离线自检（零风险，先做）**：拿用户手上那个真机 `DUP4awltXU__0ABOiBz7NwkxTrv4hbvIeRgd`，反查它当初上报的画像是否与我们现在发的头一致——
   ```
   X-App-Device 明文第 5-8 段 vs X-Sdk-Int / UA 里的 Android 版本与 model
   ```
   如果不一致（例如那台真机是 Android 14，而我们发 `X-Sdk-Int: 33`/`Android 13`），方案 B 直接被判为「画像不自洽」，必须先对齐。
2. **让用户在手机上导出完整参数**：c001apk 设置里有「参数」页（`ui/settings/ParamsScreen.kt`）。让用户**逐项**抄下 `szlmId / manufacturer / brand / model / buildNumber / sdkInt / androidVersion / versionName / versionCode / apiVersion`，一字不改地作为 `DeviceProfile`（`crates/coolall-core/src/device.rs:37-57`）——这是**零猜测**的画像对齐。
3. 拿对齐后的参数重跑 `probe_breakthrough.py`（A 组已经支持传 szlmId；需要小幅改成接受整份 profile）。
   - 若变成 200 → 突破口确认，且**改动极小**。
   - 若仍 1004 → 这条排除，且顺带解释了「为什么数盟 ID 那条实测没效果」。

**若成立怎么修**

1. `DeviceProfile` 增加持久化字段 `widevine: Option<String>`、`boot_uuid: String`、`boot_time: Option<String>`，与 `szlm_id` 一起落盘（`identity.json`）。
2. `shuzilm.rs::build_payload` 从「常量」改成「从 profile 读取」，缺失时**不要伪造**，而是把 `6yY`/`UOv` 直接省略（参考 `DuClient.kt:93` 的 `?.let`，缺失时本来就不写这个字段）。
3. **最简版本（几乎零代码）**：设置页加一个「导入真机参数」入口，接受用户粘贴的 9 段设备串明文 + UA，一次性写进 `identity.json`。这等价于把「随机 + 硬编码」换成「真机快照」，和上游 `OfficialDeviceIdentity.parse` 的定位一致。

---

### 发现 3（★★☆）：**WebView 加载后服务端会下发 `token=deleted` —— 说明「会话绑定」是通过 cookie 收敛的**

**依据**（本次实测，探针 A/B 组）

```
HTTP 200 {"status":1004,"error":null,"message":"请求验证错误"}
Set-Cookie: SESSID=a1-352883bda9e73faf75b733d514b3a50171adaf9f4b9b5e97730143c93dabdc72;
            expires=Mon, 08 Feb 2027 16:02:03 GMT; Max-Age=8640000; path=/; domain=.coolapk.com
Set-Cookie: token=deleted; Max-Age=0; path=/; domain=.coolapk.com; HttpOnly; SameSite=lax
Set-Cookie: token=deleted; Max-Age=0; path=/; domain=.coolapk.com
```

**关键推论**

1. `token=deleted; Max-Age=0` 是**主动清除**动作。服务端在说：「你带了一个 token 相关的会话，我把它作废了」。而 `AddCookiesInterceptor.kt:50-59` 在**登录态**才发 `uid/username/token`。我们游客态不发 token，服务端依然下发 `token=deleted` ——说明它**认的会话维度比我们以为的宽**（它认得不只是 `SESSID`）。
2. `SameSite=lax; HttpOnly` 这一份与不带属性的那一份**同时存在**，是「两份 cookie 域/属性并存」的典型形态：一个给 JS 读，一个给服务端读。这正是浏览器环境才会产生的结构。
3. `Max-Age=8640000`（100 天）——比 `ApiSessionStore.kt:110` 里 `minOf(cookie.expiresAt, now + 7 天)` 的本地截断长得多。**我们本地把服务端会话截短到 7 天，并把它当作 cookie 罐自管**，而服务端期望的是浏览器式长期罐。

**为什么可能有效**：如果门禁会「看有没有被作废的会话」，那么我们每次请求都从零会话开始，很可能每次都触发作废流程，永远拿不到「已认可」的状态。而这需要「有一个长期存在的罐」——又是发现 1 的同一结论。

**验证方法**

1. 连续 4 次请求（`probe1004.rs` 已经做过这件事，SESSID 稳定）后，**逐个比对 4 次响应的 `token=deleted` 是否都出现**。若每次都出现，说明会话从未被「认可」过。
2. **反向实验**：连续请求序列里，第 2 次起把第一次的全部 `Set-Cookie`（含 `token=deleted` 的语义）按浏览器规则落罐（`token` 因 `Max-Age=0` 被删、`SESSID` 保留），观察响应体是否变化。
3. 在浏览器控制台做同样 4 连击，看 `document.cookie` 与 Network 面板里 `token` 是否被反复删。

**若成立怎么修**：`crates/coolall-core/src/request.rs::update_from_set_cookie` 目前**只认 `SESSID`**（`:79-81` 的 `if name.trim() != "SESSID" { continue; }`）。应扩展成「除 `SESSID` 外，也维护 `token`（含删除语义）与 `forward`」，并把 7 天截断（`:110`，`client.rs` 对应位置）放宽到服务端给的 `Max-Age`，否则我们在本地把服务端的长期会话反复截断。

---

### 发现 4（★★☆）：**首屏根本不依赖 `indexV8` —— 上游主页首 tab 走的是 `/v6/page/dataList`**

**依据**

| 文件:行 | 原文 |
| --- | --- |
| `_upstream/c001apk/.../ui/home/HomeScreen.kt:81` | `val homeTabTypes = TabType.entries.filter { it != TabType.FOLLOW }` —— 首个 tab 是 `APP` |
| 同上 `:314-400` | 首 tab 的页面是 `AppListScreen`，`TabType.FEED`（动态）才用 `HomeFeedScreen` → `getHomeFeed` |
| `_upstream/c001apk/.../ui/home/feed/HomeFeedScreen.kt:39-48,69-74` | `TabType.FOLLOW`/`HOT` 走 `/page?url=…`，`COOLPIC` 走 `/page?url=V11_FIND_COOLPIC` |
| `_upstream/c001apk/.../logic/repository/NetworkRepo.kt:243-250` | `getDataList(…) = flowList { api2Service.getDataList(url, title, subTitle, lastItem, page).await() }` |
| `_upstream/c001apk/.../ui/home/topic/HomeTopicScreen.kt:60-61` | `TabType.TOPIC -> "/v6/page/dataList?url=V11_VERTICAL_TOPIC&title=话题&page=1"`；`TabType.PRODUCT -> "/v6/product/categoryList"` |
| `_upstream/c001apk/.../logic/network/ApiService.kt:35-42` | `indexV8` 需要 `page/firstLaunch/installTime/firstItem/lastItem/ids` 六个参数 |

**关键推论**：我们一直拿 `indexV8`（且 `installTime=0`）当「首页」在测。上游的「能浏览」主要体现在 `/v6/page/dataList?url=V11_*`、`/v6/product/categoryList` 这类**配置化首页卡**。如果 1004 的门禁对不同 `url=` 的卡片有不同的策略（很常见：A/B 或灰度），我们可能一直在测一个对我们更严的入口。

**本次实测（已顺带做了）**

```
/v6/__nope__              → {"message":"\/__nope__ not found.","code":404}   ← 路由 404，说明 1004 在路由之前
/v6/page/dataList?url=V11_VERTICAL_TOPIC → 1004
/v6/product/categoryList → 1004
/v6/feed/detail?id=1     → 1004
/v6/account/checkLoginInfo → 1004
```

→ **这条被削弱了**：所有 `/v6/*` 的全新端点同样 1004，说明「换端点」不足以绕过。但注意上面 404 对照组的价值（见 §2 小结），以及 `installTime` 参数的真实差异（见发现 5）。

**为什么仍值得保留**：它是**最省成本的旁路验证**——如果 `V9_HOME_TAB_FOLLOW` / `V11_HOME_TAB_RANKING` 有任何一个能 200，就说明门禁是**按卡片**而不是全局。

**验证方法**：把 `HomeFeedScreen.kt:39-74` 列出的全部 `url=` 值（`V9_HOME_TAB_FOLLOW`、`V9_HOME_TAB_FOLLOW&type=circle|topic|product|apk`、`V11_HOME_TAB_RANKING`、`V11_FIND_COOLPIC`、`V11_VERTICAL_TOPIC`）**各打一次** `/v6/page/dataList?url=<值>&page=1`。探针只需加一行循环。

**若成立怎么修**：`client.rs::BrowseTarget::path`（`:43-48`）增加 `HomeCard(String)` 变体，首页默认走 `V11_VERTICAL_TOPIC` 而不是 `indexV8`。

---

### 发现 5（★☆☆ 但零成本）：**`installTime` 我们发的是 `0`，上游发的是毫秒时间戳；`firstLaunch` 我们也错了**

**依据**

| 文件:行 | 原文 |
| --- | --- |
| `src-tauri/src/client.rs:45`（我方） | `Self::Home => format!("/v6/main/indexV8?page={page}&firstLaunch=0&installTime=0")` |
| `_upstream/c001apk/.../ui/home/feed/HomeFeedViewModel.kt:49-55` | `if (installTime.isEmpty()) { installTime = System.currentTimeMillis().toString(); setInstallTime() }` |
| `_upstream/c001apk/.../ui/base/BaseViewModel.kt:47` | `var firstLaunch = 1`（首屏是 `1`，只有刷新才置 `0`，见 `HomeFeedViewModel.kt:72`） |
| `_upstream/c001apk/.../logic/datastore/UserPreferencesCompat.kt:287` | `installTime = EMPTY_STRING`，**首次启动才生成，之后持久不变** |
| 我方 | `installTime` 常量 0，**没有持久化安装时间** |

**为什么可能有效**：`installTime=0` 是「1970 年安装」的客户端，属于典型脚本特征。这是一条**服务端零成本可判**的字段，且我们与真机**必然不同**。它单独不太可能触发 1004，但叠加在别的判定上完全可能把「疑似脚本」升格为「拒绝」。

**验证方法**：`probe_breakthrough.py` 的 `home` URL 直接改成 `firstLaunch=1&installTime=<当前毫秒>`，与 `installTime=0` 对比一次。

**若成立怎么修**：`DeviceProfile`/`PersistedConfig` 增加 `install_time: i64`（首次生成为「现在 - 随机 1~180 天」），`client.rs::BrowseTarget::path` 与 `firstLaunch` 改为遵守 `BaseViewModel` 的语义。

---

### 发现 6（★★☆，成本最高但结论最硬）：**「web 会话」与「TLS 指纹」必须做一次判别实验，否则永远在原地打转**

**依据**

| 来源 | 原文 / 现状 |
| --- | --- |
| `src-tauri/Cargo.toml:37-44`（我方） | `reqwest = { version = "0.12", default-features = false, features = ["json","cookies","gzip","rustls-tls","stream"] }` → **rustls** ClientHello |
| `_upstream/c001apk/app/build.gradle.kts:210-212` | `okhttp` + `retrofit` + `gson` → **JDK/BoringSSL** 风格的 ClientHello |
| `docs/PROTOCOL.md:424`（我方自述） | `服务端会识别非浏览器/非 App 的 TLS 特征` |
| `docs/_1004-测试矩阵.txt:13` | 已测 `HTTP 层`，但**没有 TLS 指纹**一条 |

**为什么这是「第 8 方向」的必要组成**

如果门禁判定的是 TLS 指纹，那么发现 1/2/4/5 全部无效，而且会**长期无效**（当前所有测试都在 rustls 上）。做一次判别实验能把这两种可能**彻底分开**，代价远低于继续盲试。

**验证方法（这是本次给出的「最高优先验证动作」）**

1. 用 **curl（schannel/openssl）** 打一次 `api2.coolapk.com/v6/main/indexV8`，带上完整 15 个头（`tools/rs-probe` 里现成的 `headers()` 可以直接打印出来）。
2. 用 **Windows 内置 `HttpClient`（.NET/schannel）** 或 **浏览器 fetch** 打同一次。
3. 三方结果矩阵：

   | 客户端 | TLS 栈 | 期望如果「TLS 指纹成立」 |
   | --- | --- | --- |
   | reqwest(rustls) | rustls | 1004（已知） |
   | curl | OpenSSL/Schannel | 200 |
   | 浏览器 fetch | Chromium | 200 |
   | reqwest(openssl) | OpenSSL | 200 |

   - 只要有一行是 **200** 而 rustls 是 1004 → **TLS 指纹成立**，这将是最终答案，修法明确（切 native-tls / 或复用 WebView 发请求）。
   - 全部 1004 → TLS 指纹排除，重心回到发现 1（web 会话）。

**若成立怎么修**：`src-tauri/Cargo.toml` 把 `rustls-tls` 换成 `native-tls`（Windows 走 Schannel，与浏览器一致），并在 `transport.rs::ReqwestTransport::new` 显式设置 `tls_built_in_root_certs`、`min_tls_version(TLSv1_2)`、`http1_only()`（浏览器到 api 域实际也是 h2，但先 h1 排除变量）。更彻底的做法是**所有 API 请求改由 WebView 发出**（`fetch` + 自定义头），代价是丢掉 Rust 侧的类型安全，但一次性解决 TLS + 会话两件事。

---

### 发现 7（★☆☆，调试噪声，会误导排查）：**「需要人机验证」日志是假的，`pending_captcha` 从未被设置过**

**依据**

| 文件:行 | 原文 / 现状 |
| --- | --- |
| `Kuan/_run/coolall.log`（历史运行日志） | `响应 … status=200` 紧跟 `需要人机验证 id=414e5c9b866a03db03f860a1a9101672 field=_v2_post_token` |
| `crates/coolall-core/src/error.rs:204-243` | 纯 1004 走到最后的 `Some(Error::Business { status: 1004, .. })`，**不会**产生 `CaptchaNeeded` |
| `src-tauri/src/client.rs:443-453` | `track_challenge` 只在 `Err(Error::CaptchaNeeded{..})` 时记录 |
| `src-tauri/src/commands.rs:123-129` | `open_captcha_window` 的报错文案「当前没有待完成的验证」 |
| 对照 `_upstream/c001apk/.../logic/network/CaptchaChallenge.kt:24-29` | 上游只在 `messageStatus == "err_request_captcha_v2"` 时才认挑战 |

**关键推论**：那份日志属于**修正前**的版本（`docs/1004-排查报告.md §3` 记录已删除 `status==1004 → CaptchaNeeded`）。但它留在 `_run/` 里，后来者读到会以为「已经拿到挑战了」。这类陈旧证据正是「卡很久」的原因之一。

**验证方法**：跑一次当前构建，确认日志里 1004 之后**没有**「需要人机验证」一行。

**若成立怎么修**：`_run/coolall.log` 归档或删除；`docs/1004-排查报告.md` 顶部加一行「`_run/` 内历史日志已作废」。

---

## 2. 本次实测小结（可复现）

探针脚本：`_probe1004\probe_breakthrough.py`（仓库外的临时目录，只读，不改 `Kuan`）。

```
=== A 组：X-App-Device 第 2-4 段（slot），首页 indexV8 ===
 - 空 slot（我方现状）   9 段       → 1004
 - slot4=MAC            9 段       → 1004
=== B 组：按其 Domain/Path 回带 Cookie ===
 [首访]                  → 1004    Set-Cookie: SESSID=…; Max-Age=8640000; path=/; domain=.coolapk.com
                                    （无 Secure）
 [SESSID 裸值]           → 1004
 [SESSID + forward]      → 1004
 [DID + SESSID]          → 1004    ← 三种 Cookie 组合的 Set-Cookie 完全一致 → 我方 Cookie 被忽略
=== C 组 ===
 /v6/__nope__            → {"message":"\/__nope__ not found.","code":404}   ← 路由层，证明 1004 在它之前
 /v6/page/dataList?url=V11_VERTICAL_TOPIC → 1004
 /v6/product/categoryList                 → 1004
 /v6/feed/detail?id=1                     → 1004
 /v6/account/checkLoginInfo               → 1004
```

**两个新事实**（不是已排除的 7 个方向）：
1. **1004 发生在路由之前**：不存在的路径返回的是路由 404，不是 1004。所以 1004 是**前置门禁**，不是业务层。
2. **服务端忽略我们送的 Cookie，并按自己的规则重新下发**（无 `Secure`、100 天、同时下发 `token=deleted`）。说明它认为「会话」应该由**它自己种、我方长期养护**，而不是我方现造。

---

## 3. 优先级建议（按「成本 / 信息量」排序）

| # | 动作 | 成本 | 信息量 |
| --- | --- | --- | --- |
| 1 | **TLS 指纹判别实验**（curl / .NET / 浏览器 vs rustls，同一套头） | 15 分钟 | 极高：直接二选一定生死 |
| 2 | **让用户从手机 c001apk「参数页」抄全 10 项真机参数**，逐字写入 `identity.json` 重测 | 用户 5 分钟 | 高：确定「画像自洽」是否必要条件 |
| 3 | **真机抓包（唯一金标准）**：`X-App-Device` 明文 + 完整 Cookie + 是否有 `DID` | 用户 15 分钟 | 决定性 |
| 4 | 方案 A：`warm_up()` 用隐藏 WebView 访问 `www.coolapk.com`，把 cookie 罐导给 reqwest | 半天 | 高 |
| 5 | 全部 `V11_*` / `V9_*` 卡片各打一次 | 5 分钟 | 中 |
| 6 | `installTime` / `firstLaunch` 对齐上游 | 10 分钟 | 低但零风险 |

**为什么把「真机抓包」放第 3 而不是第 1**：它是最强的证据，但需要用户配合；第 1、2 步我们可以自己先跑完，且无论结果如何都会显著缩小范围。

---

## 4. 明确未做的事

- 未修改任何源码（`git status` 干净，仅本文件为新增）。
- 未 push。
- 未重复已排除的 7 个方向：网络、设备串格式、SESSID 存在性、端点存在性、数盟 ID 有无、请求头增删、协议版本。
  - 说明：§发现 4/5 讨论的是**参数取值与卡片选择**（`installTime`/`firstLaunch`/`url=` 值），不是「端点是否被拒」；§发现 2 讨论的是**数盟 ID 与画像的一致性**，不是「数盟 ID 有无」；§发现 3 讨论的是**Cookie 的长期养护与 `token` 清除语义**，不是「SESSID 是否存在」。

# 补充实验记录（2026-11-01）

## 实验 1：不存在的路径

| 路径 | 响应 |
| --- | --- |
| `/v6/__nope__` | `{"message":"/__nope__ not found.","code":404}` |
| `/v6/main/indexV8` | `1004 请求验证错误` |

**结论：1004 发生在路由之前，是门禁层判定，与端点无关。**

## 实验 2：TLS 指纹

| 客户端 | TLS 栈 | 结果 |
| --- | --- | --- |
| curl | OpenSSL 3.0.13 | 1004 |
| .NET HttpClient | Windows Schannel | 1004 |
| reqwest | rustls | 1004 |

**结论：三个不同 TLS 栈结果相同，排除 TLS 指纹假设。**

## 实验 3：真实 WebView 会话（Edge CDP）

用 Edge 全新用户目录访问 `https://www.coolapk.com/`，
通过 CDP 读取真实 cookie：

| Cookie | 值 |
| --- | --- |
| SESSID | `w1-4f20ff2f44e0e57c6ec2732012ddbee5-1793489173-58d4aae46ca9d6d3` |
| Hm_lvt_ad71... | 1793489173 |
| Hm_lpvt_ad71... | 1793489173 |
| HMACCOUNT | 40DDCB0F727EA588 |

**用这个真实 session cookie 打 API，仍然是 1004。**

### 重要发现：web 域与 api 域的 SESSID 前缀不同

| 来源 | SESSID 前缀 |
| --- | --- |
| `www.coolapk.com`（web） | `w1-` |
| `api2.coolapk.com`（api） | `a1-` |

说明**两个域有独立的会话体系**，web 会话不能用于 api。

## 排除汇总（共 12 个方向）

1. IP/网络 —— 绑定 WLAN 网卡
2. 设备串格式 —— 5 种形态
3. 数盟 ID —— 真实 ID `DUP4awltXU__0ABOiBz7NwkxTrv4hbvIeRgd`
4. SESSID —— 有/无/稳定/真实 web 会话
5. 端点 —— 5 个端点
6. 请求头 —— 逐个移除
7. 协议版本 —— 16.x / 13.x
8. HTTP 层 —— gzip/HTTP1.1/keep-alive
9. installTime —— 真实时间戳 vs 0
10. TLS 指纹 —— 3 个不同栈
11. WebView 会话 —— Edge 真实 cookie
12. 路由层 —— 不存在的路径返回 404 而非 1004

## 当前判断

所有「请求构造」相关的可能性都已排除。
1004 是服务端在**门禁层**做的判定，而我们缺少某个**无法从客户端单向推导**的条件。

剩余可能：

| 假设 | 可行性 |
| --- | --- |
| 需要真实 Android 设备环境（Widevine/真实 AndroidID） | 桌面端无法满足 |
| 需要数盟 ID 与真机指纹绑定 | 需要真机数据 |
| 服务端对这个 IP 段/客户端群体有额外策略 | 无法验证 |
| **必须抓包对比才能确定** | ✅ 唯一可行路径 |

# 决定性实验：SESSID 与 TLS 指纹绑定（2026-11-01 07:50）

## 实验设计

在同一台手机（MEIZU 21）上，同时抓取：
1. c001apk（OkHttp）的请求
2. 手机自带 curl（BoringSSL）的请求

两者**同设备、同网络、同 BoringSSL 基础库**，只有 TLS 配置不同。

## 抓包结果对比

| 项 | App (OkHttp) | curl (BoringSSL) |
| --- | --- | --- |
| 套件数 | **15** | **17** |
| 套件顺序 | 1301 1302 1303 c02b c02c cca9 c02f c030 cca8 c013 c014 009c 009d 002f 0035 | 1301 1302 1303 c02b c02f c02c c030 cca9 cca8 c009 c013 c00a c014 009c 009d 002f 0035 |
| 扩展顺序 | 0,23,65281,10,11,35,16,5,13,51,45,43,21 | 0,23,65281,10,11,16,13,51,45,43,21 |
| 结果 | ✅ 能看内容 | ❌ 1004 |

**同为 BoringSSL，仅因配置差异导致指纹不同，结果就完全不同。**

## 更关键的实验

从 App 的 `shared_prefs/coolapk_api_session.xml` 读出它**当前有效的 SESSID**：

```
a1-a068b5f8cc9fedc28816110f7de39ded6a63e99dbaa5ac5ab05e77568eae62ad
```

用这个**真实有效**的 SESSID，通过 curl 打 API：

```
结果：{"status":1004,"error":"null","message":"请求验证错误"}
```

**即使持有 App 的有效会话，换客户端仍然失败。**

## 结论

**SESSID 与 TLS 指纹绑定。** 服务端在建立会话时就记录了客户端指纹，
之后每个请求都校验「会话 + 指纹」的匹配关系。

这解释了此前所有失败：
- 换任何设备串 → 1004（指纹不匹配）
- 换任何 SESSID → 1004（指纹不匹配）
- 用数盟真实 ID → 1004（指纹不匹配）
- 用 WebView 会话 → 1004（指纹不匹配）
- 用浏览器指纹 → 1004（不是 App 的指纹）

## 为什么浏览器指纹也不行

curl_cffi 模拟 chrome110/120/124/131、safari、firefox 全部 1004。
说明服务端**只接受 OkHttp 的精确指纹**，不接受任何浏览器指纹。

## 验证 App 的验证流程后发现了什么

App 完成滑块验证后，`coolapk_api_session.xml` 里：

```json
{"session": {
  "domain": "coolapk.com",
  "expires": 1794095227538,
  "hostname": false,
  "identity": "ea687d68...",
  "path": "/",
  "value": "a1-a068b5f8..."
}}
```

**只有 session，没有 validation 字段。**
说明验证通过后拿到的就是一个正常 SESSID，没有额外的 validate 凭证。

## 这意味着什么

之前关于「人机验证」的整条链路设计（CaptchaBox / validate / proof）**可能是错的**：
- App 弹验证 ≠ API 返回验证码挑战
- 验证通过后只是换了个有效 SESSID
- 1004 的本质是「会话未通过门禁认证」，认证手段包括 TLS 指纹

## 当前技术判断

| 路线 | 可行性 |
| --- | --- |
| 调整 rustls 指纹 | 需精确模拟 OkHttp 的 15 套件顺序 + 13 扩展顺序 |
| 用浏览器指纹 | ❌ 已实测失败 |
| 用 Android BoringSSL | 需精确配置，手机 curl 默认配置失败 |
| WebView 发请求 | ❌ 已实测失败 |

**唯一可行的方向：精确复刻 OkHttp 的 ClientHello。**

---

# 补充验证：手工构造 ClientHello 的可行性（2026-11-01 08:10）

## 实验：用 Python 手工构造与 OkHttp 一致的 ClientHello

构造内容：
- 套件列表：`1301 1302 1303 c02b c02c cca9 c02f c030 cca8 c013 c014 009c 009d 002f 0035`（15 个，与 App 一致）
- 扩展顺序：`0, 23, 65281, 10, 11, 35, 16, 5, 13, 51, 45, 43, 21`（与 App 一致）
- SNI / ALPN / padding 都按 OkHttp 特征

### 结果

| 尝试 | 响应 |
| --- | --- |
| 随机 key_share | `Alert 0x32 (illegal_parameter)` |
| 真实 x25519 公钥 | `Alert 0x32 (illegal_parameter)` |

**但这是一个重要信号**：服务器**回应了 Alert**，而不是直接断连。

说明：
1. 服务器的 TLS 层**接受了我们的 ClientHello 到达并解析**
2. 只是某个参数不对（可能是我构造的 padding 位置、或 TLS 1.3 key_share 格式细节）
3. **说明「手工构造 ClientHello」这条路在协议层是可行的**

### 与「被 WAF 拒绝」的区别

| 现象 | 含义 |
| --- | --- |
| 直接断连 / 空响应 | WAF 层拒绝（如 Chromium 的 403） |
| **Alert 响应** | **TLS 层已接收，参数错误** |

我们拿到的是 Alert，说明 TLS 层在和我们对话。

## 三条路线的最终判定

| 路线 | 结论 | 依据 |
| --- | --- | --- |
| A：调整 rustls 指纹 | ⏳ 未完成 | rustls 不支持自定义扩展顺序，需要底层方案 |
| B：WebView/浏览器指纹 | ❌ **已排除** | Chromium 拿到 403，比 1004 拒得更早 |
| C：curl_cffi 浏览器指纹 | ❌ **已排除** | 7 种浏览器指纹全 1004 |
| D：手机 BoringSSL curl | ❌ **已排除** | 同设备同库，配置差异导致 1004 |
| E：直接用 App 的 SESSID | ❌ **已排除** | 会话与指纹绑定，换客户端即失效 |

## 现状总结

**根因已确认**：服务端校验 TLS 指纹与会话的绑定关系。
**唯一可行方向**：精确复刻 OkHttp 的完整客户端画像（TLS ClientHello + HTTP/2 帧序 + 头部顺序）。

**难度评估**：需要实现完整的 TLS 1.3 握手状态机，工作量极大。
替代方案是找到一个能精确控制 ClientHello 的 Rust TLS 库。

## 对项目的现实建议

阶段 1 若无法解决 TLS 指纹问题，可考虑的折中：

1. **接受现状**：应用能构建、能运行、能显示 UI，但无法获取内容
2. **改用其他数据源**：比如公开的网页接口（也需验证）
3. **等 Rust 生态出现可用的指纹库**

需要用户决定方向。

---

# 🎯 突破：完整 TLS 握手成功（2026-11-01 08:20）

## 实验过程

### 实验 1：重放真机原始 ClientHello

从 tcpdump 抓到的真机包中提取 `api2.coolapk.com` 的完整 ClientHello 原始字节（517 字节），直接发给服务器：

```
✅✅ ServerHello！握手可以继续
前32字节: 160303007a020000760303bdc83ce3a274a185251a2c9ea1239ab60f9c9b7445
```

**服务器正常响应 ServerHello。**

对比我之前手工构造的（Alert 0x32 illegal_parameter），说明：
- 我构造的结构有细节错误
- **真机的原始字节完全可用**

### 实验 2：替换 key_share

真机 ClientHello 里唯一的「一次性」数据是 key_share 里的 x25519 公钥。
它的私钥我们不知道，无法完成密钥交换。

**关键操作**：保留真机 ClientHello 的所有其他字节（套件、扩展顺序、padding 全部不变），
只把 key_share 里的 32 字节公钥换成我们自己生成的。

```
原公钥: 66c424519f8a588ddbfa15d4cec0bc893818a1abdc13daa17ecb3c61bba8a42c
新公钥: b275ffd3e1efbfebb093a339a3f3386069ed314cd4a63d83f6ff324d69c4f243

发送后：
✅✅ ServerHello
服务器响应 5478 字节（含完整握手链）
```

**服务器完全接受了我们替换公钥后的 ClientHello。**

## 结论：这条路完全可行

| 要素 | 来源 | 是否可复用 |
| --- | --- | --- |
| 套件列表（15 个） | 真机模板 | ✅ 固定 |
| 扩展顺序（13 个） | 真机模板 | ✅ 固定 |
| padding 长度 | 真机模板 | ✅ 固定 |
| **key_share 公钥** | **自己生成** | ✅ **可替换** |
| random / session_id | 任意 | ✅ 不影响 |

**只要有一份真机 ClientHello 的模板字节，就可以生成任意多个合法的握手。**

## 技术路径

```
1. 保存真机 ClientHello 作为模板（517 字节）
2. 每次握手时：
   a. 生成新的 x25519 密钥对
   b. 把公钥写入模板的 key_share 位置（偏移 224，长度 32）
   c. 发送，接收 ServerHello
   d. 用私钥完成密钥交换
   e. 继续握手直到加密通道建立
3. 在加密通道上发 HTTP 请求
```

## 剩余工作

1. 实现完整的 TLS 1.3 握手状态机（ServerHello → EncryptedExtensions → Certificate → CertificateVerify → Finished）
2. HTTP/2 或 HTTP/1.1 帧处理
3. 集成到 Rust 项目

## 关键数据（供实现参考）

```
ClientHello 总长: 517 字节
key_share 扩展起点: 214（从 handshake 消息起始算）
公钥偏移: 224（从 handshake 消息起始算）
公钥长度: 32
密钥组: 0x001d (x25519)
record header: 5 字节（16 03 01 + 长度）
```

---

# 🎯🎯 TLS 1.3 握手完全打通（2026-11-01 08:35）

## 最终验证结果

用真机 ClientHello 模板 + 自生成 key_share，完成完整 TLS 1.3 握手：

```
ClientHello: 517 字节
植入公钥: 01d8f8c93e1e85c3f4b3c0c61677b58e...
收到 5478 字节, 6 个 record
套件 0x1302  hash=sha384  key_len=32
服务端公钥: c3724828920d2c297602e40f33a7054f...

✅ 握手密钥派生完成
✅ record #1 解密成功 (26 字节)   -> EncryptedExtensions
✅ record #2 解密成功 (4917 字节) -> Certificate
✅ record #3 解密成功 (265 字节)  -> CertificateVerify
✅ record #4 解密成功 (53 字节)   -> Finished
```

**服务端的全部握手消息都已成功解密。**

## 实现要点（踩过的坑）

| 坑 | 正确做法 |
| --- | --- |
| key_share 偏移 | record 内 **229**（含 5 字节 record header） |
| 哈希算法 | 按协商套件选：0x1302 用 **SHA-384**，0x1301/0x1303 用 SHA-256 |
| ServerHello 解析偏移 | session_id_len 在 **4+2+32=38** 位置（含 handshake header） |
| transcript hash | 必须包含 **ClientHello + ServerHello 的完整 handshake 消息**（不含 record header） |
| 密钥调度顺序 | `early_secret -> derived -> handshake_secret`，derived 的 ctx 用空串的哈希 |

## 剩余步骤

1. 发送 client Finished（用 c_hs_traffic 加密）
2. 派生应用数据密钥（client/server application traffic）
3. 在加密通道上发 HTTP GET /v6/main/indexV8
4. **看返回 1004 还是 data**

## 文件

- `tools/tls-handshake3.py` —— 当前最完整的实现（握手 + 解密全通）

## 意义

这条路**技术上完全可行**。剩下的都是标准 TLS 1.3 流程，
不需要任何特殊技巧。

现在唯一的问题是：**握手成功后，业务层会不会放行**。

---

# 🔴 最终结论：TLS 指纹已完全对齐，但仍是 1004（2026-11-01 08:55）

## 实验：完整 TLS 1.3 握手 + 真实 HTTP 请求

用真机 ClientHello 模板（517 字节）+ 自生成 key_share，完整走完：

```
ServerHello: suite=0x1302 hash=sha384 key=32B
✅ 服务端 Finished 校验: 通过
✅ HTTP 请求发送成功
✅ 响应解密成功

HTTP/1.1 200 OK
{"status":1004,"error":null,"message":"请求验证错误"}
```

**TLS 层完全通过，业务层仍然拒绝。**

## 这意味着什么

**TLS 指纹不是 1004 的原因。**

之前的所有推断都被推翻了：
- 不是 TLS 指纹（现已精确对齐，仍失败）
- 不是设备串（用真机数据）
- 不是 SESSID（用真机有效会话）
- 不是请求头（15 个全对齐）
- 不是网络出口（绕过代理测试）
- 不是数盟 ID（用真实 ID）

## 已排除方向的完整清单（13 项）

| # | 方向 | 验证方式 | 结果 |
| --- | --- | --- | --- |
| 1 | IP/网络 | 绑定 WLAN 网卡 | 1004 |
| 2 | 设备串格式 | 5 种形态 | 1004 |
| 3 | 数盟 ID | 真实 ID（服务端签发） | 1004 |
| 4 | SESSID | 真机有效会话 | 1004 |
| 5 | 端点 | 5 个端点 | 1004 |
| 6 | 请求头 | 逐个移除测试 | 1004 |
| 7 | 协议版本 | 16.x / 13.x | 1004 / 403 |
| 8 | HTTP 层 | gzip/HTTP1.1/keep-alive | 1004 |
| 9 | installTime | 真实时间戳 | 1004 |
| 10 | curl_cffi 浏览器指纹 | 7 种 | 1004 |
| 11 | Chromium/WebView | Edge CDP | 403 |
| 12 | 手机 BoringSSL curl | 同设备同库 | 1004 |
| 13 | **TLS 指纹完全对齐** | **真机 ClientHello 模板 + 完整握手** | **1004** |

## 唯一剩下的可能

**App 在完成人机验证后，服务端会话状态发生了变化。**

回顾关键事实：
- 用户明确说：c001apk 打开时会弹人机验证，完成滑块后才能看内容
- App 的 `coolapk_api_session.xml` 里只有 `session`，没有 `validation`
- SESSID 前缀：api 域是 `a1-`，web 域是 `w1-`

**推论**：验证不是通过某个 header 或 cookie 传递的，
而是**服务端记录了「这个 SESSID 已经通过验证」的状态**。

我们的 curl 拿到的 SESSID 从未经过验证，所以永远 1004。

## 真正的解法

必须**在 App 的 WebView 里完成人机验证**，拿到「已验证」的 SESSID，
然后用这个 SESSID 发请求。

但这又回到了「会话与客户端绑定」的问题——除非验证后的 SESSID 不再校验指纹。

**需要用验证后的 SESSID 测试。** 这需要：
1. 在 App 里触发验证并完成
2. 立即读出 SESSID
3. 用我们的客户端带这个 SESSID 发请求

## 技术资产

已经实现了完整的 TLS 1.3 客户端（Python）：
- `tools/tls-probe/gen/tls13.py` —— 可复用的 TLS 1.3 实现
- `tools/tls-probe/gen/probe.py` —— 端到端探针
- 关键参数：sha384（0x1302）、key_share 偏移、transcript 计算

如果后续需要，这套代码可以直接用于任何需要特定 TLS 指纹的场景。

---

# 最终定位：验证弹窗的触发条件（2026-11-01 09:20）

## 决定性代码分析

### 拦截器执行顺序（NetworkModule.kt:108-118）

```kotlin
.addInterceptor(RequestSession.capture)
.addInterceptor(ApiFailureInterceptor())    // ← 先执行
.addInterceptor(CaptchaInterceptor())       // ← 后执行
.addNetworkInterceptor(AddCookiesInterceptor)
```

### ApiFailureInterceptor 的判定（ApiFailure.kt:89-101）

```kotlin
// 只有这一种情况放行给 CaptchaInterceptor
if (body.get("messageStatus")?.asString == "err_request_captcha") {
    return response
}
// 其他所有失败都抛异常，CaptchaInterceptor 收不到响应
val failure = ApiFailures.fromBody(body)
if (failure != null) { throw failure }
```

**结论：只有 `messageStatus == "err_request_captcha"` 才会弹滑块。**

### 我们收到的响应

```json
{"status":1004,"error":null,"message":"请求验证错误"}
```

- 没有 `messageStatus` 字段
- 因此不会命中放行条件
- 因此永远不会弹滑块

## 「err_request_captcha」出现的位置

全仓搜索确认，只在两处：

| 文件 | 行 | 场景 |
| --- | --- | --- |
| ReplyViewModel.kt | 52 | 发评论 |
| ReplyViewModel.kt | 129 | 发回复 |
| ReplyActivity.kt | 601 | 请求验证参数（配合 code/mobile/idcard 字段） |

这些都带 `requestValidateData["type"] = "err_request_captcha"`，
且伴随**账号验证字段**（验证码、手机号、身份证）。

## 推论

1. `err_request_captcha` 是**账号级验证**，不是浏览级
2. 用户描述的「点开动态弹滑块」可能是：
   - 登录态相关的引导
   - 或者触发了 app 内 webview 的验证
3. 我们纯游客 + 无登录态，服务端只返回 1004，不给验证入口

## 当前真实的阻塞

**不是 UI，不是 TLS，不是设备串，而是：**
服务端不给我们 `err_request_captcha`，所以我们连验证的机会都没有。

## 待验证（需要手机）

必须抓包对比 App 打开动态时的真实响应体，
看它是否真的包含 `messageStatus: err_request_captcha`。

如果包含 → 找出触发条件
如果不包含 → 用户的滑块来自别的路径（如登录流程）

## 阶段 1 的现实结论

在当前条件下（游客、无登录），**API 无法返回内容**。
这是服务端策略，不是实现缺陷。

可选路径：
1. 实现登录流程（用户扫码/密码登录）
2. 用 WebView 直接渲染酷安网页版（绕过 API）
3. 等拿到真实的验证触发条件
