//! 验证码窗口：自建本地页面加载网易易盾 SDK，通过 Tauri IPC 回传 validate。
//!
//! # 真实机制（从参考项目 c001apk 的 CaptchaActivity.kt 核实）
//!
//! 关键发现：**不是打开酷安的验证页，而是自己构造一个 HTML 页面**。
//!
//! ```
//! 1. 本地页面引入 https://cstaticdun.126.net/load.min.js
//! 2. 调 initNECaptcha({ captchaId, onVerify })
//! 3. onVerify 回调里拿到 data.validate
//! 4. 通过 JS 桥回传给原生
//! ```
//!
//! 之前我打开 `account.coolapk.com/auth/login` 是错的——
//! 那个页面不会把 validate 回传给应用。
//!
//! # 为什么用 IPC 而不是读 Cookie
//!
//! 易盾的 validate 是 JS 回调里的值，不一定会写进 Cookie。
//! 参考项目用的是 `WebMessageListener`，Tauri 的等价物是
//! `window.__TAURI_INTERNALS__.invoke`。
//!
//! # 安全约束
//!
//! 验证页加载的是**远程脚本**，必须限制它能做什么：
//! - 窗口不给任何额外 IPC 权限（见 capabilities/captcha.json）
//! - 页面禁用文件访问、禁止弹出新窗口
//! - 只接受 `captcha_result` 这一个命令

use serde::{Deserialize, Serialize};
// get_webview_window 来自 Manager trait，必须引入
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// 验证窗口标签。
pub const CAPTCHA_WINDOW_LABEL: &str = "captcha";

/// 易盾脚本地址。
pub const NECAPTCHA_SDK_URL: &str = "https://cstaticdun.126.net/load.min.js";

/// 验证结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptchaResult {
    pub validate: Option<String>,
    pub message: String,
}

/// 校验 validate 格式。
///
/// 规则来自 PROTOCOL.md §9.3：
/// 非空、长度 ≤ 8192、不含空白与控制字符。
pub fn valid_validate(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8192
        && !value.chars().any(|c| c.is_whitespace() || (c as u32) < 32)
}

/// 构造验证页面的 HTML。
///
/// `captcha_id` 来自挑战本身（`CaptchaChallenge.id`），
/// 不是硬编码的——每个挑战有自己的 id。
pub fn build_captcha_html(captcha_id: &str) -> String {
    // 防止注入：id 只允许字母数字
    let safe_id: String = captcha_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>
  :root {{ color-scheme: light dark; }}
  body {{
    font: 16px/1.6 -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif;
    margin: 24px; background: Canvas; color: CanvasText;
    display: flex; flex-direction: column; align-items: center;
  }}
  #status {{ color: GrayText; margin-bottom: 16px; text-align: center; }}
  #captcha {{ display: flex; justify-content: center; }}
</style>
<p id="status">正在加载验证组件…</p>
<div id="captcha"></div>
<script>
(function () {{
  var CAPTCHA_ID = "{safe_id}";
  var statusEl = document.getElementById('status');

  function statusText(t) {{ statusEl.textContent = t; }}

  function report(payload) {{
    // 通过 Tauri IPC 回传给 Rust 端
    if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) {{
      window.__TAURI_INTERNALS__.invoke('captcha_result', payload);
    }} else {{
      statusText('内部错误：IPC 不可用');
    }}
  }}

  var timeout = setTimeout(function () {{
    statusText('加载超时，请点击重新加载');
  }}, 15000);

  var script = document.createElement('script');
  script.src = '{NECAPTCHA_SDK_URL}';
  script.onerror = function () {{
    clearTimeout(timeout);
    statusText('验证组件加载失败，请检查网络后重试');
  }};
  script.onload = function () {{
    try {{
      initNECaptcha({{
        captchaId: CAPTCHA_ID,
        element: '#captcha',
        protocol: 'https',
        mode: 'embed',
        apiVersion: 2,
        width: Math.min(320, window.innerWidth - 48) + 'px',
        lang: 'zh-CN',
        onVerify: function (err, data) {{
          clearTimeout(timeout);
          if (!err && data && data.validate) {{
            statusText('验证通过');
            report({{ validate: data.validate, message: 'ok' }});
          }} else {{
            statusText('验证未通过，请重试');
          }}
        }}
      }},
      function () {{
        clearTimeout(timeout);
        statusText('请按下方提示完成人机验证');
      }},
      function () {{
        clearTimeout(timeout);
        statusText('初始化失败，请点击重新加载');
      }});
    }} catch (e) {{
      clearTimeout(timeout);
      statusText('初始化失败：' + e);
    }}
  }};
  document.head.appendChild(script);
}})();
</script>
</html>"#
    )
}

/// 打开验证窗口。
pub fn open_window<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    captcha_id: &str,
) -> Result<(), String> {
    // 若已存在则先关掉，避免用旧 captchaId 的页面
    if let Some(old) = app.get_webview_window(CAPTCHA_WINDOW_LABEL) {
        let _ = old.close();
    }

    let html = build_captcha_html(captcha_id);

    // 用一个无效域名做 base，确保页面无同源来源，
    // 无法读取应用自身的任何资源。
    let base: url::Url = "https://captcha.coolall.invalid/"
        .parse()
        .map_err(|e| format!("base url 无效：{e}"))?;

    WebviewWindowBuilder::new(app, CAPTCHA_WINDOW_LABEL, WebviewUrl::External(base))
        .title("人机验证")
        .inner_size(420.0, 560.0)
        .min_inner_size(360.0, 480.0)
        .center()
        .build()
        .map_err(|e| format!("创建验证窗口失败：{e}"))?
        .eval(&format!(
            "document.open();document.write({});document.close();",
            serde_json::to_string(&html).map_err(|e| e.to_string())?
        ))
        .map_err(|e| format!("写入验证页面失败：{e}"))?;

    Ok(())
}

/// 关闭验证窗口。
pub fn close_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(win) = app.get_webview_window(CAPTCHA_WINDOW_LABEL) {
        let _ = win.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_contains_sdk_and_id() {
        let html = build_captcha_html("414e5c9b866a03db03f860a1a9101672");
        assert!(html.contains(NECAPTCHA_SDK_URL));
        assert!(html.contains("414e5c9b866a03db03f860a1a9101672"));
        assert!(html.contains("initNECaptcha"));
        assert!(html.contains("onVerify"));
    }

    #[test]
    fn html_uses_ipc_invoke() {
        let html = build_captcha_html("abc123");
        assert!(html.contains("__TAURI_INTERNALS__"));
        assert!(html.contains("captcha_result"));
    }

    #[test]
    fn html_filters_dangerous_id_chars() {
        // 防止把引号/尖括号注入到 JS 字符串
        let html = build_captcha_html("ab\"c<script>alert(1)</script>");
        assert!(!html.contains("<script>alert"), "不应能注入脚本");
        assert!(html.contains("abcscriptalert1script"), "非字母数字应被过滤");
    }

    #[test]
    fn valid_validate_accepts_normal() {
        assert!(valid_validate("abc123"));
        assert!(valid_validate("NEC%3A414e5c9b%3Axyz"));
    }

    #[test]
    fn valid_validate_rejects_bad() {
        assert!(!valid_validate(""));
        assert!(!valid_validate("has space"));
        assert!(!valid_validate("line\nbreak"));
        assert!(!valid_validate(&"x".repeat(8193)));
    }
}
