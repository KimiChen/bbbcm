//! 统一错误出口：领域错误码定义在各自 crate 的 `contract.rs`，
//! `apps/web/src/error.rs` 把 `AppError` 映射为 HTTP 状态与错误页。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("资源不存在: {0}")]
    NotFound(String),
    #[error("需要登录")]
    Unauthorized,
    #[error("没有权限: {0}")]
    Forbidden(String),
    #[error("请求无效: {0}")]
    Invalid(String),
    #[error("状态冲突: {0}")]
    Conflict(String),
    /// `If-Match` 或实体版本不匹配：客户端持有的是旧版本（`02` 第 9 节）。
    #[error("版本不匹配: {0}")]
    PreconditionFailed(String),
    /// 高危写缺少幂等键、重新认证或审批引用（`03` 第 9 节）。
    #[error("缺少前置条件: {0}")]
    PreconditionRequired(String),
    /// 触发限流（`DEC-024`）。
    #[error("请求过于频繁: {0}")]
    TooManyRequests(String),
    /// 命令已受理但提交结果未知，客户端须凭幂等键查询（`02` 第 9 节）。
    #[error("已受理，结果待确认: {0}")]
    Accepted(String),
    /// 依赖不可用导致的失败关闭（`02` 第 18 节）。
    #[error("服务暂时不可用: {0}")]
    Unavailable(String),
    #[error("内部错误: {0}")]
    Internal(String),
}

impl AppError {
    /// HTTP 状态码；web 层据此映射页面或 JSON 响应。
    pub fn http_status(&self) -> u16 {
        match self {
            AppError::Accepted(_) => 202,
            AppError::Invalid(_) => 400,
            AppError::Unauthorized => 401,
            AppError::Forbidden(_) => 403,
            AppError::NotFound(_) => 404,
            AppError::Conflict(_) => 409,
            AppError::PreconditionFailed(_) => 412,
            AppError::PreconditionRequired(_) => 428,
            AppError::TooManyRequests(_) => 429,
            AppError::Internal(_) => 500,
            AppError::Unavailable(_) => 503,
        }
    }

    /// 可安全展示给客户端的文案。
    ///
    /// `Internal` 与 `Unavailable` 的原文可能含连接串、SQL 片段或内部路径，
    /// 一律替换为固定文案；原文只进日志。
    pub fn public_message(&self) -> &str {
        match self {
            AppError::Accepted(_) => "请求已受理，请凭幂等键查询结果",
            AppError::Invalid(msg)
            | AppError::Forbidden(msg)
            | AppError::NotFound(msg)
            | AppError::Conflict(msg)
            | AppError::PreconditionFailed(msg)
            | AppError::PreconditionRequired(msg)
            | AppError::TooManyRequests(msg) => msg,
            AppError::Unauthorized => "需要登录",
            AppError::Unavailable(_) => "服务暂时不可用，请稍后重试",
            AppError::Internal(_) => "服务出现内部错误，请稍后重试",
        }
    }

    /// 该错误的原文是否只能进日志。
    pub fn is_internal(&self) -> bool {
        matches!(self, AppError::Internal(_) | AppError::Unavailable(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_cover_the_contract() {
        let cases = [
            (AppError::Accepted("x".into()), 202),
            (AppError::Invalid("x".into()), 400),
            (AppError::Unauthorized, 401),
            (AppError::Forbidden("x".into()), 403),
            (AppError::NotFound("x".into()), 404),
            (AppError::Conflict("x".into()), 409),
            (AppError::PreconditionFailed("x".into()), 412),
            (AppError::PreconditionRequired("x".into()), 428),
            (AppError::TooManyRequests("x".into()), 429),
            (AppError::Internal("x".into()), 500),
            (AppError::Unavailable("x".into()), 503),
        ];
        for (err, want) in cases {
            assert_eq!(err.http_status(), want, "{err}");
        }
    }

    #[test]
    fn internal_detail_never_reaches_the_client() {
        let secret = "postgres://user:pw@10.0.0.1/db";
        for err in [
            AppError::Internal(secret.into()),
            AppError::Unavailable(secret.into()),
        ] {
            assert!(err.is_internal());
            assert!(
                !err.public_message().contains(secret),
                "对外文案不得泄露内部细节: {}",
                err.public_message()
            );
        }
    }

    #[test]
    fn client_errors_keep_their_message() {
        let err = AppError::NotFound("主题不存在".into());
        assert_eq!(err.public_message(), "主题不存在");
        assert!(!err.is_internal());
    }
}
