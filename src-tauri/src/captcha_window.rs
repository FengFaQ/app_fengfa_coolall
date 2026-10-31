//! 验证码窗口：自建本地页面加载网易易盾 SDK，通过 Tauri IPC 回传 validate。
//!
//! # 真实机制（从参考项目 c001apk 的 CaptchaActivity.kt 核实）
//!
//! **不是打开酷安的验证页，而是自己构造一个 HTML 页面**：
//!
//! ```text
//! 1. 页面引入 https://cstaticdun.126.net/load.min.js
//! 2. 调 initNECaptcha({ captchaId, onVerify })
//! 3. onVerify 回调拿到 data.validate
//! 4. 通过 JS 桥回传给原生
//! ```
//!
//! `validate` 是易盾 JS 回调里的值，**不是 Cookie**，
//! 所以读 Cookie 的做法从根上就是错的。
//!
//! # 为什么用自定义协议而不是外部 URL
//!
//! Tauri 的 IPC 权限检查里有这段逻辑（tauri-2.12.5 `webview/mod.rs:2080`）：
//!
//! ```text
//! if (plugin_command.is_some() || has_app_acl_manifest || !is_local)
//!    && invoke.acl.is_none()
//! { 拒绝 }
//! ```
//!
//! 也就是说**远程来源**（`is_local == false`）调用命令必须有 ACL 条目。
//! 而 `is_local_url()` 只认三种来源：
//!
//! - `tauri://` 协议
//! - 相对于 `devUrl` / `frontendDist` 的地址
//! - **用户注册的自定义协议**
//!
//! 所以验证页必须走 `coolall-captcha://` 自定义协议，
//! 不能用 `https://captcha.xxx.invalid/`——那样会被判定为远程来源。
//!
//! # 安全约束
//!
//! - 页面只允许加载易盾脚本，不建立其它外部连接
//! - 窗口只授予 `captcha_result` 一个命令的权限
//! - 禁止弹出新窗口、禁止访问本地文件

use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// 验证窗口标签。
pub const CAPTCHA_WINDOW_LABEL: &str = "captcha";

/// 自定义协议名。用它提供验证页，使其被判定为本地来源。
pub const CAPTCHA_PROTOCOL: &str = "coolall-captcha";

/// 易盾脚本地址。
pub const NECAPTCHA_SDK_URL: &str = "https://cstaticdun.126.net/load.min.js";

/// 验证结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptchaResult {
    pub validate: Option<String>,
    pub message: String,
}

/// 校验 validate 格式（PROTOCOL.md §9.3）。
pub fn valid_validate(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8192
        && !value.chars().any(|c| c.is_whitespace() || (c as u32) < 32)
}

/// 校验 captchaId 格式，对齐参考项目的 `^[a-zA-Z0-9]{16,64}$`。
pub fn valid_captcha_id(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// 构造验证页面的 HTML。
///
/// `captcha_id` 来自挑战本身（`CaptchaChallenge.id`）
/// 或自定义协议 URL 的查询参数。
///
/// 注意 `connect-src` 必须放行易盾域名，否则 CSP 会拦掉 SDK 的请求。
pub fn build_captcha_html(captcha_id: &str) -> String {
    // 双重保护：先校验格式（调用方应已校验），再过滤字符
    let safe_id: String = captcha_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();

    let sdk = NECAPTCHA_SDK_URL;
    let template = r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>
  body { font: 16px/1.6 -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif;
         margin: 24px; display: flex; flex-direction: column; align-items: center; }
  #status { color: GrayText; text-align: center; margin-bottom: 16px; }
</style>
</head>
<body>
<p id="status">正在加载验证组件…</p>
<div id="captcha"></div>
<button id="reload" style="display:none;margin-top:16px;padding:8px 16px;cursor:pointer">重新加载</button>
<script>
(function () {
  var CAPTCHA_ID = "__ID__";
  var statusEl = document.getElementById('status');

  function statusText(t) { statusEl.textContent = t; }

  function showReload() {
    var btn = document.getElementById('reload');
    if (btn) btn.style.display = '';
  }

  document.getElementById('reload').onclick = function () { location.reload(); };

  function report(validate) {
    // 注意：invoke 返回 Promise，同步 try/catch 抓不到异步失败。
    // 必须用 .then/.catch，否则回传失败时页面仍显示「验证通过」，
    // 用户会以为是应用卡了。
    try {
      var p = window.__TAURI_INTERNALS__.invoke('captcha_result', {
        validate: validate,
        message: 'ok'
      });
      if (p && typeof p.then === 'function') {
        p.then(function () {
          statusText('验证通过，请回到主窗口点「我已完成验证」');
        }).catch(function (e) {
          statusText('回传失败：' + e);
          showReload();
        });
      } else {
        statusText('验证通过，请回到主窗口点「我已完成验证」');
      }
    } catch (e) {
      statusText('回传失败：' + e);
      showReload();
    }
  }

  // 监听 CSP 拦截事件，让静默失败可见（排查用）
  document.addEventListener('securitypolicyviolation', function (e) {
    statusText('资源被安全策略拦截：' + (e.violatedDirective || '') +
               ' ' + (e.blockedURI || ''));
    showReload();
  });

  if (!CAPTCHA_ID) {
    statusText('缺少验证标识，请关闭窗口后重试');
    return;
  }

  var timeout = setTimeout(function () {
    statusText('加载超时，请重新加载'); showReload();
  }, 20000);

  var script = document.createElement('script');
  script.src = "__SDK__";
  script.onerror = function () {
    clearTimeout(timeout);
    statusText('验证组件加载失败，请检查网络后重试');
    showReload();
  };
  script.onload = function () {
    try {
      initNECaptcha({
        captchaId: CAPTCHA_ID,
        element: '#captcha',
        protocol: 'https',
        mode: 'embed',
        apiVersion: 2,
        width: Math.min(320, window.innerWidth - 48),
        lang: 'zh-CN',
        onVerify: function (err, data) {
          clearTimeout(timeout);
          if (!err && data && data.validate) {
            report(data.validate);
          } else {
            statusText('验证未通过，请重试');
          }
        }
      },
      function () {
        clearTimeout(timeout);
        statusText('请按下方提示完成人机验证');
      },
      function () {
        clearTimeout(timeout);
        statusText('初始化失败，请关闭窗口后重试');
        showReload();
      });
    } catch (e) {
      clearTimeout(timeout);
      statusText('初始化失败：' + e);
    }
  };
  document.head.appendChild(script);
})();
</script>
</body>
</html>"#;

    template.replace("__ID__", &safe_id).replace("__SDK__", sdk)
}

/// 打开验证窗口。
///
/// 走自定义协议 `coolall-captcha://localhost/`，使 Tauri 判定为本地来源，
/// 从而允许调用 `captcha_result` 命令。
pub fn open_window<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    captcha_id: &str,
) -> Result<(), String> {
    if !valid_captcha_id(captcha_id) {
        return Err(format!(
            "验证标识格式不正确（期望 16-64 位字母数字，实际 {} 位）",
            captcha_id.len()
        ));
    }

    // 若已存在则先关掉，避免用旧 captchaId 的页面
    if let Some(old) = app.get_webview_window(CAPTCHA_WINDOW_LABEL) {
        let _ = old.close();
    }

    // captchaId 编码进**路径**而不是查询参数。
    //
    // 原因：页面上的「重新加载」按钮走 location.reload()，
    // 它不会保留 query，但会保留路径。如果 id 只在 query 里，
    // 重载后 handler 拿到空 id，页面显示「缺少验证标识」——
    // 而这是唯一的自救路径，一旦失败链路就彻底锁死。
    let url = format!("{CAPTCHA_PROTOCOL}://localhost/c/{captcha_id}");

    WebviewWindowBuilder::new(
        app,
        CAPTCHA_WINDOW_LABEL,
        WebviewUrl::CustomProtocol(
            url.parse().map_err(|e| format!("协议地址无效：{e}"))?,
        ),
    )
    .title("人机验证")
    .inner_size(420.0, 560.0)
    .min_inner_size(360.0, 480.0)
    .center()
    // 导航白名单：只允许停留在验证页自身。
    //
    // # Windows 上的关键陷阱
    //
    // wry 会把自定义协议 URL 改写成 http：
    //   coolall-captcha://localhost/  →  http://coolall-captcha.localhost/
    // （见 wry-0.57.1/src/custom_protocol_workaround.rs:21-27）
    //
    // 而 `on_navigation` 收到的是**改写后的 http URL**，
    // 所以写 `url.scheme() == CAPTCHA_PROTOCOL` 会永远为假，
    // 连首屏都会被拒——这是必须分平台处理的原因。
    //
    // 判定逻辑与 Tauri 自身的 is_local_url 保持一致
    // （tauri-2.12.5/src/webview/mod.rs:1986-1998）。
    .on_navigation(|url| {
        #[cfg(any(windows, target_os = "android"))]
        {
            let scheme_ok = url.scheme() == "http" || url.scheme() == "https";
            let host = url.domain().unwrap_or("");
            return scheme_ok && host == "coolall-captcha.localhost";
        }

        #[cfg(not(any(windows, target_os = "android")))]
        {
            let _ = url;
            url.scheme() == CAPTCHA_PROTOCOL && url.domain() == Some("localhost")
        }
    })
    // 禁止弹出新窗口：否则 window.open 会创建新的 Tauri 窗口
    .on_new_window(|_url, _features| tauri::webview::NewWindowResponse::Deny)
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
        let html = build_captcha_html("abc1234567890123456");
        assert!(html.contains("__TAURI_INTERNALS__"));
        assert!(html.contains("captcha_result"));
    }

    #[test]
    fn html_filters_dangerous_id_chars() {
        let html = build_captcha_html("ab\"c<script>alert(1)</script>");
        assert!(!html.contains("<script>alert"), "不应能注入脚本");
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

    #[test]
    fn captcha_id_validation_matches_reference() {
        assert!(valid_captcha_id("414e5c9b866a03db03f860a1a9101672"));
        assert!(valid_captcha_id(&"a".repeat(16)));
        assert!(valid_captcha_id(&"a".repeat(64)));
        assert!(!valid_captcha_id(&"a".repeat(15)));
        assert!(!valid_captcha_id(&"a".repeat(65)));
        assert!(!valid_captcha_id("has-dash-in-it-12345"));
        assert!(!valid_captcha_id(""));
    }
}

