use super::*;
use platform::config::{Config, DatabaseConfig};
use std::path::{Path, PathBuf};

struct Fixture {
    dir: PathBuf,
    cfg: DatabaseConfig,
    pool: Pool,
    renderer: SchemaRenderer,
}
impl Fixture {
    async fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-outbox-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default().database;
        cfg.sqlite_path = dir.join("outbox.sqlite3").to_string_lossy().into();
        cfg.schema_prefix = prefix.into();
        crate::migrations::migrate(
            &cfg,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&cfg).await.unwrap();
        let renderer = SchemaRenderer::new(prefix).unwrap();
        Self {
            dir,
            cfg,
            pool,
            renderer,
        }
    }
    fn store(&self) -> DbOutboxStore {
        DbOutboxStore::new(self.pool.clone(), self.renderer.clone())
    }
    async fn event(&self, sinks: &[&str]) -> OutboxEvent {
        let event = event().with_inherited_trace().unwrap();
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        enqueue(&mut tx.connection(), &self.renderer, &event, sinks)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        event
    }
    async fn state(&self, event: &OutboxEvent) -> (String, i64) {
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        let events = table(&self.renderer, self.pool.backend(), "outbox_events").unwrap();
        let row = tx
            .connection()
            .fetch_optional(
                &format!("SELECT state, attempts FROM {events} WHERE event_id = ?"),
                &[text(&event.event_id)],
            )
            .await
            .unwrap()
            .unwrap();
        tx.commit().await.unwrap();
        (row.text("state").unwrap(), row.i64("attempts").unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn now() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-05T01:00:00Z").unwrap()
}
fn event() -> OutboxEvent {
    OutboxEvent {
        event_id: platform::transaction::new_operation_id(),
        aggregate_type: "local".into(),
        aggregate_id: "1".into(),
        event_type: "local.probe".into(),
        version: 1,
        occurred_at: now(),
        payload: SafePayload::empty(),
        trace_id: String::new(),
    }
}
fn request(owner: &str, at: DateTime<Utc>, limit: u32) -> Claim {
    Claim {
        owner: owner.into(),
        batch_id: platform::transaction::new_operation_id(),
        now: at,
        lease_until: at + chrono::Duration::seconds(10),
        limit,
    }
}

#[tokio::test]
async fn enqueue_rollback_and_persistence_use_actual_sqlite_and_both_prefixes() {
    for prefix in ["bcm_", "ob_test_"] {
        let f = Fixture::new(prefix).await;
        let aborted = event();
        let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
        enqueue(
            &mut tx.connection(),
            &f.renderer,
            &aborted,
            &["local.probe"],
        )
        .await
        .unwrap();
        tx.rollback().await.unwrap();
        assert!(
            f.store()
                .claim(request("worker", now(), 10))
                .await
                .unwrap()
                .is_empty()
        );
        let saved = f.event(&["local.probe"]).await;
        f.pool.close().await;
        let restarted = Pool::connect(&f.cfg).await.unwrap();
        let store = DbOutboxStore::new(restarted.clone(), f.renderer.clone());
        let batch = store.claim(request("restart", now(), 10)).await.unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].event, saved);
        restarted.close().await;
    }
}
#[tokio::test]
async fn leases_isolate_batches_recover_expiry_and_reject_previous_holder() {
    let f = Fixture::new("bcm_").await;
    f.event(&["local.probe"]).await;
    f.event(&["local.probe"]).await;
    let store = f.store();
    let first = store.claim(request("same-owner", now(), 1)).await.unwrap();
    let second = store.claim(request("same-owner", now(), 1)).await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_ne!(first[0].event.event_id, second[0].event.event_id);
    assert_ne!(first[0].batch_id, second[0].batch_id);
    assert!(
        store
            .claim(request("other", now(), 10))
            .await
            .unwrap()
            .is_empty()
    );
    store
        .renew(
            &first[0],
            now() + chrono::Duration::seconds(9),
            now() + chrono::Duration::seconds(20),
        )
        .await
        .unwrap();
    let recovered = store
        .claim(request("new", now() + chrono::Duration::seconds(11), 10))
        .await
        .unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].event.event_id, second[0].event.event_id);
    let after = now() + chrono::Duration::seconds(12);
    assert_eq!(
        store
            .renew(&second[0], after, after + chrono::Duration::seconds(10))
            .await,
        Err(OutboxError::LeaseLost)
    );
    assert_eq!(
        store.delivery_done(&second[0], "local.probe", after).await,
        Err(OutboxError::LeaseLost)
    );
    assert_eq!(
        store.complete(&second[0], after).await,
        Err(OutboxError::LeaseLost)
    );
    assert_eq!(
        store
            .fail(
                &second[0],
                Failure {
                    code: FailureCode::Retryable,
                    now: after,
                    available_at: after,
                    dead_letter: false
                }
            )
            .await,
        Err(OutboxError::LeaseLost)
    );
    f.pool.close().await;
}
#[tokio::test]
async fn dedupe_survives_retry_and_completed_event_is_not_claimed_again() {
    let f = Fixture::new("bcm_").await;
    let event = f.event(&["local.one", "local.two"]).await;
    let store = f.store();
    let first = store
        .claim(request("worker", now(), 10))
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        store.complete(&first, now()).await,
        Err(OutboxError::InvalidInput)
    );
    store
        .delivery_done(&first, "local.one", now())
        .await
        .unwrap();
    store
        .delivery_done(&first, "local.one", now())
        .await
        .unwrap();
    let later = now() + chrono::Duration::seconds(3);
    store
        .fail(
            &first,
            Failure {
                code: FailureCode::Retryable,
                now: now(),
                available_at: later,
                dead_letter: false,
            },
        )
        .await
        .unwrap();
    assert!(
        store
            .claim(request("worker", now(), 10))
            .await
            .unwrap()
            .is_empty()
    );
    let retry = store
        .claim(request("worker", later, 10))
        .await
        .unwrap()
        .remove(0);
    assert_eq!(retry.pending_sinks, vec!["local.two"]);
    assert_eq!(retry.attempts, 2);
    store
        .delivery_done(&retry, "local.two", later)
        .await
        .unwrap();
    store.complete(&retry, later).await.unwrap();
    assert_eq!(f.state(&event).await.0, "delivered");
    assert!(
        store
            .claim(request("worker", later + chrono::Duration::days(1), 10))
            .await
            .unwrap()
            .is_empty()
    );
    f.pool.close().await;
}
struct TestReplayPolicy {
    allow: bool,
    audit_fails: bool,
    renderer: SchemaRenderer,
}
impl ReplayPolicy for TestReplayPolicy {
    fn authorize<'a, 'c>(
        &'a self,
        _: &'a mut TransactionConnection<'c>,
        request: &'a ReplayRequest,
    ) -> BoxFuture<'a, Result<(), OutboxError>> {
        Box::pin(async move {
            if !self.allow || request.actor_id != "1" || request.reason_code != "local.retry" {
                return Err(OutboxError::ReplayDenied);
            }
            Ok(())
        })
    }
    fn append_audit<'a, 'c>(
        &'a self,
        tx: &'a mut TransactionConnection<'c>,
        request: &'a ReplayRequest,
    ) -> BoxFuture<'a, Result<(), OutboxError>> {
        Box::pin(async move {
            let mut audit = platform::audit::AuditEvent::synthetic(1, request.operation_id.clone());
            audit.action = platform::audit::AuditAction::OutboxReplay;
            audit.reason = platform::audit::AuditReason::Recovery;
            audit.target = platform::audit::AuditTarget {
                kind: platform::audit::TargetKind::Outbox,
                id: request.event_id.clone(),
            };
            if self.audit_fails {
                audit.version = 0;
            }
            crate::audit_repo::append_audit(tx, &self.renderer, &audit)
                .await
                .map_err(unavailable)?;
            Ok(())
        })
    }
}
#[tokio::test]
async fn dlq_replay_requires_authorization_and_atomic_audit_preserves_dedupe() {
    let f = Fixture::new("bcm_").await;
    let event = f.event(&["local.one", "local.two"]).await;
    let store = f.store();
    let lease = store
        .claim(request("worker", now(), 10))
        .await
        .unwrap()
        .remove(0);
    store
        .delivery_done(&lease, "local.one", now())
        .await
        .unwrap();
    store
        .fail(
            &lease,
            Failure {
                code: FailureCode::Permanent,
                now: now(),
                available_at: now(),
                dead_letter: true,
            },
        )
        .await
        .unwrap();
    assert!(
        store
            .claim(request("worker", now(), 10))
            .await
            .unwrap()
            .is_empty()
    );
    let req = ReplayRequest {
        event_id: event.event_id.clone(),
        actor_id: "1".into(),
        reason_code: "local.retry".into(),
        operation_id: platform::transaction::new_operation_id(),
    };
    let denied = TestReplayPolicy {
        allow: false,
        audit_fails: false,
        renderer: f.renderer.clone(),
    };
    assert_eq!(
        store.replay(&req, &denied, now()).await,
        Err(OutboxError::ReplayDenied)
    );
    let failed = TestReplayPolicy {
        allow: true,
        audit_fails: true,
        renderer: f.renderer.clone(),
    };
    assert_eq!(
        store.replay(&req, &failed, now()).await,
        Err(OutboxError::Unavailable)
    );
    assert_eq!(f.state(&event).await.0, "dlq");
    let allowed = TestReplayPolicy {
        allow: true,
        audit_fails: false,
        renderer: f.renderer.clone(),
    };
    assert!(!store.replay(&req, &allowed, now()).await.unwrap().replayed);
    assert!(store.replay(&req, &allowed, now()).await.unwrap().replayed);
    assert_eq!(
        store.replay(&req, &denied, now()).await,
        Err(OutboxError::ReplayDenied),
        "a cached replay result still requires current authorization"
    );
    let mut changed = req.clone();
    changed.reason_code = "other.reason".into();
    assert_eq!(
        store.replay(&changed, &allowed, now()).await,
        Err(OutboxError::ReplayConflict)
    );
    let mut changed_actor = req.clone();
    changed_actor.actor_id = "2".into();
    assert_eq!(
        store.replay(&changed_actor, &allowed, now()).await,
        Err(OutboxError::ReplayConflict)
    );
    let replayed = store
        .claim(request("worker", now(), 10))
        .await
        .unwrap()
        .remove(0);
    assert_eq!(replayed.pending_sinks, vec!["local.two"]);
    assert_eq!(replayed.attempts, 1);
    store
        .fail(
            &replayed,
            Failure {
                code: FailureCode::Permanent,
                now: now(),
                available_at: now(),
                dead_letter: true,
            },
        )
        .await
        .unwrap();
    assert!(store.replay(&req, &allowed, now()).await.unwrap().replayed);
    assert_eq!(
        f.state(&event).await.0,
        "dlq",
        "an old approved operation must not replay a second DLQ cycle"
    );
    assert_eq!(
        store
            .replay(&req, &allowed, now() + chrono::Duration::days(31))
            .await,
        Err(OutboxError::ReplayExpired)
    );
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let audit = f
        .renderer
        .table(f.pool.backend(), "audit", "audit_events")
        .unwrap();
    let row = tx
        .connection()
        .fetch_optional(&format!("SELECT COUNT(*) AS n FROM {audit}"), &[])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.i64("n").unwrap(), 1);
    tx.commit().await.unwrap();
    f.pool.close().await;
}
#[tokio::test]
async fn external_pause_and_cancel_release_without_consuming_retry_budget() {
    let f = Fixture::new("bcm_").await;
    let event = f.event(&["local.probe"]).await;
    let store = f.store();
    for code in [FailureCode::ExternalDisabled, FailureCode::Cancelled] {
        let lease = store
            .claim(request("worker", now(), 10))
            .await
            .unwrap()
            .remove(0);
        store
            .fail(
                &lease,
                Failure {
                    code,
                    now: now(),
                    available_at: now(),
                    dead_letter: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(f.state(&event).await, ("ready".into(), 0));
    }
    f.pool.close().await;
}

#[tokio::test]
async fn appending_outbox_prevents_reverse_business_lock_and_transaction_rolls_back() {
    let f = Fixture::new("bcm_").await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut conn = tx.connection();
    enqueue(&mut conn, &f.renderer, &event(), &["local.probe"])
        .await
        .unwrap();
    let earlier = platform::transaction::LockTarget::new("community", "forums", 1).unwrap();
    assert_eq!(
        conn.lock(&f.renderer, &earlier).await,
        Err(TransactionError::LockOrder)
    );
    tx.rollback().await.unwrap();
    assert!(
        f.store()
            .claim(request("worker", now(), 10))
            .await
            .unwrap()
            .is_empty()
    );
    f.pool.close().await;
}
