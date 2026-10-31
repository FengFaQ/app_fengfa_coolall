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
pub mod device;
pub mod error;
pub mod request;
pub mod token_v3;

pub use error::Error;
pub use token_v3::TokenError;

use std::sync::Mutex;
use tauri::State;

/// 应用全局状态。
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
            .map_err(|_| "内部状态锁失败".to_string())?;
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
            .map_err(|_| "内部状态锁失败".to_string())?;
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
            .map_err(|_| "内部状态锁失败".to_string())?;
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
            .map_err(|_| "内部状态锁失败".to_string())?;
        client.search(&keyword, page).await
    };
    Ok(match outcome {
        Ok(data) => InvokeResult::success(data),
        Err(e) => InvokeResult::failure(e),
    })
}

/// 当前会话状态（是否登录、是否有验证凭证）。
#[tauri::command]
pub fn session_info(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let client = state
        .client
        .lock()
        .map_err(|_| "内部状态锁失败".to_string())?;
    Ok(client.session_info())
}

/// 提交人机验证结果。
#[tauri::command]
pub fn submit_validation(
    state: State<'_, AppState>,
    challenge_id: String,
    field: String,
    validate: String,
) -> Result<bool, String> {
    let mut client = state
        .client
        .lock()
        .map_err(|_| "内部状态锁失败".to_string())?;
    Ok(client.submit_validation(&challenge_id, &field, &validate))
}

/// 导入 ShuzlmID 文本。
#[tauri::command]
pub fn import_shuzlm_id(state: State<'_, AppState>, text: String) -> Result<(), String> {
    let mut client = state
        .client
        .lock()
        .map_err(|_| "内部状态锁失败".to_string())?;
    client.import_shuzlm_id(&text)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
