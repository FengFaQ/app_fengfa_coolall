//! 验证码流程：识别挑战、拼装凭证、控制重放。
//!
//! 协议依据见 `docs/PROTOCOL.md` §9。
//!
//! # 为什么这个模块存在
//!
//! 游客态请求会稳定返回 `status: 1004`，这是服务端的强制人机验证。
//! 应用能否真正使用，取决于这条链路是否完整：
//!
//! ```text
//! 原请求 → 1004/err_request_captcha_v2
//!        → 返回 CaptchaNeeded{challenge} 给前端
//!        → 前端打开验证窗口，用户完成验证
//!        → 前端回传 validate
//!        → 模块拼装 proof（URL 编码的 NEC:id:validate）
//!        → 带凭证**重放一次**原请求
//!        → 成功：保留凭证；失败：清除凭证
//! ```
//!
//! 关键约束：**只重放一次**。传输错误不触发重放。

use serde::{Deserialize, Serialize};

/// 重放策略。
///
/// 凭证有两种携带方式，取决于服务端下发的 `captchaField`：
/// - `field` 为空 → 作为 `validate` Cookie
/// - `field` 非空 → 作为该字段的查询参数附加到原 URL
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofPlacement {
    /// 放进 Cookie: validate=...
    Cookie,
    /// 放进 URL 查询参数
    QueryParam,
}

/// 一次待完成的验证。
#[derive(Debug, Clone)]
pub struct PendingCaptcha {
    /// 挑战 ID，用于校验前端回传的是否为同一次挑战
    pub id: String,
    /// 服务端要求的字段名，空字符串表示按 Cookie 处理
    pub field: String,
    /// 原始请求的完整 URL，重放用
    pub original_url: String,
}

impl PendingCaptcha {
    pub fn placement(&self) -> ProofPlacement {
        if self.field.trim().is_empty() {
            ProofPlacement::Cookie
        } else {
            ProofPlacement::QueryParam
        }
    }

    /// 校验前端回传的 challenge_id 是否与当前待办一致。
    ///
    /// 必须校验：否则旧窗口或错误参数会污染新会话。
    pub fn matches(&self, challenge_id: &str) -> bool {
        self.id == challenge_id
    }

    /// 把 proof 附加到原 URL 上（仅 QueryParam 模式）。
    pub fn url_with_proof(&self, proof: &str) -> String {
        let sep = if self.original_url.contains('?') { '&' } else { '?' };
        format!(
            "{}{}{}={}",
            self.original_url,
            sep,
            urlencoding::encode(&self.field),
            proof
        )
    }
}

/// 重放结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayOutcome {
    /// 重放成功，凭证有效，保留
    Accepted,
    /// 重放仍被拒，凭证应清除
    Rejected,
}

/// 重放决策。
///
/// 封装「只重放一次」这条规则，避免调用方各写一遍。
#[derive(Debug, Default)]
pub struct ReplayGuard {
    used: bool,
}

impl ReplayGuard {
    /// 是否还能重放。每个验证流程只允许一次。
    pub fn may_replay(&self) -> bool {
        !self.used
    }

    /// 标记已用掉这次重放机会。
    pub fn consume(&mut self) {
        self.used = true;
    }

    /// 开新一轮（用户重新验证成功后调用）。
    pub fn reset(&mut self) {
        self.used = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(field: &str) -> PendingCaptcha {
        PendingCaptcha {
            id: "414e5c9b866a03db03f860a1a9101672".into(),
            field: field.into(),
            original_url: "https://api.coolapk.com/v6/feed/detail?id=123".into(),
        }
    }

    #[test]
    fn empty_field_means_cookie() {
        assert_eq!(pending("").placement(), ProofPlacement::Cookie);
        assert_eq!(pending("   ").placement(), ProofPlacement::Cookie);
    }

    #[test]
    fn non_empty_field_means_query_param() {
        assert_eq!(
            pending("_v2_post_token").placement(),
            ProofPlacement::QueryParam
        );
    }

    #[test]
    fn url_gets_query_param_appended() {
        let p = pending("_v2_post_token");
        let url = p.url_with_proof("NEC%3A414e5c9b%3Aabc");
        assert!(url.contains("_v2_post_token=NEC%3A414e5c9b%3Aabc"));
        assert!(url.contains("id=123&"), "已有查询参数时应追加");
    }

    #[test]
    fn url_without_query_gets_question_mark() {
        let mut p = pending("_v2_post_token");
        p.original_url = "https://api.coolapk.com/v6/feed/detail".into();
        let url = p.url_with_proof("x");
        assert!(url.contains("?"));
    }

    #[test]
    fn challenge_id_must_match() {
        let p = pending("");
        assert!(p.matches("414e5c9b866a03db03f860a1a9101672"));
        assert!(!p.matches("something-else"), "不匹配的挑战 ID 必须拒绝");
    }

    #[test]
    fn replay_only_once() {
        let mut g = ReplayGuard::default();
        assert!(g.may_replay());
        g.consume();
        assert!(!g.may_replay(), "用过一次后不得再重放");
        g.reset();
        assert!(g.may_replay(), "重新验证后应允许再试");
    }
}
