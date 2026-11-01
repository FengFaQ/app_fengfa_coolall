# 路线 B：用 WebView / 浏览器指纹发 API 请求

> 任务：task-8「用 WebView/浏览器指纹发酷安 API 请求」
> 时间：2026-11-01（Asia/Shanghai）
> 结论：**已实测排除。此路线不可行，不必再投入。**

---

## 0. 结论先行

**用 Chromium 内核（Edge / WebView2）的 TLS 指纹发请求，比 rustls 更差，不是更好。**

| 客户端 TLS 栈 | 完整 App 头下的响应 |
| --- | --- |
| rustls（我方现状） | `200 {"status":1004,"message":"请求验证错误"}` |
| **Chromium / Edge 156** | **`200 {"status":403,"message":"API unsupported."}`** |

关键点：Chromium 拿到的 **`403 API unsupported.`** 是一个**恒定响应**，与请求头完全无关。
在 8 种请求头组合下（完整头、最小头、浏览器 UA、真机 UA、带 Cookie、带 Accept-Language……）
它**一次都没变过**；而 rustls 同一批请求返回的是有区别的、会随头部变化的业务态
（`1004 请求验证错误` / `1004 请求未验证` / `1000 请求方式错误`）。

**这说明 Chromium 的请求根本没走到酷安业务层，更没有走到 Token 校验那一步。**
`403 API unsupported.` 是 TencentEdgeOne（CDN/WAF）层给出的「我不支持这类客户端」，
比 rustls 的 1004 还要早一层的拒绝。

因此路线 B 不只是「不成功」，而是**方向相反**：换成浏览器内核会把请求推得更远、拒得更早。

---

## 1. 前提验证实验（本次实测，可复现）

### 1.1 方法

用 Edge（Chromium 156.0.4314.50）以 `--remote-debugging-port=9222` 启动独立
user-data-dir，通过 CDP 的 `Fetch` 域**在 Request 阶段拦截并改写请求头**，
再用 `Network.getResponseBody` 读原始响应体。

为什么不用页面里的 `fetch()`：页面 JS 的 fetch 会被 Chromium 强制补上
`Origin` / `Referer` / `Sec-Fetch-*`，并且响应受 CORS 约束，看不到真实状态。
用 `Fetch.continueRequest` 才能做到「TLS 握手全部由 Chromium 产生，
而请求头 100% 由我们控制」——这正是本路线想验证的条件。

脚本见 `tools/webview-api/`（`cdp_intercept.py` 为核心）。

### 1.2 请求头变体矩阵（全部固定 Chromium TLS）

| 变体 | 请求头 | 响应 |
| --- | --- | --- |
| A | 完整 15 个 App 头（剥浏览器头） | `{"status":403,"message":"API unsupported."}` |
| V1 | 完整 App 头 + 保留浏览器头（Origin/Sec-Fetch 等 19 个） | 同上 |
| V2 | 完整 App 头 + Chromium UA | 同上 |
| V3 | 去掉 `X-Requested-With` | 同上 |
| V4 | 最小 8 头集 | 同上 |
| X1 | 完整头 + 真实 `w1-` SESSID | 同上 |
| X2 | 完整头 + `Accept-Language` / `Accept-Encoding` | 同上 |
| X3 | 完整头 + `Connection: keep-alive` | 同上 |

**8 / 8 全部为 `403 API unsupported.`，响应体一字不差。**

### 1.3 对照组：同一批请求头 vs rustls（决定性）

| 用例 | rustls (requests) | Chromium (Edge CDP) |
| --- | --- | --- |
| 裸请求（无头） | `403`（EdgeOne HTML 错误页） | `{"status":403,...}` |
| 仅 `Accept` | `403`（EdgeOne HTML 错误页） | `{"status":403,...}` |
| 仅真机 UA | `{"status":1000,"message":"请求方式错误"}` | `{"status":403,...}` |
| **完整 App 头** | **`{"status":1004,"message":"请求验证错误"}`** | `{"status":403,...}` |
| App 头但 Token 为空 | `{"status":1004,"message":"请求未验证"}` | `{"status":403,...}` |

**读法**：rustls 的响应**随请求头分化**（1000 / 1004验证错误 / 1004未验证），
说明它确实进入了业务层、并被逐个头部判定；
Chromium 的响应**恒定不变**，说明它在更早的一层就被整体拒绝。

> 本次 Token 已用 `tools/verify_token_v3.py` 自校验，结构全部通过
> （长度 60 / `$2y` 变体 / bcrypt cost=4 / 盐表合法），
> 所以 rustls 的 1004 不是「签名算错」，而是服务端对客户端身份的判定。

### 1.4 浏览器原生导航（无任何自定义头）

| 目标 | Chromium 原生 navigate 结果 |
| --- | --- |
| `https://www.coolapk.com/` | **200，64890 字节，正常渲染首页**（「酷安 - 分享美好科技生活」） |
| `https://api2.coolapk.com/v6/main/indexV8?...` | **403 + Chromium 错误页（309396 字节）** |

值得注意：**Chromium 连 `www.coolapk.com` 都能正常打开**，说明这台机器、
这个 Edge、这个 IP 访问酷安**没有被整体封禁**。被拒的是**api 域**，且是 CDN 层拒的。

---

## 2. 与 lead 侧实验的交叉印证

lead 用 `curl_cffi` 逐一模拟 7 种浏览器指纹（chrome110/120/124/131、safari15_5/17_0、firefox133）
**全部 1004**；用 Edge 真实 web 会话（`w1-` SESSID）**仍 1004**；
用 App 自身 XML 里的真实有效 SESSID（`a1-`）**仍 1004**。

本次结论与之一致，并补充了一条更强的证据：

- lead 的 `curl_cffi` 得到 **1004** —— 因为 curl_cffi 只换 TLS 指纹，
  但 HTTP 层仍是「标准 HTTP 客户端」，仍能进入业务层。
- 本次的真 Edge 得到 **`403 API unsupported.`** —— 真正的 Chromium 连业务层都没进。

两者共同指向同一结论：**服务端要的是 OkHttp 的完整客户端画像（TLS + HTTP/2 帧序 + 头部序），
而不只是「任意一种合法 TLS 指纹」**。WebView2 / Edge 的内核是 Chromium，
指纹与浏览器一致——**恰恰是它被拒的原因**，而不是被放行的理由。

`w1-` 与 `a1-` 的 SESSID 前缀差异也印证：web 域与 api 域是**独立的会话体系**，
web 会话在 api 域本来就不生效。

---

## 3. 为什么这条路「从原理上」不可能成功

WebView2（Tauri 在 Windows 上的运行时）与 Edge 共用同一 Chromium 网络栈 Thread。
因此 WebView2 发出的 ClientHello 与 Edge **逐字节相同**——本次实验已经拿到了
「与未来 Tauri WebView 完全等价」的那个指纹，并测出它被拒。

也就是说：**不需要真的去写 Tauri 代码，本次实验已经等价完成了那次验证。**
在 Tauri 里实现只会得到同样的 `403 API unsupported.`。

---

## 4. 保留的可复现工具

`tools/webview-api/` 下的脚本保持可运行，供后续核查（均为只读探针，不改主项目源码）：

| 文件 | 用途 |
| --- | --- |
| `kuan_headers.py` | 按真机数据构造 15 个请求头 + TokenV3（`python kuan_headers.py` 自检） |
| `cdp_fetch.py` | Edge + CDP 基础封装（启动/连接/求值），核心实验的最初版本 |
| `cdp_intercept.py` | **核心实验**：`Fetch` 拦截改写头 + 读原始 body |
| `variants.py` | 请求头变体矩阵 |
| `control_matrix.py` | **rustls vs Chromium 对照矩阵**（最有说服力的一张表） |
| `diag_branch.py` | Cookie / Accept-Language / Connection 补充判别 |
| `diag_web.py` | www 站与 api 域的原生导航对照 |

运行前提：Python 3.12+、`bcrypt` / `requests` / `websocket-client`；
Edge 安装在默认路径；脚本会自动启动并关闭独立的 Edge 实例
（user-data-dir 为 `tools/webview-api/.edge-profile`，可随时删除）。

复现核心结论只需一条命令：

```
cd Kuan/tools/webview-api
python control_matrix.py
```

---

## 5. 对后续方向的建议

既然「浏览器指纹」比「rustls」更糟，说明门禁做的是**精确客户端认定**而非**黑名单**。
纯请求侧（TLS / 头 / Cookie / 会话）的调参空间已经基本穷尽，剩余可选方向：

1. **拿到 OkHttp 的真实 ClientHello 逐字节复刻**（JA3/JA4 + TLS 扩展顺序 + HTTP/2 SETTINGS/优先级帧），
   例如用 `curl_cffi` 的自定义指纹能力或 Go/Rust 侧手工构造 ClientHello。
   这是唯一与「已被证明可用的客户端」完全对齐的做法。
2. **真机侧代理**：在真安卓设备上运行，让 App 自己发请求，我方只做展示层。
3. 若产品可接受，走 **web 接口**（`www.coolapk.com` 的页面数据，本次实测浏览器 200 正常），
   放弃 `/v6` API。

> 注：本次未修改任何主项目源码，未 push，只新增 `docs/WEBVIEW-API.md`
> 与 `tools/webview-api/`。
