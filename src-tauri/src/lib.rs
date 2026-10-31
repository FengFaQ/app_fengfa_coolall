//! coolall —— 非官方酷安客户端核心库
//!
//! 模块划分见 `docs/ARCHITECTURE.md`。
//!
//! - [`token_v3`]  请求签名
//! - [`device`]    设备身份与 UA
//! - [`request`]   请求头与会话
//! - [`error`]     错误分类
//! - [`client`]    对外的深模块接口

pub mod client;

// 协议核心从独立 crate 引入，避免与 GUI 依赖耦合
pub use coolall_core::{device, error, request, token_v3};

pub use coolall_core::Error;
pub use coolall_core::TokenError;

use tokio::sync::Mutex;
use tauri::State;

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

/// 首页/列表浏览。
#[tauri::command]
pub async fn browse(
    state: State<'_, AppState>,
    target: String,
    page: u32,
) -> Result<InvokeResult<serde_json::Value>, String> {
    let outcome = {
        let mut client = state
            .client
            .lock()
            .await;
        client.browse(&target, page).await
    };
    Ok(match outcome {
        Ok(data) => InvokeResult::success(data),
        Err(e) => InvokeResult::failure(e),
    })
}

/// 内容详情。失败时自动尝试公开页兜底。
#[tauri::command]
pub async fn detail(
    state: State<'_, AppState>,
    id: String,
) -> Result<InvokeResult<serde_json::Value>, String> {
    let outcome = {
        let mut client = state
            .client
            .lock()
            .await;
        client.detail(&id).await
    };
    Ok(match outcome {
        Ok(data) => InvokeResult::success(data),
        Err(e) => InvokeResult::failure(e),
    })
}

/// 评论列表。
#[tauri::command]
pub async fn replies(
    state: State<'_, AppState>,
    id: String,
    page: u32,
) -> Result<InvokeResult<serde_json::Value>, String> {
    let outcome = {
        let mut client = state
            .client
            .lock()
            .await;
        client.replies(&id, page).await
    };
    Ok(match outcome {
        Ok(data) => InvokeResult::success(data),
        Err(e) => InvokeResult::failure(e),
    })
}

/// 搜索。
#[tauri::command]
pub async fn search(
    state: State<'_, AppState>,
    keyword: String,
    page: u32,
) -> Result<InvokeResult<serde_json::Value>, String> {
    let outcome = {
        let mut client = state
            .client
            .lock()
            .await;
        client.search(&keyword, page).await
    };
    Ok(match outcome {
        Ok(data) => InvokeResult::success(data),
        Err(e) => InvokeResult::failure(e),
    })
}

/// 当前会话状态（是否登录、是否有验证凭证）。
#[tauri::command]
pub async fn session_info(
    state: State<'_, AppState>,
) -> Result<InvokeResult<serde_json::Value>, String> {
    let client = state.client.lock().await;
    Ok(InvokeResult::success(client.session_info()))
}

/// 提交人机验证结果。
#[tauri::command]
pub async fn submit_validation(
    state: State<'_, AppState>,
    challenge_id: String,
    field: String,
    validate: String,
) -> Result<bool, String> {
    let mut client = state
        .client
        .lock()
        .await;
    Ok(client.submit_validation(&challenge_id, &field, &validate))
}

/// 导入 ShuzlmID 文本。
#[tauri::command]
pub async fn import_shuzlm_id(state: State<'_, AppState>, text: String) -> Result<(), String> {
    let mut client = state
        .client
        .lock()
        .await;
    client.import_shuzlm_id(&text)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `manage` 来自 Manager trait，必须在作用域内
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
            browse,
            detail,
            replies,
            search,
            session_info,
            submit_validation,
            import_shuzlm_id
        ])
        .run(tauri::generate_context!())
        .expect("启动 coolall 失败");
}



