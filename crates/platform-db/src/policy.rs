//! Database-aware injection boundary. Applications supply a moderation core service;
//! identity depends on this contract without depending on the moderation crate.
use crate::transaction::TransactionConnection;
use platform::{
    identifiers::SchemaRenderer,
    outbox::BoxFuture,
    policy::{PolicyError, PolicySet, PolicyVersion},
};

pub trait PolicyReader: Send + Sync {
    fn current<'a, 'c>(
        &'a self,
        tx: &'a mut TransactionConnection<'c>,
        renderer: &'a SchemaRenderer,
    ) -> BoxFuture<'a, Result<PolicySet, PolicyError>>;

    fn documents<'a, 'c>(
        &'a self,
        tx: &'a mut TransactionConnection<'c>,
        renderer: &'a SchemaRenderer,
    ) -> BoxFuture<'a, Result<Vec<PolicyVersion>, PolicyError>>;

    fn require_current<'a, 'c>(
        &'a self,
        tx: &'a mut TransactionConnection<'c>,
        renderer: &'a SchemaRenderer,
        supplied: &'a PolicySet,
    ) -> BoxFuture<'a, Result<(), PolicyError>> {
        Box::pin(async move {
            supplied.validate()?;
            if self.current(tx, renderer).await? != *supplied {
                return Err(PolicyError::Changed);
            }
            Ok(())
        })
    }
}
