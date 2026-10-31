# 酷安客户端协议实现文档

> 本文档记录 `coolall` 客户端实现酷安接口所需的全部协议细节。
> 所有结论均来自对上游参考项目的精读与本机实测验证，实测结果在文末统一列出。
>
> 项目：`coolall`（非官方客户端）
> 许可证：AGPL-3.0
> 参考上游：`KiritoXDone/c001apk`（AGPL-3.0）、`daimiaopeng/coolapk-desktop`（MIT）

---

## 1. 总览

酷安客户端与 `api.coolapk.com` 通信时，服务端会通过多重机制识别请求来源：

```
TLS 指纹 → 域名 WAF → 请求头完整性 → TokenV3 签名 → 会话 Cookie → 人机验证
```

**任何一层不通过，都会表现为 403 或业务错误码。** 本文件逐层说明。

---

## 2. 常量表

所有常量集中在实现层的 `constants` 模块，禁止散落。

| 名称 | 值 | 说明 |
| --- | --- | --- |
| `APP_ID` | `com.coolapk.market` | 包名，所有请求必须带 |
| `APP_LABEL` | `token://com.coolapk.market/dcf01e569c1e3db93a3d0fcf191a622c` | TokenV2 用，V3 不需要 |
| `CHANNEL` | `coolapk` | 分发渠道 |
| `MODE` | `universal` | 通用版 |
| `LOCALE` | `zh-CN` | 请求语言 |
| `REQUEST_WITH` | `XMLHttpRequest` | 固定值 |
| `VERSION_NAME` | `16.7.0` | **必须是 16.x**，见 §7 |
| `VERSION_CODE` | `2610291` | 同时作为 TokenV3 的 `appCode` |
| `API_VERSION` | `16` | `X-Api-Version` |
| `BASE_URL` | `https://api.coolapk.com` | 主接口 |
| `BASE_URL_2` | `https://api2.coolapk.com` | 首页等接口走这个 |
| `FOOTER_URL` | `https://www.coolapk.com` | 公开分享页，无需鉴权 |

> ⚠️ `VERSION_NAME`、`VERSION_CODE`、`API_VERSION` 三者必须自洽，混搭会被拒。

---

## 3. TokenV3 签名算法

### 3.1 算法步骤

输入：`device`（即 `X-App-Device` 的值）、`appCode`（= `VERSION_CODE` 的整数形式）、`timestamp`（秒）。

```
for offset in 0..15:
    t      = timestamp + offset
    index  = ((t + appCode) % 100) * 4 + 0x80
    chunk  = LOOKUP[index : index + 0x80]        # 字节数组切片，128 字节
    segment= base64_decode(chunk)                # 再次解码，得到 96 字符的字符串

    plain    = "com.coolapk.market&" + segment + "&" + md5hex(device)
               + "&" + t + "&" + appCode
    password = md5hex(base64_nopad(plain))
    source   = base64_nopad(hex(t) + "/" + md5hex(plain))[:22]
    salt     = source[:21] + ALPHABET[(ALPHABET.index(source[21]) - 5) mod 64]

    if salt 不匹配 ^[./A-Za-z0-9]{22}$:           # 含 '+' 时必须跳过
        continue

    hash = bcrypt(password, "$2a$04$" + salt)
    hash = "$2y" + hash[3:]                      # 前缀改写
    return "v3" + base64(hash)
```

失败则抛错。

### 3.2 三个必须理解的细节

**① 两层解码**
`LOOKUP` 是 930 字节的字节数组。切片得到 128 字节**之后还要再做一次 base64 解码**，才得到 `segment` 字符串（96 字符）。
这是最容易搞错的一步。

**② 盐的合法性有两条规则，不止一条**

盐字符串 22 个字符，用 bcrypt 专属字母表 `. / A-Z a-z 0-9` 表示。
转换为 16 字节原始盐时，`22 × 6 = 132` 位，而 16 字节只需要 128 位，
**末尾多出的 4 位必须为 0**，否则该盐非法。

因此盐可能因两种原因被拒：

| 原因 | 例子 | 说明 |
| --- | --- | --- |
| 末位为 `+` | `...ZGJlN+` | `+` 属于标准 Base64 但不属于 bcrypt 盐字符集 |
| 末尾 4 位残余非 0 | `...jI4OR` | 正则能过，但无法还原成 16 字节 |

> ⚠️ 上游实现只靠 `try/except` 捕获 bcrypt 的 `Invalid salt` 异常来跳过，
> 把第二条规则掩盖成了「运行时异常」。
> 本项目**显式检查**这两条规则，不依赖异常控制流。
> 实测：`Njc3NDg1ODEvN2RiMjg4OR` 正则合法但被 bcrypt 拒绝，正属此类。

**③ 重试不是冗余，且相邻时间戳会收敛**

外层 `for offset in 0..16` 逐个推进时间戳，直到找到合法盐。
**这意味着相邻时间戳可能落到同一个可用槽位**，产生完全相同的 token。

实测：`t=1735689600` 与 `t=1735689601` 输出相同（两者都跳到 `t+2` 的槽位）。
这不是 bug，是算法容错机制的必然结果；测试时须用间隔较远的时间戳验证时间敏感性。

**④ `device` 是编码后的设备串，不是 ID**
传入 `TokenV3.create()` 的第一个参数是 `X-App-Device` 的**完整编码值**，不是 `szlmId`。

### 3.3 输出格式

固定 60 字节的 bcrypt 结果，Base64 后加 `v3` 前缀：

```
v3 <base64( $2y$04$<22位盐><31位哈希> )>
```

总长约 83 字符。

### 3.4 LOOKUP 常量

查找表共 930 字节。实现时以文件形式内嵌（`assets/token_lookup.b64`），
**不要手工抄写**——抄错一个字符会导致索引错位且难以发现。

原始来源：参考实现中的 Base64 字符串字面量，长度 1240 字符。

---

## 4. 设备身份

### 4.1 设备串格式

`X-App-Device` 的**明文**是 9 段分号分隔的字符串：

```
{szlmId};{?};{?};{?};{manufacturer};{brand};{model};{build};null
```

字段含义：

| # | 字段 | 说明 |
| --- | --- | --- |
| 1 | `szlmId` | 数盟 ID，设备唯一标识 |
| 2-4 | 空 | 保留位 |
| 5 | `manufacturer` | 如 `Xiaomi` |
| 6 | `brand` | 如 `Xiaomi` |
| 7 | `model` | 如 `2211133C` |
| 8 | `build` | 如 `TKQ1.221114.001` |
| 9 | `null` | 固定值 |

**注意**：第 2–4 段为空时，格式是 `id; ; ; ; maker; ...`（分号间有空格）。

### 4.2 编码规则

```python
encoded = base64_nopad(plain)           # UTF-8 编码后 Base64，去掉尾部 '='
encoded = encoded[::-1]                 # 整体字符串反转
encoded = encoded.replace('\r','').replace('\n','').replace('=', '')
```

结果即 `X-App-Device` 的值。**校验方式**：反转 → 补 `=` → Base64 解码，应能还原出 9 段明文。

### 4.3 szlmId 获取策略

桌面端无法运行数盟 SDK，因此采用**两者结合**：

1. **首选**：首次启动随机生成一个合法 ID 并持久化
2. **兜底**：设置中提供「导入 ShuzlmID」入口，用户可从官方 App「复制 OAID」后粘贴

合法格式（`OfficialDeviceIdentity.valid`）：

```
长度 8..4096，字符集 ^[A-Za-z0-9_+/.=-]+$，且不等于 "null" / "undefined"
```

导入文本的匹配规则：

```
^\s*ShuzlmID\s*[:：]\s*([^\r\n]*)$   （MULTILINE + IGNORE_CASE）
必须唯一匹配一条
```

---

## 5. User-Agent

### 5.1 格式

```
Dalvik/2.1.0 (Linux; U; Android {release}; {model} {build}) \
(#Build; {brand}; {model}; {build}; {release}) \
+CoolMarket/{VERSION_NAME}-{VERSION_CODE}-{MODE}
```

### 5.2 示例

```
Dalvik/2.1.0 (Linux; U; Android 13; 2211133C TKQ1.221114.001) (#Build; Xiaomi; 2211133C; TKQ1.221114.001; 13) +CoolMarket/16.7.0-2610291-universal
```

### 5.3 为什么必须严格照抄

服务端会把 UA 与 `X-App-Version`、`X-App-Code`、`X-App-Mode` **交叉校验**，
且 EdgeOne 会按 UA 特征做 WAF 匹配。

**实测**：使用残缺 UA（缺 `(#Build; ...)` 段或 `+CoolMarket/...` 尾巴）时，请求被 WAF 直接返回 403 空响应。
补全后立刻变成 200 + 业务 JSON。

---

## 6. 请求头

### 6.1 完整列表

| Header | 值来源 | 必需 |
| --- | --- | --- |
| `User-Agent` | §5 | ✅ |
| `X-Requested-With` | `XMLHttpRequest` | ✅ |
| `X-Sdk-Int` | 系统 API 级别，如 `33` | ✅ |
| `X-Sdk-Locale` | `zh-CN` | ✅ |
| `X-App-Id` | `com.coolapk.market` | ✅ |
| `X-App-Token` | TokenV3（§3） | ✅ |
| `X-App-Version` | `16.7.0` | ✅ |
| `X-App-Code` | `2610291` | ✅ |
| `X-Api-Version` | `16` | ✅ |
| `X-App-Device` | §4 编码值 | ✅ |
| `X-Dark-Mode` | `0` / `1` | ✅ |
| `X-App-Channel` | `coolapk` | ✅ |
| `X-App-Mode` | `universal` | ✅ |
| `X-App-Supported` | 同 `X-App-Code` | ✅ |
| `Accept` | `application/json` | 建议 |
| `Cookie` | §8.2 | 会话建立后 |

### 6.2 易错点

- `X-App-Code` 与 `X-App-Supported` 是**两个不同的头**，值相同但不能只发一个
- `X-App-Channel` 缺失会导致风控判定
- 参考实现里 `X-Dark-Mode` 是跟主题联动的变量，不是固定值

---

## 7. 协议版本约束（重要）

**实测结论**：只有 `16.x` 协议能通过 WAF。

| VERSION_NAME / CODE | 结果 |
| --- | --- |
| `16.7.0` / `2610291` | 200 + 业务 JSON |
| `16.2.0` / `2604201` | 200 + 业务 JSON |
| `13.4.1` / `2312121` | **403 空响应** |

因此实现中**不得**允许用户随意降级协议版本。

---

## 8. 会话与 Cookie

### 8.1 SESSID 生命周期

```
首次请求（无 Cookie）
  → 响应 Set-Cookie: SESSID=...
  → 存入本地（域 .coolapk.com，路径 /）
  → 后续请求头带 Cookie: SESSID=...
```

保存条件（对齐参考实现）：

- 名称必须是 `SESSID`
- 域必须是请求主机或 `coolapk.com`
- 值匹配 `^[A-Za-z0-9,._~%+/-]{1,512}$`
- 过期时间上限 7 天
- SESSID 变化时**必须清除已有的 validate 凭证**

### 8.2 登录态 Cookie

```
Cookie: uid={uid}; username={username}; token={token}; SESSID={sessid}
```

游客态只带 `SESSID`。

---

## 9. 人机验证（1004）

### 9.1 现象

游客态请求会稳定返回：

```json
{"status": 1004, "error": null, "message": "请求验证错误"}
```

**实测**：换设备 ID、换 SDK 版本、换接口、回写 Cookie、多轮重试，全部稳定 1004。
这是服务端对脚本流量的强制要求，不是参数错误。

### 9.2 挑战响应格式

参考实现识别的是：

```json
{
  "messageStatus": "err_request_captcha_v2",
  "messageExtra": {
    "captchaId": "<16..64 位字母数字>",
    "captchaField": "_v2_post_token"
  }
}
```

字段缺省时的兜底值：

- `captchaId`：`414e5c9b866a03db03f860a1a9101672`
- `captchaField`：`_v2_post_token`

### 9.3 凭证构造

用户完成验证后拿到 `validate` 字符串，然后：

```
proof = "NEC:" + captchaId[:8] + ":" + validate
proof = URLEncode(proof, "UTF-8")
```

- `validate` 校验：非空、长度 ≤ 8192、不含空白和控制字符
- `captchaField` 非空 → 作为表单字段/查询参数附加到重放请求
- `captchaField` 为空 → 作为 `validate` Cookie 附带

### 9.4 重放规则

**只重放一次。** 传输错误不触发重放。重放后仍失败则清除凭证。

### 9.5 桌面端实现要求

Tauri 用**独立窗口**承载验证码 WebView，通过事件通道回传 `validate` 字符串。
凭证不得写入日志、URL 或本地明文存储。

---

## 10. 公开兜底路径

当 API 请求因验证码或鉴权失败，且内容 ID 为纯数字时，参考实现会降级到公开分享页：

```
https://www.coolapk.com/feed/{id}
```

**实测**：返回 HTTP 200 + 完整 HTML，**无需任何鉴权、不触发验证码**。

用途：保证游客至少能看内容。UI 上标注「公开页面」。

> 注意：`www.coolapk.com/v6/*` 返回 404，公开页只有 HTML，没有 JSON 接口。

---

## 11. 接口清单（阶段 1）

| 用途 | 方法 | 基础域 | 路径 |
| --- | --- | --- | --- |
| 首页信息流 | GET | `api2` | `/v6/main/indexV8` |
| 动态列表 | GET | `api2` | `/v6/feed/list` |
| 搜索 | GET | `api` | `/v6/search` |
| 动态详情 | GET | `api` | `/v6/feed/detail` |
| 评论列表 | GET | `api2` | `/v6/feed/replyList` |
| 用户空间 | GET | `api` | `/v6/user/space` |
| 用户动态 | GET | `api` | `/v6/user/feedList` |
| 登录检查 | GET | `api` | `/v6/account/checkLoginInfo` |

首页常用参数：`page`、`firstLaunch=0`、`installTime`、`firstItem`、`lastItem`、`ids`。

---

## 12. 错误分类

对齐参考实现的判定顺序：

| 类型 | 触发条件 | 用户提示 |
| --- | --- | --- |
| `RISK_CONTROL` | `status == -10` 或消息含「网络环境」 | 服务端检测到网络环境异常 |
| `CLIENT_UPGRADE` | `messageStatus == err_request_need_upgrade_new_version` | 协议未被接受，请更新客户端 |
| `CAPTCHA` | 含 `captcha` / 「验证」 | 请求需要人机验证 |
| `AUTHENTICATION` | 含 `login` / `auth` / 「登录」或 `code == 401` | 登录状态失效 |
| `RATE_LIMIT` | 含 `rate_limit` / 「频繁」或 `code == 429` | 请求过于频繁 |
| `BUSINESS` | 其他业务错误 | 透传服务端消息 |
| `HTTP` | 状态码不在 200..399 | 请求失败（HTTP xxx） |

成功判定：

```
无 messageStatus 或不以 err_ 开头
且 status ∈ {0, 1}
且 code ∈ {0, 1, 200}
且 error ∈ {null, "0", "false", ""}
```

---

## 13. 实测记录

验证环境：Windows / Ubuntu 24.04，中国河南联通出口，2026-10-31。

| # | 测试项 | 结果 |
| --- | --- | --- |
| 1 | TokenV3 确定性与结构 | ✅ 7/7 通过 |
| 2 | 盐表 100 个余数可解码 | ✅ 100/100 |
| 3 | 盐含 `+` 的重试路径 | ✅ 16 个时间戳中 3 个被跳过 |
| 4 | 残缺 UA 请求 | ❌ 403 空响应 |
| 5 | 完整 UA + 完整头 | ✅ 200 + 业务 JSON |
| 6 | 协议版本 13.4.1 | ❌ 403 |
| 7 | 协议版本 16.x | ✅ 200 |
| 8 | 游客态任意接口 | ⚠️ 1004 请求验证错误 |
| 9 | 换设备 ID / SDK / 接口 | ⚠️ 均 1004 |
| 10 | Cookie 回写 + 多轮重试 | ⚠️ 仍 1004 |
| 11 | `www.coolapk.com/feed/{id}` | ✅ 200 + HTML |

**结论**：请求构造已完全正确，剩余 1004 必须由真实客户端的 WebView 完成人机验证。

---

## 14. 已知限制

1. **TLS 指纹**：服务端会识别非浏览器/非 App 的 TLS 特征。真实客户端（Tauri WebView / Android）天然通过，脚本客户端无法通过。
2. **数盟 SDK**：桌面端不可用，设备 ID 采用随机 + 手动导入。
3. **协议漂移**：酷安会随时调整。所有协议常量必须集中管理，便于快速更新。
4. **分发**：本项目为 AGPL-3.0，任何分发必须开源完整源码。

---

## 15. 参考与致谢

- [KiritoXDone/c001apk](https://github.com/KiritoXDone/c001apk)（AGPL-3.0）：协议结构、设备身份、验证码流程的权威参考
- [daimiaopeng/coolapk-desktop](https://github.com/daimiaopeng/coolapk-desktop)（MIT）：Tauri 跨端实践参考
- [TG-Twilight/c001apk](https://github.com/TG-Twilight/c001apk)：验证码与数盟流程的早期实现

本项目对上述项目的协议理解做了独立重写与实测校验，未直接复制其源代码。

### 9.6 桌面端实现：内嵌 WebView 窗口（2026-10-31 修订）

**早期错误设计**：用外部浏览器打开验证页，让用户手动复制 validate 粘贴回来。
**为什么错**：外部浏览器是独立进程，其 Cookie 归浏览器管，应用读不到；
用户手动复制既不可靠，凭证还会经过剪贴板。

**当前实现**：

```
用户点「打开验证窗口」
  → Tauri WebviewWindowBuilder 创建 label="captcha" 的窗口
  → 加载 https://account.coolapk.com/auth/login?type=mobile
  → 用户在窗口内完成易盾验证
用户点「我已完成验证」
  → 后端调用 WebviewWindow::cookies() 读取全部 Cookie（含 HttpOnly）
  → 用 extract_validate() 找出 validate 并做 URL 解码
  → 组装 NEC:{id前8位}:{validate}，写入会话
  → 立刻重放原请求，返回结果
```

**关键实现约束**：

1. **`cookies()` 在 Windows 同步命令里会死锁**（wry#583）。
   必须用 `async` 命令，并用 `tokio::task::spawn_blocking` 包裹读取。
2. **验证窗口不给任何 IPC 权限**（`capabilities/captcha.json` 的 permissions 为空）。
   它加载的是远程页面，不应获得应用能力。
3. 凭证只在 Rust 内存中流转，**不经前端、不进日志、不落盘**。
4. 窗口关闭时（组件卸载）要清理，避免孤儿窗口。

**Tauri 版本要求**：`>= 2.12`（`cookies()` API 的稳定性见官方文档）。
