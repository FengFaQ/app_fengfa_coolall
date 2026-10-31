<div align="center">

# coolall

基于 Tauri 2 与 Vue 3 的非官方酷安客户端

[![构建](https://github.com/FengFaQ/app_fengfa_coolall/actions/workflows/build.yml/badge.svg)](https://github.com/FengFaQ/app_fengfa_coolall/actions/workflows/build.yml)
[![许可证](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

</div>

> **非官方项目**。与酷安官方及深圳酷安网络科技有限公司无隶属、授权或合作关系。

---

## 当前状态

**阶段 1（进行中）**：协议层已完成并通过测试，正在打通端到端链路。

| 模块 | 状态 |
| --- | --- |
| TokenV3 签名算法 | ✅ 10 项测试通过，与 Python 参考实现逐字节一致 |
| 设备身份编码 | ✅ 8 项测试通过，9 段明文可逆 |
| 请求头拼装与会话 | ✅ 10 项测试通过 |
| 错误分类与验证码识别 | ✅ 13 项测试通过 |
| 前端界面 | ✅ 首页 / 搜索 / 详情 / 设置，构建通过 |
| Windows 打包 | ⏳ 待 CI 验证 |

## 功能规划

- **阶段 1**：首页、搜索、动态详情、评论
- **阶段 2**：用户主页、话题、应用、数码、文章
- **阶段 3**：消息、私信、发帖、收藏、历史
- **阶段 4**：剩余页面与桌面端专属能力

## 技术栈

| 层 | 技术 |
| --- | --- |
| 界面 | Vue 3 + TypeScript + Pinia + Vue Router |
| 外壳 | Tauri 2 |
| 核心 | Rust（签名、请求、会话、错误处理） |
| 构建 | Vite + GitHub Actions |

## 文档

| 文档 | 内容 |
| --- | --- |
| [协议说明](docs/PROTOCOL.md) | 签名算法、设备码、请求头、会话、验证码、实测记录 |
| [架构设计](docs/ARCHITECTURE.md) | 深模块划分、seam 清单、测试策略 |
| [第三方声明](NOTICE.md) | 商标、许可证、参考项目与使用方式 |

## 开发

需要 Node.js 22+ 与 Rust 稳定版。

```sh
npm install
npm run tauri:dev      # 开发模式
npm run tauri:build    # 打包（需对应平台工具链）
```

### 测试

```sh
npm run typecheck                              # 前端类型检查
npm test                                       # 前端单元测试
cargo test --manifest-path src-tauri/Cargo.toml --lib   # Rust 单元测试
python3 tools/verify_token_v3.py               # 签名算法交叉校验
```

### 构建产物

推送到 `main` 后由 GitHub Actions 自动构建，产物在 Actions 页面的 Artifacts 中下载。

## 协议说明

酷安接口未公开，且服务端有多层防护。本项目的实现依据见
[协议说明](docs/PROTOCOL.md)，其中记录了：

- TokenV3 签名的完整算法（含两条盐合法性规则）
- `X-App-Device` 的 9 段编码格式
- 请求头完整性要求（缺任一项会 403）
- 协议版本约束（仅 16.x 可过 WAF）
- 游客态 1004 验证要求与公开页兜底

## 许可证

[AGPL-3.0](LICENSE)。本项目在协议实现上参考了同为 AGPL-3.0 的
[KiritoXDone/c001apk](https://github.com/KiritoXDone/c001apk)，
保持相同许可证是最稳妥的合规选择。

## 免责

- 本项目面向未公开接口，可能随服务端调整随时失效。
- 不授予绕过访问控制或平台规则的任何权利。
- 使用者自行承担账号风险，并应遵守酷安用户协议。
