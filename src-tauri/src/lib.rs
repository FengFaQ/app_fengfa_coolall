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

pub mod client;
pub mod commands;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `manage` / `try_state` 来自 Manager trait
    use tauri::Manager;

    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::default().build())
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
            commands::set_public_fallback
        ])
        .run(tauri::generate_context!())
        .expect("启动 coolall 失败");
}
