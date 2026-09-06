//! WP-MVP-D02：处置动作与申诉。
//!
//! 本模块是处置的唯一事实源：内容状态与账号限制都由这里驱动，不存在第二条修改路径。
//! 授权判定属 `WP-MVP-D04` 的权限 registry，调用方负责；本模块只保证
//! 「记录完整、职责分离、历史只追加」。
use crate::contract::ModerationError;
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::TransactionConnection;

/// `MOD-005` 的十类审核动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 忽略：驳回举报，不处置。仍然要留记录，否则「看过并认为没问题」无从证明。
    Ignore,
    /// 遮蔽：内容折叠但可展开。
    Obscure,
    /// 隐藏：内容对普通用户不可见。
    Hide,
    /// 软删除
    SoftDelete,
    /// 锁帖：禁止继续回复。
    Lock,
    /// 移帖：换版块。实际执行在内容领域（`CAT-014`），这里只登记处置事实。
    Move,
    /// 警告：只通知，不限制。
    Warn,
    /// 限言：限制发布能力，可带期限。
    Mute,
    /// 临时封禁
    Suspend,
    /// 永久封禁
    Ban,
}
impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ignore => "ignore",
            Self::Obscure => "obscure",
            Self::Hide => "hide",
            Self::SoftDelete => "soft_delete",
            Self::Lock => "lock",
            Self::Move => "move",
            Self::Warn => "warn",
            Self::Mute => "mute",
            Self::Suspend => "suspend",
            Self::Ban => "ban",
        }
    }
    pub fn parse(value: &str) -> Result<Self, ModerationError> {
        Ok(match value {
            "ignore" => Self::Ignore,
            "obscure" => Self::Obscure,
            "hide" => Self::Hide,
            "soft_delete" => Self::SoftDelete,
            "lock" => Self::Lock,
            "move" => Self::Move,
            "warn" => Self::Warn,
            "mute" => Self::Mute,
            "suspend" => Self::Suspend,
            "ban" => Self::Ban,
            _ => return Err(ModerationError::InvalidInput),
        })
    }
    /// 作用于账号（而非某一条内容）的动作。
    ///
    /// 区分作用面是为了避免「对一条回复执行封禁」这类目标错配：账号级动作的目标必须是
    /// 账号，内容级动作的目标必须是具体内容。
    pub fn targets_account(self) -> bool {
        matches!(self, Self::Warn | Self::Mute | Self::Suspend | Self::Ban)
    }
    /// 是否可以带期限。
    ///
    /// 永久封禁与一次性的内容动作都不带期限；限言与临时封禁必须带，
    /// 否则「临时」没有意义，也无法自动失效。
    pub fn requires_expiry(self) -> bool {
        matches!(self, Self::Mute | Self::Suspend)
    }
}

/// `MOD-011` 申诉复核结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppealOutcome {
    /// 待复核
    Pending,
    /// 维持原处罚
    Upheld,
    /// 撤销原处罚
    Overturned,
    /// 减轻：缩短期限或降级，具体新期限由复核人给出。
    Reduced,
}
impl AppealOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Upheld => "upheld",
            Self::Overturned => "overturned",
            Self::Reduced => "reduced",
        }
    }
    pub fn parse(value: &str) -> Result<Self, ModerationError> {
        Ok(match value {
            "pending" => Self::Pending,
            "upheld" => Self::Upheld,
            "overturned" => Self::Overturned,
            "reduced" => Self::Reduced,
            _ => return Err(ModerationError::InvalidInput),
        })
    }
}

/// 一次处置的完整输入（`MOD-006`）。
///
/// 原因码、规则版本、证据引用、前后状态缺一不可：少了任何一项，事后都无法回答
/// 「凭什么处罚、按哪版规则、当时内容是什么样」。
#[derive(Debug, Clone)]
pub struct Enforcement {
    pub case_id: i64,
    pub action: Action,
    pub target_kind: TargetKind,
    pub target_id: i64,
    /// 被处置的账号。内容级动作也要记，用于按人汇总处罚历史。
    pub subject_id: i64,
    pub operator_id: i64,
    pub reason_code: String,
    pub rule_version: i64,
    pub evidence_digest: String,
    pub before_state: String,
    pub after_state: String,
    /// 临时处罚的到期时间；永久或一次性动作为 `None`。
    pub expires_at: Option<DateTime<Utc>>,
}

/// 处置目标的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Topic,
    Post,
    Account,
}
impl TargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::Post => "post",
            Self::Account => "account",
        }
    }
    pub fn parse(value: &str) -> Result<Self, ModerationError> {
        Ok(match value {
            "topic" => Self::Topic,
            "post" => Self::Post,
            "account" => Self::Account,
            _ => return Err(ModerationError::InvalidInput),
        })
    }
}

/// 记录一次处置，返回处置 ID。
///
/// 校验的是「记录本身是否自洽」：作用面与目标类型必须匹配，需要期限的动作必须带期限，
/// 不该带期限的动作不得带。真正的状态变更由调用方在同一事务内完成——本函数只保证
/// 「没有无记录的处罚」，不保证「没有无处罚的状态变更」，后者靠调用路径唯一性保证。
pub async fn record(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    input: &Enforcement,
    now: DateTime<Utc>,
) -> Result<i64, ModerationError> {
    if input.target_id <= 0
        || input.subject_id <= 0
        || input.operator_id <= 0
        || input.rule_version <= 0
        || input.reason_code.trim().is_empty()
        || input.reason_code.chars().count() > 64
    {
        return Err(ModerationError::InvalidInput);
    }
    // 作用面必须与目标类型一致：不能「对一条回复执行封禁」，也不能「对账号执行锁帖」。
    if input.action.targets_account() != (input.target_kind == TargetKind::Account) {
        return Err(ModerationError::InvalidInput);
    }
    if input.action.requires_expiry() != input.expires_at.is_some() {
        return Err(ModerationError::InvalidInput);
    }
    if input.expires_at.is_some_and(|at| at <= now) {
        return Err(ModerationError::InvalidInput);
    }
    crate::enforcement_repo::insert_action(tx, r, input, canonical(now)?).await
}

/// 撤销一次处置（`MOD-006`：历史只追加）。
///
/// 写撤销人与撤销原因，不删除原记录。撤销后限制立即停止生效，但「曾经被处罚过」
/// 这一事实必须留存，否则申诉与复盘都失去依据。
pub async fn revoke(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    action_id: i64,
    revoked_by: i64,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let reason = reason.trim();
    if revoked_by <= 0 || reason.is_empty() || reason.chars().count() > 512 {
        return Err(ModerationError::InvalidInput);
    }
    let action = crate::enforcement_repo::action(tx, r, action_id)
        .await?
        .ok_or(ModerationError::NotFound)?;
    if action.revoked {
        return Err(ModerationError::InvalidInput);
    }
    crate::enforcement_repo::revoke_action(tx, r, &action, revoked_by, reason, canonical(now)?)
        .await
}

/// 提交一次申诉（`MOD-011`：每次处罚只有一次正常申诉）。
///
/// 只有被处置本人可以申诉；已撤销的处罚不能申诉（没有可申诉的对象）。
/// 重复申诉由库层唯一键与这里的显式检查共同挡住。
pub async fn submit_appeal(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    action_id: i64,
    appellant_id: i64,
    statement: &str,
    now: DateTime<Utc>,
) -> Result<i64, ModerationError> {
    let statement = statement.trim();
    if appellant_id <= 0 || statement.is_empty() || statement.chars().count() > 4_000 {
        return Err(ModerationError::InvalidInput);
    }
    let action = crate::enforcement_repo::action(tx, r, action_id)
        .await?
        .ok_or(ModerationError::NotFound)?;
    if action.subject_id != appellant_id || action.revoked {
        return Err(ModerationError::InvalidInput);
    }
    if crate::enforcement_repo::appeal_for(tx, r, action_id)
        .await?
        .is_some()
    {
        return Err(ModerationError::Conflict);
    }
    crate::enforcement_repo::insert_appeal(
        tx,
        r,
        action_id,
        appellant_id,
        statement,
        canonical(now)?,
    )
    .await
}

/// 复核一次申诉（`MOD-011`：由非原处理人复核）。
///
/// 职责分离在服务端强制：复核人不得是原处置执行人，也不得是申诉人本人。
/// 这是本函数存在的主要理由——把它交给调用方判断迟早会漏。
///
/// 三种结论的效果：
///   * `Upheld` 维持，处罚不变；
///   * `Overturned` 撤销，同事务撤销原处罚；
///   * `Reduced` 减轻，调用方给出新的、必须更早的到期时间。
#[allow(clippy::too_many_arguments)]
pub async fn review_appeal(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    appeal_id: i64,
    reviewer_id: i64,
    outcome: AppealOutcome,
    note: &str,
    reduced_expiry: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let note = note.trim();
    if reviewer_id <= 0
        || outcome == AppealOutcome::Pending
        || note.is_empty()
        || note.chars().count() > 4_000
    {
        return Err(ModerationError::InvalidInput);
    }
    let appeal = crate::enforcement_repo::appeal(tx, r, appeal_id)
        .await?
        .ok_or(ModerationError::NotFound)?;
    if appeal.outcome != AppealOutcome::Pending {
        return Err(ModerationError::Conflict);
    }
    let action = crate::enforcement_repo::action(tx, r, appeal.action_id)
        .await?
        .ok_or(ModerationError::NotFound)?;
    // MOD-011 职责分离：原处理人不能复核自己的处罚，申诉人不能复核自己的申诉。
    if reviewer_id == action.operator_id || reviewer_id == appeal.appellant_id {
        return Err(ModerationError::InvalidInput);
    }
    let now = canonical(now)?;
    match outcome {
        AppealOutcome::Overturned => {
            crate::enforcement_repo::revoke_action(
                tx,
                r,
                &action,
                reviewer_id,
                "appeal_overturned",
                now,
            )
            .await?;
        }
        AppealOutcome::Reduced => {
            // 减轻必须真的更轻：新期限要早于原期限，且仍在未来。
            let next = reduced_expiry.ok_or(ModerationError::InvalidInput)?;
            let current = action.expires_at.ok_or(ModerationError::InvalidInput)?;
            if next >= current || next <= now {
                return Err(ModerationError::InvalidInput);
            }
            crate::enforcement_repo::shorten_action(tx, r, &action, next, now).await?;
        }
        AppealOutcome::Upheld => {}
        AppealOutcome::Pending => unreachable!("已在入口拒绝"),
    }
    crate::enforcement_repo::set_appeal_outcome(tx, r, &appeal, reviewer_id, outcome, note, now)
        .await
}

/// `MOD-007`：面向被处置用户的可理解说明。
///
/// 只包含用户有权知道的内容：动作、理由码、范围、期限与申诉入口；
/// 不含内部备注、证据正文与执行人身份——那些属 C2，泄露会让举报人与审核员承担风险。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFacingNotice {
    pub action: Action,
    pub reason_code: String,
    pub scope: TargetKind,
    pub expires_at: Option<DateTime<Utc>>,
    pub appeal_available: bool,
    pub appeal_href: String,
}

/// 生成给被处置用户的说明。
pub async fn notice_for(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    action_id: i64,
) -> Result<UserFacingNotice, ModerationError> {
    let action = crate::enforcement_repo::action(tx, r, action_id)
        .await?
        .ok_or(ModerationError::NotFound)?;
    let appealed = crate::enforcement_repo::appeal_for(tx, r, action_id)
        .await?
        .is_some();
    Ok(UserFacingNotice {
        action: action.action,
        reason_code: action.reason_code,
        scope: action.target_kind,
        expires_at: action.expires_at,
        // 已撤销或已申诉过的处罚不再提供申诉入口（MOD-011：只有一次正常申诉）。
        appeal_available: !action.revoked && !appealed,
        appeal_href: format!("/appeals/{action_id}"),
    })
}

fn canonical(now: DateTime<Utc>) -> Result<DateTime<Utc>, ModerationError> {
    platform::db::decode_time(&platform::db::encode_time(now))
        .map_err(|_| ModerationError::InvalidInput)
}

#[cfg(test)]
mod tests;
