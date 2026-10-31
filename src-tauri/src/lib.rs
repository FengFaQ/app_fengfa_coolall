//! coolall —— 非官方酷安客户端
//!
//! 工程结构：
//!
//! - `crates/coolall-core` —— 协议核心（签名、设备身份、请求头、错误分类、验证码）
//!   **不依赖 tauri**，可独立编译测试
//! - `src-tauri` —— 桌面外壳
//!   - [`client`]   HTTP、会话、兜底
//!   - [`commands`] 7 个 Tauri 命令
//!
//! 详细设计见 `docs/ARCHITECTURE.md`。

pub mod captcha_window;
pub mod client;
pub mod commands;
pub mod transport;

#[cfg(test)]
mod mock_transport;

// 协议核心从独立 crate 引入，避免与 GUI 依赖耦合
pub use coolall_core::{captcha, device, error, request, token_v3};

pub use coolall_core::Error;
pub use coolall_core::TokenError;

use tokio::sync::Mutex;

/// 应用全局状态。
///
/// 使用 `tokio::sync::Mutex` 而非 `std::sync::Mutex`：
/// 命令处理函数里需要在持锁期间 `.await` 网络请求，
/// `std` 的守卫不满足 `Send`，会让 future 无法跨线程。
pub struct AppState {
    pub client: Mutex<client::CoolapkClient>,
}

/// 前端调用的统一入口返回结构。
#[derive(serde::Serialize)]
pub struct InvokeResult<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<Error>,
}

impl<T> InvokeResult<T> {
    pub fn success(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn failure(error: Error) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

/// 日志目录：优先用 exe 所在目录，失败则回退到系统临时目录。
///
/// 设计意图：出问题时用户能在安装目录直接找到 `coolall.log`，
/// 不用去 `%LOCALAPPDATA%` 里翻。
///
/// 为什么可能失败：NSIS 默认安装到 `%LOCALAPPDATA%\Programs\coolall`，
/// 那里可写；但如果用户装到 Program Files，普通用户无写权限。
fn log_dir_near_exe() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .filter(|d| {
            // 探测可写性：在目录里创建一个临时文件再删掉
            let probe = d.join(".write_probe");
            match std::fs::write(&probe, b"") {
                Ok(()) => {
                    let _ = std::fs::remove_file(&probe);
                    true
                }
                Err(_) => false,
            }
        })
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `manage` / `try_state` 来自 Manager trait
    use tauri::Manager;

    tauri::Builder::default()
        // 日志配置：同时输出到控制台和 exe 同目录下的 coolall.log
        //
        // 为什么不写 AppData（LogDir 的默认行为）：
        // 出问题时用户可以直接在安装目录找到日志发过来，
        // 不用去 %LOCALAPPDATA% 里翻。这是为排查方便刻意选的。
        .plugin(
            tauri_plugin_log::Builder::default()
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Folder {
                        path: log_dir_near_exe(),
                        file_name: Some("coolall".into()),
                    },
                ))
                .max_file_size(2 * 1024 * 1024)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        // 注册验证页的自定义协议。
        //
        // 必须走自定义协议而不是外部 https 域名：Tauri 的 IPC 权限检查
        // 会拒绝远程来源调用命令（`webview/mod.rs:2080` 的 `!is_local` 分支），
        // 而 `is_local_url()` 只认 tauri:// 、app 相对路径、用户注册的自定义协议。
        //
        // # 关于 captcha_result 的授权（重要，别被误导）
        //
        // `webview/mod.rs:2080` 的拦截条件是：
        //   (plugin_command.is_some() || has_app_acl_manifest || !is_local) && acl.is_none()
        //
        // 本仓库**没有 `permissions/` 目录**，所以 `has_app_manifest()` 为假；
        // 而自定义协议算本地来源，`!is_local` 也为假。
        // 于是整个条件不成立，`captcha_result` 会被直接放行，**不需要**在
        // capability 里声明。
        //
        // 反直觉之处：一旦在 `permissions/` 下建了任何权限文件，
        // `has_app_acl_manifest` 就变成 true，届时**所有**应用命令
        // 都必须显式授权，反而更容易配错。所以这里刻意不做权限声明。
        //
        // 命令侧仍用窗口 label 校验调用者（见 commands.rs::captcha_result），
        // 那才是真正的安全边界。
        //
        // captchaId 从**路径**读取（`/c/<id>`），不是查询参数——
        // 因为页面的 location.reload() 会丢 query 但保留路径。
        .register_uri_scheme_protocol(captcha_window::CAPTCHA_PROTOCOL, |ctx, request| {
            use tauri::http::Response;

            let _ = ctx;
            let path = request.uri().path();

            // 期望形式：/c/<captchaId>
            let captcha_id = path
                .strip_prefix("/c/")
                .unwrap_or("")
                .trim_end_matches('/');

            let html = captcha_window::build_captcha_html(captcha_id);

            Response::builder()
                .header("Content-Type", "text/html; charset=utf-8")
                // CSP 说明：
                // - script-src 必须含 'unsafe-inline'（页面内联脚本）
                //   与 'unsafe-eval'（易盾 SDK 内部用 new Function）
                // - 易盾会从 getconf 返回的多个域名加载资源，逐个放行
                .header(
                    "Content-Security-Policy",
                    "default-src 'none'; \
                     script-src 'unsafe-inline' 'unsafe-eval' \
                       https://cstaticdun.126.net https://*.dun.163.com https://*.163yun.com; \
                     connect-src https://cstaticdun.126.net https://*.dun.163.com https://*.163yun.com; \
                     style-src 'unsafe-inline'; \
                     img-src data: https:; \
                     font-src data: https:; \
                     frame-src https://*.dun.163.com https://*.163yun.com; \
                     object-src 'none'; base-uri 'none'; form-action 'none'",
                )
                .body(html.into_bytes())
                .expect("构造验证页响应失败")
        })
        .setup(|app| {
            let client = client::CoolapkClient::new(app.handle())?;
            app.manage(AppState {
                client: Mutex::new(client),
            });
            // 恢复上次会话（SESSID / 登录态）。
            // 必须放在 manage 之后，否则 try_state 取不到 AppState。
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Some(state) = handle.try_state::<AppState>() {
                    let guard = state.client.lock().await;
                    guard.restore_session().await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::browse,
            commands::detail,
            commands::replies,
            commands::search,
            commands::session_info,
            commands::submit_validation,
            commands::import_shuzlm_id,
            commands::set_public_fallback,
            commands::open_captcha_window,
            commands::close_captcha_window,
            commands::captcha_result,
            commands::complete_captcha
        ])
        .run(tauri::generate_context!())
        .expect("启动 coolall 失败");
}



