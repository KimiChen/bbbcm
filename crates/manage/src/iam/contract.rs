//! IAM 领域的错误。
use platform_db::transaction::TransactionError;

/// 对外一律给出统一语义，不透露「失败在哪一步」——枚举错误信息就能反推角色与范围归属。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IamError {
    #[error("未授权")]
    Denied,
    #[error("输入无效")]
    InvalidInput,
    #[error("角色或权限目录与 registry 不一致")]
    RegistryMismatch,
    #[error("数据已变更，请重新载入")]
    Conflict,
    #[error("服务暂不可用")]
    Unavailable,
}
impl From<TransactionError> for IamError {
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
