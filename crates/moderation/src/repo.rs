//! Read-only current consent documents; no policy publication capability lives here.
use crate::contract::{PolicyError, PolicyVersion};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::{DbValue, TransactionConnection};
use std::collections::BTreeMap;

pub(crate) async fn current_documents(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    allow_synthetic: bool,
) -> Result<Vec<PolicyVersion>, PolicyError> {
    let table = renderer
        .table(tx.backend(), "moderation", "policy_versions")
        .map_err(|_| PolicyError::Unavailable)?;
    let rows = tx.fetch_all(&format!("SELECT id, kind, version, body, status FROM {table} WHERE status = 'published' OR status = ? ORDER BY kind, version DESC"),
        &[DbValue::Text(if allow_synthetic { "synthetic" } else { "published" }.into())]).await.map_err(|_| PolicyError::Unavailable)?;
    let mut documents = BTreeMap::new();
    for row in rows {
        let kind = match row
            .text("kind")
            .map_err(|_| PolicyError::Unavailable)?
            .as_str()
        {
            "terms" => "terms",
            "privacy" => "privacy",
            "community" => "community",
            _ => return Err(PolicyError::Unavailable),
        };
        if documents.contains_key(kind) {
            continue;
        }
        let doc = PolicyVersion {
            id: row.i64("id").map_err(|_| PolicyError::Unavailable)?,
            kind,
            version: row.i64("version").map_err(|_| PolicyError::Unavailable)?,
            body: row.text("body").map_err(|_| PolicyError::Unavailable)?,
            synthetic: row.text("status").map_err(|_| PolicyError::Unavailable)? == "synthetic",
        };
        if doc.id <= 0 || doc.version <= 0 || doc.body.is_empty() {
            return Err(PolicyError::NotReady);
        }
        documents.insert(kind, doc);
    }
    if documents.len() != 3 {
        return Err(PolicyError::NotReady);
    }
    Ok(documents.into_values().collect())
}

pub(crate) async fn content_restricted(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
    capability: crate::contract::ContentCapability,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, platform_db::transaction::TransactionError> {
    let table = renderer.table(tx.backend(), "moderation", "content_restrictions")?;
    let rows = tx.fetch_all(&format!("SELECT id FROM {table} WHERE user_id = ? AND (capability = ? OR capability = ?) AND revoked = ? AND starts_at <= ? AND expires_at > ? ORDER BY id"),
        &[user_id.into(),DbValue::Text("content".into()),DbValue::Text(capability.as_str().into()),DbValue::Bool(false),DbValue::Time(now),DbValue::Time(now)]).await?;
    Ok(!rows.is_empty())
}

pub(crate) async fn published_forum_rule(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    forum_id: i64,
) -> Result<Option<String>, platform_db::transaction::TransactionError> {
    let table = renderer.table(tx.backend(), "moderation", "rule_versions")?;
    tx.fetch_optional(
        &format!(
            "SELECT body FROM {table} WHERE forum_id = ? AND status = ? ORDER BY version DESC"
        ),
        &[forum_id.into(), DbValue::Text("published".into())],
    )
    .await?
    .map(|row| row.text("body"))
    .transpose()
}
