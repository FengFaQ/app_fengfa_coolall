//! 核心深模块：把签名、设备、请求头、会话、验证码、错误分类收拢在一个接口后面。
//!
//! 接口只有 5 个方法（browse / detail / replies / search / session_info），
//! 调用者不需要知道 TokenV3、设备编码、Cookie 维护的任何细节。
//! 设计依据见 `docs/ARCHITECTURE.md` §2.1。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::device::{self, constants, DeviceIdentity, DeviceProfile};
use crate::error::{classify, Error};
use crate::request::{build_headers, RequestContext, Session};

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
            Self::Home => format!(
                "/v6/main/indexV8?page={page}&firstLaunch=0&installTime=0"
            ),
            Self::Feed => format!("/v6/feed/list?page={page}"),
        }
    }
}

/// 持久化的身份配置。只存必要的几项，其余每次启动重新生成。
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
    /// 当前待完成的验证挑战
    pending_challenge: Option<crate::error::CaptchaChallenge>,
    store_path: Option<std::path::PathBuf>,
}

impl CoolapkClient {
    /// 创建客户端。会尝试从本地读取身份配置，不存在则生成随机 szlmId。
    pub fn new<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<Self, Box<dyn std::error::Error>> {
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
            _ => DeviceIdentity::new(
                DeviceIdentity::random_szlm_id(),
                DeviceProfile::default(),
            ),
        };

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent("coolall/0.1.0")
            .build()?;

        Ok(Self {
            http,
            identity,
            session: Arc::new(Mutex::new(Session::default())),
            pending_challenge: None,
            store_path,
        })
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

        // 无论成功失败都要回写 SESSID
        {
            let mut guard = self.session.lock().await;
            guard.update_from_set_cookie(&set_cookies);
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

    pub async fn browse(&mut self, target: &str, page: u32) -> Result<Value, Error> {
        let t = BrowseTarget::parse(target).ok_or_else(|| Error::Parse {
            message: format!("未知浏览目标: {target}"),
        })?;
        let result = self.send(t.base_url(), &t.path(page.max(1))).await;
        self.track_challenge(&result);
        result
    }

    pub async fn search(&mut self, keyword: &str, page: u32) -> Result<Value, Error> {
        let kw = urlencoding::encode(keyword);
        let path = format!(
            "/v6/search?type=feed&feedType=feed&sort=default&searchValue={kw}&page={}&showAnonymous=-1",
            page.max(1)
        );
        let result = self.send(constants::BASE_URL, &path).await;
        self.track_challenge(&result);
        result
    }

    pub async fn replies(&mut self, id: &str, page: u32) -> Result<Value, Error> {
        let iid = urlencoding::encode(id);
        let path = format!(
            "/v6/feed/replyList?id={iid}&listType=&page={}&discussMode=0&feedType=feed_reply&blockStatus=0&fromFeedAuthor=0",
            page.max(1)
        );
        let result = self.send(constants::BASE_URL_2, &path).await;
        self.track_challenge(&result);
        result
    }

    /// 详情。API 失败且 id 为纯数字时，降级到公开分享页。
    pub async fn detail(&mut self, id: &str) -> Result<Value, Error> {
        let iid = urlencoding::encode(id);
        let path = format!("/v6/feed/detail?id={iid}");
        let result = self.send(constants::BASE_URL, &path).await;
        match result {
            Ok(v) => Ok(v),
            Err(primary) => {
                if id.chars().all(|c| c.is_ascii_digit()) {
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

    fn track_challenge(&mut self, result: &Result<Value, Error>) {
        if let Err(Error::CaptchaNeeded { challenge }) = result {
            self.pending_challenge = Some(challenge.clone());
        }
    }

    pub fn session_info(&self) -> Value {
        let pending = self.pending_challenge.as_ref();
        serde_json::json!({
            "challengeId": pending.map(|c| c.id.clone()),
            "challengeField": pending.map(|c| c.field.clone()),
            "szlmId": self.identity.szlm_id,
            "userAgent": self.identity.user_agent(),
        })
    }

    /// 接收前端完成验证后回传的 validate。
    ///
    /// 这里只处理**同步部分**：拼装 proof 并写入会话。
    /// 备注：由于 Mutex 为异步锁，实际写入由调用方在 async 上下文中完成。
    pub fn submit_validation(&mut self, challenge_id: &str, field: &str, validate: &str) -> bool {
        let challenge = crate::error::CaptchaChallenge {
            id: challenge_id.to_string(),
            field: field.to_string(),
        };
        let Some(proof) = challenge.proof(validate) else {
            return false;
        };
        // 用阻塞方式拿到锁：该命令本身是同步命令，调用频率极低
        if let Ok(mut guard) = self.session.try_lock() {
            guard.accept_validation(proof);
            true
        } else {
            false
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
