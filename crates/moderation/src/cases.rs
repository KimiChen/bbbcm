//! WP-MVP-D01：举报、案件、内部备注与证据快照。
//!
//! 领域边界：本模块只管「谁举报了什么、案件走到哪一步、证据长什么样」，
//! **不做任何处置**——遮蔽、删除、封禁属 `MOD-005`～`MOD-007`（`WP-MVP-D02`），
//! 权限判定属 `WP-MVP-D04` 的 registry。举报本身不改变被举报内容的任何状态。
use crate::{
    authority::{
        ManagementAuthority, ManagementCapability, ManagementCredentials, ManagementTarget,
    },
    contract::ModerationError,
};
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::TransactionConnection;

/// `MOD-002` 的封闭举报类型。
///
/// 用封闭枚举而不是自由文本：类型要参与分流优先级与统计口径，自由文本会让两者失控。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportCategory {
    /// 垃圾广告
    Spam,
    /// 辱骂骚扰
    Harassment,
    /// 违法违规
    Illegal,
    /// 隐私泄露
    Privacy,
    /// 侵权
    Infringement,
    /// 诈骗
    Fraud,
    /// 恶意文件
    Malware,
    /// 其他
    Other,
}
impl ReportCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spam => "spam",
            Self::Harassment => "harassment",
            Self::Illegal => "illegal",
            Self::Privacy => "privacy",
            Self::Infringement => "infringement",
            Self::Fraud => "fraud",
            Self::Malware => "malware",
            Self::Other => "other",
        }
    }
    pub fn parse(value: &str) -> Result<Self, ModerationError> {
        Ok(match value {
            "spam" => Self::Spam,
            "harassment" => Self::Harassment,
            "illegal" => Self::Illegal,
            "privacy" => Self::Privacy,
            "infringement" => Self::Infringement,
            "fraud" => Self::Fraud,
            "malware" => Self::Malware,
            "other" => Self::Other,
            _ => return Err(ModerationError::InvalidInput),
        })
    }
    /// `MOD-012`：需要最小分流、止损与升级的六类高危请求。
    ///
    /// 这些类型一进来就置最高优先级并打升级标志，因为它们的共同点是「拖延本身就是伤害」：
    /// 隐私泄露每多挂一分钟就多一分扩散，人身威胁与重大违法涉及线下安全。
    /// 本轮只保证标记不会丢，自动处置与告警接口属 `D02`／`D04`。
    pub fn is_urgent(self) -> bool {
        matches!(
            self,
            Self::Illegal | Self::Privacy | Self::Infringement | Self::Fraud | Self::Malware
        )
    }
}

/// 可被举报的目标类型（`MOD-001`／`SOCIAL-005`）。
///
/// 附件随 `C03` 媒体接入后补，私信随 P1 私信功能接入；两者都会复用同一套案件模型，
/// 因此这里的枚举是可扩展点而不是最终清单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Topic,
    Post,
    Profile,
}
impl TargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::Post => "post",
            Self::Profile => "profile",
        }
    }
    pub fn parse(value: &str) -> Result<Self, ModerationError> {
        Ok(match value {
            "topic" => Self::Topic,
            "post" => Self::Post,
            "profile" => Self::Profile,
            _ => return Err(ModerationError::InvalidInput),
        })
    }
}

/// `MOD-003` 案件状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseStatus {
    /// 待分流
    Triage,
    /// 调查中
    Investigating,
    /// 等待补充
    AwaitingInfo,
    /// 已处理
    Resolved,
    /// 已申诉
    Appealed,
    /// 已关闭
    Closed,
}
impl CaseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Investigating => "investigating",
            Self::AwaitingInfo => "awaiting_info",
            Self::Resolved => "resolved",
            Self::Appealed => "appealed",
            Self::Closed => "closed",
        }
    }
    pub fn parse(value: &str) -> Result<Self, ModerationError> {
        Ok(match value {
            "triage" => Self::Triage,
            "investigating" => Self::Investigating,
            "awaiting_info" => Self::AwaitingInfo,
            "resolved" => Self::Resolved,
            "appealed" => Self::Appealed,
            "closed" => Self::Closed,
            _ => return Err(ModerationError::InvalidInput),
        })
    }
    /// 封闭状态迁移表。
    ///
    /// 关键约束：
    ///   * `Closed` 是终态，任何迁出都拒绝——重开必须是新案件，否则审计链会断。
    ///   * `Resolved` 只能走向 `Appealed`（用户申诉）或 `Closed`，不能倒回调查中；
    ///     处置已经发生，倒回会让「处置时的案件状态」无法复原。
    ///   * `Appealed` 由 `D02` 的申诉复核推进，这里只允许它走向 `Resolved` 或 `Closed`。
    ///
    pub fn permits(self, next: Self) -> bool {
        use CaseStatus::*;
        match self {
            Triage => matches!(next, Investigating | AwaitingInfo | Resolved | Closed),
            Investigating => matches!(next, AwaitingInfo | Resolved | Closed),
            AwaitingInfo => matches!(next, Investigating | Resolved | Closed),
            Resolved => matches!(next, Appealed | Closed),
            Appealed => matches!(next, Resolved | Closed),
            Closed => false,
        }
    }
}

/// 一次举报的提交结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportOutcome {
    pub case_id: i64,
    /// 真为「本次新建了案件」，假为「挂到了既有未关闭案件」。
    pub created_case: bool,
    /// 真为「本次登记了新举报」，假为「该举报人已举报过同一案件，返回既有工单」。
    pub recorded: bool,
}

/// 提交一次举报（`MOD-001`／`SOCIAL-005`）。
///
/// 调用方必须已经在同一事务内完成身份校验；本函数不做授权判定，也不校验目标是否可见——
/// 「能不能举报我看不见的内容」由调用方按业务决定。
///
/// 去重语义（`MOD-001`）：
///   * 目标已有未关闭案件时挂上去，不新建案件；
///   * 同一举报人对同一案件只登记一次，重复提交返回既有工单且不增加计数。
///
/// 这样「一条广告被 50 人举报」是一个案件 50 条举报，而不是 50 个案件；
/// 也让刷举报无法把队列淹掉。
#[allow(clippy::too_many_arguments)]
pub async fn submit_report(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    reporter_id: i64,
    kind: TargetKind,
    target_id: i64,
    category: ReportCategory,
    note: &str,
    snapshot: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ReportOutcome, ModerationError> {
    if reporter_id <= 0 || target_id <= 0 || note.chars().count() > 2_000 {
        return Err(ModerationError::InvalidInput);
    }
    let now = canonical(now)?;
    // `DEC-066` 序列化点：同一目标还没有案件时无行可锁，并发的多路会同时读到
    // 「没有」，各自建案——`MOD-002` 的合并语义就此失效。这里先按
    // `(target_kind, target_id)` 取一把事务级顾问锁，把「查—建」整段串起来。
    //
    // 用顾问锁而不是唯一偏索引：偏索引语义更贴切，但 MySQL 8.0 不支持，
    // 与 `DEC-022` 的三库交集子集冲突。锁随事务结束自动释放，无需显式解锁。
    // SQLite 返回 `None`：单写者天然互斥，不需要也没有顾问锁。
    if let Some(sql) = platform::db::advisory_lock_sql(tx.backend()) {
        let key = platform::db::advisory_lock_key(kind.as_str(), target_id);
        tx.fetch_optional(sql, &[platform_db::transaction::DbValue::I64(key)])
            .await?;
    }
    let existing = crate::case_repo::open_case(tx, r, kind, target_id).await?;
    let (case_id, created_case) = match existing {
        Some(case) => {
            // 已有案件：高危类型可以把优先级提上去，但绝不下调——
            // 先到的高危举报不应被后来的一条「其他」冲淡。
            if category.is_urgent() {
                crate::case_repo::escalate(tx, r, &case, now).await?;
            }
            (case.id, false)
        }
        None => {
            let id =
                crate::case_repo::insert_case(tx, r, kind, target_id, category.is_urgent(), now)
                    .await?;
            // MOD-008：建案时对目标内容做一次不可变快照，落 SHA-256 摘要与时间戳。
            // 快照必须在此刻取：等到处置时再取，内容可能已被作者改掉或删掉。
            if let Some(body) = snapshot {
                crate::case_repo::insert_evidence(tx, r, id, body, &digest(body), now).await?;
            }
            (id, true)
        }
    };
    let recorded =
        crate::case_repo::insert_report(tx, r, case_id, reporter_id, category, note, now).await?;
    Ok(ReportOutcome {
        case_id,
        created_case,
        recorded,
    })
}

/// 管理侧：推进案件状态。
///
/// 只做状态机校验与落库；「谁可以推进」由调用方按 `D04` 的权限 registry 判定。
/// 非法迁移一律拒绝，不做「就近取一个合法态」的宽容处理——那会让审计链失真。
#[allow(clippy::too_many_arguments)]
pub async fn advance_status(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    authority: &dyn ManagementAuthority,
    credentials: &ManagementCredentials,
    target: ManagementTarget,
    case_id: i64,
    next: CaseStatus,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let now = canonical(now)?;
    authority
        .authorize(
            tx,
            credentials,
            ManagementCapability::WriteCase,
            target,
            now,
        )
        .await?;
    let case = crate::case_repo::case(tx, r, case_id)
        .await?
        .ok_or(ModerationError::NotFound)?;
    if !case.status.permits(next) {
        return Err(ModerationError::InvalidInput);
    }
    crate::case_repo::set_status(tx, r, &case, next, now).await
}

/// 管理侧：指派处理人。
#[allow(clippy::too_many_arguments)]
pub async fn assign(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    authority: &dyn ManagementAuthority,
    credentials: &ManagementCredentials,
    target: ManagementTarget,
    case_id: i64,
    assignee_id: i64,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    if assignee_id <= 0 {
        return Err(ModerationError::InvalidInput);
    }
    let now = canonical(now)?;
    authority
        .authorize(
            tx,
            credentials,
            ManagementCapability::WriteCase,
            target,
            now,
        )
        .await?;
    let case = crate::case_repo::case(tx, r, case_id)
        .await?
        .ok_or(ModerationError::NotFound)?;
    if case.status == CaseStatus::Closed {
        return Err(ModerationError::InvalidInput);
    }
    crate::case_repo::set_assignee(tx, r, &case, assignee_id, now).await
}

/// 管理侧：追加内部备注（`MOD-004`）。
///
/// 内部备注属 C2，与用户可见的处罚理由（`MOD-007`，属 `D02`）分开存放，
/// 避免把内部讨论误发给被处置用户。备注只追加不修改。
#[allow(clippy::too_many_arguments)]
pub async fn add_note(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    authority: &dyn ManagementAuthority,
    credentials: &ManagementCredentials,
    target: ManagementTarget,
    case_id: i64,
    body: &str,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let trimmed = body.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 4_000 {
        return Err(ModerationError::InvalidInput);
    }
    let now = canonical(now)?;
    // 备注作者取自授权结果，而不是调用方传入的 ID：内部备注要能追责，
    // 让调用方自报作者等于给冒名留了口子。
    let actor = authority
        .authorize(
            tx,
            credentials,
            ManagementCapability::WriteCase,
            target,
            now,
        )
        .await?;
    crate::case_repo::insert_note(tx, r, case_id, actor.user_id, trimmed, now).await
}

/// 证据快照的 SHA-256 摘要（`MOD-008`）。
///
/// 摘要用于事后证明「这份快照没被改过」：任何一处改动都会让摘要对不上。
/// 算法固定在领域层，不随存储实现变化。
fn digest(body: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(body.as_bytes()))
}

fn canonical(now: DateTime<Utc>) -> Result<DateTime<Utc>, ModerationError> {
    platform::db::decode_time(&platform::db::encode_time(now))
        .map_err(|_| ModerationError::InvalidInput)
}

#[cfg(test)]
mod tests;
