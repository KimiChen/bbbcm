//! Identity trust repositories borrow the existing transaction and never commit.
use crate::{
    contract::{INVITATION_POLICY_VERSION, TrustError, TrustLevel, TrustSnapshot},
    risk::RiskEvent,
};
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::{DbRow, DbValue, TransactionConnection};

fn table(
    tx: &TransactionConnection<'_>,
    r: &SchemaRenderer,
    name: &str,
) -> Result<String, TrustError> {
    r.table(tx.backend(), "trust", name)
        .map_err(|_| TrustError::Unavailable)
}
fn text(value: &str) -> DbValue {
    DbValue::Text(value.into())
}
fn snapshot(row: &DbRow) -> Result<TrustSnapshot, TrustError> {
    let value = TrustSnapshot {
        id: row.i64("id")?,
        user_id: row.i64("user_id")?,
        level: TrustLevel::parse(&row.text("level")?)?,
        email_verified: row.boolean("email_verified")?,
        invitation_allowed: row.boolean("invitation_allowed")?,
        policy_version: row.i64("policy_version")?,
        reason_code: row.text("reason_code")?,
        created_at: row.time("created_at")?,
        updated_at: row.time("updated_at")?,
        version: row.i64("version")?,
    };
    if value.id <= 0
        || value.user_id <= 0
        || value.version <= 0
        || value.policy_version < 0
        || value.updated_at < value.created_at
        || (value.level != TrustLevel::Pending
            && (!value.email_verified || value.policy_version == 0))
        || (value.invitation_allowed && (!value.email_verified || value.policy_version == 0))
        || !matches!(
            value.reason_code.as_str(),
            "awaiting_email_verification"
                | "awaiting_trust_policy"
                | "approved_policy"
                | "email_verified_invitation_eligible"
                // DEC-054 授级新增的两个原因码。这里是封闭集合：库里出现集合外的原因码
                // 一律按数据损坏处理，而不是宽容接受——防止有人绕过授级路径直接改库。
                | "email_verified_tier_t0"
                | "tier_rule_v1_days_contributions"
        )
    {
        return Err(TrustError::CorruptData);
    }
    Ok(value)
}
pub(crate) async fn load_snapshot(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<Option<TrustSnapshot>, TrustError> {
    let table = table(tx, r, "identity_snapshots")?;
    tx.fetch_optional(&format!("SELECT id, user_id, level, email_verified, invitation_allowed, policy_version, reason_code, created_at, updated_at, version FROM {table} WHERE user_id = ?"), &[DbValue::I64(user_id)]).await?
        .map(|row| snapshot(&row)).transpose()
}
pub(crate) async fn insert_initial(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    invitation_id: i64,
    inviter_id: i64,
    now: DateTime<Utc>,
) -> Result<(), TrustError> {
    let snapshots = table(tx, r, "identity_snapshots")?;
    let uses = table(tx, r, "invite_uses")?;
    tx.execute(&format!("INSERT INTO {snapshots} (user_id, level, email_verified, invitation_allowed, policy_version, reason_code, created_at, updated_at, version) VALUES (?, 'pending', ?, ?, 0, 'awaiting_email_verification', ?, ?, 1)"),
        &[DbValue::I64(user_id), DbValue::Bool(false), DbValue::Bool(false), DbValue::Time(now), DbValue::Time(now)]).await?;
    tx.execute(&format!("INSERT INTO {uses} (invitation_id, inviter_id, invitee_id, registered_at, email_verified, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, 1)"),
        &[DbValue::I64(invitation_id), DbValue::I64(inviter_id), DbValue::I64(user_id), DbValue::Time(now), DbValue::Bool(false), DbValue::Time(now)]).await?;
    Ok(())
}
pub(crate) async fn invite_use_id(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<Option<i64>, TrustError> {
    let table = table(tx, r, "invite_uses")?;
    tx.fetch_optional(
        &format!("SELECT id FROM {table} WHERE invitee_id = ?"),
        &[DbValue::I64(user_id)],
    )
    .await?
    .map(|row| row.i64("id").map_err(TrustError::from))
    .transpose()
}
pub(crate) async fn confirm_email(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    previous: &TrustSnapshot,
    now: DateTime<Utc>,
) -> Result<(), TrustError> {
    let snapshots = table(tx, r, "identity_snapshots")?;
    let uses = table(tx, r, "invite_uses")?;
    tx.execute_versioned(&format!("UPDATE {snapshots} SET email_verified = ?, invitation_allowed = ?, policy_version = ?, reason_code = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[DbValue::Bool(true), DbValue::Bool(true), DbValue::I64(INVITATION_POLICY_VERSION), text("email_verified_invitation_eligible"), DbValue::Time(now), DbValue::I64(previous.id), DbValue::I64(previous.version)]).await?;
    tx.execute(&format!("UPDATE {uses} SET email_verified = ?, updated_at = ?, version = version + 1 WHERE invitee_id = ? AND email_verified = ?"),
        &[DbValue::Bool(true), DbValue::Time(now), DbValue::I64(previous.user_id), DbValue::Bool(false)]).await?;
    Ok(())
}
/// 在调用方已持有快照行锁的前提下写入信任等级。
///
/// 使用 `execute_versioned` 做乐观版本校验，防止并发覆盖。本函数不含任何资格判断——
/// 规则评估在 `core` 层完成，仓储层只忠实落库，避免同一规则出现两份实现。
pub(crate) async fn set_level(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    previous: &TrustSnapshot,
    level: TrustLevel,
    reason_code: &str,
    now: DateTime<Utc>,
) -> Result<(), TrustError> {
    let snapshots = table(tx, r, "identity_snapshots")?;
    tx.execute_versioned(
        &format!(
            "UPDATE {snapshots} SET level = ?, reason_code = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?"
        ),
        &[
            text(level.as_str()),
            text(reason_code),
            DbValue::Time(now),
            DbValue::I64(previous.id),
            DbValue::I64(previous.version),
        ],
    )
    .await?;
    Ok(())
}

/// 追加一条有效贡献事实。
///
/// `post_id` 唯一：同一帖子重复调用时保留既有行并直接返回，使命令重放与幂等重试
/// 不会把贡献数刷高。这里刻意先查后插而不是依赖唯一键冲突，因为 DEC-022 的
/// 三库交集子集禁用 `ON CONFLICT`。
pub(crate) async fn insert_contribution(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    kind: crate::contract::ContributionKind,
    post_id: i64,
    body_chars: i64,
    now: DateTime<Utc>,
) -> Result<(), TrustError> {
    let facts = table(tx, r, "contribution_facts")?;
    if tx
        .fetch_optional(
            &format!("SELECT post_id FROM {facts} WHERE post_id = ?"),
            &[DbValue::I64(post_id)],
        )
        .await?
        .is_some()
    {
        return Ok(());
    }
    tx.execute(
        &format!(
            "INSERT INTO {facts} (user_id, kind, post_id, body_chars, counted, rule_version, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ),
        &[
            DbValue::I64(user_id),
            text(kind.as_str()),
            DbValue::I64(post_id),
            DbValue::I64(body_chars),
            DbValue::Bool(true),
            DbValue::I64(crate::contract::TIER_RULE_VERSION),
            DbValue::Time(now),
            DbValue::Time(now),
            DbValue::I64(1),
        ],
    )
    .await?;
    Ok(())
}

/// 把一条已撤销的贡献置为不计数。
///
/// 只改 `counted` 标志，保留事实行本身，使贡献总数可以随时从追加历史重算
/// （`TRUST-002` 要求信任等级可解释、可重算、可复核）。
/// 条件里带上 `counted = true`，因此重复撤销是无副作用的空操作。
pub(crate) async fn withdraw_contribution(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    post_id: i64,
    now: DateTime<Utc>,
) -> Result<(), TrustError> {
    let facts = table(tx, r, "contribution_facts")?;
    tx.execute(
        &format!(
            "UPDATE {facts} SET counted = ?, updated_at = ?, version = version + 1 WHERE post_id = ? AND counted = ?"
        ),
        &[
            DbValue::Bool(false),
            DbValue::Time(now),
            DbValue::I64(post_id),
            DbValue::Bool(true),
        ],
    )
    .await?;
    Ok(())
}

/// 统计某账号当前计入的有效贡献条数，供 T1 授级规则使用。
///
/// 返回负数视为数据损坏并报错，避免异常值被当成「未达标」而静默吞掉。
pub(crate) async fn count_contributions(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<i64, TrustError> {
    let facts = table(tx, r, "contribution_facts")?;
    let row = tx
        .fetch_optional(
            &format!("SELECT COUNT(*) AS total FROM {facts} WHERE user_id = ? AND counted = ?"),
            &[DbValue::I64(user_id), DbValue::Bool(true)],
        )
        .await?
        .ok_or(TrustError::CorruptData)?;
    let total = row.i64("total")?;
    if total < 0 {
        return Err(TrustError::CorruptData);
    }
    Ok(total)
}

pub(crate) async fn append_risk(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    event: &RiskEvent,
) -> Result<(), TrustError> {
    let table = table(tx, r, "risk_events")?;
    let fingerprint = event.fingerprint()?;
    if let Some(row) = tx
        .fetch_optional(
            &format!("SELECT fingerprint FROM {table} WHERE event_id = ?"),
            &[text(&event.event_id)],
        )
        .await?
    {
        if row.text("fingerprint")? != fingerprint {
            return Err(TrustError::Conflict);
        }
        return Ok(());
    }
    let signals = serde_json::to_string(&event.signals).map_err(|_| TrustError::InvalidInput)?;
    tx.enter_append_phase();
    tx.execute(&format!("INSERT INTO {table} (event_id, event_version, event_kind, actor_id, occurred_at, rule_version, outcome, reason_code, signals_json, fingerprint) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),
        &[text(&event.event_id), DbValue::I64(event.event_version), text(event.kind.as_str()), DbValue::I64(event.actor_id.unwrap_or(0)), DbValue::Time(event.occurred_at), DbValue::I64(event.decision.rule_version()), text(event.decision.action().as_str()), text(event.decision.reason().as_str()), text(&signals), text(&fingerprint)]).await?;
    Ok(())
}
