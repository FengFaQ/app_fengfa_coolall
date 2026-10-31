//! 验证码窗口：用应用内嵌 WebView 打开酷安验证页，
//! 用户完成后由应用直接读取 WebView 的 Cookie 获得 `validate`。
//!
//! # 为什么不用外部浏览器
//!
//! 外部浏览器是独立进程，其 Cookie 归浏览器管，应用无法读取。
//! 用户手动复制粘贴既不可靠也不安全（凭证会经过剪贴板）。
//!
//! # 为什么用 `cookies()` 而不是 CDP
//!
//! Tauri 2.12+ 的 `WebviewWindow::cookies()` 是官方 API，
//! 能读到 HttpOnly / Secure cookie，比调用 WebView2 的 CDP 更稳。
//!
//! # Windows 上的死锁陷阱
//!
//! 官方文档明确说明：在**同步**命令或事件处理器里调用 `cookies()` 会死锁
//! （wry#583）。因此本模块的所有对外函数都必须是 `async`，
//! 且读取动作放在 `spawn_blocking` 里。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// 验证窗口的标签。窗口能力配置里按这个标签授权。
pub const CAPTCHA_WINDOW_LABEL: &str = "captcha";

/// 酷安验证页地址。
///
/// 用登录页而不是单独的验证码页：
/// 酷安的人机验证是在登录流程中触发的，登录页会自己弹出易盾。
pub const CAPTCHA_PAGE_URL: &str = "https://account.coolapk.com/auth/login?type=mobile";

/// 读取结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieReadResult {
    /// 从 Cookie 里提取到的 validate（已解码）
    pub validate: Option<String>,
    /// 找到的相关 Cookie 名（便于诊断，不含值）
    pub cookie_names: Vec<String>,
    /// 诊断信息
    pub message: String,
}

/// 从一组 Cookie 里提取 validate。
///
/// 凭证可能以两种形式存在：
/// - 名称为 `validate` 的 Cookie
/// - 名称含 `validate` 的其它形态（兼容服务端命名变化）
///
/// 值可能被 URL 编码，这里还原一次。
pub fn extract_validate(
    cookies: &[(String, String)],
) -> (Option<String>, Vec<String>) {
    let names: Vec<String> = cookies.iter().map(|(n, _)| n.clone()).collect();

    for (name, value) in cookies {
        if name.eq_ignore_ascii_case("validate") || name.to_lowercase().contains("validate") {
            let decoded = urlencoding::decode(value)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| value.clone());
            if !decoded.is_empty() {
                return (Some(decoded), names);
            }
        }
    }
    (None, names)
}

/// 打开验证窗口。若已存在则聚焦。
pub fn open_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(CAPTCHA_WINDOW_LABEL) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    // WebviewUrl::External 需要 url::Url，不是 String
    let url = CAPTCHA_PAGE_URL
        .parse::<url::Url>()
        .map_err(|e| format!("验证页地址无效：{e}"))?;

    WebviewWindowBuilder::new(app, CAPTCHA_WINDOW_LABEL, WebviewUrl::External(url))
        .title("完成人机验证")
        .inner_size(520.0, 720.0)
        .min_inner_size(400.0, 560.0)
        .center()
        .build()
        .map_err(|e| format!("创建验证窗口失败：{e}"))?;

    Ok(())
}

/// 关闭验证窗口。
pub fn close_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(win) = app.get_webview_window(CAPTCHA_WINDOW_LABEL) {
        let _ = win.close();
    }
}

/// 读取验证窗口里的 Cookie 并提取 validate。
///
/// **必须是 async**：Windows 上同步调用 `cookies()` 会死锁（wry#583）。
pub async fn read_validate<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<CookieReadResult, String> {
    let Some(win) = app.get_webview_window(CAPTCHA_WINDOW_LABEL) else {
        return Err("验证窗口未打开".into());
    };

    // 再套一层 spawn_blocking：官方建议用独立线程读取
    let raw = tokio::task::spawn_blocking(move || win.cookies())
        .await
        .map_err(|e| format!("读取任务失败：{e}"))?;

    let cookies = raw.map_err(|e| format!("读取 Cookie 失败：{e}"))?;

    let pairs: Vec<(String, String)> = cookies
        .into_iter()
        .map(|c| (c.name().to_string(), c.value().to_string()))
        .collect();

    let (validate, names) = extract_validate(&pairs);

    let message = if validate.is_some() {
        "已从验证窗口读取到凭证".to_string()
    } else {
        format!(
            "未找到 validate Cookie。当前有 {} 个 Cookie：{}",
            names.len(),
            names.join(", ")
        )
    };

    Ok(CookieReadResult {
        validate,
        cookie_names: names,
        message,
    })
}

/// 一次性标记：防止同一窗口被重复读取造成凭证混乱。
#[derive(Default)]
pub struct CaptchaWindowState {
    pub reading: Arc<AtomicBool>,
}

impl CaptchaWindowState {
    pub fn try_begin(&self) -> bool {
        !self.reading.swap(true, Ordering::SeqCst)
    }

    pub fn end(&self) {
        self.reading.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn finds_plain_validate_cookie() {
        let c = pairs(&[("SESSID", "abc"), ("validate", "xyz123")]);
        let (v, names) = extract_validate(&c);
        assert_eq!(v.as_deref(), Some("xyz123"));
        assert!(names.contains(&"validate".to_string()));
    }

    #[test]
    fn decodes_url_encoded_value() {
        let c = pairs(&[("validate", "a%2Fb%3Dc")]);
        let (v, _) = extract_validate(&c);
        assert_eq!(v.as_deref(), Some("a/b=c"), "应还原 URL 编码");
    }

    #[test]
    fn finds_validate_with_suffix() {
        let c = pairs(&[("validate_token", "tok")]);
        let (v, _) = extract_validate(&c);
        assert_eq!(v.as_deref(), Some("tok"));
    }

    #[test]
    fn returns_none_when_absent() {
        let c = pairs(&[("SESSID", "abc"), ("uid", "42")]);
        let (v, names) = extract_validate(&c);
        assert!(v.is_none());
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn ignores_empty_value() {
        let c = pairs(&[("validate", "")]);
        let (v, _) = extract_validate(&c);
        assert!(v.is_none(), "空值不算有效凭证");
    }

    #[test]
    fn is_case_insensitive() {
        let c = pairs(&[("Validate", "V1")]);
        let (v, _) = extract_validate(&c);
        assert_eq!(v.as_deref(), Some("V1"));
    }
}
