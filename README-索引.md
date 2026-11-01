# coolall 项目索引

> 这是**接手入口**。先读本文件，再按需深入 `00-记忆与交接/`。
>
> **项目根目录**：`C:\Users\Administrator\Documents\dsh\Kuan`
> （下文中「项目根」即指此目录）

---

## 一句话说明

基于 Tauri 2 + Vue 3 + Rust 的**非官方酷安客户端**，目标全端通用（Windows / Android / Linux）。
许可证 **AGPL-3.0**。

---

## 🎉 关键里程碑（2026-11-01）

**首次拿到真实首页数据，1004 已解决。**

```
[真机 + indexV8] HTTP 200 OK -> ✅ 成功！ 长度 221718
[真机 + indexV8] HTTP 200 OK -> ✅ 成功！ 长度 212200   （二次复现）
```

**根因**：TokenV3 密码输入的 Base64 被我错误地去掉 padding。
仓库 1 里 `password` 用 `encoder.encodeToString`（保留 `=`），
只有 `source` 串才用 `withoutPadding`。

**提交**：`336638d 修复根因：TokenV3 密码输入的 Base64 必须保留 padding`

详见 [`00-记忆与交接/08-代码现状与验证结果.md`](00-记忆与交接/08-代码现状与验证结果.md)。

---

## 当前状态（2026-11-01 10:20）

| 项 | 状态 |
| --- | --- |
| TokenV3 签名 | ✅ **已修正 padding，实测通过** |
| 首页接口 | ✅ **返回 212~222 KB 真实数据** |
| 协议层测试 | ✅ 53 项通过 |
| 前端 | ✅ 类型检查 / 24 项测试通过（2026-11-01 复测） |
| Windows 打包 | ✅ CI 连续成功（产物 6.19 MiB） |
| 客户端显示首页 | ⏳ 待重新构建验证 |
| Android / Linux | ⏸ 未开始 |
| UI 改造（coolwin + WinUI 3） | ⏸ 用户要求跑通后进行 |

**最近提交**：`336638d`（根因修复）

---

## 目录结构

```
Kuan/                              ← 项目根
├─ README.md                       ← 对外的项目说明
├─ LICENSE                         ← AGPL-3.0 全文
├─ NOTICE.md                       ← 第三方声明（含图标商标说明）
├─ Cargo.toml                      ← workspace 根（release profile 在此）
│
├─ 00-记忆与交接/                  ← ⭐ 记忆与交接（接手先读这里）
│   ├─ 01-项目全景.md              这是什么、做到哪了、为什么这么做
│   ├─ 02-技术档案.md              协议细节、踩坑、关键结论
│   ├─ 03-环境与凭据.md            工具链、SSH、Token、网络事实
│   ├─ 04-交接-下一任必读.md        接手行动清单
│   ├─ 05-构建与 CI 实录.md         CI 失败与修复的完整记录
│   ├─ 06-完整对话与决策归档.md     ⭐ 与用户的全部交流、决策变更
│   ├─ 07-1004攻坚全记录.md         ⭐ 14 个排除方向与根因
│   ├─ 08-代码现状与验证结果.md     ⭐ 成功请求参数与响应
│   └─ 历史handoff/                按轮次归档的交接快照
│
├─ crates/
│   └─ coolall-core/               ← 协议核心，不依赖 GUI
│       ├─ src/token_v3.rs         请求签名（已修正 padding）
│       ├─ src/device.rs           设备身份编码与 UA
│       ├─ src/request.rs          请求头拼装与会话
│       ├─ src/error.rs            错误分类与验证码识别
│       ├─ src/captcha.rs          验证码状态机
│       └─ src/shuzilm.rs          数盟设备 ID 获取
│
├─ src-tauri/                      ← 桌面外壳
│   ├─ src/lib.rs                  应用入口、状态定义
│   ├─ src/commands.rs             Tauri 命令
│   ├─ src/client.rs               核心客户端（HTTP/会话/重放）
│   ├─ src/transport.rs            HTTP seam
│   ├─ src/mock_transport.rs       测试用 transport
│   ├─ tauri.conf.json             应用配置
│   ├─ icons/                      图标（酷安官方 Logo）
│   └─ assets/                     （已移走，见 coolall-core）
│
├─ src/                            ← 前端
│   ├─ api/                        与 Rust 通信的唯一封装层
│   ├─ pages/                      首页/搜索/详情/设置
│   ├─ components/                 FeedItem / ErrorBox / CaptchaBox / LoadingList
│   ├─ utils/sanitize.ts           公开页 HTML 清洗
│   └─ router/
│
├─ docs/                           ← 技术文档
│   ├─ PROTOCOL.md                 ⭐ 协议说明（唯一事实来源）
│   ├─ ARCHITECTURE.md             架构设计
│   ├─ CI-DIAGNOSIS.md             CI 诊断记录
│   ├─ CODE-REVIEW.md              协议层审查（含一个已证伪的误报）
│   ├─ FRONTEND-REVIEW.md          前端审查
│   └─ STATUS-SYNC.md              与监督者的同步记录
│
├─ tools/
│   ├─ verify_token_v3.py          签名算法独立实现，用于交叉校验
│   └─ rs-probe/                   临时验证 crate（gitignore）
│
└─ _upstream/                      ← 参考仓库（gitignore，不入库）
    ├─ c001apk/                    AGPL-3.0，协议权威参考
    └─ coolapk-desktop/            MIT，Tauri 实践参考 + 图标来源
```

---

## 三条最重要的技术结论

### 1. 请求 403 的根因是指纹不完整，不是 IP 被封

UA 必须三段式，含 `(#Build; ...)` 与 `+CoolMarket/版本-版本码-mode` 尾巴；
设备码必须是 9 段 Base64 反转，不能是随机 hex。

### 2. TokenV3 的盐有两条合法性规则

- 末位不能是 `+`
- **末尾 4 位残余必须为 0**（22 字符 × 6 位 = 132 位，16 字节只用 128 位）

上游只靠捕获异常遮掩第二条，本项目显式检查。

### 3. 游客态必然 1004，是硬约束

换设备 ID、换 SDK、换接口、回写 Cookie、多轮重试，全部稳定 1004。
这是服务端强制人机验证，**必须在真实 WebView 完成**。
公开页 `www.coolapk.com/feed/{id}` 是无鉴权兜底。

---

## 团队分工（多智能体协作）

| 角色 | 职责 | 产出 |
| --- | --- | --- |
| Lead（主智能体） | 统筹、编码、推送、决策 | 全部源码与文档 |
| `ci-watcher` | **独占** CI 监控与诊断权 | `docs/CI-DIAGNOSIS.md` |
| `protocol-reviewer` | 协议层审查 | `docs/CODE-REVIEW.md` |
| `frontend-reviewer` | 前端与通信契约审查 | `docs/FRONTEND-REVIEW.md` |
| 监督者（`oxaapi/kimi-k3.1-pro`） | 架构裁决、风险预判 | `docs/STATUS-SYNC.md` 与轮次报告 |

**协作约定**：

- 长时间等待与监控一律交给 `ci-watcher`，Lead 不重复轮询
- 监督者后台运行，Lead 不等它，继续推进
- 每轮对话结束必须向监督者同步状态
- 各智能体写入不同文件，避免冲突

---

## 快速上手

```powershell
cd C:\Users\Administrator\Documents\dsh\Kuan

npm install                    # 前端依赖
npm run tauri:dev              # 开发模式（需本机有 Rust + MSVC）
npm run tauri:build            # 打包

cargo test -p coolall-core     # 协议核心测试（47 项，无需 GUI 依赖）
cd src-tauri; cargo test --lib # 外壳测试（14 项，含 5 组链路集成测试）

npm run typecheck              # 前端类型检查
npm test                       # 前端测试（33 项）
python tools/verify_token_v3.py # 签名算法交叉校验
```

构建产物由 GitHub Actions 产出，在本机无法直接打 Windows 包
（缺 Rust + MSVC + Android SDK）。

---

## 下一步

见 `00-记忆与交接/04-交接-下一任必读.md`。
