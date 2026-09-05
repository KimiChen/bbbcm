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
