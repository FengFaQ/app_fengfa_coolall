# WebView 会话验证（Edge CDP 方式）

> 目的：验证「1004 是因为缺少真实 web 会话」这个假设。
>
> 不编译 Rust，直接用 Edge 的远程调试协议操作，
> 因为 WebView2 与 Edge 是同一个内核，cookie 行为等价。

## 前提

- Edge 已安装（`C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`）
- Edge WebView2 运行时 155/156 已装

## 步骤

### 1. 启动带远程调试的 Edge

```powershell
$profile = "$env:TEMP\edge-probe-profile"
Remove-Item $profile -Recurse -Force -ErrorAction SilentlyContinue
& "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" `
  --remote-debugging-port=9222 `
  --user-data-dir=$profile `
  --no-first-run `
  --no-default-browser-check `
  "https://www.coolapk.com/"
```

### 2. 等待页面加载（约 10 秒）

### 3. 通过 CDP 读取 cookie

```powershell
$targets = Invoke-RestMethod 'http://127.0.0.1:9222/json'
$page = $targets | Where-Object { $_.type -eq 'page' } | Select-Object -First 1
$ws = $page.webSocketDebuggerUrl
# 用 CDP 的 Network.getAllCookies
```

### 4. 用 cookie 打 API

### 5. 对比结果

## 判定标准

| 结果 | 含义 |
| --- | --- |
| API 返回 `data` | ✅ **WebView 会话是必需的**，必须实现预热流程 |
| API 仍是 1004 | ❌ 会话不是原因，方向要换 |

## 注意事项

- Edge 的用户数据目录必须全新（`--user-data-dir` 指向临时目录），
  否则会带上已有 cookie，污染实验
- 实验结束后要关闭 Edge 并清理临时目录
