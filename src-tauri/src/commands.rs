//! Tauri 命令层。
//!
//! **为什么命令要单独放一个模块**：
//! `#[tauri::command]` 会在模块级宏命名空间生成 `__cmd__{name}` 与
//! `__tauri_command_name_{name}` 两个符号。当 `lib.rs` 同时作为库入口
//! （被 `main.rs` 引用）并直接定义这些命令时，宏展开会产生
//! `E0255: defined multiple times`。
//!
//! 把命令集中到独立子模块后，宏注入发生在该模块自己的命名空间里，
//! 冲突消失。这也是 Tauri 官方推荐的工程结构。

use tauri::State;

use crate::{InvokeResult, AppState};

/// 首页/列表浏览。
#[tauri::command]
pub async fn browse(
    state: State<'_, AppState>,
    target: String,
    page: u32,
) -> Result<InvokeResult<serde_json::Value>, String> {
    let mut client = state.client.lock().await;
    Ok(match client.browse(&target, page).await {
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
    let mut client = state.client.lock().await;
    Ok(match client.detail(&id).await {
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
    let mut client = state.client.lock().await;
    Ok(match client.replies(&id, page).await {
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
    let mut client = state.client.lock().await;
    Ok(match client.search(&keyword, page).await {
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
    let client = state.client.lock().await;
    client
        .submit_validation(&challenge_id, &field, &validate)
        .await
}

/// 导入 ShuzlmID 文本。
#[tauri::command]
pub async fn import_shuzlm_id(
    state: State<'_, AppState>,
    text: String,
) -> Result<(), String> {
    let mut client = state.client.lock().await;
    client.import_shuzlm_id(&text)
}

/// 设置是否允许公开页兜底。
///
/// 前端在用户明确放弃人机验证时调用，随后重新请求详情。
/// 这样「先验证、后降级」的顺序由用户行为决定，
/// 而不是一遇到 1004 就静默降级——后者会让用户永远无法通过验证。
#[tauri::command]
pub async fn set_public_fallback(
    state: State<'_, AppState>,
    allow: bool,
) -> Result<(), String> {
    let mut client = state.client.lock().await;
    client.set_public_fallback(allow);
    Ok(())
}
