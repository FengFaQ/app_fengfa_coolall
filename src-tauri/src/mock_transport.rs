//! 脚本化 HTTP transport，仅在 `cargo test` 时编译。
//!
//! 让完整的「1004 → 验证 → 重放」链路可以在不联网的情况下被测试。
//!
//! 注意：内部用 `std::sync::Mutex` 是安全的——锁内只做 push/pop/clone，
//! 没有跨 `.await` 持锁。

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::transport::{BoxFuture, HttpRequest, HttpTransport, TransportResult};

#[derive(Default)]
pub struct MockTransport {
    responses: Mutex<VecDeque<TransportResult>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl MockTransport {
    pub fn new(responses: impl IntoIterator<Item = TransportResult>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// 已发出的全部请求，供断言使用。
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .expect("mock requests lock")
            .clone()
    }
}

impl HttpTransport for MockTransport {
    fn execute<'a>(&'a self, request: HttpRequest) -> BoxFuture<'a, TransportResult> {
        Box::pin(async move {
            self.requests
                .lock()
                .expect("mock requests lock")
                .push(request);

            self.responses
                .lock()
                .expect("mock responses lock")
                .pop_front()
                .expect("MockTransport 没有剩余响应")
        })
    }
}
