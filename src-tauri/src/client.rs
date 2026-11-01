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

use crate::transport::{HttpRequest, HttpResponse, HttpTransport, ReqwestTransport};

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
    http: Arc<dyn HttpTransport>,
    identity: DeviceIdentity,
    session: Arc<Mutex<Session>>,
    /// 当前待完成的验证挑战（含原 URL，供重放使用）
    pending_captcha: Option<PendingCaptcha>,
    /// 验证页回传但尚未确认提交的 validate
    staged_validate: Option<String>,
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

        let http: Arc<dyn HttpTransport> = Arc::new(ReqwestTransport::new()?);

        let client = Self {
            http,
            identity,
            session: Arc::new(Mutex::new(Session::default())),
            pending_captcha: None,
            staged_validate: None,
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
            if let Err(e) = std::fs::create_dir_all(dir) {
                // 只记路径与错误类型，不记文件内容
                log::warn!("创建配置目录失败 dir={dir:?} err={e}");
                return;
            }
        }
        let cfg = PersistedConfig {
            szlm_id: self.identity.szlm_id.clone(),
            profile: self.identity.profile.clone(),
        };
        match serde_json::to_string_pretty(&cfg) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    log::warn!("写入身份配置失败 path={path:?} err={e}");
                }
            }
            Err(e) => log::warn!("序列化身份配置失败 err={e}"),
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
            if let Err(e) = std::fs::create_dir_all(dir) {
                log::warn!("创建会话目录失败 dir={dir:?} err={e}");
                return;
            }
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
        match serde_json::to_string_pretty(&snapshot) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    log::warn!("写入会话文件失败 path={path:?} err={e}");
                }
            }
            Err(e) => log::warn!("序列化会话失败 err={e}"),
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
        let url = format!("{base}{path}");
        let request = HttpRequest::get(url.clone(), self.headers_map().await);

        log::info!("请求 {url}");

        let response = self.http.execute(request).await.map_err(|error| {
            log::warn!("请求失败 {url}：{}", error.message);
            Error::Network {
                message: error.message,
            }
        })?;

        log::info!("响应 {url} status={}", response.status);

        self.process_response(response).await
    }

    /// 回写响应 Cookie，然后按 HTTP 状态与业务体分类。
    ///
    /// 所有响应（普通请求、Cookie 重放、QueryParam 重放）都走这里，
    /// 避免出现某条路径忘记维护 SESSID 的不一致。
    async fn process_response(&self, response: HttpResponse) -> Result<Value, Error> {
        let set_cookies = response.set_cookies();

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

        decode_response(&response)
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

    /// Cookie 模式重放：凭证已在会话里，走标准 send 会自动带上。
    ///
    /// 字段模式请用 [`replay_with_proof`]，它把 proof 直接拼进 URL，
    /// 不写会话。两条路径分开是为了避免「查询参数凭证被误当 Cookie 复用」。
    ///
    /// 规则见 PROTOCOL.md §9.4：只重放一次；成功保留凭证，失败清除。
    pub async fn retry_after_validation(
        &mut self,
        pending: &PendingCaptcha,
    ) -> Result<Value, Error> {
        if !self.replay_guard.may_replay() {
            return Err(Error::Business {
                status: -1,
                message: "本次验证已重放过一次，请重新发起请求".into(),
            });
        }
        self.replay_guard.consume();

        match pending.placement() {
            ProofPlacement::Cookie => {
                let (base, path) = split_url(&pending.original_url);
                self.finish_response_send(base, path).await
            }
            ProofPlacement::QueryParam => {
                // 理论上不会走到这里（submit_validation 已分流），
                // 但保留兜底：取出会话里的凭证再拼一次。
                let proof = {
                    let g = self.session.lock().await;
                    g.validation.clone()
                };
                let Some(proof) = proof else {
                    return Err(Error::AuthRequired);
                };
                let url = pending.url_with_proof(&proof);
                let request = HttpRequest::get(url, self.headers_map().await);
                let response = self
                    .http
                    .execute(request)
                    .await
                    .map_err(|error| Error::Network {
                        message: error.message,
                    })?;
                self.finish_response(response).await
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

    /// 处理重放响应，失败时清除凭证。
    async fn finish_response(&mut self, response: HttpResponse) -> Result<Value, Error> {
        let result = self.process_response(response).await;
        if result.is_err() {
            let mut g = self.session.lock().await;
            g.clear_validation();
        }
        result
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

    /// 详情。对齐参考项目 `FeedViewModel.refresh()` 的两段式流程。
    ///
    /// # 为什么要两段
    ///
    /// 参考项目（`FeedViewModel.kt:321-341`）：
    ///
    /// ```text
    /// 首次打开：fetchFeedData(verifyCaptcha = false)   // 不弹验证
    ///   ↓ 失败
    /// 用户重试：fetchFeedData(verifyCaptcha = true)    // 这次才走验证
    /// ```
    ///
    /// 首次失败**不打扰用户**；只有用户在看见错误后主动重试，
    /// 才把请求标记为「允许验证」。这样服务端下发的
    /// `err_request_captcha_v2` 才会变成 `CaptchaNeeded`，前端才会弹滑块。
    ///
    /// 早期实现缺了这一环——我们从不「带验证重试」，
    /// 所以即使服务端要求验证，我们也只是反复收到同一个错误。
    pub async fn detail(&mut self, id: &str) -> Result<Value, Error> {
        self.detail_inner(id, false).await
    }

    /// 带验证重试的详情。由前端在用户主动重试时调用。
    pub async fn detail_with_captcha(&mut self, id: &str) -> Result<Value, Error> {
        self.detail_inner(id, true).await
    }

    async fn detail_inner(&mut self, id: &str, verify_captcha: bool) -> Result<Value, Error> {
        let iid = urlencoding::encode(id);
        let path = format!("/v6/feed/detail?id={iid}");

        log::info!("详情 id={id} verify_captcha={verify_captcha}");

        let result = self.send(constants::BASE_URL, &path).await;
        self.track_challenge(&result, constants::BASE_URL, &path);

        match result {
            Ok(v) => Ok(v),
            Err(primary) => {
                // 未开启验证时直接返回原始错误，由前端展示「重试」按钮。
                if !verify_captcha {
                    return Err(primary);
                }

                // 开启验证后，仅 CAPTCHA / AUTH 且 id 为纯数字时才降级公开页。
                let may_degrade =
                    matches!(primary, Error::CaptchaNeeded { .. } | Error::AuthRequired)
                        && self.allow_public_fallback;

                if may_degrade && id.chars().all(|c| c.is_ascii_digit()) {
                    log::info!("    降级到公开页 id={id}");
                    let public_url = format!("{}/feed/{}", constants::PUBLIC_URL, id);
                    let request =
                        HttpRequest::get(&public_url, reqwest::header::HeaderMap::new());
                    if let Ok(response) = self.http.execute(request).await {
                        if response.is_success() {
                            return Ok(serde_json::json!({
                                "isPublic": true,
                                "sourceUrl": public_url,
                                "html": response.text_body(),
                            }));
                        }
                    }
                }
                Err(primary)
            }
        }
    }

    /// 当前待完成的验证挑战（若有）。
    ///
    /// 供 `open_captcha_window` 取 captchaId（要传给易盾），
    /// 也供 `complete_captcha` 取 field（决定凭证放 Cookie 还是查询参数）。
    pub fn pending_captcha(&self) -> Option<&PendingCaptcha> {
        self.pending_captcha.as_ref()
    }

    /// 暂存验证页回传的 validate。
    ///
    /// 为什么不直接重放：页面显示「验证通过」后还需要用户点确认，
    /// 中间会有短暂间隔。暂存让它与重放解耦。
    pub fn stage_validate(&mut self, validate: String) {
        self.staged_validate = Some(validate);
    }

    /// 取出并清空暂存的 validate。
    pub fn take_staged_validate(&mut self) -> Option<String> {
        self.staged_validate.take()
    }

    /// 设置是否允许公开页兜底。
    ///
    /// 前端在用户放弃验证时调用 `set_public_fallback(true)` 再重新请求，
    /// 这样「验证」与「降级」的顺序就由用户行为决定，而不是由错误码决定。
    pub fn set_public_fallback(&mut self, allow: bool) {
        self.allow_public_fallback = allow;
    }

    /// 记录挑战，供后续重放使用。
    ///
    /// 每次遇到新挑战都重置重放守卫——否则重放被拒后，
    /// 同一个挑战再也无法重试，用户只能重启应用。
    /// 参考项目靠 CaptchaCoordinator 的 epoch 机制达到同样效果。
    fn track_challenge(&mut self, result: &Result<Value, Error>, base: &str, path: &str) {
        if let Err(Error::CaptchaNeeded { challenge }) = result {
            log::info!("    需要人机验证 id={} field={}", challenge.id, challenge.field);
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
    ) -> Result<Value, Error> {
        let Some(pending) = self.pending_captcha.clone() else {
            return Err(Error::Business {
                status: -1,
                message: "当前没有待完成的验证".into(),
            });
        };
        if !pending.matches(challenge_id) {
            return Err(Error::Business {
                status: -1,
                message: "验证标识不匹配，请重新发起请求".into(),
            });
        }
        let challenge = coolall_core::error::CaptchaChallenge {
            id: challenge_id.to_string(),
            field: field.to_string(),
        };
        let Some(proof) = challenge.proof(validate) else {
            return Err(Error::Business {
                status: -1,
                message: "验证结果格式不正确".into(),
            });
        };

        // 凭证的落点由 Challenge 决定，不是一律写进会话。
        //
        // 对齐参考项目 CaptchaInterceptor.kt:
        // - field 为空（Cookie 模式）→ 写入会话，后续请求自动携带
        // - field 非空（查询参数/表单）→ 只用于本次重放，不写会话
        //
        // 早期实现无条件写会话，导致查询参数模式下凭证被误当成
        // 长期 Cookie 使用，而服务端并不认，表现为「验一次管不了一片」。
        let cookie_mode = pending.placement() == ProofPlacement::Cookie;
        if cookie_mode {
            let mut guard = self.session.lock().await;
            guard.accept_validation(proof.clone());
            log::info!("    验证凭证已写入会话（Cookie 模式），开始重放");
        } else {
            log::info!("    验证凭证仅用于本次重放（字段模式），开始重放");
        }

        // 走一次性重放路径：字段模式下需要把 proof 直接带进 URL
        let result = if cookie_mode {
            self.retry_after_validation(&pending).await
        } else {
            self.replay_with_proof(&pending, &proof).await
        };

        match result {
            Ok(v) => Ok(v),
            Err(e) => {
                // 重放失败：清除可能写入的凭证
                let mut guard = self.session.lock().await;
                guard.clear_validation();
                Err(e)
            }
        }
    }

    /// 字段模式重放：proof 只拼进本次请求的 URL，不进会话。
    async fn replay_with_proof(
        &mut self,
        pending: &PendingCaptcha,
        proof: &str,
    ) -> Result<Value, Error> {
        if !self.replay_guard.may_replay() {
            return Err(Error::Business {
                status: -1,
                message: "本次验证已重放过一次，请重新发起请求".into(),
            });
        }
        self.replay_guard.consume();

        let url = pending.url_with_proof(proof);
        let request = HttpRequest::get(url.clone(), self.headers_map().await);
        log::info!("重放 {url}");

        let response = self.http.execute(request).await.map_err(|error| {
            log::warn!("重放失败 {url}：{}", error.message);
            Error::Network {
                message: error.message,
            }
        })?;

        self.process_response(response).await
    }

    /// 测试构造入口：生产代码仍必须走 `new(AppHandle)`。
    ///
    /// 有了 transport seam 之后，测试可以注入 MockTransport，
    /// 从而覆盖完整的「1004 → 验证 → 重放」链路，不需要 GUI 上下文。
    #[cfg(test)]
    fn with_test_transport(http: Arc<dyn HttpTransport>) -> Self {
        Self {
            http,
            identity: DeviceIdentity::new(
                "00000000000000000000000000000000",
                DeviceProfile::default(),
            ),
            session: Arc::new(Mutex::new(Session::default())),
            pending_captcha: None,
            replay_guard: ReplayGuard::default(),
            staged_validate: None,
            allow_public_fallback: false,
            store_path: None,
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

/// 把传输响应映射为业务结果。
fn decode_response(response: &HttpResponse) -> Result<Value, Error> {
    if response.status == 403 {
        return Err(Error::Http { status: 403 });
    }
    if !response.is_success() {
        return Err(Error::Http {
            status: response.status,
        });
    }

    let body = response.json_body().map_err(|error| Error::Parse {
        message: error.to_string(),
    })?;

    match classify(&body) {
        Some(error) => Err(error),
        None => Ok(body),
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
    use crate::mock_transport::MockTransport;
    use crate::transport::HttpMethod;
    use coolall_core::error::CaptchaChallenge;

    /// 测试专用构造：绕过 Tauri AppHandle，不落盘。
    ///
    /// 这正是「缺少 HttpTransport seam」的代价——真实构造需要 GUI 上下文。
    /// 这里只测与 HTTP 无关的纯状态逻辑。
    fn test_client() -> CoolapkClient {
        CoolapkClient::with_test_transport(Arc::new(
            ReqwestTransport::new().expect("创建测试 transport"),
        ))
    }

    fn mock_client(mock: Arc<MockTransport>) -> CoolapkClient {
        CoolapkClient::with_test_transport(mock)
    }

    const CHALLENGE_ID: &str = "abcdef0123456789";

    fn captcha_body(field: &str) -> Value {
        serde_json::json!({
            "messageStatus": "err_request_captcha_v2",
            "messageExtra": {
                "captchaId": CHALLENGE_ID,
                "captchaField": field
            }
        })
    }

    fn success_body() -> Value {
        serde_json::json!({
            "status": 1,
            "data": { "id": 123, "title": "replay succeeded" }
        })
    }

    /// 第一组：普通 200，验证协议头齐全且 SESSID 正确回写。
    #[tokio::test]
    async fn transport_success_reuses_headers_and_updates_sessid() {
        let mock = Arc::new(MockTransport::new(vec![Ok(
            HttpResponse::json(200, success_body())
                .with_header("set-cookie", "SESSID=session-1; Path=/"),
        )]));
        let mut client = mock_client(mock.clone());

        let value = client.detail("123").await.expect("普通 200 应成功");
        assert_eq!(value["data"]["id"], 123);

        let requests = mock.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, HttpMethod::Get);
        assert!(requests[0].url.ends_with("/v6/feed/detail?id=123"));

        // 证明 transport 收到的是 build_headers 生成的完整协议头
        assert!(requests[0].headers.contains_key("x-app-token"));
        assert!(requests[0].headers.contains_key("x-app-device"));
        assert!(requests[0].headers.contains_key("user-agent"));

        let session = client.session.lock().await;
        assert_eq!(session.sessid.as_deref(), Some("session-1"));
    }

    /// 第二组：1004 → QueryParam 验证 → 重放成功。
    #[tokio::test]
    async fn captcha_query_param_validation_replays_original_request() {
        let mock = Arc::new(MockTransport::new(vec![
            Ok(HttpResponse::json(200, captcha_body("_v2_post_token"))),
            Ok(HttpResponse::json(200, success_body())),
        ]));
        let mut client = mock_client(mock.clone());

        let error = client.detail("123").await.expect_err("第一次应要求验证");
        assert!(matches!(error, Error::CaptchaNeeded { .. }));

        let pending = client.pending_captcha.as_ref().expect("应保存原始挑战");
        assert_eq!(
            pending.original_url,
            "https://api.coolapk.com/v6/feed/detail?id=123"
        );

        let value = client
            .submit_validation(CHALLENGE_ID, "_v2_post_token", "proof")
            .await
            .expect("验证后应直接返回重放结果");
        assert_eq!(value["data"]["title"], "replay succeeded");

        let requests = mock.requests();
        assert_eq!(requests.len(), 2, "原请求一次，重放一次");
        assert!(
            requests[1]
                .url
                .contains("_v2_post_token=NEC%3Aabcdef01%3Aproof"),
            "实际重放 URL：{}",
            requests[1].url
        );
        assert!(requests[1].url.contains("id=123&"));

        // 字段模式**不写会话**：凭证只活在这次重放的 URL 里。
        //
        // 这是 2026-10-31 的修正：早期无条件写会话，导致查询参数凭证
        // 被误当成长期 Cookie 复用，而服务端并不认，
        // 表现为「验一次管不了一片请求」。
        let session = client.session.lock().await;
        assert_eq!(
            session.validation, None,
            "字段模式的凭证不应写入会话"
        );
        assert!(!client.replay_guard.may_replay());
    }

    /// 第三组：重放仍失败 → 清除凭证，且不会有第三次请求。
    #[tokio::test]
    async fn rejected_replay_clears_proof_and_does_not_retry_again() {
        let mock = Arc::new(MockTransport::new(vec![
            Ok(HttpResponse::json(200, captcha_body("_v2_post_token"))),
            Ok(HttpResponse::json(
                200,
                serde_json::json!({ "status": 1004, "message": "请求验证错误" }),
            )),
        ]));
        let mut client = mock_client(mock.clone());

        assert!(matches!(
            client.detail("123").await,
            Err(Error::CaptchaNeeded { .. })
        ));

        let error = client
            .submit_validation(CHALLENGE_ID, "_v2_post_token", "bad-proof")
            .await
            .expect_err("重放仍被拒时必须返回错误");
        assert!(error.to_string().contains("验证"), "实际错误：{error}");

        // 每个挑战只消费一次重放机会，不允许隐式第三次请求
        assert_eq!(mock.requests().len(), 2);
        assert!(!client.replay_guard.may_replay());

        let session = client.session.lock().await;
        assert_eq!(session.validation, None, "重放失败后必须清除无效凭证");
    }

    /// 第四组：空 captchaField 走 Cookie 重放，不改 URL。
    #[tokio::test]
    async fn empty_captcha_field_replays_proof_in_cookie() {
        let mock = Arc::new(MockTransport::new(vec![
            Ok(HttpResponse::json(200, captcha_body(""))),
            Ok(HttpResponse::json(200, success_body())),
        ]));
        let mut client = mock_client(mock.clone());

        assert!(matches!(
            client.detail("123").await,
            Err(Error::CaptchaNeeded { .. })
        ));

        client
            .submit_validation(CHALLENGE_ID, "", "cookie-proof")
            .await
            .expect("Cookie 模式重放应成功");

        let requests = mock.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].url,
            "https://api.coolapk.com/v6/feed/detail?id=123",
            "Cookie 模式不得修改原 URL"
        );

        let cookie = requests[1]
            .headers
            .get("cookie")
            .expect("Cookie 模式必须带 Cookie")
            .to_str()
            .expect("Cookie 应为合法文本");
        assert!(
            cookie.contains("validate=NEC%3Aabcdef01%3Acookie-proof"),
            "实际 Cookie：{cookie}"
        );

        // Cookie 模式**应该**写会话：凭证作为 validate Cookie
        // 对后续所有请求生效，这正是「验一次管一片」的基础。
        // 与上一个测试（字段模式不写会话）形成对照。
        let session = client.session.lock().await;
        assert_eq!(
            session.validation.as_deref(),
            Some("NEC%3Aabcdef01%3Acookie-proof"),
            "Cookie 模式的凭证必须写入会话以便复用"
        );
    }

    /// 挑战 ID 不匹配时不得写入凭证，也不得发请求。
    #[tokio::test]
    async fn mismatched_challenge_id_is_rejected_without_request() {
        let mock = Arc::new(MockTransport::new(vec![Ok(HttpResponse::json(
            200,
            captcha_body("_v2_post_token"),
        ))]));
        let mut client = mock_client(mock.clone());

        assert!(client.detail("123").await.is_err());

        let error = client
            .submit_validation("wrong-id", "_v2_post_token", "proof")
            .await
            .expect_err("挑战 ID 不匹配必须拒绝");
        assert!(error.to_string().contains("不匹配"));

        assert_eq!(mock.requests().len(), 1, "拒绝时不应发新请求");
        let session = client.session.lock().await;
        assert_eq!(session.validation, None);
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









