//! WP-MVP-D01 案件仓储。借用调用方事务、绝不提交，也不做授权判定。
use crate::{
    cases::{CaseStatus, ReportCategory, TargetKind},
    contract::ModerationError,
};
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::{DbValue, TransactionConnection};

/// 未关闭的哨兵值。与其他领域一致，用 Unix 纪元表示「尚未发生」。
fn never() -> DbValue {
    DbValue::Time(DateTime::UNIX_EPOCH)
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

/// 优先级：1 最高（高危），3 最低。用小数值表示高优先级，便于队列直接按升序排。
pub const PRIORITY_URGENT: i64 = 1;
pub const PRIORITY_NORMAL: i64 = 2;

#[derive(Debug, Clone)]
pub struct Case {
    pub id: i64,
    pub status: CaseStatus,
    pub priority: i64,
    pub escalated: bool,
    pub report_count: i64,
    pub version: i64,
}

/// 查询某个目标当前未关闭的案件。
///
/// 「未关闭」以 `closed_at` 为纪元判定，而不是看 status：`resolved` 的案件仍可能被申诉，
/// 新举报应当挂上去而不是另开一个案件。
pub async fn open_case(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    kind: TargetKind,
    target_id: i64,
) -> Result<Option<Case>, ModerationError> {
    let t = table(tx, r, "cases")?;
    tx.fetch_optional(&format!("SELECT id, status, priority, escalated, report_count, version FROM {t} WHERE target_kind = ? AND target_id = ? AND closed_at = ? ORDER BY id"),
        &[text(kind.as_str()), target_id.into(), never()]).await?
        .map(|row| Ok(Case {
            id: row.i64("id")?,
            status: CaseStatus::parse(&row.text("status")?)?,
            priority: row.i64("priority")?,
            escalated: row.boolean("escalated")?,
            report_count: row.i64("report_count")?,
            version: row.i64("version")?,
        }))
        .transpose()
}

/// 案件队列行：比 `Case` 多带目标信息，供 `/manage` 队列渲染。
#[derive(Debug, Clone)]
pub struct QueueRow {
    pub id: i64,
    pub target_kind: String,
    pub target_id: i64,
    pub status: CaseStatus,
    pub priority: i64,
    pub escalated: bool,
    pub report_count: i64,
}

/// 未关闭案件队列，按优先级升序（1 最高）、再按 ID。
///
/// 只返回未关闭案件：已关闭的属历史，不占工作队列。排序把高危顶到最前，
/// 这与 `MOD-012` 「拖延本身就是伤害」的分流意图一致。
pub async fn queue(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    limit: i64,
) -> Result<Vec<QueueRow>, ModerationError> {
    let t = table(tx, r, "cases")?;
    let rows = tx.fetch_all(&format!("SELECT id, target_kind, target_id, status, priority, escalated, report_count FROM {t} WHERE closed_at = ? ORDER BY priority, id LIMIT ?"),
        &[never(), limit.into()]).await?;
    rows.iter()
        .map(|row| {
            Ok(QueueRow {
                id: row.i64("id")?,
                target_kind: row.text("target_kind")?,
                target_id: row.i64("target_id")?,
                status: CaseStatus::parse(&row.text("status")?)?,
                priority: row.i64("priority")?,
                escalated: row.boolean("escalated")?,
                report_count: row.i64("report_count")?,
            })
        })
        .collect()
}

pub async fn case(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
) -> Result<Option<Case>, ModerationError> {
    let t = table(tx, r, "cases")?;
    tx.fetch_optional(
        &format!(
            "SELECT id, status, priority, escalated, report_count, version FROM {t} WHERE id = ?"
        ),
        &[id.into()],
    )
    .await?
    .map(|row| {
        Ok(Case {
            id: row.i64("id")?,
            status: CaseStatus::parse(&row.text("status")?)?,
            priority: row.i64("priority")?,
            escalated: row.boolean("escalated")?,
            report_count: row.i64("report_count")?,
            version: row.i64("version")?,
        })
    })
    .transpose()
}

pub async fn insert_case(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    kind: TargetKind,
    target_id: i64,
    urgent: bool,
    now: DateTime<Utc>,
) -> Result<i64, ModerationError> {
    let t = table(tx, r, "cases")?;
    tx.execute(&format!("INSERT INTO {t} (target_kind, target_id, status, priority, escalated, assignee_id, report_count, created_at, updated_at, closed_at, version) VALUES (?, ?, ?, ?, ?, 0, 0, ?, ?, ?, ?)"),
        &[text(kind.as_str()), target_id.into(), text(CaseStatus::Triage.as_str()),
          DbValue::I64(if urgent { PRIORITY_URGENT } else { PRIORITY_NORMAL }),
          DbValue::Bool(urgent), DbValue::Time(now), DbValue::Time(now), never(), DbValue::I64(1)]).await?;
    tx.last_insert_id().await.map_err(Into::into)
}

/// 把既有案件提升到最高优先级并打升级标志。
///
/// 只升不降：先到的高危举报不应被后来的一条「其他」冲淡。已是最高优先级且已升级时
/// 直接返回，避免无意义地推高版本号。
pub async fn escalate(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    case: &Case,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    if case.escalated && case.priority <= PRIORITY_URGENT {
        return Ok(());
    }
    let t = table(tx, r, "cases")?;
    tx.execute_versioned(&format!("UPDATE {t} SET priority = ?, escalated = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[DbValue::I64(PRIORITY_URGENT), DbValue::Bool(true), DbValue::Time(now), case.id.into(), case.version.into()]).await?;
    Ok(())
}

pub async fn set_status(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    case: &Case,
    next: CaseStatus,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let t = table(tx, r, "cases")?;
    // 进入 closed 的同时落 closed_at：后续新举报据此另开新案件，而不是复活终态案件。
    let closed = if next == CaseStatus::Closed {
        DbValue::Time(now)
    } else {
        never()
    };
    tx.execute_versioned(&format!("UPDATE {t} SET status = ?, closed_at = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[text(next.as_str()), closed, DbValue::Time(now), case.id.into(), case.version.into()]).await?;
    Ok(())
}

pub async fn set_assignee(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    case: &Case,
    assignee_id: i64,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let t = table(tx, r, "cases")?;
    tx.execute_versioned(&format!("UPDATE {t} SET assignee_id = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[assignee_id.into(), DbValue::Time(now), case.id.into(), case.version.into()]).await?;
    Ok(())
}

/// 登记一条举报。已存在同一举报人对同一案件的举报时返回假且不新增行。
///
/// 先查后插而不是依赖唯一键冲突：DEC-022 的三库交集子集禁用 `ON CONFLICT`，
/// 且这里需要把「重复」当成正常业务结果返回，而不是当成错误。
pub async fn insert_report(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    case_id: i64,
    reporter_id: i64,
    category: ReportCategory,
    note: &str,
    now: DateTime<Utc>,
) -> Result<bool, ModerationError> {
    let t = table(tx, r, "reports")?;
    if tx
        .fetch_optional(
            &format!("SELECT id FROM {t} WHERE case_id = ? AND reporter_id = ?"),
            &[case_id.into(), reporter_id.into()],
        )
        .await?
        .is_some()
    {
        return Ok(false);
    }
    tx.execute(&format!("INSERT INTO {t} (case_id, reporter_id, category, note, created_at, version) VALUES (?, ?, ?, ?, ?, ?)"),
        &[case_id.into(), reporter_id.into(), text(category.as_str()), text(note), DbValue::Time(now), DbValue::I64(1)]).await?;
    let cases = table(tx, r, "cases")?;
    tx.execute(
        &format!("UPDATE {cases} SET report_count = report_count + 1, updated_at = ?, version = version + 1 WHERE id = ?"),
        &[DbValue::Time(now), case_id.into()],
    )
    .await?;
    Ok(true)
}

/// MOD-008：保存带 SHA-256 摘要与时间戳的受限证据快照。
///
/// 摘要由领域层计算后传入，仓储层只负责落库——摘要算法属业务事实，
/// 放在仓储里会让「怎么算的」散落在数据访问层，也违反本项目的仓储 SQL 静态守卫。
/// 正文属 C2，只能由授权管理路径读取。
pub async fn insert_evidence(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    case_id: i64,
    body: &str,
    digest: &str,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let t = table(tx, r, "evidence_snapshots")?;
    tx.execute(&format!("INSERT INTO {t} (case_id, captured_body, body_digest, captured_at, version) VALUES (?, ?, ?, ?, ?)"),
        &[case_id.into(), text(body), text(digest), DbValue::Time(now), DbValue::I64(1)]).await?;
    Ok(())
}

/// 读取某案件的证据摘要清单。只返回摘要与时间，不返回正文。
///
/// 正文要单独走授权读取路径并留敏感访问审计，避免任何列表接口顺手把 C2 正文带出去。
pub async fn evidence_digests(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    case_id: i64,
) -> Result<Vec<(String, DateTime<Utc>)>, ModerationError> {
    let t = table(tx, r, "evidence_snapshots")?;
    tx.fetch_all(
        &format!("SELECT body_digest, captured_at FROM {t} WHERE case_id = ? ORDER BY id"),
        &[case_id.into()],
    )
    .await?
    .iter()
    .map(|row| Ok((row.text("body_digest")?, row.time("captured_at")?)))
    .collect()
}

pub async fn insert_note(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    case_id: i64,
    author_id: i64,
    body: &str,
    now: DateTime<Utc>,
) -> Result<(), ModerationError> {
    let t = table(tx, r, "case_notes")?;
    tx.execute(
        &format!(
            "INSERT INTO {t} (case_id, author_id, body, created_at, version) VALUES (?, ?, ?, ?, ?)"
        ),
        &[
            case_id.into(),
            author_id.into(),
            text(body),
            DbValue::Time(now),
            DbValue::I64(1),
        ],
    )
    .await?;
    Ok(())
}
