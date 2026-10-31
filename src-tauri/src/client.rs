//! 核心深模块：把签名、设备、请求头、会话、验证码、错误分类收拢在一个接口后面。
//!
//! 调用者不需要知道 TokenV3、设备编码、Cookie 维护的任何细节。
//! 设计依据见 `docs/ARCHITECTURE.md`。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use coolall_core::captcha::{PendingCaptcha, ProofPlacement, ReplayGuard};
use coolall_core::device::{constants, DeviceIdentity, DeviceProfile};
use coolall_core::error::{classify, Error};
use coolall_core::request::{build_headers, RequestContext, Session};

/// 浏览目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseTarget {
    /// 首页推荐流，走 api2
    Home,
    /// 动态列表，走 api2
    Feed,
}

impl BrowseTarget {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "home" => Some(Self::Home),
            "feed" => Some(Self::Feed),
            _ => None,
        }
    }

    fn base_url(self) -> &'static str {
        match self {
            Self::Home | Self::Feed => constants::BASE_URL_2,
        }
    }

    fn path(self, page: u32) -> String {
        match self {
            Self::Home => format!("/v6/main/indexV8?page={page}&firstLaunch=0&installTime=0"),
            Self::Feed => format!("/v6/feed/list?page={page}"),
        }
    }
}

/// 持久化的身份配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedConfig {
    szlm_id: String,
    profile: DeviceProfile,
}

/// 对外客户端。
pub struct CoolapkClient {
    http: reqwest::Client,
    identity: DeviceIdentity,
    session: Arc<Mutex<Session>>,
    /// 当前待完成的验证挑战（含原 URL，供重放使用）
    pending_captcha: Option<PendingCaptcha>,
    /// 是否允许公开页兜底。由前端在用户放弃验证时显式开启。
    allow_public_fallback: bool,
    /// 重放守卫：保证每个验证流程只重放一次
    replay_guard: ReplayGuard,
    store_path: Option<std::path::PathBuf>,
}

impl CoolapkClient {
    /// 创建客户端。会尝试从本地读取身份配置，不存在则生成随机 szlmId 并立即持久化。
    pub fn new<R: tauri::Runtime>(
        app: &tauri::AppHandle<R>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        use tauri::Manager;
        let store_path = app.path().app_config_dir().ok().map(|mut p| {
            p.push("identity.json");
            p
        });

        let persisted = store_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<PersistedConfig>(&t).ok());

        let identity = match persisted {
            Some(cfg) if DeviceIdentity::valid_szlm_id(&cfg.szlm_id) => {
                DeviceIdentity::new(cfg.szlm_id, cfg.profile)
            }
            _ => DeviceIdentity::new(DeviceIdentity::random_szlm_id(), DeviceProfile::default()),
        };

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            // 不设默认 UA：UA 由 request::build_headers 统一提供
            .build()?;

        let client = Self {
            http,
            identity,
            session: Arc::new(Mutex::new(Session::default())),
            pending_captcha: None,
            allow_public_fallback: false,
            replay_guard: ReplayGuard::default(),
            store_path,
        };

        // 首次生成的设备标识必须立刻落盘，否则每次启动都是新设备
        client.persist();

        Ok(client)
    }

    fn persist(&self) {
        let Some(path) = &self.store_path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let cfg = PersistedConfig {
            szlm_id: self.identity.szlm_id.clone(),
            profile: self.identity.profile.clone(),
        };
        if let Ok(text) = serde_json::to_string_pretty(&cfg) {
            let _ = std::fs::write(path, text);
        }
    }

    /// 会话文件路径：与身份配置同目录。
    fn session_path(&self) -> Option<std::path::PathBuf> {
        self.store_path
            .as_ref()
            .map(|p| p.with_file_name("session.json"))
    }

    /// 持久化 SESSID 与登录态。
    ///
    /// `validation` 凭证**不落盘**——它是短时效的人机验证结果，
    /// 写入磁盘只会扩大泄露面。
    async fn persist_session(&self) {
        let Some(path) = self.session_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let snapshot = {
            let g = self.session.lock().await;
            Session {
                sessid: g.sessid.clone(),
                validation: None,
                uid: g.uid.clone(),
                username: g.username.clone(),
                token: g.token.clone(),
            }
        };
        if let Ok(text) = serde_json::to_string_pretty(&snapshot) {
            let _ = std::fs::write(path, text);
        }
    }

    /// 启动时恢复上次的会话。
    pub async fn restore_session(&self) {
        let Some(path) = self.session_path() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(mut saved) = serde_json::from_str::<Session>(&text) else {
            return;
        };
        saved.validation = None;
        let mut g = self.session.lock().await;
        *g = saved;
    }

    /// 发一次请求。内部完成头拼装、Cookie 回写、错误分类。
    async fn send(&self, base: &str, path: &str) -> Result<Value, Error> {
        let session = {
            let guard = self.session.lock().await;
            guard.clone()
        };

        let ctx = RequestContext {
            identity: &self.identity,
            session: &session,
            dark_mode: false,
            timestamp: chrono::Utc::now().timestamp(),
        };

        let headers = build_headers(&ctx).map_err(|e| Error::Parse {
            message: e.to_string(),
        })?;

        let url = format!("{base}{path}");
        let mut req = self.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| Error::Network {
            message: e.to_string(),
        })?;

        let status = resp.status();
        let set_cookies: Vec<String> = resp
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(str::to_string)
            .collect();

        // 回写 SESSID，变化时持久化
        let before = {
            let g = self.session.lock().await;
            g.sessid.clone()
        };
        {
            let mut guard = self.session.lock().await;
            guard.update_from_set_cookie(&set_cookies);
        }
        let after = {
            let g = self.session.lock().await;
            g.sessid.clone()
        };
        if before != after {
            self.persist_session().await;
        }

        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(Error::Http { status: 403 });
        }
        if !status.is_success() {
            return Err(Error::Http {
                status: status.as_u16(),
            });
        }

        let body: Value = resp.json().await.map_err(|e| Error::Parse {
            message: e.to_string(),
        })?;

        match classify(&body) {
            Some(err) => Err(err),
            None => Ok(body),
        }
    }

    /// 构造当前会话对应的请求头，供需要自定义 URL 的请求复用。
    async fn headers_map(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let session = {
            let g = self.session.lock().await;
            g.clone()
        };
        let ctx = RequestContext {
            identity: &self.identity,
            session: &session,
            dark_mode: false,
            timestamp: chrono::Utc::now().timestamp(),
        };
        let mut map = HeaderMap::new();
        if let Ok(hdrs) = build_headers(&ctx) {
            for (k, v) in hdrs {
                if let (Ok(name), Ok(val)) = (
                    HeaderName::from_bytes(k.as_bytes()),
                    HeaderValue::from_str(&v),
                ) {
                    map.insert(name, val);
                }
            }
        }
        map
    }

    /// 用户完成验证后，用凭证重放一次原请求。
    ///
    /// 规则见 PROTOCOL.md §9.4：
    /// - 只重放一次
    /// - 成功 → 保留凭证
    /// - 失败 → 清除凭证
    pub async fn retry_after_validation(
        &mut self,
        pending: &PendingCaptcha,
    ) -> Result<Value, Error> {
        if !self.replay_guard.may_replay() {
            return Err(Error::Business {
                status: 1004,
                message: "本次验证已重放过一次，请重新发起请求".into(),
            });
        }
        self.replay_guard.consume();

        match pending.placement() {
            ProofPlacement::QueryParam => {
                let proof = {
                    let g = self.session.lock().await;
                    g.validation.clone()
                };
                let Some(proof) = proof else {
                    return Err(Error::AuthRequired);
                };
                let url = pending.url_with_proof(&proof);
                let resp = self
                    .http
                    .get(&url)
                    .headers(self.headers_map().await)
                    .send()
                    .await
                    .map_err(|e| Error::Network {
                        message: e.to_string(),
                    })?;
                self.finish_response(resp).await
            }
            ProofPlacement::Cookie => {
                // Cookie 模式：走标准 send，会自动带上 session 里的 validate
                let (base, path) = split_url(&pending.original_url);
                self.finish_response_send(base, path).await
            }
        }
    }

    /// 发送并处理结果，失败时清除凭证。
    async fn finish_response_send(&mut self, base: &str, path: &str) -> Result<Value, Error> {
        let result = self.send(base, path).await;
        if result.is_err() {
            let mut g = self.session.lock().await;
            g.clear_validation();
        }
        result
    }

    /// 处理已构造好的响应，失败时清除凭证。
    async fn finish_response(&mut self, resp: reqwest::Response) -> Result<Value, Error> {
        let status = resp.status();
        if !status.is_success() {
            let mut g = self.session.lock().await;
            g.clear_validation();
            return Err(Error::Http {
                status: status.as_u16(),
            });
        }
        let body: Value = resp.json().await.map_err(|e| Error::Parse {
            message: e.to_string(),
        })?;
        match classify(&body) {
            Some(err) => {
                let mut g = self.session.lock().await;
                g.clear_validation();
                Err(err)
            }
            None => Ok(body),
        }
    }

    pub async fn browse(&mut self, target: &str, page: u32) -> Result<Value, Error> {
        let t = BrowseTarget::parse(target).ok_or_else(|| Error::Parse {
            message: format!("未知浏览目标: {target}"),
        })?;
        let base = t.base_url();
        let path = t.path(page.max(1));
        let result = self.send(base, &path).await;
        self.track_challenge(&result, base, &path);
        result
    }

    pub async fn search(&mut self, keyword: &str, page: u32) -> Result<Value, Error> {
        let kw = urlencoding::encode(keyword);
        let path = format!(
            "/v6/search?type=feed&feedType=feed&sort=default&searchValue={kw}&page={}&showAnonymous=-1",
            page.max(1)
        );
        let result = self.send(constants::BASE_URL, &path).await;
        self.track_challenge(&result, constants::BASE_URL, &path);
        result
    }

    pub async fn replies(&mut self, id: &str, page: u32) -> Result<Value, Error> {
        let iid = urlencoding::encode(id);
        let path = format!(
            "/v6/feed/replyList?id={iid}&listType=&page={}&discussMode=0&feedType=feed_reply&blockStatus=0&fromFeedAuthor=0",
            page.max(1)
        );
        let result = self.send(constants::BASE_URL_2, &path).await;
        self.track_challenge(&result, constants::BASE_URL_2, &path);
        result
    }

    /// 详情。仅在「验证码 / 鉴权」失败且 id 为纯数字时，降级到公开分享页。
    pub async fn detail(&mut self, id: &str) -> Result<Value, Error> {
        let iid = urlencoding::encode(id);
        let path = format!("/v6/feed/detail?id={iid}");
        let result = self.send(constants::BASE_URL, &path).await;
        self.track_challenge(&result, constants::BASE_URL, &path);

        match result {
            Ok(v) => Ok(v),
            Err(primary) => {
                // 只在「用户明确放弃验证」后才降级到公开页。
                //
                // 这是个刻意的顺序约束：如果一遇到 1004 就抓公开页，
                // 用户永远看不到验证入口，也就永远不会真正通过验证，
                // 后续所有请求都会继续 1004。必须先走验证链路。
                let may_degrade = matches!(
                    primary,
                    Error::CaptchaNeeded { .. } | Error::AuthRequired
                ) && self.allow_public_fallback;
                if may_degrade && id.chars().all(|c| c.is_ascii_digit()) {
                    let public_url = format!("{}/feed/{}", constants::PUBLIC_URL, id);
                    if let Ok(resp) = self.http.get(&public_url).send().await {
                        if resp.status().is_success() {
                            if let Ok(html) = resp.text().await {
                                return Ok(serde_json::json!({
                                    "isPublic": true,
                                    "sourceUrl": public_url,
                                    "html": html,
                                }));
                            }
                        }
                    }
                }
                Err(primary)
            }
        }
    }

    /// 设置是否允许公开页兜底。
    ///
    /// 前端在用户放弃验证时调用 `set_public_fallback(true)` 再重新请求，
    /// 这样「验证」与「降级」的顺序就由用户行为决定，而不是由错误码决定。
    pub fn set_public_fallback(&mut self, allow: bool) {
        self.allow_public_fallback = allow;
    }

    /// 记录挑战，供后续重放使用。
    fn track_challenge(&mut self, result: &Result<Value, Error>, base: &str, path: &str) {
        if let Err(Error::CaptchaNeeded { challenge }) = result {
            self.pending_captcha = Some(PendingCaptcha {
                id: challenge.id.clone(),
                field: challenge.field.clone(),
                original_url: format!("{base}{path}"),
            });
            self.replay_guard.reset();
        }
    }

    pub fn session_info(&self) -> Value {
        let pending = self.pending_captcha.as_ref();
        serde_json::json!({
            "challengeId": pending.map(|c| c.id.clone()),
            "challengeField": pending.map(|c| c.field.clone()),
            "szlmId": self.identity.szlm_id,
            "userAgent": self.identity.user_agent(),
        })
    }

    /// 接收前端完成验证后回传的 validate，并**立刻重放原请求**。
    ///
    /// 这是验证链路真正闭合的地方：只存下凭证是不够的，
    /// 必须用凭证把最初失败的那个请求重发一次，用户才能拿到内容。
    ///
    /// 参数校验：challenge_id 必须与当前待办一致，否则拒绝——
    /// 避免旧窗口或错误参数污染新会话。
    ///
    /// 返回：重放成功返回响应体；各类失败返回对应错误。
    pub async fn submit_validation(
        &mut self,
        challenge_id: &str,
        field: &str,
        validate: &str,
    ) -> Result<Value, String> {
        let Some(pending) = self.pending_captcha.clone() else {
            return Err("当前没有待完成的验证".into());
        };
        if !pending.matches(challenge_id) {
            return Err("验证标识不匹配，请重新发起请求".into());
        }
        let challenge = coolall_core::error::CaptchaChallenge {
            id: challenge_id.to_string(),
            field: field.to_string(),
        };
        let Some(proof) = challenge.proof(validate) else {
            return Err("验证结果格式不正确".into());
        };
        {
            let mut guard = self.session.lock().await;
            guard.accept_validation(proof);
        }

        // 写凭证后立刻重放，把最初失败的请求补上
        match self.retry_after_validation(&pending).await {
            Ok(v) => Ok(v),
            Err(e) => Err(format!("{e}")),
        }
    }

    /// 导入 ShuzlmID 并持久化。
    pub fn import_shuzlm_id(&mut self, text: &str) -> Result<(), String> {
        let id = DeviceIdentity::parse_shuzlm_id(text)?;
        self.identity = DeviceIdentity::new(id, self.identity.profile.clone());
        self.persist();
        Ok(())
    }
}

/// 把完整 URL 拆成 (base, path)。
fn split_url(url: &str) -> (&str, &str) {
    match url.find("/v6/") {
        Some(idx) => (&url[..idx], &url[idx..]),
        None => (url, "/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coolall_core::error::CaptchaChallenge;

    /// 测试专用构造：绕过 Tauri AppHandle，不落盘。
    ///
    /// 这正是「缺少 HttpTransport seam」的代价——真实构造需要 GUI 上下文。
    /// 这里只测与 HTTP 无关的纯状态逻辑。
    fn test_client() -> CoolapkClient {
        CoolapkClient {
            http: reqwest::Client::new(),
            identity: DeviceIdentity::new(
                "00000000000000000000000000000000",
                DeviceProfile::default(),
            ),
            session: Arc::new(Mutex::new(Session::default())),
            pending_captcha: None,
            replay_guard: ReplayGuard::default(),
            allow_public_fallback: false,
            store_path: None,
        }
    }

    #[test]
    fn split_url_extracts_base_and_path() {
        let (b, p) = split_url("https://api.coolapk.com/v6/feed/detail?id=1");
        assert_eq!(b, "https://api.coolapk.com");
        assert_eq!(p, "/v6/feed/detail?id=1");
    }

    #[test]
    fn split_url_handles_api2() {
        let (b, p) = split_url("https://api2.coolapk.com/v6/main/indexV8?page=1");
        assert_eq!(b, "https://api2.coolapk.com");
        assert_eq!(p, "/v6/main/indexV8?page=1");
    }

    #[test]
    fn public_fallback_defaults_to_off() {
        // 关键安全默认值：默认不允许降级，否则用户永远看不到验证入口
        assert!(!test_client().allow_public_fallback);
    }

    #[test]
    fn public_fallback_can_be_toggled() {
        let mut c = test_client();
        c.set_public_fallback(true);
        assert!(c.allow_public_fallback);
        c.set_public_fallback(false);
        assert!(!c.allow_public_fallback);
    }

    #[test]
    fn captcha_error_records_pending_with_full_url() {
        let mut c = test_client();
        let challenge = CaptchaChallenge {
            id: "abcdef0123456789".into(),
            field: "_v2_post_token".into(),
        };
        let result: Result<Value, Error> = Err(Error::CaptchaNeeded {
            challenge: challenge.clone(),
        });
        c.track_challenge(&result, "https://api.coolapk.com", "/v6/feed/detail?id=1");

        let pending = c.pending_captcha.as_ref().expect("应记录挑战");
        assert_eq!(pending.id, challenge.id);
        assert_eq!(pending.field, challenge.field);
        // 必须保存完整 URL 才能重放，只存 path 是不够的
        assert_eq!(
            pending.original_url,
            "https://api.coolapk.com/v6/feed/detail?id=1"
        );
        assert!(c.replay_guard.may_replay());
    }

    #[test]
    fn non_captcha_error_does_not_record_challenge() {
        let mut c = test_client();
        let result: Result<Value, Error> = Err(Error::Http { status: 403 });
        c.track_challenge(&result, "https://api.coolapk.com", "/v6/x");
        assert!(c.pending_captcha.is_none(), "非验证码错误不该记录挑战");
    }

    #[test]
    fn concurrent_captcha_resets_replay_guard() {
        // 一旦又遇到新挑战，重放机会应重置，否则用户第二次验证无法重放
        let mut c = test_client();
        c.replay_guard.consume();
        assert!(!c.replay_guard.may_replay());

        let result: Result<Value, Error> = Err(Error::CaptchaNeeded {
            challenge: CaptchaChallenge::default(),
        });
        c.track_challenge(&result, "https://api.coolapk.com", "/v6/x");
        assert!(c.replay_guard.may_replay(), "新挑战应重置重放机会");
    }

    #[test]
    fn session_info_exposes_challenge_when_pending() {
        let mut c = test_client();
        let info = c.session_info();
        assert!(info["challengeId"].is_null(), "无挑战时应为 null");

        let result: Result<Value, Error> = Err(Error::CaptchaNeeded {
            challenge: CaptchaChallenge {
                id: "abcdef0123456789".into(),
                field: "_v2_post_token".into(),
            },
        });
        c.track_challenge(&result, "https://api.coolapk.com", "/v6/x");
        let info = c.session_info();
        assert_eq!(info["challengeId"], "abcdef0123456789");
        assert_eq!(info["challengeField"], "_v2_post_token");
    }

    #[test]
    fn browse_target_parsing() {
        assert_eq!(BrowseTarget::parse("home"), Some(BrowseTarget::Home));
        assert_eq!(BrowseTarget::parse("feed"), Some(BrowseTarget::Feed));
        assert_eq!(BrowseTarget::parse("nope"), None);
    }
}

