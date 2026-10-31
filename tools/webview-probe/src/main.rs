//! WebView 会话验证程序。
//!
//! # 要验证的假设
//!
//! 1004 是「路由前的前置门禁」，判的不是请求构造，
//! 而是「这个客户端有没有真实的 web 会话」。
//!
//! 参考项目 c001apk 有一条我们完全没有的路径：
//! 用 WebView 真正加载 www.coolapk.com，让页面 JS 初始化会话、
//! 种下 cookie，之后同一进程的 API 请求才被认可。
//!
//! # 程序流程
//!
//! ```text
//! 1. 启动 Tauri 窗口，加载 https://www.coolapk.com/
//! 2. 等页面加载完（10 秒）
//! 3. 读取 WebView 的全部 cookie
//! 4. 用这些 cookie + 完整协议头打 API
//! 5. 打印结果：1004 还是 200+data
//! ```
//!
//! 结果直接写入 exe 同目录的 probe-result.txt，便于查看。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// 用来统计是否已经跑过一次。
static DONE: AtomicBool = AtomicBool::new(false);

#[derive(serde::Serialize)]
struct ProbeResult {
    /// 从 WebView 读到的 cookies（名 + 值前 40 字符）
    cookies: Vec<(String, String)>,
    /// cookie 总数
    cookie_count: usize,
    /// 直接用 WebView 会话打 API 的结果
    api_status: u16,
    api_body: String,
    /// 是否有 data 字段（表示成功）
    success: bool,
}

fn out_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir)
        .join("probe-result.txt")
}

fn write_result(text: &str) {
    let p = out_path();
    let _ = std::fs::write(&p, text);
    println!("[结果已写入 {}]", p.display());
    println!("{text}");
}

#[tokio::main]
async fn main() {
    println!("=== WebView 会话验证 ===");

    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();

            // 建一个可见窗口加载酷安首页。
            // 必须用 External URL（远程来源），这样才有真实的 web cookie 罐。
            let url: url::Url = "https://www.coolapk.com/".parse().expect("url");
            WebviewWindowBuilder::new(
                &handle,
                "warmup",
                WebviewUrl::External(url),
            )
            .title("WebView 预热中…")
            .inner_size(900.0, 700.0)
            .center()
            .build()
            .expect("创建窗口失败");

            // 后台等待并验证
            tauri::async_runtime::spawn(async move {
                println!("[1] 等待页面加载与 JS 初始化（12 秒）...");
                tokio::time::sleep(std::time::Duration::from_secs(12)).await;

                if DONE.swap(true, Ordering::SeqCst) {
                    return;
                }

                // 读取 WebView cookie（Windows 上必须 spawn_blocking）
                let win = match handle.get_webview_window("warmup") {
                    Some(w) => w,
                    None => {
                        write_result("错误：预热窗口已被关闭");
                        return;
                    }
                };

                println!("[2] 读取 WebView cookies...");
                let raw = tokio::task::spawn_blocking(move || win.cookies()).await;

                let cookies = match raw {
                    Ok(Ok(c)) => c,
                    Ok(Err(e)) => {
                        write_result(&format!("读取 cookie 失败: {e}"));
                        return;
                    }
                    Err(e) => {
                        write_result(&format!("读取任务失败: {e}"));
                        return;
                    }
                };

                let pairs: Vec<(String, String)> = cookies
                    .into_iter()
                    .map(|c| (c.name().to_string(), c.value().to_string()))
                    .collect();

                println!("[3] 读到 {} 个 cookie", pairs.len());
                for (n, v) in &pairs {
                    let short: String = v.chars().take(40).collect();
                    println!("    {n} = {short}");
                }

                // 用这些 cookie 打 API
                println!("[4] 用 WebView cookie 请求 API...");
                let (status, body) = call_api(&pairs).await;

                let success = body.contains("\"data\"") && !body.contains("\"status\":1004");

                let display: Vec<(String, String)> = pairs
                    .iter()
                    .map(|(n, v)| {
                        (n.clone(), v.chars().take(40).collect::<String>())
                    })
                    .collect();

                let result = ProbeResult {
                    cookie_count: pairs.len(),
                    cookies: display,
                    api_status: status,
                    api_body: body.chars().take(400).collect(),
                    success,
                };

                let text = format!(
                    "=== WebView 会话验证结果 ===\n\n\
                     cookie 数：{}\n\
                     {}\n\n\
                     API 状态码：{}\n\
                     API 响应：{}\n\n\
                     结论：{}\n",
                    result.cookie_count,
                    result
                        .cookies
                        .iter()
                        .map(|(n, v)| format!("  {n} = {v}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    result.api_status,
                    result.api_body,
                    if result.success {
                        "✅ 成功！WebView 会话是必需的"
                    } else {
                        "❌ 仍失败，WebView 会话不是原因"
                    }
                );

                write_result(&text);

                // 关闭窗口
                if let Some(w) = handle.get_webview_window("warmup") {
                    let _ = w.close();
                }
                std::process::exit(0);
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动失败");
}

/// 用给定的 cookies 请求酷安 API。
async fn call_api(cookies: &[(String, String)]) -> (u16, String) {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use coolall_core::token_v3;

    // 用真实数盟 ID（之前实测拿到的）
    let szlm = "DUP4awltXU__0ABOiBz7NwkxTrv4hbvIeRgd";
    let plain = format!("{szlm}; ; ; ; Xiaomi; Xiaomi; 2211133C; TKQ1.221114.001; null");
    let device = STANDARD
        .encode(plain.as_bytes())
        .trim_end_matches('=')
        .chars()
        .rev()
        .collect::<String>()
        .replace(['\r', '\n', '='], "");

    let token = token_v3::create(&device, 2_610_291, chrono::Utc::now().timestamp())
        .unwrap_or_default();

    let ua = "Dalvik/2.1.0 (Linux; U; Android 13; 2211133C TKQ1.221114.001) (#Build; Xiaomi; 2211133C; TKQ1.221114.001; 13) +CoolMarket/16.7.0-2610291-universal";

    // 把 WebView cookie 拼成 Cookie 头
    let cookie_header = cookies
        .iter()
        .filter(|(n, _)| n != "token")
        .map(|(n, v)| format!("{n}={v}"))
        .collect::<Vec<_>>()
        .join("; ");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .expect("client");

    let url = "https://api2.coolapk.com/v6/main/indexV8?page=1&firstLaunch=1&installTime=1761926400000";

    let mut req = client
        .get(url)
        .header("User-Agent", ua)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("X-Sdk-Int", "33")
        .header("X-Sdk-Locale", "zh-CN")
        .header("X-App-Id", "com.coolapk.market")
        .header("X-App-Token", token)
        .header("X-App-Version", "16.7.0")
        .header("X-App-Code", "2610291")
        .header("X-Api-Version", "16")
        .header("X-App-Device", device)
        .header("X-Dark-Mode", "0")
        .header("X-App-Channel", "coolapk")
        .header("X-App-Mode", "universal")
        .header("X-App-Supported", "2610291")
        .header("Accept", "application/json");

    if !cookie_header.is_empty() {
        req = req.header("Cookie", cookie_header);
    }

    match req.send().await {
        Ok(r) => {
            let s = r.status().as_u16();
            let b = r.text().await.unwrap_or_default();
            (s, b)
        }
        Err(e) => (0, format!("请求失败: {e}")),
    }
}
