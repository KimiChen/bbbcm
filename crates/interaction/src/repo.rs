//! 互动领域的存储访问。只借用当前事务连接，不持有池，也不开事务——
//! 事务边界由调用方掌握，否则「反馈与通知同事务」这类要求无从表达。
use crate::contract::{FeedbackTarget, InteractionError};
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::{DbValue, TransactionConnection};

type Result<T> = std::result::Result<T, InteractionError>;

fn table(c: &TransactionConnection<'_>, r: &SchemaRenderer, name: &str) -> Result<String> {
    r.table(c.backend(), "interaction", name)
        .map_err(|_| InteractionError::Unavailable)
}

/// 当前用户对该对象的反馈行：`(id, withdrawn, version)`。
pub async fn current(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    target: FeedbackTarget,
    target_id: i64,
) -> Result<Option<(i64, bool, i64)>> {
    let t = table(c, r, "feedback")?;
    let row = c
        .fetch_optional(
            &format!(
                "SELECT id, withdrawn, version FROM {t} \
                 WHERE user_id = ? AND target_kind = ? AND target_id = ?"
            ),
            &[
                DbValue::I64(user_id),
                DbValue::Text(target.as_str().into()),
                DbValue::I64(target_id),
            ],
        )
        .await?;
    row.map(|row| {
        Ok((
            row.i64("id")?,
            row.boolean("withdrawn")?,
            row.i64("version")?,
        ))
    })
    .transpose()
}

/// 首次反馈。唯一约束由数据库兜底：并发下第二路会命中 `feedback_once`，
/// 由调用方按 `Conflict` 重读处理，而不是先查再写。
pub async fn insert(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    target: FeedbackTarget,
    target_id: i64,
    author_id: i64,
    now: DateTime<Utc>,
) -> Result<()> {
    let t = table(c, r, "feedback")?;
    c.execute(
        &format!(
            "INSERT INTO {t} (user_id, target_kind, target_id, author_id, withdrawn, \
             created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, 1)"
        ),
        &[
            DbValue::I64(user_id),
            DbValue::Text(target.as_str().into()),
            DbValue::I64(target_id),
            DbValue::I64(author_id),
            DbValue::Bool(false),
            DbValue::Time(now),
            DbValue::Time(now),
        ],
    )
    .await?;
    Ok(())
}

/// 翻转撤销标记。带版本 CAS：并发撤销/重做不会互相覆盖。
pub async fn set_withdrawn(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
    version: i64,
    withdrawn: bool,
    now: DateTime<Utc>,
) -> Result<()> {
    let t = table(c, r, "feedback")?;
    c.execute_versioned(
        &format!(
            "UPDATE {t} SET withdrawn = ?, updated_at = ?, version = version + 1 \
             WHERE id = ? AND version = ?"
        ),
        &[
            DbValue::Bool(withdrawn),
            DbValue::Time(now),
            DbValue::I64(id),
            DbValue::I64(version),
        ],
    )
    .await?;
    Ok(())
}

/// 有效反馈数：已撤销的不计入。
///
/// 不维护冗余计数列——冗余计数一旦与唯一约束不一致，排查成本远高于一次聚合。
pub async fn count(
    c: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    target: FeedbackTarget,
    target_id: i64,
) -> Result<i64> {
    let t = table(c, r, "feedback")?;
    let row = c
        .fetch_optional(
            &format!(
                "SELECT count(*) AS total FROM {t} \
                 WHERE target_kind = ? AND target_id = ? AND withdrawn = ?"
            ),
            &[
                DbValue::Text(target.as_str().into()),
                DbValue::I64(target_id),
                DbValue::Bool(false),
            ],
        )
        .await?
        .ok_or(InteractionError::Unavailable)?;
    Ok(row.i64("total")?)
}
