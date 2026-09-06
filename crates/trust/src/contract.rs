//! Minimal, versioned identity trust facts. Invitation eligibility never promotes a trust level.
use chrono::{DateTime, Utc};

pub const INVITATION_POLICY_VERSION: i64 = 1;

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
