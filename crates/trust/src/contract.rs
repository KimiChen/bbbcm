//! Minimal, versioned identity trust facts. Invitation eligibility never promotes a trust level.
use chrono::{DateTime, Utc};

pub const INVITATION_POLICY_VERSION: i64 = 1;
/// DEC-054 信任等级规则的版本号。
///
/// 贡献事实行会记录评估时的规则版本，规则取值一旦变化必须同步递增本常量，
/// 以便事后能区分「某条贡献是按哪一版规则计入的」。
pub const TIER_RULE_VERSION: i64 = 1;

/// 评估 T1 确定性规则所需、且 trust 自身读不到的外部事实。
///
/// 领域边界：规则与版本号属于 trust，而「该账号是否存在生效处罚」属于 moderation。
/// trust 不允许直接读其他领域的仓储，因此由应用装配层查询后经本结构注入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierInputs {
    /// An active, unexpired moderation penalty on the account, read by the caller.
    pub active_penalty: bool,
}

/// 一条已发布且合格的贡献类型。
///
/// 合格与否在内容领域按 DEC-054 判定后才会到达 trust：主题看正文长度，回复还要求
/// 发布者不是主题作者。字符数一律按 Unicode 标量计（DEC-055：1 标量 = 1 字符，
/// 不做中日韩宽度折算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContributionKind {
    Topic,
    Reply,
}
impl ContributionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::Reply => "reply",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    Pending,
    T0,
    T1,
}
impl TrustLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::T0 => "t0",
            Self::T1 => "t1",
        }
    }
    pub(crate) fn parse(value: &str) -> Result<Self, TrustError> {
        match value {
            "pending" => Ok(Self::Pending),
            "t0" => Ok(Self::T0),
            "t1" => Ok(Self::T1),
            _ => Err(TrustError::CorruptData),
        }
    }
}

#[derive(Clone)]
pub struct TrustSnapshot {
    pub(crate) id: i64,
    pub(crate) user_id: i64,
    pub(crate) level: TrustLevel,
    pub(crate) email_verified: bool,
    pub(crate) invitation_allowed: bool,
    pub(crate) policy_version: i64,
    pub(crate) reason_code: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) version: i64,
}
impl std::fmt::Debug for TrustSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustSnapshot")
            .field("level", &self.level)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
impl TrustSnapshot {
    pub fn user_id(&self) -> i64 {
        self.user_id
    }
    pub fn level(&self) -> TrustLevel {
        self.level
    }
    pub fn email_verified(&self) -> bool {
        self.email_verified
    }
    pub fn invitation_allowed(&self) -> bool {
        self.invitation_allowed
    }
    pub fn policy_version(&self) -> i64 {
        self.policy_version
    }
    pub fn reason_code(&self) -> &str {
        &self.reason_code
    }
    pub fn version(&self) -> i64 {
        self.version
    }
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvitationAuthorization {
    Allowed { policy_version: i64 },
    EmailUnverified,
    NotGranted,
}
impl InvitationAuthorization {
    pub fn allowed(self) -> bool {
        matches!(self, Self::Allowed { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TrustError {
    #[error("信任上下文无效")]
    InvalidInput,
    #[error("信任事实不存在")]
    NotFound,
    #[error("信任事实冲突")]
    Conflict,
    #[error("信任事实不可用")]
    Unavailable,
    #[error("信任事实损坏")]
    CorruptData,
}
impl From<platform_db::transaction::TransactionError> for TrustError {
    fn from(error: platform_db::transaction::TransactionError) -> Self {
        use platform_db::transaction::TransactionError;
        match error {
            TransactionError::Database(platform::transaction::DbErrorClass::UniqueConflict)
            | TransactionError::VersionConflict => Self::Conflict,
            TransactionError::InvalidInput => Self::InvalidInput,
            TransactionError::CorruptData => Self::CorruptData,
            _ => Self::Unavailable,
        }
    }
}
