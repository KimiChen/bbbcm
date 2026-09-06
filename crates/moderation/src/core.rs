//! MOD-010 consent reading only. Publishing, staff authorization and legal text are later work.
use crate::contract::{PolicyError, PolicySet, PolicyVersion};
use platform::{config::Environment, identifiers::SchemaRenderer, outbox::BoxFuture};
use platform_db::{policy::PolicyReader, transaction::TransactionConnection};

pub struct CurrentPolicyReader {
    allow_synthetic: bool,
}
impl CurrentPolicyReader {
    pub fn new(environment: Environment) -> Self {
        Self {
            allow_synthetic: matches!(environment, Environment::Development | Environment::Test),
        }
    }
}
impl PolicyReader for CurrentPolicyReader {
    fn current<'a, 'c>(
        &'a self,
        tx: &'a mut TransactionConnection<'c>,
        renderer: &'a SchemaRenderer,
    ) -> BoxFuture<'a, Result<PolicySet, PolicyError>> {
        Box::pin(async move {
            let documents = self.documents(tx, renderer).await?;
            let find = |kind| {
                documents
                    .iter()
                    .find(|v| v.kind == kind)
                    .map(|v| v.id)
                    .ok_or(PolicyError::NotReady)
            };
            let current = PolicySet {
                terms: find("terms")?,
                privacy: find("privacy")?,
                community: find("community")?,
            };
            current.validate()?;
            Ok(current)
        })
    }
    fn documents<'a, 'c>(
        &'a self,
        tx: &'a mut TransactionConnection<'c>,
        renderer: &'a SchemaRenderer,
    ) -> BoxFuture<'a, Result<Vec<PolicyVersion>, PolicyError>> {
        Box::pin(
            async move { crate::repo::current_documents(tx, renderer, self.allow_synthetic).await },
        )
    }
}

#[cfg(test)]
mod tests;

/// Read current restrictions while the caller holds the identity.users root lock.
/// Future restriction writes must take that same root; this slice has no mutation,
/// granting, management UI or application INSERT privilege.
pub async fn content_restricted(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
    capability: crate::contract::ContentCapability,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, platform_db::transaction::TransactionError> {
    if user_id <= 0 {
        return Err(platform_db::transaction::TransactionError::InvalidInput);
    }
    crate::repo::content_restricted(tx, renderer, user_id, capability, now).await
}

/// Current published forum rules only. No publication or moderation command.
pub async fn content_forum_rules(
    tx: &mut platform_db::transaction::TransactionConnection<'_>,
    renderer: &platform::identifiers::SchemaRenderer,
    ids: &[i64],
) -> Result<Vec<(i64, String)>, platform_db::transaction::TransactionError> {
    if ids.iter().any(|id| *id <= 0) {
        return Err(platform_db::transaction::TransactionError::InvalidInput);
    }
    let mut result = Vec::new();
    for id in ids {
        if let Some(body) = crate::repo::published_forum_rule(tx, renderer, *id).await? {
            result.push((*id, body));
        }
    }
    Ok(result)
}
