//! 请求头拼装与会话 Cookie 管理。
//!
//! 协议见 `docs/PROTOCOL.md` §6 与 §8。
//!
//! 这一层是 WAF 能否放行的关键：15 个请求头缺任何一个都可能被拒。
//! 实测对比见 PROTOCOL.md §13。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::device::{constants, DeviceIdentity};
use crate::token_v3;

/// 一次请求所需的全部上下文。
pub struct RequestContext<'a> {
    pub identity: &'a DeviceIdentity,
    pub session: &'a Session,
    pub dark_mode: bool,
    pub timestamp: i64,
}

/// 本地维护的会话状态。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    /// 服务端下发的 SESSID
    pub sessid: Option<String>,
    /// 人机验证通过后的凭证
    pub validation: Option<String>,
    /// 登录态：uid / username / token
    pub uid: Option<String>,
    pub username: Option<String>,
    pub token: Option<String>,
}

impl Session {
    pub fn is_logged_in(&self) -> bool {
        self.uid.is_some() && self.token.is_some()
    }

    /// 构造 `Cookie` 头。
    ///
    /// 游客态只带 SESSID；登录态额外带 uid/username/token。
    /// validate 凭证单独拼装，且必须放在最后。
    pub fn cookie_header(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if self.is_logged_in() {
            parts.push(format!("uid={}", self.uid.as_deref().unwrap_or_default()));
            parts.push(format!(
                "username={}",
                self.username.as_deref().unwrap_or_default()
            ));
            parts.push(format!("token={}", self.token.as_deref().unwrap_or_default()));
        }
        if let Some(sessid) = &self.sessid {
            parts.push(format!("SESSID={sessid}"));
        }
        if let Some(validate) = &self.validation {
            parts.push(format!("validate={validate}"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("; "))
        }
    }

    /// 从响应的 `Set-Cookie` 中提取并更新 SESSID。
    ///
    /// SESSID 变化时必须清除 validate —— 旧凭证对新会话无效。
    pub fn update_from_set_cookie(&mut self, headers: &[String]) {
        for raw in headers {
            let Some((pair, attrs)) = raw.split_once(';').or(Some((raw.as_str(), ""))) else {
                continue;
            };
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            if name.trim() != "SESSID" {
                continue;
            }
            let value = value.trim();
            let lower = attrs.to_lowercase();
            // 服务端删除 cookie 时 value 为空或立即过期
            if value.is_empty() || lower.contains("max-age=0") {
                self.sessid = None;
                self.validation = None;
                continue;
            }
            if !is_valid_cookie_value(value) {
                continue;
            }
            if self.sessid.as_deref() != Some(value) {
                self.validation = None;
            }
            self.sessid = Some(value.to_string());
        }
    }

    pub fn accept_validation(&mut self, proof: impl Into<String>) {
        self.validation = Some(proof.into());
    }

    pub fn clear_validation(&mut self) {
        self.validation = None;
    }
}

/// Cookie 值合法性，对齐参考实现的正则 `^[A-Za-z0-9,._~%+/-]{1,512}$`
fn is_valid_cookie_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, ',' | '.' | '_' | '~' | '%' | '+' | '/' | '-')
        })
}

/// 构造完整的请求头集合。
///
/// 顺序无所谓，完整性才是关键。
pub fn build_headers(ctx: &RequestContext<'_>) -> Result<HashMap<String, String>, token_v3::TokenError> {
    let device_header = ctx.identity.device_header();
    let token = token_v3::create(
        &device_header,
        constants::APP_CODE,
        ctx.timestamp,
    )?;

    let mut headers = HashMap::new();
    headers.insert("User-Agent".into(), ctx.identity.user_agent());
    headers.insert(
        "X-Requested-With".into(),
        constants::REQUEST_WITH.to_string(),
    );
    headers.insert(
        "X-Sdk-Int".into(),
        ctx.identity.profile.sdk_int.clone(),
    );
    headers.insert("X-Sdk-Locale".into(), constants::LOCALE.to_string());
    headers.insert("X-App-Id".into(), constants::APP_ID.to_string());
    headers.insert("X-App-Token".into(), token);
    headers.insert(
        "X-App-Version".into(),
        constants::VERSION_NAME.to_string(),
    );
    headers.insert("X-App-Code".into(), constants::VERSION_CODE.to_string());
    headers.insert("X-Api-Version".into(), constants::API_VERSION.to_string());
    headers.insert("X-App-Device".into(), device_header);
    headers.insert(
        "X-Dark-Mode".into(),
        if ctx.dark_mode { "1" } else { "0" }.to_string(),
    );
    headers.insert("X-App-Channel".into(), constants::CHANNEL.to_string());
    headers.insert("X-App-Mode".into(), constants::MODE.to_string());
    headers.insert(
        "X-App-Supported".into(),
        constants::VERSION_CODE.to_string(),
    );
    headers.insert("Accept".into(), "application/json".into());

    if let Some(cookie) = ctx.session.cookie_header() {
        headers.insert("Cookie".into(), cookie);
    }

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceProfile;

    fn identity() -> DeviceIdentity {
        DeviceIdentity::new("63a31cc4b95ae992233b215af7cb1f0d", DeviceProfile::default())
    }

    fn ctx<'a>(id: &'a DeviceIdentity, s: &'a Session) -> RequestContext<'a> {
        RequestContext {
            identity: id,
            session: s,
            dark_mode: false,
            timestamp: 1735689600,
        }
    }

    #[test]
    fn headers_contain_all_required() {
        let id = identity();
        let s = Session::default();
        let h = build_headers(&ctx(&id, &s)).unwrap();

        for key in [
            "User-Agent",
            "X-Requested-With",
            "X-Sdk-Int",
            "X-Sdk-Locale",
            "X-App-Id",
            "X-App-Token",
            "X-App-Version",
            "X-App-Code",
            "X-Api-Version",
            "X-App-Device",
            "X-Dark-Mode",
            "X-App-Channel",
            "X-App-Mode",
            "X-App-Supported",
            "Accept",
        ] {
            assert!(h.contains_key(key), "缺少请求头 {key}");
        }
    }

    #[test]
    fn app_code_and_supported_are_present_separately() {
        let id = identity();
        let s = Session::default();
        let h = build_headers(&ctx(&id, &s)).unwrap();
        assert_eq!(h["X-App-Code"], "2610291");
        assert_eq!(h["X-App-Supported"], "2610291");
    }

    #[test]
    fn guest_has_no_cookie() {
        let s = Session::default();
        assert!(s.cookie_header().is_none());
    }

    #[test]
    fn guest_with_sessid_only() {
        let mut s = Session::default();
        s.update_from_set_cookie(&["SESSID=abc123; Path=/; Domain=.coolapk.com".into()]);
        assert_eq!(s.cookie_header().unwrap(), "SESSID=abc123");
    }

    #[test]
    fn logged_in_cookie_order() {
        let mut s = Session::default();
        s.uid = Some("42".into());
        s.username = Some("bob".into());
        s.token = Some("tk".into());
        s.update_from_set_cookie(&["SESSID=abc; Path=/".into()]);
        assert_eq!(
            s.cookie_header().unwrap(),
            "uid=42; username=bob; token=tk; SESSID=abc"
        );
    }

    #[test]
    fn validation_is_appended_last() {
        let mut s = Session::default();
        s.update_from_set_cookie(&["SESSID=abc; Path=/".into()]);
        s.accept_validation("NEC:1234:xyz");
        assert_eq!(
            s.cookie_header().unwrap(),
            "SESSID=abc; validate=NEC:1234:xyz"
        );
    }

    #[test]
    fn sessid_change_clears_validation() {
        let mut s = Session::default();
        s.update_from_set_cookie(&["SESSID=old; Path=/".into()]);
        s.accept_validation("proof");
        s.update_from_set_cookie(&["SESSID=new; Path=/".into()]);
        assert!(s.validation.is_none(), "SESSID 变化后应清除 validate");
    }

    #[test]
    fn deleted_sessid_clears_all() {
        let mut s = Session::default();
        s.update_from_set_cookie(&["SESSID=old; Path=/".into()]);
        s.accept_validation("proof");
        s.update_from_set_cookie(&["token=deleted; Max-Age=0; Path=/".into()]);
        // 非 SESSID 的 cookie 不影响
        assert!(s.sessid.is_some());
        s.update_from_set_cookie(&["SESSID=; Max-Age=0; Path=/".into()]);
        assert!(s.sessid.is_none());
        assert!(s.validation.is_none());
    }

    #[test]
    fn rejects_invalid_cookie_value() {
        let mut s = Session::default();
        s.update_from_set_cookie(&["SESSID=has space; Path=/".into()]);
        assert!(s.sessid.is_none());
    }

    #[test]
    fn dark_mode_reflects_flag() {
        let id = identity();
        let s = Session::default();
        let mut c = ctx(&id, &s);
        c.dark_mode = true;
        let h = build_headers(&c).unwrap();
        assert_eq!(h["X-Dark-Mode"], "1");
    }
}
