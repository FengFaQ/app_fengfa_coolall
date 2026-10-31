//! 错误分类：把 HTTP 状态、业务码、验证码挑战统一映射为可处理的错误类型。
//!
//! 协议见 `docs/PROTOCOL.md` §12。
//!
//! 关键：`status: 1004` 与 `messageStatus: err_request_captcha_v2` 都表示
//! 需要人机验证，但表现形式不同——前者是游客态的常态，后者是挑战下发。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 人机验证挑战。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptchaChallenge {
    pub id: String,
    pub field: String,
}

impl Default for CaptchaChallenge {
    fn default() -> Self {
        Self {
            // 服务端未下发时的兜底值，对齐参考实现
            id: "414e5c9b866a03db03f860a1a9101672".into(),
            field: "_v2_post_token".into(),
        }
    }
}

impl CaptchaChallenge {
    /// 把用户拿到的 validate 拼成可回传的凭证。
    ///
    /// 校验：非空、长度 ≤ 8192、不含空白与控制字符。
    /// 按 PROTOCOL.md §9.3，结果需要做 UTF-8 URL 编码后再使用。
    pub fn proof(&self, validate: &str) -> Option<String> {
        let ok = !validate.is_empty()
            && validate.len() <= 8192
            && !validate.chars().any(|c| c.is_whitespace() || (c as u32) < 32);
        if !ok {
            return None;
        }
        let short: String = self.id.chars().take(8).collect();
        // NEC:{captchaId 前 8 位}:{validate}，再整体 URL 编码
        Some(urlencoding::encode(&format!("NEC:{short}:{validate}")).into_owned())
    }

    /// 从响应体识别挑战。返回 None 表示不是挑战。
    pub fn from_body(body: &Value) -> Option<Self> {
        let status = body.get("messageStatus").and_then(Value::as_str);
        if status != Some("err_request_captcha_v2") {
            return None;
        }
        let extra = body
            .get("messageExtra")
            .or_else(|| body.get("extra"))
            .map(|raw| match raw {
                Value::Object(_) => raw.clone(),
                Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
                _ => Value::Null,
            })
            .unwrap_or_else(|| body.clone());

        let default_challenge = CaptchaChallenge::default();
        let id = extra
            .get("captchaId")
            .and_then(Value::as_str)
            .filter(|s| {
                (16..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric())
            })
            .map(str::to_string)
            .unwrap_or_else(|| default_challenge.id.clone());
        let field = extra
            .get("captchaField")
            .and_then(Value::as_str)
            .filter(|s| {
                s.is_empty()
                    || (s.starts_with('_')
                        && s.len() <= 64
                        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            })
            .map(str::trim)
            .unwrap_or("_v2_post_token");

        Some(Self {
            id,
            field: field.to_string(),
        })
    }
}

/// 统一错误类型。序列化给前端时带上 `kind` 与 `retryable`。
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Error {
    #[error("连接失败，请检查网络后重试")]
    Network { message: String },

    #[error("请求失败（HTTP {status}）")]
    Http { status: u16 },

    #[error("{message}")]
    Business { status: i64, message: String },

    #[error("请求需要人机验证，请完成验证后重试")]
    CaptchaNeeded { challenge: CaptchaChallenge },

    #[error("登录状态已失效或需要登录，请重新登录")]
    AuthRequired,

    #[error("请求过于频繁，请稍后重试")]
    RateLimited,

    #[error("当前客户端协议未被服务端接受，请检查协议设置或更新客户端")]
    ClientUpgrade,

    #[error("服务端检测到网络环境异常，请检查代理、DNS 或网络过滤设置")]
    RiskControl,

    #[error("服务器响应格式异常，请稍后重试")]
    Parse { message: String },
}

impl Error {
    /// 前端据此决定是否显示「重试」按钮。
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Error::Network { .. } | Error::RateLimited | Error::Http { .. }
        )
    }
}

/// 判定响应是否为成功。
///
/// 规则：无 `err_` 前缀的 messageStatus；
/// status ∈ {0,1}；code ∈ {0,1,200}；error ∈ {null,"0","false",""}。
pub fn is_success(body: &Value) -> bool {
    if let Some(ms) = body.get("messageStatus").and_then(Value::as_str) {
        if ms.starts_with("err_") {
            return false;
        }
    }
    if let Some(s) = body.get("status") {
        if !s.is_null() {
            match s.as_i64() {
                Some(v) if v == 0 || v == 1 => {}
                _ => return false,
            }
        }
    }
    if let Some(c) = body.get("code") {
        if !c.is_null() {
            match c.as_i64() {
                Some(v) if v == 0 || v == 1 || v == 200 => {}
                _ => return false,
            }
        }
    }
    if let Some(e) = body.get("error") {
        let text = match e {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if !matches!(text.as_str(), "" | "0" | "false") {
            return false;
        }
    }
    true
}

/// 把响应体映射为错误。返回 None 表示成功。
pub fn classify(body: &Value) -> Option<Error> {
    let text = |key: &str| -> String {
        body.get(key)
            .filter(|v| !v.is_null())
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default()
    };

    let marker = text("messageStatus").to_lowercase();
    let message = text("message");
    let clues = format!("{} {} {}", marker, message, text("error")).to_lowercase();
    let status = text("status").parse::<i64>().ok();
    let code = text("code").parse::<i64>().ok();

    let has_data = body.get("data").map(|d| !d.is_null()).unwrap_or(false);
    let failed = marker.starts_with("err_")
        || status.map(|s| s != 0 && s != 1).unwrap_or(false)
        || code.map(|c| c != 0 && c != 1 && c != 200).unwrap_or(false)
        || !matches!(text("error").as_str(), "" | "0" | "false")
        || (!has_data && !message.is_empty() && code.is_none() && status.is_none());

    if !failed {
        return None;
    }

    // 先识别验证码挑战
    if let Some(challenge) = CaptchaChallenge::from_body(body) {
        return Some(Error::CaptchaNeeded { challenge });
    }

    // 1004 是游客态的强制验证，与 err_request_captcha_v2 不同层
    if status == Some(1004) {
        return Some(Error::CaptchaNeeded {
            challenge: CaptchaChallenge::default(),
        });
    }

    if status == Some(-10) || clues.contains("网络环境") {
        return Some(Error::RiskControl);
    }
    if marker == "err_request_need_upgrade_new_version" {
        return Some(Error::ClientUpgrade);
    }
    if clues.contains("captcha") || clues.contains("验证码") || clues.contains("验证") {
        return Some(Error::CaptchaNeeded {
            challenge: CaptchaChallenge::default(),
        });
    }
    if clues.contains("login")
        || clues.contains("auth")
        || clues.contains("登录")
        || code == Some(401)
    {
        return Some(Error::AuthRequired);
    }
    if clues.contains("rate_limit") || clues.contains("频繁") || code == Some(429) {
        return Some(Error::RateLimited);
    }

    Some(Error::Business {
        status: status.unwrap_or(-1),
        message: if message.is_empty() {
            "服务器未能完成请求，请稍后重试".into()
        } else {
            message
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn success_body() {
        let b = json!({"status": 0, "data": [1,2,3]});
        assert!(is_success(&b));
        assert!(classify(&b).is_none());
    }

    #[test]
    fn success_with_code_200() {
        let b = json!({"code": 200, "data": {"a": 1}});
        assert!(classify(&b).is_none());
    }

    #[test]
    fn status_1004_maps_to_captcha() {
        let b = json!({"status": 1004, "error": null, "message": "请求验证错误"});
        let e = classify(&b).expect("应判定为失败");
        assert!(matches!(e, Error::CaptchaNeeded { .. }), "实际: {e:?}");
    }

    #[test]
    fn captcha_v2_challenge_parsed() {
        let b = json!({
            "messageStatus": "err_request_captcha_v2",
            "messageExtra": {
                "captchaId": "abcdef0123456789",
                "captchaField": "_v2_post_token"
            }
        });
        let e = classify(&b).unwrap();
        match e {
            Error::CaptchaNeeded { challenge } => {
                assert_eq!(challenge.id, "abcdef0123456789");
                assert_eq!(challenge.field, "_v2_post_token");
            }
            other => panic!("类型错误: {other:?}"),
        }
    }

    #[test]
    fn captcha_falls_back_to_defaults() {
        let b = json!({"messageStatus": "err_request_captcha_v2"});
        let e = classify(&b).unwrap();
        match e {
            Error::CaptchaNeeded { challenge } => {
                assert_eq!(challenge.id, "414e5c9b866a03db03f860a1a9101672");
                assert_eq!(challenge.field, "_v2_post_token");
            }
            other => panic!("类型错误: {other:?}"),
        }
    }

    #[test]
    fn proof_format() {
        let c = CaptchaChallenge::default();
        let p = c.proof("abc123").unwrap();
        // proof 结果按 PROTOCOL.md §9.3 做 UTF-8 URL 编码，冒号编码为 %3A
        assert_eq!(p, "NEC%3A414e5c9b%3Aabc123");
        assert!(p.contains("%3A"), "冒号应被 URL 编码");
    }

    #[test]
    fn proof_rejects_whitespace() {
        let c = CaptchaChallenge::default();
        assert!(c.proof("has space").is_none());
        assert!(c.proof("").is_none());
    }

    #[test]
    fn auth_required() {
        let b = json!({"status": -1, "message": "登录信息有误"});
        assert!(matches!(classify(&b), Some(Error::AuthRequired)));
    }

    #[test]
    fn rate_limited() {
        let b = json!({"status": -1, "message": "操作过于频繁"});
        assert!(matches!(classify(&b), Some(Error::RateLimited)));
    }

    #[test]
    fn risk_control() {
        let b = json!({"status": -10, "message": "x"});
        assert!(matches!(classify(&b), Some(Error::RiskControl)));
    }

    #[test]
    fn client_upgrade() {
        let b = json!({"messageStatus": "err_request_need_upgrade_new_version"});
        assert!(matches!(classify(&b), Some(Error::ClientUpgrade)));
    }

    #[test]
    fn business_passthrough() {
        let b = json!({"status": -2, "message": "应用不存在"});
        match classify(&b) {
            Some(Error::Business { message, .. }) => assert_eq!(message, "应用不存在"),
            other => panic!("类型错误: {other:?}"),
        }
    }

    #[test]
    fn retryable_flags() {
        assert!(Error::RateLimited.retryable());
        assert!(!Error::AuthRequired.retryable());
        assert!(!Error::ClientUpgrade.retryable());
    }
}
