//! 身份合同。秘密不实现 Serialize；Debug 永不显示凭证或私人资料。
use chrono::{DateTime, Utc};
use std::sync::Arc;

pub const SESSION_ABSOLUTE_DAYS: i64 = 30;
pub const SESSION_IDLE_DAYS: i64 = 14;
pub const REAUTH_SECONDS: i64 = 300;
pub const USERNAME_COOLDOWN_DAYS: i64 = 30;
pub const USERNAME_RESERVATION_DAYS: i64 = 90;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("请求未能完成，请检查输入后重试")]
    Rejected,
    #[error("需要登录")]
    Unauthenticated,
    #[error("需要重新验证密码")]
    ReauthenticationRequired,
    #[error("请求校验失败")]
    Csrf,
    #[error("产品策略尚未批准")]
    PolicyPending,
    #[error("功能暂未开放")]
    Disabled,
    #[error("请求过于频繁")]
    RateLimited,
    #[error("服务暂不可用")]
    Unavailable,
    #[error("需要完成本地测试挑战")]
    ChallengeRequired { challenge_id: String },
    #[error("提交结果待确认，请使用原请求查询")]
    CommitUnknown { operation_id: String },
}
impl From<platform_db::transaction::TransactionError> for IdentityError {
    fn from(value: platform_db::transaction::TransactionError) -> Self {
        match value {
            platform_db::transaction::TransactionError::CommitUnknown => Self::Unavailable,
            platform_db::transaction::TransactionError::Database(
                platform::transaction::DbErrorClass::UniqueConflict,
            )
            | platform_db::transaction::TransactionError::InvalidInput
            | platform_db::transaction::TransactionError::VersionConflict
            | platform_db::transaction::TransactionError::IdempotencyConflict => Self::Rejected,
            _ => Self::Unavailable,
        }
    }
}
#[derive(Clone)]
pub struct Secret(pub(crate) String);
impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone)]
pub struct NormalizedIdentity {
    pub display: String,
    pub key: String,
}
/// 产品待决参数只由装配层注入，不能由请求提供。无生产默认。
pub trait IdentityPolicy: Send + Sync {
    fn username(&self, input: &str) -> Result<NormalizedIdentity, IdentityError>;
    fn email(&self, input: &str) -> Result<NormalizedIdentity, IdentityError>;
    fn bio(&self, input: &str) -> Result<String, IdentityError>;
    fn preauth_expires_at(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>, IdentityError>;
    fn email_change_expires_at(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>, IdentityError>;
    fn invitation_window(&self, now: DateTime<Utc>) -> Result<InvitationWindow, IdentityError>;
}
pub struct UnapprovedIdentityPolicy;
impl IdentityPolicy for UnapprovedIdentityPolicy {
    fn username(&self, _: &str) -> Result<NormalizedIdentity, IdentityError> {
        Err(IdentityError::PolicyPending)
    }
    fn email(&self, _: &str) -> Result<NormalizedIdentity, IdentityError> {
        Err(IdentityError::PolicyPending)
    }
    fn bio(&self, _: &str) -> Result<String, IdentityError> {
        Err(IdentityError::PolicyPending)
    }
    fn preauth_expires_at(&self, _: DateTime<Utc>) -> Result<DateTime<Utc>, IdentityError> {
        Err(IdentityError::PolicyPending)
    }
    fn email_change_expires_at(&self, _: DateTime<Utc>) -> Result<DateTime<Utc>, IdentityError> {
        Err(IdentityError::PolicyPending)
    }
    fn invitation_window(&self, _: DateTime<Utc>) -> Result<InvitationWindow, IdentityError> {
        Err(IdentityError::PolicyPending)
    }
}
#[derive(Clone)]
pub struct InvitationWindow {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub daily_limit: i64,
    pub unused_limit: i64,
}
#[derive(Clone)]
pub struct RequestContext {
    pub preauth: Secret,
    pub csrf: Secret,
    pub idempotency_key: String,
    pub(crate) risk_signals: trust::risk::RiskSignals,
    pub(crate) challenge_required: bool,
    pub(crate) challenge_submission: Option<crate::challenge::ChallengeSubmission>,
    pub(crate) hard_rate_limited: bool,
}
impl RequestContext {
    pub fn new(preauth: Secret, csrf: Secret, idempotency_key: String) -> Self {
        Self {
            preauth,
            csrf,
            idempotency_key,
            risk_signals: Default::default(),
            challenge_required: false,
            challenge_submission: None,
            hard_rate_limited: false,
        }
    }
    /// Only the trusted request adapter supplies keyed, purpose-separated digests.
    pub fn with_risk_signals(mut self, signals: trust::risk::RiskSignals) -> Self {
        self.risk_signals = signals;
        self
    }
    /// The HTTP adapter supplies the independently evaluated soft rate policy.
    /// A submitted proof is still bound and consumed by core in the command transaction.
    pub fn with_challenge(
        mut self,
        required: bool,
        submission: Option<crate::challenge::ChallengeSubmission>,
    ) -> Self {
        self.challenge_required = required;
        self.challenge_submission = submission;
        self
    }
    /// A trusted transport result only. Core still issues the same public challenge
    /// and then terminates without business writes when a hard budget is exhausted.
    pub fn with_hard_rate_limited(mut self, limited: bool) -> Self {
        self.hard_rate_limited = limited;
        self
    }
}
#[derive(Debug)]
pub struct PreauthGrant {
    pub token: Secret,
    pub csrf: Secret,
}
#[derive(Clone)]
pub struct RegistrationInput {
    pub username: String,
    pub email: String,
    pub password: Secret,
    pub invitation: Secret,
    pub policies: Vec<(String, String)>,
}
#[derive(Clone)]
pub struct LoginInput {
    pub identifier: String,
    pub password: Secret,
    pub device_summary: String,
}
#[derive(Debug)]
pub struct SessionGrant {
    pub token: Secret,
    pub csrf: Secret,
    pub session_ref: String,
}
#[derive(Debug)]
pub struct LoginResult {
    pub grant: Option<SessionGrant>,
    pub replayed: bool,
}
#[derive(Debug, Clone)]
pub struct MutationResult {
    pub resource_ref: Option<String>,
    pub replayed: bool,
}

/// 管理登录成功的结果（`03` §6）。
///
/// 原始令牌只在这里出现一次，由调用方立即写进独立的管理 cookie；
/// 库中只存哈希。`bound` 为假时调用方必须把用户引到 TOTP 绑定流程（`AUTH-014`）。
pub struct ManagementLoginResult {
    pub token: Secret,
    pub csrf: Secret,
    pub bound: bool,
}
impl std::fmt::Debug for ManagementLoginResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagementLoginResult")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}
/// 字段私有：只有数据库重读且检查有效的core能构造身份；客户端不能声明owner。
#[derive(Clone)]
pub struct SessionIdentity {
    pub(crate) user_id: i64,
    pub(crate) session_id: i64,
    pub(crate) session_ref: String,
    pub(crate) credential_version: i64,
    pub(crate) csrf_hash: String,
    pub(crate) reauthenticated_at: DateTime<Utc>,
}
impl SessionIdentity {
    pub fn user_id(&self) -> i64 {
        self.user_id
    }
    pub fn reauthenticated_at(&self) -> DateTime<Utc> {
        self.reauthenticated_at
    }
    pub fn session_ref(&self) -> &str {
        &self.session_ref
    }
}
impl std::fmt::Debug for SessionIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionIdentity([REDACTED])")
    }
}
#[derive(Clone)]
pub struct Account {
    pub id: i64,
    pub public_id: String,
    pub username: String,
    pub username_key: String,
    pub email: String,
    pub email_key: String,
    pub status: String,
    pub email_verified: bool,
    pub bio: String,
    pub created_at: DateTime<Utc>,
    pub username_changed_at: DateTime<Utc>,
    pub credential_version: i64,
    /// PRIV-001：个人主页的主题/回帖列表是否对未登录访客可见。DEC-054 默认公开。
    pub profile_lists_public: bool,
    /// PRIV-002：本人主题正文是否对未登录访客可见。DEC-054 默认公开，标题另按规则保留。
    pub topic_body_public: bool,
    pub version: i64,
}
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Account([REDACTED])")
    }
}
#[derive(Debug, Clone)]
pub struct InvitationRecord {
    pub invitation_ref: String,
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub used: bool,
}
#[derive(Debug)]
pub struct InvitationGrant {
    pub code: Secret,
    pub invitation_ref: String,
}
#[derive(Debug)]
pub struct InvitationPage {
    pub generated_today: i64,
    pub unused: i64,
    pub daily_limit: i64,
    pub unused_limit: i64,
    pub records: Vec<InvitationRecord>,
}

/// 泄露密码检查保持显式可注入；DEC-011/CR-014未批准时不连接供应商。
pub trait PasswordScreen: Send + Sync {
    fn allowed(&self, password: &str) -> Result<bool, IdentityError>;
}
pub struct DisabledPasswordScreen;
impl PasswordScreen for DisabledPasswordScreen {
    fn allowed(&self, _: &str) -> Result<bool, IdentityError> {
        Ok(true)
    }
}
pub type SharedPolicy = Arc<dyn IdentityPolicy>;
