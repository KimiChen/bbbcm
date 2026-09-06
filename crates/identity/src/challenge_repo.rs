//! Challenge SQL borrows only the current identity transaction.
use crate::repo::{table, text};
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::{DbRow, DbValue, TransactionConnection, TransactionError};

pub(crate) struct ChallengeRecord {
    pub challenge_id: String,
    pub principal_id: i64,
    pub action: String,
    pub request_hash: String,
    pub proof_hash: String,
    pub nonce: String,
    pub key_version: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed: bool,
}
fn record(row: DbRow) -> Result<ChallengeRecord, TransactionError> {
    Ok(ChallengeRecord {
        challenge_id: row.text("challenge_id")?,
        principal_id: row.i64("principal_id")?,
        action: row.text("action")?,
        request_hash: row.text("request_hash")?,
        proof_hash: row.text("proof_hash")?,
        nonce: row.text("nonce")?,
        key_version: row.text("key_version")?,
        created_at: row.time("created_at")?,
        expires_at: row.time("expires_at")?,
        consumed: row.boolean("consumed")?,
    })
}
pub(crate) async fn by_reference(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: &str,
) -> Result<Option<ChallengeRecord>, TransactionError> {
    let t = table(tx, r, "challenges")?;
    tx.fetch_optional(&format!("SELECT challenge_id, principal_id, action, request_hash, proof_hash, nonce, key_version, created_at, expires_at, consumed FROM {t} WHERE challenge_id = ?"), &[text(id)]).await?.map(record).transpose()
}
pub(crate) async fn by_binding(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    principal_id: i64,
    action: &str,
    hash: &str,
) -> Result<Option<ChallengeRecord>, TransactionError> {
    let t = table(tx, r, "challenges")?;
    tx.fetch_optional(&format!("SELECT challenge_id, principal_id, action, request_hash, proof_hash, nonce, key_version, created_at, expires_at, consumed FROM {t} WHERE principal_id = ? AND action = ? AND request_hash = ?"), &[principal_id.into(),text(action),text(hash)]).await?.map(record).transpose()
}
pub(crate) async fn insert(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    record: &ChallengeRecord,
) -> Result<bool, TransactionError> {
    let t = table(tx, r, "challenges")?;
    tx.insert_if_absent(&format!("INSERT INTO {t} (challenge_id, principal_id, action, request_hash, proof_hash, nonce, key_version, created_at, expires_at, consumed) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"), &[text(&record.challenge_id),record.principal_id.into(),text(&record.action),text(&record.request_hash),text(&record.proof_hash),text(&record.nonce),text(&record.key_version),DbValue::Time(record.created_at),DbValue::Time(record.expires_at),DbValue::Bool(false)]).await
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn consume(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: &str,
    principal_id: i64,
    action: &str,
    request_hash: &str,
    proof_hash: &str,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "challenges")?;
    let changed = tx.execute(&format!("UPDATE {t} SET consumed = ? WHERE challenge_id = ? AND principal_id = ? AND action = ? AND request_hash = ? AND proof_hash = ? AND consumed = ? AND expires_at > ?"), &[DbValue::Bool(true),text(id),principal_id.into(),text(action),text(request_hash),text(proof_hash),DbValue::Bool(false),DbValue::Time(now)]).await?;
    if changed != 1 {
        return Err(TransactionError::InvalidInput);
    }
    Ok(())
}
