//! Content-owned contracts. No identity/trust implementation or HTTP claims grant authority.
use chrono::{DateTime, Utc};
use platform_db::transaction::{TransactionConnection, TransactionError};
use serde::{Deserialize, Serialize};
use std::{any::Any, future::Future, pin::Pin, sync::Arc};

pub type FutureResult<'a, T> = Pin<Box<dyn Future<Output = Result<T, ContentError>> + Send + 'a>>;
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContentError {
    #[error("内容不存在或不可访问")]
    NotFound,
    #[error("当前账号不能执行此操作")]
    Forbidden,
    #[error("内容输入无效")]
    InvalidInput,
    #[error("内容已经变更，请重新载入")]
    Conflict,
    #[error("内容政策尚未批准或配置")]
    PolicyPending,
    #[error("发布额度已用完")]
    QuotaExceeded,
    #[error("请求过于频繁")]
    RateLimited,
    #[error("数据库事务暂时繁忙")]
    RetryableDatabase(platform::transaction::DbErrorClass),
    #[error("服务暂不可用")]
    Unavailable,
    #[error("分页凭据无效，请从第一页重新开始")]
    InvalidCursor,
    #[error("提交结果未知，请查询原操作")]
    CommitUnknown { operation_id: String },
}
impl From<TransactionError> for ContentError {
    fn from(value: TransactionError) -> Self {
        match value {
            TransactionError::Database(
                class @ (platform::transaction::DbErrorClass::Serialization
                | platform::transaction::DbErrorClass::Deadlock
                | platform::transaction::DbErrorClass::Busy
                | platform::transaction::DbErrorClass::BusySnapshot),
            ) => Self::RetryableDatabase(class),
            TransactionError::VersionConflict
            | TransactionError::IdempotencyConflict
            | TransactionError::Database(platform::transaction::DbErrorClass::UniqueConflict) => {
                Self::Conflict
            }
            TransactionError::InvalidInput => Self::InvalidInput,
            _ => Self::Unavailable,
        }
    }
}
#[derive(Clone)]
pub struct SecretText(String);
impl SecretText {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for SecretText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretText([REDACTED])")
    }
}
#[derive(Clone)]
pub struct Credentials {
    pub session_token: SecretText,
    pub csrf: SecretText,
    pub idempotency_key: String,
    /// Only trusted application assembly supplies a typed server request context.
    pub trusted_context: Option<Arc<dyn Any + Send + Sync>>,
}
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials([REDACTED])")
    }
}
impl Credentials {
    pub fn new(
        session: impl Into<String>,
        csrf: impl Into<String>,
        key: impl Into<String>,
    ) -> Self {
        Self {
            session_token: SecretText::new(session),
            csrf: SecretText::new(csrf),
            idempotency_key: key.into(),
            trusted_context: None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Members,
}
impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Members => "members",
        }
    }
    pub fn parse(s: &str) -> Result<Self, ContentError> {
        match s {
            "public" => Ok(Self::Public),
            "members" => Ok(Self::Members),
            _ => Err(ContentError::InvalidInput),
        }
    }
    pub fn permits(self, viewer: &Viewer) -> bool {
        self == Self::Public || viewer.member
    }
    pub fn strictest(self, other: Self) -> Self {
        if self == Self::Members || other == Self::Members {
            Self::Members
        } else {
            Self::Public
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Publication {
    Draft,
    Pending,
    Published,
}
impl Publication {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Pending => "pending",
            Self::Published => "published",
        }
    }
    pub fn parse(s: &str) -> Result<Self, ContentError> {
        match s {
            "draft" => Ok(Self::Draft),
            "pending" => Ok(Self::Pending),
            "published" => Ok(Self::Published),
            _ => Err(ContentError::InvalidInput),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Moderation {
    Normal,
    Obscured,
    Hidden,
}
impl Moderation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Obscured => "obscured",
            Self::Hidden => "hidden",
        }
    }
    pub fn parse(s: &str) -> Result<Self, ContentError> {
        match s {
            "normal" => Ok(Self::Normal),
            "obscured" => Ok(Self::Obscured),
            "hidden" => Ok(Self::Hidden),
            _ => Err(ContentError::InvalidInput),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentState {
    pub publication: Publication,
    pub moderation: Moderation,
    pub locked: bool,
    pub visibility: Visibility,
    pub deleted_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
}
impl ContentState {
    pub fn published(visibility: Visibility) -> Self {
        Self {
            publication: Publication::Published,
            moderation: Moderation::Normal,
            locked: false,
            visibility,
            deleted_at: None,
            archived_at: None,
        }
    }
    /// 主题级可读性。
    ///
    /// 按 DEC-061，作者自助删除**不**再让主题整体不可读：删除态主题继续可以打开、
    /// 继续出现在列表与搜索中，只有首帖正文以占位呈现（见 `BodyState::Deleted`）。
    /// 真正把内容移出公开视图的手段仍是 `moderation` 轴的 `obscured`／`hidden`，
    /// 以及未发布状态对非作者的不可见，这两条在下面照旧生效。
    pub fn can_read(&self, viewer: &Viewer, owner_id: i64) -> bool {
        self.moderation == Moderation::Normal
            && self.visibility.permits(viewer)
            && match self.publication {
                Publication::Published => true,
                Publication::Pending | Publication::Draft => viewer.user_id == Some(owner_id),
            }
    }
    pub fn permits_write(&self) -> bool {
        self.deleted_at.is_none()
            && self.archived_at.is_none()
            && self.moderation == Moderation::Normal
            && !self.locked
            && self.publication == Publication::Published
    }
    /// No authorization is granted here. Caller must prove owner in the same transaction.
    /// Ordinary changes cannot alter publication, moderation, discussion or archive axes.
    pub fn apply_owner_change(&self, change: OwnerStateChange) -> Result<Self, ContentError> {
        if !self.permits_write() {
            return Err(ContentError::Forbidden);
        }
        let mut next = self.clone();
        match change {
            OwnerStateChange::Visibility(value) => next.visibility = value,
            OwnerStateChange::Delete { at } => next.deleted_at = Some(at),
        }
        next.validate()?;
        Ok(next)
    }
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.deleted_at.is_some_and(|t| t <= DateTime::UNIX_EPOCH)
            || self.archived_at.is_some_and(|t| t <= DateTime::UNIX_EPOCH)
        {
            return Err(ContentError::InvalidInput);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerStateChange {
    Visibility(Visibility),
    Delete { at: DateTime<Utc> },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Viewer {
    pub user_id: Option<i64>,
    pub member: bool,
    /// 该账号是否被标记为分析排除（系统账号、测试账号、已确认作弊流量）。
    /// DEC-054 要求把这类流量排除在浏览量之外；未登录访客恒为假。
    pub excluded: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    pub user_id: i64,
    pub public_id: String,
    pub username: String,
    /// PRIV-002：本人主题正文对未登录访客的可见性。`Members` 表示作者已关闭，
    /// 未登录访客只能看到标题与元信息，正文位置渲染占位提示。
    pub body_visibility: Visibility,
    /// PRIV-001：个人主页主题/回帖列表对未登录访客的可见性。`Members` 表示作者已关闭。
    pub lists_visibility: Visibility,
}

/// 一项管理能力。按 DEC-058，具体角色到能力的映射由权限 registry 决定，
/// 内容领域只声明自己需要哪一项，不关心谁持有它。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagementCapability {
    /// CAT-013／CAT-014：把主题移动到另一个版块。
    MoveTopic,
}
impl ManagementCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MoveTopic => "move_topic",
        }
    }
}

/// 已通过授权的管理操作者，仅携带审计所需的最小事实。
///
/// 刻意不带角色名、权限集合或会话信息：内容领域拿到它只是为了把「谁做的」写进审计，
/// 不应该、也无法据此推导出更多权限。
#[derive(Clone, PartialEq, Eq)]
pub struct ManagementActor {
    pub user_id: i64,
}
impl std::fmt::Debug for ManagementActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagementActor([REDACTED])")
    }
}

/// DEC-054 的浏览去重主体。
///
/// `hash` 是应用装配层用用途隔离 HMAC 派生出的不可逆摘要，原始用户 ID、会话 Cookie 与
/// 设备指纹都不会到达内容领域，也不落库。`kind` 只用于观测与测试断言，不参与去重键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewSubject {
    pub kind: ViewSubjectKind,
    pub hash: String,
}
/// 去重主体的来源，优先级为 登录用户 > 会话 Cookie > 设备指纹（DEC-054）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewSubjectKind {
    Account,
    Session,
    Device,
}

/// 一条正文当前对本次访问者的呈现状态。
///
/// 正文被扣留时 `markdown` 与 `html` 一律为空串，绝不把原文塞进「占位」再靠模板隐藏——
/// 扣留必须发生在服务端投影阶段，否则任何模板疏漏都会导致正文泄露。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyState {
    /// 正常展示。
    Visible,
    /// 作者按 PRIV-002 关闭了对未登录访客的正文展示（DEC-054）。
    WithheldByAuthor,
    /// 作者已删除该内容，按 DEC-061 保留主题可读性并以占位呈现。
    Deleted,
}
impl BodyState {
    pub fn visible(self) -> bool {
        self == Self::Visible
    }
    /// 占位提示文案。放在合同层而不是模板里，保证服务端与任何渲染端口径一致。
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Visible => "",
            Self::WithheldByAuthor => "作者已设置该主题正文仅对登录成员可见。",
            Self::Deleted => "该内容已删除。",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustTier {
    Pending,
    T0,
    T1,
}
#[derive(Clone)]
pub struct AuthorityFacts {
    pub user_id: i64,
    pub created_at: DateTime<Utc>,
    pub tier: TrustTier,
    pub email_verified: bool,
    pub allowed: bool,
    pub default_visibility: Visibility,
    pub publication: Publication,
    pub analytics_pseudonym: String,
    pub analytics_exclusions: platform::analytics::TrustedExclusions,
}
impl std::fmt::Debug for AuthorityFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorityFacts")
            .field("tier", &self.tier)
            .field("allowed", &self.allowed)
            .finish_non_exhaustive()
    }
}
/// 从发布事务里带出的一次有效贡献变更。
///
/// `body_chars` 是 Unicode 标量计数（DEC-055），`is_reply` 用于区分 DEC-054 里主题与回复
/// 各自的长度门槛。`Withdrawn` 表示本人删除后该条不再计入总数，但事实行本身会保留，
/// 以便信任等级可以从追加历史重算（`TRUST-002` 要求可解释、可重算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contribution {
    Recorded {
        post_id: i64,
        is_reply: bool,
        body_chars: i64,
    },
    Withdrawn {
        post_id: i64,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    CreateTopic,
    CreateReply,
    EditPost,
    DeletePost,
}
impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreateTopic => "create_topic",
            Self::CreateReply => "create_reply",
            Self::EditPost => "edit_post",
            Self::DeletePost => "delete_post",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskDisposition {
    Allowed,
    Denied,
    Unavailable,
}
/// Implement in apps using only identity/trust core services. authorize must lock and
/// reread the account/session/trust before any community lock. append_risk runs last.
pub trait ContentAuthority: Send + Sync {
    fn forum_rules<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _forum_ids: &'a [i64],
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Vec<(i64, String)>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn read_viewer<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        session: Option<&'a str>,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, Viewer>;
    fn authorize<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        action: Action,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, AuthorityFacts>;
    fn resolve_author<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        public_id: &'a str,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, Option<Author>>;
    fn authors<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        user_ids: &'a [i64],
        now: DateTime<Utc>,
    ) -> FutureResult<'a, Vec<Author>>;
    /// 校验当前请求是否被授权执行一次管理动作，并返回可用于审计的操作者事实。
    ///
    /// 领域边界：内容领域只知道「需要某项管理能力」，具体由哪个角色、按哪条权限规则放行，
    /// 完全由应用装配层依据 DEC-058 批准的权限 registry 判定。内容领域不持有角色枚举，
    /// 也不接受任何来自 HTTP 的角色声明。
    ///
    /// 未装配时必须整体失败关闭（返回 `PolicyPending`），绝不退化成「放行」。
    fn authorize_management<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        capability: ManagementCapability,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ManagementActor>;

    /// 追加一条移帖审计事实（`CAT-014` 要求移动必须审计）。
    ///
    /// 由应用装配层落到平台审计表，内容领域不直接依赖审计存储。调用发生在追加段，
    /// 实现方不得再获取任何领域锁。
    #[allow(clippy::too_many_arguments)]
    fn append_move_audit<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        actor: &'a ManagementActor,
        topic_id: i64,
        from_forum_id: i64,
        to_forum_id: i64,
        operation_id: &'a str,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ()>;

    /// 记录或撤销一条 DEC-054 定义的有效贡献，供信任等级规则统计。
    ///
    /// 职责边界：**是否构成有效贡献由 content 判定**（长度、发布状态、回复者是否为主题
    /// 作者），实现方只负责把判定结果存下来。调用发生在写事务的追加段，此时已经
    /// `enter_append_phase`，因此实现方**不得再获取任何新锁**，只能做纯插入或更新。
    fn record_contribution<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        user_id: i64,
        contribution: Contribution,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ()>;
    #[allow(clippy::too_many_arguments)]
    fn append_risk<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        facts: &'a AuthorityFacts,
        action: Action,
        disposition: RiskDisposition,
        event_id: &'a str,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ()>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forum {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub position: i64,
    pub visibility: Visibility,
    pub status: String,
    pub rule: String,
    pub version: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub restricted: bool,
    pub merged_into: Option<i64>,
    pub version: i64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListOrder {
    LatestReply,
    LatestTopic,
    Pinned,
    Featured,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    Topics,
    Replies,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListFilter {
    pub order: ListOrder,
    pub forum_id: Option<i64>,
    pub tag_id: Option<i64>,
    pub owner_id: Option<i64>,
    pub kind: ListKind,
}
impl Default for ListFilter {
    fn default() -> Self {
        Self {
            order: ListOrder::LatestReply,
            forum_id: None,
            tag_id: None,
            owner_id: None,
            kind: ListKind::Topics,
        }
    }
}
#[derive(Clone)]
pub struct TopicView {
    pub id: i64,
    pub title: String,
    pub forum: Forum,
    pub tags: Vec<Tag>,
    pub author: Author,
    pub state: ContentState,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub version: i64,
    pub pinned: bool,
    pub globally_pinned: bool,
    pub featured: bool,
    pub reply_count: Option<i64>,
    pub view_count: Option<i64>,
    pub last_reply_author: Option<Author>,
    pub last_reply_at: Option<DateTime<Utc>>,
}
#[derive(Clone)]
pub struct PostView {
    pub id: i64,
    pub topic_id: i64,
    pub floor_no: i64,
    pub author: Author,
    /// 正文原文。`body_state` 非 `Visible` 时为空串。
    pub markdown: String,
    /// 渲染后的安全 HTML。`body_state` 非 `Visible` 时为空串。
    pub html: String,
    /// 本次访问者看到的正文状态，决定是否渲染占位提示。
    pub body_state: BodyState,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub version: i64,
    pub quote_post_id: Option<i64>,
    pub publication: Publication,
}
#[derive(Debug, Clone)]
pub struct ContentList {
    pub viewer: Viewer,
    pub topics: Vec<TopicView>,
    pub replies: Vec<PostView>,
    pub forums: Vec<Forum>,
    pub tags: Vec<Tag>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone)]
pub struct TopicDetail {
    pub viewer: Viewer,
    pub topic: TopicView,
    pub first_post: PostView,
    pub replies: Vec<PostView>,
    pub next_cursor: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct NewTopic {
    pub forum_id: i64,
    pub title: String,
    pub markdown: String,
    pub tag_ids: Vec<i64>,
    pub visibility: Visibility,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct NewReply {
    pub topic_id: i64,
    pub markdown: String,
    pub quote_post_id: Option<i64>,
    pub expected_topic_version: i64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct EditPost {
    pub post_id: i64,
    pub expected_version: i64,
    pub title: Option<String>,
    pub markdown: String,
    pub tag_ids: Option<Vec<i64>>,
    pub visibility: Option<Visibility>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct DeletePost {
    pub post_id: i64,
    pub expected_version: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationResult {
    pub topic_id: i64,
    pub post_id: i64,
    pub floor_no: i64,
    pub version: i64,
    pub replayed: bool,
}
#[derive(Debug, Clone)]
pub struct PostLocation {
    pub topic_id: i64,
    pub floor_no: i64,
    pub cursor: Option<String>,
}
#[derive(Debug, Clone)]
pub struct Preview {
    pub html: String,
    pub external_links: usize,
}

impl std::fmt::Debug for PostView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostView")
            .field("id", &self.id)
            .field("floor", &self.floor_no)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for TopicView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TopicView")
            .field("id", &self.id)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
