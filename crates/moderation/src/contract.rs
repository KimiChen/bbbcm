//! The MOD-010 identity slice only reads immutable consent versions.
use platform_db::transaction::TransactionError;

/// 治理领域的错误。
///
/// 对外一律给出统一语义：举报接口不因「目标不存在」与「目标不可见」返回不同错误，
/// 否则举报入口会变成一个探测内容是否存在的接口。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModerationError {
    #[error("举报或案件不存在")]
    NotFound,
    #[error("输入无效")]
    InvalidInput,
    #[error("服务暂不可用")]
    Unavailable,
    #[error("数据已变更，请重新载入")]
    Conflict,
}
impl From<TransactionError> for ModerationError {
    fn from(value: TransactionError) -> Self {
        match value {
            TransactionError::InvalidInput => Self::InvalidInput,
            TransactionError::VersionConflict | TransactionError::IdempotencyConflict => {
                Self::Conflict
            }
            _ => Self::Unavailable,
        }
    }
}

pub use platform::policy::{PolicyError, PolicySet, PolicyVersion};

/// Closed content-only restriction scope, independent of unapproved staff permissions.
#[derive(Debug, Clone, Copy)]
pub enum ContentCapability {
    Topic,
    Reply,
    Edit,
    Delete,
}
impl ContentCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::Reply => "reply",
            Self::Edit => "edit",
            Self::Delete => "delete",
        }
    }
}
