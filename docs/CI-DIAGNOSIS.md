# CI 构建诊断

> 本文件按 run 顺序累积。**最新一轮在最前面**，历史轮次在下方【历史】小节。

---

# 轮次 2：run `41205566621`（commit `34fb069`）—— ⚠️ 首次跑通 Windows 打包，但编译失败

> 诊断时间：2026-10-31T21:32+08:00（UTC 13:31）
> 报告性质：只读诊断，未修改任何源码，未执行任何 push。
> **重要**：GitHub 上 HEAD 实际已推进到 `7bf2909`（"架构文档对齐实现现状"），本 run 打的是 `34fb069`。本地工作区另有未提交改动，下列行号均以**本 run 实际编译的 `34fb069`** 为准。

## 运行概况

| 项 | 值 |
| --- | --- |
| run id | `41205566621` |
| commit | `34fb0695bcb7e7bae4e75f9e76d1b63f19802447`（修复编译阻塞与安全漏洞，拆分协议核心为独立 crate） |
| 状态 | `completed` / **failure** |
| 起止 | 2026-10-31T13:26:25Z → 13:31:55Z（约 5 分 30 秒） |
| 并发 | 前端检查 / Rust 测试 并行启动，Windows 打包在两者都绿后才开始 |

## 各 Job 结果

| Job | Job ID | 状态 | 耗时 | 说明 |
| --- | --- | --- | --- | --- |
| 前端检查 | `124549825337` | ✅ success | 13:26:28 → 13:26:45（17s） | 类型检查、构建、上传 `dist` 全部通过 |
| Rust 测试 | `124549825469` | ✅ success | 13:26:28 → 13:26:54（26s） | 拆 crate 后不再需要 WebKitGTK，**修复生效** |
| Windows 打包 | `124549926905` | ❌ **failure** | 13:26:57 → 13:31:55（4m58s） | 失败 step：**构建**（`[7]`）；`列出产物` `[8]`、`上传产物` `[9]` 均 skipped |

**产物**：仅 1 个 artifact —— `dist`（61,273 bytes，前端产物）。**没有 exe，没有 `coolall-windows`**。

## 本轮修复的验证结论

| 上一轮问题 | 本次修复 | 结果 |
| --- | --- | --- |
| 前端无测试文件 → 失败 | 删除测试 step / 新增 `src/api/index.test.ts` | ✅ 前端检查已绿 |
| `std::sync::Mutex` 跨 await（4 个 future not Send） | 改为 `tokio::sync::Mutex` | ✅ **已消除**，本轮无 `Send` 错误 |
| 缺 `use tauri::Manager`（E0599） | 已补（`lib.rs:172` 函数内） | ✅ **已消除** |
| 测试 job 未建 `dist/` → `generate_context!` panic | Rust 测试 job 改只测 `coolall-core` | ✅ **已消除** |
| **E0255 宏重复定义（14 个）** | 拆 crate 但 **`lib.rs` 命令函数未拆分** | ❌ **仍在**（本轮 Windows 打包 14 个） |
| — | — | 🆕 **新暴露 E0308**（`submit_validation` 缺 `.await`） |

> 结论：**不是"修复没生效"，而是"修复不完整"** —— 上一轮我预警过的 E0255 结构性根因一直没修，只是被"Rust 测试 job 不再编译 GUI"掩盖了；Windows 打包一旦真正编译 `src-tauri`，它就原样重现。

## 失败分析：Windows 打包 / 构建 step

- **Job**：`Windows 打包`（`124549926905`）
- **失败 step**：`[7] 构建` → `npm run tauri:build`（`build.yml:91-92`）
- **耗时**：编译约 4 分（13:27:33 开始，13:31:51 失败）
- **退出码**：1

### 前置步骤均正常（排除你提的 4 类猜测中的 3 类）

从日志可确认以下**都不是**本次失败原因：

- `beforeBuildCommand npm run build` → `vite build` 成功（`13:27:33` 执行，产物已上传）
- `Updating crates.io index` → 依赖解析正常，`tauri 2.12.5` 单版本，无 workspace 路径错误
- 已进入 `Compiling coolall v0.1.0`（`13:31:32`），说明 **Rust 工具链、WebView2、NSIS 都还没轮到报错**——失败发生在 **Rust 编译期**，压根没走到打包阶段

> 你猜的 icon.ico / NSIS / WebView2 SDK 问题，**本轮均未被触发**（因为编译就没过）。等编译修好后才会知道这些是否是真问题。

### 原始报错（关键行）

```
error[E0255]: the name `__cmd__browse` is defined multiple times
  --> src-tauri\src\lib.rs:59:14
58 | #[tauri::command]
   | ----------------- previous definition of the macro `__cmd__browse` here
59 | pub async fn browse(
   |              -^^^^^
   |              `__cmd__browse` reimported here
   |              help: remove unnecessary import
   = note: `__cmd__browse` must be defined only once in the macro namespace of this module
```

同型错误覆盖 7 个命令 × 2 个符号 = **14 个 E0255**：

| 命令 | `#[tauri::command]` 行 | fn 行 |
| --- | --- | --- |
| `browse` | 58 | 59 |
| `detail` | 78 | 79 |
| `replies` | 97 | 98 |
| `search` | 117 | 118 |
| `session_info` | 137 | 138 |
| `submit_validation` | 147 | 148 |
| `import_shuzlm_id` | 162 | 163 |

新增 E0308：

```
error[E0308]: mismatched types
   --> src-tauri\src\lib.rs:158:8
158 |     Ok(client.submit_validation(&challenge_id, &field, &validate))
    |     -- ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `bool`, found future
note: calling an async function returns a future
help: consider `await`ing on the `Future`
    |     Ok(client.submit_validation(&challenge_id, &field, &validate).await)
    |                                                                  ++++++
```

收尾：

```
error: could not compile `coolall` (lib) due to 15 previous errors
failed to build app: failed to build app
##[error]Process completed with exit code 1.
```

## 根因判断

### 根因 1（P0，阻断打包）：`src-tauri/src/lib.rs` 的命令函数与 `generate_handler!` 仍在同一模块

我核对了 `34fb069` 的源码，**排除了几个常见误判**：

- ✅ 源码里 `#[tauri::command]` **只有 7 处**，每个 fn 各一次，**没有重复写属性**
- ✅ 没有任何 `use ...::*` glob 导入（`lib.rs:14-17` 只导出 `device/error/request/token_v3/Error/TokenError`）
- ✅ `crates/coolall-core` 里**不存在** `browse/detail/...` 等同名函数（那些在 `client.rs` 的 `client::` 模块内，不污染 `lib.rs` 命名空间）
- ✅ `Cargo.lock` 只有一个 `tauri 2.12.5`，无版本重复

**真正的机制**：`src-tauri/Cargo.toml` 声明了

```toml
[lib]
name = "coolall_lib"
crate-type = ["staticlib", "cdylib", "rlib"]
```

同一份 `src/lib.rs` 被以 **3 种 crate 类型**编译。`#[tauri::command]` 宏把 `__cmd__<fn>` 生成在**模块级宏命名空间**；在多 crate-type 同源编译路径下，宏导入被重复注入到同一个模块，于是宏命名空间里出现两份同名符号 → E0255「reimported here」。

> 不确定性说明：本机未安装 Rust 工具链（`cargo` 不存在），我**无法在本地复现验证**。上述为基于「源码干净 + 报错形态 + crate-type 三合一」的静态推断。E0255 的原文 `help: remove unnecessary import` 指向的正是「同一模块被重复导入」这一形态，而非「用户写了两次属性」。

**修复建议（精确到文件行，按推荐度排序）**

**方案 A（推荐，最小改动、直击病因）：把命令函数移出 crate 根模块**

1. 新建 `src-tauri/src/commands.rs`，把 `lib.rs:57-167` 整段（7 个 `#[tauri::command]` 函数）整体迁入。
2. `src-tauri/src/lib.rs:11` 附近（`pub mod client;` 之后）加一行：
   ```rust
   mod commands;
   ```
3. `lib.rs:195-203` 的 `generate_handler!` 改为带路径引用：
   ```rust
   .invoke_handler(tauri::generate_handler![
       commands::browse,
       commands::detail,
       commands::replies,
       commands::search,
       commands::session_info,
       commands::submit_validation,
       commands::import_shuzlm_id
   ])
   ```
4. `commands.rs` 顶部需要 `use crate::client;`、`use crate::AppState;`、`use crate::InvokeResult;`、`use tauri::State;`，并把 `AppState`/`InvokeResult`（`lib.rs:27-55`）保持 `pub` 不变。

**方案 B（治标、验证成本低）：收敛 crate-type**

`src-tauri/Cargo.toml` 的 `crate-type = ["staticlib", "cdylib", "rlib"]` 改为：
```toml
crate-type = ["lib", "cdylib", "staticlib"]
```
注意：`staticlib`/`cdylib` 是 Tauri 移动端需要的，桌面端主要用 rlib。**若本期只出 Windows 桌面包**，可临时只留 `["rlib"]` 来验证 E0255 是否消失；确认后再逐步恢复。**这会改变产物形态，需 Lead 决策**，不建议作为最终修复。

**方案 C（不推荐）：逐个 `#[tauri::command]` 加 `skip_rename` 或手动改宏名** —— Tauri 2 不支持该参数，此路不通，列出以免有人尝试。

### 根因 2（P0，一行修复）：`submit_validation` 漏了 `.await`

- **文件行**：`src-tauri/src/lib.rs:157`（`34fb069` 版本）
- **现状**：
  ```rust
  Ok(client.submit_validation(&challenge_id, &field, &validate))
  ```
- **应为**：
  ```rust
  Ok(client.submit_validation(&challenge_id, &field, &validate).await)
  ```
- **原因**：这是 Mutex 改造的**遗漏项**。`client.rs:340-345` 在 `34fb069` 中已是 `pub async fn submit_validation(&self, ...) -> bool`（**返回 `bool`，不是 `Result`**），但 `lib.rs:157` 仍按同步函数调用，于是 `Ok(<Future>)` 类型不匹配。
- ⚠️ **不要照抄编译器建议**：rustc 提示的是
  `Ok(client.submit_validation(...).await)` —— 这个是对的；但**必须保留外层 `Ok(...)`**，因为签名是 `-> Result<bool, String>` 而函数返回裸 `bool`。若误删 `Ok` 会得到新的类型错误。

> 本地工作区（未提交）里这行**已经改成 `.await` 了**（见 `lib.rs:153-157`），说明有人已发现并修了。**但 `34fb069` 里没有**，所以本 run 失败。请确认修复已提交。

### 附带发现：一个陈旧且未忽略的 `src-tauri/Cargo.lock`

- `src-tauri/Cargo.lock` **被 git 跟踪**（`git ls-files` 命中），且停在 `f445201` 那一版；
- 它的 `[[package]] name = "coolall"` 依赖列表里**没有 `coolall-core`**，而根 `Cargo.lock:489` 有；
- 两份 lock **内容不同**（root 123,480 字节 / src-tauri 126,772 字节）。

因为 `src-tauri/Cargo.toml` 没有自己的 `[workspace]`，它属于根 workspace，cargo 实际用**根 `Cargo.lock`**，所以本 run 看到 `Updating crates.io index` 正常重解析、并未因此直接报错。**这不是本次失败的根因**，但属于卫生问题：CI 上是全新 clone，若哪天有人单独在 `src-tauri/` 下执行 cargo，可能拿到过期锁。

建议：`git rm --cached src-tauri/Cargo.lock`（并在 `.gitignore` 的 Rust 段加 `src-tauri/Cargo.lock`，与既有的 `crates/*/Cargo.lock` 规则一致）。

## 产物清单

| artifact | 大小 | 来源 job |
| --- | --- | --- |
| `dist` | 61,273 bytes | 前端检查 |
| `coolall-windows` | **无** | Windows 打包失败，未上传 |

**没有任何 exe 产物。** 期望产物（`build.yml:104-107`）：`src-tauri/target/release/bundle/nsis/*.exe`、`bundle/msi/*.msi`、`src-tauri/target/release/coolall.exe`。

## 下一步建议

修好上述 2 个根因（`lib.rs` 命令拆分 + 补 `.await`）后重跑，Windows 打包才**第一次真正进入 NSIS 打包阶段**。届时若仍失败，再按你列的 4 类方向查（尤其 icon.ico 单尺寸 PNG 内嵌是否被 NSIS 接受），我会继续盯。

---
---

# 【历史】轮次 1：run `41204567050`（commit `f445201`）—— 失败

> 监控对象：GitHub Actions run `41204567050`　仓库 `FengFaQ/app_fengfa_coolall`
> 诊断时间：2026-10-31T21:13+08:00（UTC 13:13）
> 报告性质：只读诊断，未修改任何源码，未执行任何 push。

## 运行概况

| 项 | 值 |
| --- | --- |
| run id | `41204567050` |
| workflow | 构建（`.github/workflows/build.yml`） |
| 触发 | push → `main` |
| commit | `f4452019c64009df650dd7187338821a6ee9bec2`（阶段 1：协议层与界面骨架） |
| 尝试次数 | 1 |
| 状态 | `completed` |
| **结论** | **failure（失败）** |
| 开始 | 2026-10-31T13:13:08Z |
| 结束 | 2026-10-31T13:15:37Z |
| 总耗时 | 约 2 分 29 秒 |

## 各 Job 结果

| Job | Job ID | Runner | 状态 | 耗时 | 失败 step |
| --- | --- | --- | --- | --- | --- |
| 检查 | `124546913052` | ubuntu-26.04 | ❌ failure | 13:13:10 → 13:13:20（10s） | 前端单元测试 |
| Rust 测试 | `124546913198` | ubuntu-26.04 | ❌ failure | 13:13:22 → 13:15:37（2m15s） | 运行单元测试 |
| Windows 打包 | `124546961373` | —（windows-latest） | ⏭ skipped | —（未启动） | 因 `needs: check` 失败被跳过 |

跳过的连锁效应：`build-windows` 依赖 `check`（`build.yml:66`），`check` 失败导致 Windows 打包整体不执行，因此**本轮没有任何安装包产物**。

## 失败分析

### 失败 1：检查 / 前端单元测试 —— 仓库里一个测试文件都没有

- **Job**：`检查`（`124546913052`）
- **失败 step**：`[6] 前端单元测试`（`build.yml:33-34`，命令 `npm test -- --run`）
- **耗时**：1 秒（13:13:18 → 13:13:19）

**原始报错（日志原文）**

```
> coolall@0.1.0 test
> vitest run --run

 RUN  v2.1.9 /home/runner/work/app_fengfa_coolall/app_fengfa_coolall

include: src/**/*.{test,spec}.ts
exclude:  **/node_modules/**, **/dist/**, ...

No test files found, exiting with code 1
##[error]Process completed with exit code 1.
```

同一 job 中前置步骤均通过，失败与类型检查/依赖安装无关：
`类型检查`（`vue-tsc --noEmit`）exit 0，`安装前端依赖`（`npm ci`）added 170 packages 正常。

**根因**

`vitest.config.ts:15` 的 include 规则是 `src/**/*.{test,spec}.ts`，而 `src/` 下当前**不存在任何 `*.test.ts` / `*.spec.ts` 文件**（本地实测 count = 0，目录仅有 `App.vue`、`main.ts`、`api/`、`components/`、`pages/`、`router/`、`styles/`）。Vitest 在 `run` 模式下找不到匹配文件时默认以退出码 1 结束，直接判失败。

这是「阶段 1 只搭骨架、尚未补测试」与「CI 把测试步骤设为硬性门禁」之间的错配，不是代码逻辑错误。

**修复建议（精确到文件行）**

方案 A（推荐，保留测试门禁，尽快补齐真实测试）：新增 `src/` 下的测试文件，例如
- 新建 `src/api/__tests__` 或将测试与被测模块同目录放置，文件名须匹配 `*.test.ts`（如 `src/api/index.spec.ts`），
- `@vue/test-utils`、`jsdom` 已在 `package.json:38-39` devDependencies 中就位，`vitest.config.ts:13` 已设 `environment: 'jsdom'`，无需改配置；
- 至少覆盖一个真实模块，使 `npm test` 有可执行的用例集合。

方案 B（临时解堵，允许空测试集通过）：改 `vitest.config.ts:12-16` 的 `test` 块，增加
`passWithNoTests: true`：

```ts
  test: {
    environment: 'jsdom',
    globals: true,
    include: ['src/**/*.{test,spec}.ts'],
    passWithNoTests: true,   // ← 新增
  },
```

但方案 B 会让「测试步骤永远绿」，掩盖真实回归，建议只作为过渡。

方案 C（不改配置、改流程）：把 `package.json:17` 的脚本改为 `vitest run --passWithNoTests`。
同样属于放水，优先级低于方案 A。

> 提醒：`package.json:14` 的 `build` 是 `vite build`；`build.yml:34` 里 `npm test -- --run` 实际执行成 `vitest run --run`（重复参数，无害但冗余），修正时可顺手改为 `npm test`。

---

### 失败 2：Rust 测试 / 运行单元测试 —— `src-tauri/src/lib.rs` 有 20 个编译错误

- **Job**：`Rust 测试`（`124546913198`）
- **失败 step**：`[6] 运行单元测试`（`build.yml:60-61`，命令 `cargo test --manifest-path src-tauri/Cargo.toml --lib --release`）
- **耗时**：1 分 33 秒（编译 13:14:01 → 13:15:34）
- **暴露点**：这一段是**编译期**失败，测试根本没跑起来。注意 `cargo test --lib` 会额外编译带 `#[cfg(test)]` 的 lib test target，因此错误列表比 `tauri dev` 更严格。

**原始报错（日志原文，共 20 个 error / 2 个 warning，exit code 101）**

汇总：

```
error: proc macro panicked
error[E0255] × 14  （7 个命令 × 2 个符号）
error[E0599] × 1
error: future cannot be sent between threads safely × 4
error: could not compile `coolall` (lib test) due to 20 previous errors; 2 warnings emitted
##[error]Process completed with exit code 101.
```

六个根因按日志出现顺序逐条展开：

#### 2-A　`frontendDist` 指向不存在的 `../dist`（错误 #1）

```
error: proc macro panicked
   --> src/lib.rs:191:14
191 |         .run(tauri::generate_context!())
    |              ^^^^^^^^^^^^^^^^^^^^^^^^^^
    = help: message: The `frontendDist` configuration is set to `"../dist"` but this path doesn't exist
```

- **文件行**：`src-tauri/tauri.conf.json:10`（`"frontendDist": "../dist"`），触发点 `src-tauri/src/lib.rs:191`。
- **根因**：`generate_context!()` 在编译期校验前端产物目录。`Rust 测试` job 只做了 checkout + 装系统依赖 + 装 Rust，**没有 `npm ci`、也没有 `npm run build`**（对比 `build-windows` 在 `build.yml:85-89` 是有这两步的），所以 `dist/` 不存在。
- **为什么本地/Windows 打包不暴露**：本地 `dist/` 已存在；`build-windows` 先 `npm run build`（`tauri.conf.json:9` 的 `beforeBuildCommand` 也会兜底生成）后才编译。
- **修复建议**：
  - 方案 A（改 workflow，推荐）：在 `build.yml` 的 `Rust 测试` job 中，`缓存`（第 56 行）之后、`运行单元测试`（第 60 行）之前插入两个 step：
    ```yaml
      - name: 安装前端依赖
        run: npm ci
      - name: 前端构建
        run: npm run build
    ```
    但这会给 Rust 测试 job 增加 Node 环境，代价偏高。
  - 方案 B（改测试目标，更贴合“纯协议层测试”）：`build.yml:61` 改为让 cargo 只编译库本体而不触发 `generate_context!` 的宿主校验，例如把 `--lib` 换成对纯逻辑模块的集成测试 `cargo test --manifest-path src-tauri/Cargo.toml --release --test protocol`（需新建 `src-tauri/tests/protocol.rs`）；或至少补一个空的 `dist/`：
    ```yaml
      - name: 占位前端产物
        run: mkdir -p dist && echo '<!doctype html>' > dist/index.html
    ```
  - 方案 C（改源码，不推荐）：把 `src-tauri/tauri.conf.json:10` 的 `frontendDist` 改成绝对稳定路径或去掉该校验 —— 不可行，Tauri 2 要求该字段。**不要动这一行**。
  - 最省事且语义正确：**方案 B 的「占位前端产物」**，一行 step 且不引入 Node。

#### 2-B　7 个 Tauri 命令重复定义（E0255 × 14）

```
error[E0255]: the name `__cmd__browse` is defined multiple times
  --> src/lib.rs:56:14        (重复：browse / detail / replies / search / session_info / submit_validation / import_shuzlm_id)
      help: remove unnecessary import
```

涉及行号（每条报错的 `previous definition` / `reimported here` 位置）：

| 命令 | 行 |
| --- | --- |
| `browse` | `src/lib.rs:56` |
| `detail` | `src/lib.rs:76` |
| `replies` | `src/lib.rs:95` |
| `search` | `src/lib.rs:115` |
| `session_info` | `src/lib.rs:135` |
| `submit_validation` | `src/lib.rs:145` |
| `import_shuzlm_id` | `src/lib.rs:160` |

- **根因**：`#[tauri::command]` 宏会在宏命名空间里生成 `__cmd__<fn>` 与 `__tauri_command_name_<fn>` 两个符号。E0255 明确说「previous definition here」——同一模块里这些符号被定义了两次。编译 unit 是 `lib test` target，`src/lib.rs` 同时作为普通 lib 与 test 的根模块编译；结合 `#[tauri::command]` 在这些函数上只出现一次（每处 1 行，见上表），这说明 **`generate_handler!`（`src/lib.rs:182-190`）与这些 `#[tauri::command]` 属性在宏展开层面发生了重复导入**。
  日志里的 `help: remove unnecessary import` 指向同一根因：宏把 `__cmd__*` 当作名字**再次导入**到了当前模块。
- **判断依据与不确定性**：行号表与「每个函数带一次 `#[tauri::command]`」的事实一致，但 `#[tauri::command]` 未重复写死。因此这是**宏/模块结构问题**，最可能的落地原因是：`lib.rs` 把命令函数定义与 `generate_context!()` / `generate_handler!` 放在同一个模块里，且 lib test target 二次展开。
  建议修复时以「先消除该 target 的重复展开」为首选。
- **修复建议**：
  1. 首选：按 2-A 方案 B 调整测试 target，使 CI 不再对 `lib.rs` 做 lib-test 展开，E0255 会随之消失（这是同一批错误的连带项）。
  2. 若坚持在 lib test 下编译：把命令函数拆到独立子模块，例如新建 `src-tauri/src/commands.rs` 存放 `browse/detail/...`（对应 `lib.rs:54-166` 整段迁移），`lib.rs` 里改为 `mod commands;` 并 `use commands::*;`，让宏展开只发生一次。
  3. 顺手修 warning：`src-tauri/src/client.rs:13` 的 `use crate::device::{self, constants, DeviceIdentity, DeviceProfile};` 中 `self` 未被使用，删掉 `self,` 即消除 `warning: unused import: self`（不影响成败，但 2 个 warning 之一就是它）。

#### 2-C　`app.manage` 缺少 `tauri::Manager` trait 导入（E0599）

```
error[E0599]: no method named `manage` found for mutable reference `&mut tauri::App` in the current scope
   --> src/lib.rs:177:17
177 |             app.manage(AppState {
    |             ----^^^^^^
help: trait `Manager` which provides `manage` is implemented but not in scope; perhaps you want to import it
  11 + use tauri::Manager;
```

- **文件行**：`src-tauri/src/lib.rs:177`；根因在导入区 `src-tauri/src/lib.rs:20-21`（`use std::sync::Mutex;` / `use tauri::State;`，缺 `Manager`）。
- **根因**：`manage()` 由 `tauri::Manager` trait 提供，未导入 trait 时方法不可见。编译器给出的修复位置是 `lib.rs:11` 起（即第 20-21 行导入块）。
- **修复建议**：把 `src-tauri/src/lib.rs:21` 的
  ```rust
  use tauri::State;
  ```
  改为
  ```rust
  use tauri::{Manager, State};
  ```
  这是**低风险、确定有效**的一行修复。（注：`client.rs:71` 已有 `use tauri::Manager;`，但那是 `client.rs` 模块内作用域，不覆盖 `lib.rs`。）

#### 2-D　4 个 async 命令的 future 不是 `Send`（`std::sync::MutexGuard` 跨 await）

```
error: future cannot be sent between threads safely
   --> src/lib.rs:55:1  (browse)
   = help: within `impl Future<...>`, the trait `std::marker::Send` is not implemented for `std::sync::MutexGuard<'_, CoolapkClient>`
note: future is not `Send` as this value is used across an await
   --> src/lib.rs:66:38
 62 |         let mut client = state
    |             ---------- has type `std::sync::MutexGuard<'_, CoolapkClient>` which is not `Send`
 66 |         client.browse(&target, page).await
    |                                      ^^^^^ await occurs here, with `mut client` maybe used later
```

同型错误共 4 处，对应 4 个 `async fn`：

| 命令 | 属性行 | 守卫声明行 | `.await` 行 |
| --- | --- | --- | --- |
| `browse` | `lib.rs:55` | `lib.rs:62` | `lib.rs:66` |
| `detail` | `lib.rs:75` | `lib.rs:81` | `lib.rs:85` |
| `replies` | `lib.rs:94` | `lib.rs:101` | `lib.rs:105` |
| `search` | `lib.rs:114` | `lib.rs:121` | `lib.rs:125` |

- **根因（本报告最重要的结构性问题）**：项目里有**两种 Mutex 被混用**：
  - `src-tauri/src/lib.rs:20` → `use std::sync::Mutex;`，`AppState.client`（`lib.rs:25`）是**同步锁**；
  - `src-tauri/src/client.rs:11` → `use tokio::sync::Mutex;`，`CoolapkClient.session`（`client.rs:62`）是**异步锁**，调用处 `client.rs:123`、`client.rs:159` 用 `.lock().await`。
  `lib.rs` 的 4 个 async 命令持有 `std::sync::MutexGuard` **跨过了 `.await`**（例如 `lib.rs:62` 取得守卫后，`lib.rs:66` 才 await）。`std::sync::MutexGuard` 不实现 `Send`，而 Tauri 的 `#[tauri::command]` 生成的 future 要求 `Send`（见报错引用的 `tauri-2.12.5/src/ipc/command.rs:326`），故报错。
- **修复建议（三选一，推荐 3）**：
  1. **最小改动、不改类型**：把锁的作用域收窄到只包住「不可 await 的同步部分」，先拿数据、释放守卫，再 await。但 `client.browse()` 本身是 async 且需要 `&mut self`，此路不通。
  2. **改锁类型**（与 `client.rs` 风格统一，方向正确但改动面大）：`lib.rs:20` 改为 `use tokio::sync::Mutex;`，同时把 `lib.rs:62-65 / 81-84 / 101-104 / 121-124` 的 `.lock()` 全部改成 `.lock().await`，`lib.rs:136-139 / 151-154 / 161-164` 三个**同步**命令则需改为 `async fn` 或改用 `blocking_lock()`。注意 `tokio` 已在 `src-tauri/Cargo.toml:36`（`features = ["full"]`）就位，无需加依赖。
  3. **推荐：把 `CoolapkClient` 包成 `Arc<tokio::sync::Mutex<_>>` 并把锁交还给 `client` 内部**，命令函数不再自己 `lock`，改为调用 `CoolapkClient` 新增的 async 方法（内部自行 `lock().await`）。这样：
     - 消除 4 个 `Send` 错误（守卫不再跨 `#[tauri::command]` 的 future 边界）；
     - 与 `client.rs:3-5` 注释里「接口只有 5 个方法、调用者不需要知道内部细节」的设计意图一致；
     - `AppState`（`lib.rs:24-26`）可退化为 `Clone` 的 `Arc` 包装，`State<'_, AppState>` 的用法不变。
  无论哪种，都必须同时处理 `lib.rs:136-139`（`session_info`，同步）、`lib.rs:151-154`（`submit_validation`）、`lib.rs:161-164`（`import_shuzlm_id`）三个同步命令的加锁方式，否则改锁类型后会出现新的 `blocking_lock`/await 组合问题。

#### 2-E　2 个 warning（不致命，建议顺手清理）

```
warning: unused import: `self`
  --> src/client.rs:13:21
13 | use crate::device::{self, constants, DeviceIdentity, DeviceProfile};
warning: unused_mut （提示 has `mut` 但未被修改，位于同类 `.lock()` 调用处）
```

- `src-tauri/src/client.rs:13` → 删除 `self,`。
- `unused_mut`：`lib.rs` 中部分 `let mut client = state.client.lock()...`（如 `lib.rs:151`、`lib.rs:161`）若该 client 未被可变使用，可去掉 `mut`。

---

## 产物清单

**无产物。** 本轮 run 结论为 failure：

- `build-windows` job（`124546961373`）因 `needs: check`（`build.yml:66`）为 `skipped`，**未上传任何 artifact**；
- 期望产物名为 `coolall-windows`（`build.yml:96`），内容应为 `src-tauri/target/release/bundle/nsis/*.exe` 与 `src-tauri/target/release/coolall.exe`（`build.yml:97-99`），保留 14 天（`build.yml:101`）。

修复上述 2 个 job 后重跑，才会产生该 artifact。

---

## 修复优先级建议（供 Lead 派发）

| 优先级 | 改动 | 文件行 | 风险 | 预估影响 |
| --- | --- | --- | --- | --- |
| P0 | 前端测试缺失：补测试 或 加 `passWithNoTests` | `vitest.config.ts:12-16` / 新增 `src/**/*.test.ts` | 低 | 解开 `检查` → 连带解开 `Windows 打包` |
| P0 | 补 `use tauri::Manager` | `src/lib.rs:21` | 极低 | 消 1 个编译错误 |
| P0 | Rust 测试 job 补占位 `dist/` | `build.yml:56` 后插入 | 低 | 消 `proc macro panicked` |
| P1 | `std::sync::Mutex` → `tokio::sync::Mutex` 或重构锁归属 | `src/lib.rs:20,25,62,81,101,121`（+ 3 个同步命令 `136,151,161`） | 中 | 消 4 个 `Send` 错误，核心结构问题 |
| P1 | E0255 重复导入：拆分子模块 / 调整测试 target | `src/lib.rs:54-166`、`182-190` | 中 | 消 14 个 E0255 |
| P2 | 清理 warnings | `src/client.rs:13`、`src/lib.rs:151/161` 的 `mut` | 极低 | 消除 2 个 warning |
| P2 | `npm test -- --run` 冗余参数 | `build.yml:34` | 极低 | 可读性 |

> 说明：P0 三项若全部落地，`检查` 与 `Rust 测试` 中的**非结构性**错误即被清除；P1 两项是 Rust 侧真正的结构问题，必须处理 `Rust 测试` 才能通过。`Windows 打包` 只要 `check` 绿了就会跑起来，届时才会暴露（或顺利产出）NSIS 安装包。

---

## 附：诊断期间工作区被并行修改的观察（非本次诊断产生）

> 本节记录监控期间观察到的**其他成员的并行改动**。本诊断 agent 未修改任何源码。

监控结束时，`git status` 显示工作区有两处未提交改动，改动者非本 agent：

- `.github/workflows/build.yml`（重写）—— 已把 `check` 拆为 `check-frontend`，**删除了「前端单元测试」step**（对症失败 1），`test-rust` 增加占位 `dist/` 目录 step（`build.yml:60-61`，对症失败 2-A），`cargo test` 去掉 `--release`，`build-windows.needs` 改为 `[check-frontend, test-rust]`，产物收集改为 `if-no-files-found: error`。
- `src-tauri/src/lib.rs:170-171` —— 新增函数内 `use tauri::Manager;`（对症失败 2-C）。

**截至监控结束，这几项仍未处理，重跑预计仍会失败：**

1. **失败 2-B（14 个 E0255）未修复**：`src-tauri/src/lib.rs:20` 仍是 `use std::sync::Mutex;`，命令函数与 `generate_handler!` 结构（`lib.rs:182-190`）未拆分。测试命令仍带 `--lib`（`build.yml:64`），lib test target 依然会重复展开 `#[tauri::command]` 宏。
2. **失败 2-D（4 个 future not Send）未修复**：`std::sync::MutexGuard` 依旧跨 `.await`（`lib.rs:62/66`、`81/85`、`101/105`、`121/125`）。
3. `src/client.rs:13` 的 `unused import: self` warning 仍在。
4. **推断而非实测（重要）**：`lib.rs:170-171` 把 `use tauri::Manager;` 放在了 `run()` **函数体内部**（第 170 行注释、171 行导入，位于第 169 行 `pub fn run() {` 之后）。该写法对 `app.manage(...)`（`lib.rs:177`）有效，能消除 E0599；但若后续拆分命令模块，**放在模块顶层（第 21 行 `use tauri::State;` 处合并为 `use tauri::{Manager, State};`）更稳妥**。本机未安装 Rust 工具链（`cargo` 不存在），**未做编译验证**，此项为静态阅读推断。

**结论：两份改动方向正确，但只覆盖了「表面症状」，Rust 侧两个结构性根因（2-B / 2-D）仍在，建议 Lead 在重跑前先修这两项，否则 `Rust 测试` 仍会失败，`build-windows` 仍被 `needs` 挡住、依旧无产物。**
