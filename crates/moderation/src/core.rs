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
