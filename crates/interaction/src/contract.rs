//! `WP-MVP-C04` 互动领域的共享合同。本轮只交付 `SOCIAL-002`／`SOCIAL-003`
//! 的正向反馈，收藏、关注、阅读位置与通知随后续切片接入。
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InteractionError {
    #[error("互动输入无效")]
    InvalidInput,
    #[error("不能对自己的内容做出反馈")]
    SelfFeedback,
    #[error("目标内容不存在或不可访问")]
    NotFound,
    #[error("互动状态已变更，请重新载入")]
    Conflict,
    #[error("服务暂不可用")]
    Unavailable,
}

impl From<platform_db::transaction::TransactionError> for InteractionError {
    fn from(value: platform_db::transaction::TransactionError) -> Self {
        use platform::transaction::DbErrorClass;
        use platform_db::transaction::TransactionError as T;
        match value {
            // 唯一约束命中就是「已经反馈过」，属正常业务结果而非故障。
            T::Database(DbErrorClass::UniqueConflict) | T::VersionConflict => Self::Conflict,
            T::InvalidInput => Self::InvalidInput,
            _ => Self::Unavailable,
        }
    }
}

/// 反馈对象类型。封闭枚举而不是自由文本：类型要参与计数口径与后续
/// `TOPIC-009`「这些回复值得看」的阈值，自由文本会让两者失控。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackTarget {
    Topic,
    Post,
}

impl FeedbackTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::Post => "post",
        }
    }
}

/// 一次反馈操作的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedbackOutcome {
    /// 该对象当前的有效反馈数（已撤销的不计入）。
    pub count: i64,
    /// 本次调用后，当前用户对该对象是否处于「已反馈」状态。
    pub active: bool,
    /// 真表示本次调用确实改变了状态；假表示状态本就如此。
    ///
    /// 区分这两者是为了让重复点击成为**幂等**而不是错误：用户连点两下
    /// 不该看到报错，但调用方需要知道有没有真的发生变化（例如是否发通知）。
    pub changed: bool,
}
