//! HTTP 传输 seam。
//!
//! 生产环境使用 [`ReqwestTransport`]；测试使用脚本化的 `MockTransport`。
//! `client.rs` 只依赖这里的小接口，不再直接依赖 `reqwest::Response`。
//!
//! # 为什么手写 `BoxFuture` 而不引入 async-trait
//!
//! `CoolapkClient` 需要保存 `Arc<dyn HttpTransport>`。
//! Rust 原生 `async fn` trait 目前不是 dyn-compatible，无法直接写成
//! `Arc<dyn HttpTransport>`。手写只需两行类型定义，不必为此增加依赖。
//!
//! # 为什么响应体是 `Vec<u8>` 而不是 `Value`
//!
//! `detail()` 的公开页兜底需要的是 HTML，不是 JSON。
//! 若 seam 只支持 JSON，兜底链路会被迫重新直接依赖 reqwest。

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::header::HeaderMap;
use serde_json::Value;

pub type TransportResult = Result<HttpResponse, TransportError>;

/// 必须带 `+ Send`：Tauri 的 async command 要求 future 可跨线程。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    fn as_reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: HeaderMap,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

impl HttpRequest {
    pub fn get(url: impl Into<String>, headers: HeaderMap) -> Self {
        Self {
            method: HttpMethod::Get,
            url: url.into(),
            headers,
            body: None,
            timeout: Duration::from_secs(20),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    /// 用 Vec 而非 HashMap：同一响应可能有多个 `Set-Cookie`，
    /// 用 Map 会让同名头互相覆盖。
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: serde_json::to_vec(&value).expect("serde_json::Value 一定能序列化"),
        }
    }

    pub fn text(status: u16, text: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: text.into().into_bytes(),
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, value)| value.clone())
            .collect()
    }

    pub fn json_body(&self) -> Result<Value, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    pub fn text_body(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Debug, Clone)]
pub struct TransportError {
    pub message: String,
}

impl TransportError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub trait HttpTransport: Send + Sync {
    fn execute<'a>(&'a self, request: HttpRequest) -> BoxFuture<'a, TransportResult>;
}

/// 生产 adapter。
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            // 不设默认 UA：UA 由 request::build_headers 统一提供。
            // 超时由 HttpRequest.timeout 逐请求指定。
            .build()?;
        Ok(Self { client })
    }
}

impl HttpTransport for ReqwestTransport {
    fn execute<'a>(&'a self, request: HttpRequest) -> BoxFuture<'a, TransportResult> {
        Box::pin(async move {
            let mut builder = self
                .client
                .request(request.method.as_reqwest(), &request.url)
                .headers(request.headers)
                .timeout(request.timeout);

            if let Some(body) = request.body {
                builder = builder.body(body);
            }

            let response = builder
                .send()
                .await
                .map_err(|error| TransportError::new(error.to_string()))?;

            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.as_str().to_string(), value.to_string()))
                })
                .collect();

            let body = response
                .bytes()
                .await
                .map_err(|error| TransportError::new(error.to_string()))?;

            Ok(HttpResponse {
                status,
                headers,
                body: body.to_vec(),
            })
        })
    }
}
