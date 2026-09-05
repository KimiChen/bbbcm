//! Optional latency hint installed by the application, never a commit acknowledgement.
use crate::{cache::Wakeup, transaction::CommitOutcome};
use std::sync::{Arc, OnceLock};
static WAKEUP: OnceLock<Arc<dyn Wakeup>> = OnceLock::new();
pub fn install(wakeup: Arc<dyn Wakeup>) -> Result<(), crate::cache::CacheError> {
    WAKEUP
        .set(wakeup)
        .map_err(|_| crate::cache::CacheError::InvalidInput)
}
pub async fn notify_committed<T>(outcome: &CommitOutcome<T>) {
    if let Some(wakeup) = WAKEUP.get() {
        notify_with(outcome, wakeup.as_ref()).await;
    }
}
pub async fn notify_with<T>(outcome: &CommitOutcome<T>, wakeup: &dyn Wakeup) {
    if matches!(outcome, CommitOutcome::Committed(_)) && wakeup.notify().await.is_err() {
        crate::observability::metrics().increment(crate::observability::Metric::RedisError);
        tracing::warn!(
            dependency = "redis",
            outcome = "unavailable",
            "committed event will recover by database polling"
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cache::CacheError, outbox::BoxFuture};
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };
    struct Probe(AtomicUsize);
    impl Wakeup for Probe {
        fn notify(&self) -> BoxFuture<'_, Result<(), CacheError>> {
            Box::pin(async {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(CacheError::Unavailable)
            })
        }
        fn wait(&self, _: Duration) -> BoxFuture<'_, Result<bool, CacheError>> {
            Box::pin(async { Ok(false) })
        }
    }
    #[tokio::test]
    async fn unknown_commit_does_not_signal_and_failed_hint_does_not_undo_commit() {
        let probe = Probe(AtomicUsize::new(0));
        notify_with(
            &CommitOutcome::<()>::Unknown {
                operation_id: crate::transaction::new_operation_id(),
            },
            &probe,
        )
        .await;
        assert_eq!(probe.0.load(Ordering::SeqCst), 0);
        notify_with(&CommitOutcome::Committed(()), &probe).await;
        assert_eq!(probe.0.load(Ordering::SeqCst), 1);
    }
}
