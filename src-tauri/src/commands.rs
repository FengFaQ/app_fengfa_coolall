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

/// 提交人机验证结果，**并重放原请求**。
///
/// 返回重放成功的响应体，前端拿到后可直接渲染，无需再发一次请求。
#[tauri::command]
pub async fn submit_validation(
    state: State<'_, AppState>,
    challenge_id: String,
    field: String,
    validate: String,
) -> Result<InvokeResult<serde_json::Value>, String> {
    let mut client = state.client.lock().await;
    match client
        .submit_validation(&challenge_id, &field, &validate)
        .await
    {
        Ok(data) => Ok(InvokeResult::success(data)),
        Err(message) => Ok(InvokeResult::failure(
            coolall_core::Error::Business {
                status: 1004,
                message,
            },
        )),
    }
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

/// 打开应用内嵌的验证窗口。
///
/// 窗口里是自己构造的本地页面 + 网易易盾 SDK，
/// **不是**打开酷安的验证页——那个页面不会把 validate 回传。
/// 机制依据见 `captcha_window.rs` 的模块文档。
#[tauri::command]
pub async fn open_captcha_window(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 取当前挑战的 id，它就是要传给易盾的 captchaId
    let captcha_id = {
        let client = state.client.lock().await;
        client
            .pending_captcha()
            .map(|p| p.id.clone())
            .ok_or_else(|| "当前没有待完成的验证".to_string())?
    };
    crate::captcha_window::open_window(&app, &captcha_id)
}

/// 关闭验证窗口。
#[tauri::command]
pub async fn close_captcha_window(app: tauri::AppHandle) -> Result<(), String> {
    crate::captcha_window::close_window(&app);
    Ok(())
}

/// 验证页面回传 validate 的入口。
///
/// 由验证窗口里的 JS 通过 `__TAURI_INTERNALS__.invoke` 调用。
/// 这里只校验格式并缓存，真正的重放由 [`complete_captcha`] 触发——
/// 让用户能确认「窗口显示验证通过」后再落定，避免误提交。
#[tauri::command]
pub async fn captcha_result(
    state: State<'_, AppState>,
    validate: String,
    message: String,
) -> Result<(), String> {
    if !crate::captcha_window::valid_validate(&validate) {
        return Err(format!("验证结果格式不正确：{message}"));
    }
    let mut client = state.client.lock().await;
    client.stage_validate(validate);
    Ok(())
}

/// 用户确认完成：用已暂存的 validate 重放原请求。
///
/// 这是整条链路的收口。若页面还没回传 validate，返回明确错误。
#[tauri::command]
pub async fn complete_captcha(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<InvokeResult<serde_json::Value>, String> {
    let mut client = state.client.lock().await;

    let Some(pending) = client.pending_captcha().cloned() else {
        return Err("当前没有待完成的验证".into());
    };
    let Some(validate) = client.take_staged_validate() else {
        return Err("还没有收到验证结果，请先在验证窗口中完成验证".into());
    };

    let result = client
        .submit_validation(&pending.id, &pending.field, &validate)
        .await;

    match result {
        Ok(data) => {
            crate::captcha_window::close_window(&app);
            Ok(InvokeResult::success(data))
        }
        Err(message) => Ok(InvokeResult::failure(coolall_core::Error::Business {
            status: 1004,
            message,
        })),
    }
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
