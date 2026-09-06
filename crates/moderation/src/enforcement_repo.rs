//! WP-MVP-D02 处置与申诉仓储。借用调用方事务、绝不提交，也不做授权或职责分离判定。
use crate::{
    contract::ModerationError,
    enforcement::{Action, AppealOutcome, Enforcement, TargetKind},
};
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::{DbValue, TransactionConnection};

/// 「未发生」的哨兵值：未撤销、无期限、未复核都用 Unix 纪元表示。
fn never() -> DbValue {
    DbValue::Time(DateTime::UNIX_EPOCH)
}
fn is_never(value: DateTime<Utc>) -> bool {
    value <= DateTime::UNIX_EPOCH
}
fn text(value: &str) -> DbValue {
    DbValue::Text(value.into())
}
fn table(
    tx: &TransactionConnection<'_>,
    r: &SchemaRenderer,
    name: &str,
) -> Result<String, ModerationError> {
    r.table(tx.backend(), "moderation", name)
        .map_err(|_| ModerationError::Unavailable)
}

#[derive(Debug, Clone)]
pub struct ActionRecord {
    pub id: i64,
    pub action: Action,
    pub target_kind: TargetKind,
    pub subject_id: i64,
    pub operator_id: i64,
    pub reason_code: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked: bool,
    pub version: i64,
}

pub async fn insert_action(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    input: &Enforcement,
    now: DateTime<Utc>,
) -> Result<i64, ModerationError> {
    let t = table(tx, r, "enforcement_actions")?;
    tx.execute(&format!("INSERT INTO {t} (case_id, action, target_kind, target_id, subject_id, operator_id, reason_code, rule_version, evidence_digest, before_state, after_state, expires_at, revoked_at, revoked_by, revoke_reason, created_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, '', ?, ?)"),
        &[input.case_id.into(), text(input.action.as_str()), text(input.target_kind.as_str()), input.target_id.into(),
          input.subject_id.into(), input.operator_id.into(), text(input.reason_code.trim()), input.rule_version.into(),
          text(&input.evidence_digest), text(&input.before_state), text(&input.after_state),
          input.expires_at.map(DbValue::Time).unwrap_or_else(never), never(), DbValue::Time(now), DbValue::I64(1)]).await?;
    tx.last_insert_id().await.map_err(Into::into)
}

pub async fn action(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
) -> Result<Option<ActionRecord>, ModerationError> {
    let t = table(tx, r, "enforcement_actions")?;
    tx.fetch_optional(&format!("SELECT id, action, target_kind, subject_id, operator_id, reason_code, expires_at, revoked_at, version FROM {t} WHERE id = ?"), &[id.into()]).await?
        .map(|row| {
            let expires = row.time("expires_at")?;
            Ok(ActionRecord {
                id: row.i64("id")?,
                action: Action::parse(&row.text("action")?)?,
                target_kind: TargetKind::parse(&row.text("target_kind")?)?,
                subject_id: row.i64("subject_id")?,
                operator_id: row.i64("operator_id")?,
                reason_code: row.text("reason_code")?,
                expires_at: (!is_never(expires)).then_some(expires),
                revoked: !is_never(row.time("revoked_at")?),
                version: row.i64("version")?,
            })
        })
        .transpose()
}

/// 撤销一次处置。原记录保留，只补写撤销人、原因与时间。
pub async fn revoke_action(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    record: &ActionRecord,
    revoked_by: i64,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let t = table(tx, r, "enforcement_actions")?;
    tx.execute_versioned(&format!("UPDATE {t} SET revoked_at = ?, revoked_by = ?, revoke_reason = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[DbValue::Time(now), revoked_by.into(), text(reason), record.id.into(), record.version.into()]).await?;
    Ok(())
}

/// 缩短处罚期限（申诉减轻）。只允许改到更早的时间，调用方已校验。
pub async fn shorten_action(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    record: &ActionRecord,
    expires_at: DateTime<Utc>,
    _now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let t = table(tx, r, "enforcement_actions")?;
    tx.execute_versioned(
        &format!(
            "UPDATE {t} SET expires_at = ?, version = version + 1 WHERE id = ? AND version = ?"
        ),
        &[
            DbValue::Time(expires_at),
            record.id.into(),
            record.version.into(),
        ],
    )
    .await?;
    Ok(())
}

/// 某账号当前生效的处置：未撤销，且未到期或永久。
///
/// 到期靠比较时间而不是清理任务：没有后台任务跑的时候，处罚也必须自然失效，
/// 否则「临时封禁」会变成事实上的永久封禁。
pub async fn active_actions(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    subject_id: i64,
    now: DateTime<Utc>,
) -> Result<Vec<ActionRecord>, ModerationError> {
    let t = table(tx, r, "enforcement_actions")?;
    let rows = tx.fetch_all(&format!("SELECT id, action, target_kind, subject_id, operator_id, reason_code, expires_at, revoked_at, version FROM {t} WHERE subject_id = ? AND revoked_at = ? ORDER BY id"),
        &[subject_id.into(), never()]).await?;
    // 用迭代器收集而不是逐条 push：仓储层的 SQL 静态守卫禁止 `.push`／`.extend`
    // 这类可用于拼接语句的调用，这里顺带保持了「一次解析、一次收集」的写法。
    let decoded = rows
        .iter()
        .map(|row| {
            let expires = row.time("expires_at")?;
            // 到期靠比较时间：没有后台任务时，临时处罚也必须自然失效。
            if !is_never(expires) && expires <= now {
                return Ok(None);
            }
            Ok(Some(ActionRecord {
                id: row.i64("id")?,
                action: Action::parse(&row.text("action")?)?,
                target_kind: TargetKind::parse(&row.text("target_kind")?)?,
                subject_id: row.i64("subject_id")?,
                operator_id: row.i64("operator_id")?,
                reason_code: row.text("reason_code")?,
                expires_at: (!is_never(expires)).then_some(expires),
                revoked: false,
                version: row.i64("version")?,
            }))
        })
        .collect::<Result<Vec<Option<ActionRecord>>, ModerationError>>()?;
    Ok(decoded.into_iter().flatten().collect())
}

#[derive(Debug, Clone)]
pub struct AppealRecord {
    pub id: i64,
    pub action_id: i64,
    pub appellant_id: i64,
    pub outcome: AppealOutcome,
    pub version: i64,
}

pub async fn insert_appeal(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    action_id: i64,
    appellant_id: i64,
    statement: &str,
    now: DateTime<Utc>,
) -> Result<i64, ModerationError> {
    let t = table(tx, r, "appeals")?;
    tx.execute(&format!("INSERT INTO {t} (action_id, appellant_id, statement, outcome, reviewer_id, review_note, created_at, reviewed_at, version) VALUES (?, ?, ?, ?, 0, '', ?, ?, ?)"),
        &[action_id.into(), appellant_id.into(), text(statement), text(AppealOutcome::Pending.as_str()), DbValue::Time(now), never(), DbValue::I64(1)]).await?;
    tx.last_insert_id().await.map_err(Into::into)
}

pub async fn appeal(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
) -> Result<Option<AppealRecord>, ModerationError> {
    let t = table(tx, r, "appeals")?;
    tx.fetch_optional(
        &format!("SELECT id, action_id, appellant_id, outcome, version FROM {t} WHERE id = ?"),
        &[id.into()],
    )
    .await?
    .map(|row| {
        Ok(AppealRecord {
            id: row.i64("id")?,
            action_id: row.i64("action_id")?,
            appellant_id: row.i64("appellant_id")?,
            outcome: AppealOutcome::parse(&row.text("outcome")?)?,
            version: row.i64("version")?,
        })
    })
    .transpose()
}

pub async fn appeal_for(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    action_id: i64,
) -> Result<Option<AppealRecord>, ModerationError> {
    let t = table(tx, r, "appeals")?;
    tx.fetch_optional(
        &format!(
            "SELECT id, action_id, appellant_id, outcome, version FROM {t} WHERE action_id = ?"
        ),
        &[action_id.into()],
    )
    .await?
    .map(|row| {
        Ok(AppealRecord {
            id: row.i64("id")?,
            action_id: row.i64("action_id")?,
            appellant_id: row.i64("appellant_id")?,
            outcome: AppealOutcome::parse(&row.text("outcome")?)?,
            version: row.i64("version")?,
        })
    })
    .transpose()
}

#[allow(clippy::too_many_arguments)]
pub async fn set_appeal_outcome(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    record: &AppealRecord,
    reviewer_id: i64,
    outcome: AppealOutcome,
    note: &str,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let t = table(tx, r, "appeals")?;
    tx.execute_versioned(&format!("UPDATE {t} SET outcome = ?, reviewer_id = ?, review_note = ?, reviewed_at = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[text(outcome.as_str()), reviewer_id.into(), text(note), DbValue::Time(now), record.id.into(), record.version.into()]).await?;
    Ok(())
}
