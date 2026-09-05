//! Application-layer integration: real temporary SQLite plus the real shared scheduler.
use chrono::Utc;
use platform::config::{Config, DatabaseConfig};
use platform::identifiers::SchemaRenderer;
use platform::outbox::{BoxFuture, OutboxEvent, SafePayload, Sink, SinkError};
use platform_db::{
    Pool,
    outbox_repo::{self, DbOutboxStore},
    transaction::{DbTransaction, DbValue},
};
use platform_outbox::{
    ClosedExternalEffects, HostLock, LocalSink, Scheduler, Settings, SinkRegistry,
};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

struct Fixture {
    dir: PathBuf,
    cfg: DatabaseConfig,
    pool: Pool,
    renderer: SchemaRenderer,
}
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-worker-test-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default().database;
        cfg.sqlite_path = dir.join("worker.sqlite3").to_string_lossy().into();
        platform_db::migrations::migrate(
            &cfg,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        std::fs::write(dir.join("config.toml"), "[server]\nenvironment='test'\n").unwrap();
        let pool = Pool::connect(&cfg).await.unwrap();
        let renderer = SchemaRenderer::new(&cfg.schema_prefix).unwrap();
        Self {
            dir,
            cfg,
            pool,
            renderer,
        }
    }
    async fn enqueue(&self, sinks: &[&str]) -> OutboxEvent {
        let event = OutboxEvent {
            event_id: platform::transaction::new_operation_id(),
            aggregate_type: "local".into(),
            aggregate_id: "1".into(),
            event_type: "local.probe".into(),
            version: 1,
            occurred_at: Utc::now() - chrono::Duration::seconds(1),
            payload: SafePayload::empty(),
            trace_id: String::new(),
        };
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        outbox_repo::enqueue(&mut tx.connection(), &self.renderer, &event, sinks)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        event
    }
    async fn state(&self, event: &OutboxEvent) -> (String, i64) {
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        let table = self
            .renderer
            .table(self.pool.backend(), "platform", "outbox_events")
            .unwrap();
        let row = tx
            .connection()
            .fetch_optional(
                &format!("SELECT state, attempts FROM {table} WHERE event_id = ?"),
                &[DbValue::Text(event.event_id.clone())],
            )
            .await
            .unwrap()
            .unwrap();
        tx.commit().await.unwrap();
        (row.text("state").unwrap(), row.i64("attempts").unwrap())
    }
    fn scheduler(&self, sinks: Vec<Arc<dyn Sink>>) -> Scheduler {
        let mut registry = SinkRegistry::default();
        for sink in sinks {
            registry.register(sink).unwrap();
        }
        Scheduler::new(
            Arc::new(DbOutboxStore::new(self.pool.clone(), self.renderer.clone())),
            registry,
            Arc::new(ClosedExternalEffects),
            Settings {
                poll_interval: Duration::from_millis(3),
                lease_duration: Duration::from_millis(150),
                renew_interval: Duration::from_millis(20),
                retry_base: Duration::from_millis(5),
                retry_max: Duration::from_millis(20),
                max_attempts: 3,
                ..Settings::default()
            },
        )
        .unwrap()
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_bbbcm-worker"));
        cmd.env_clear()
            .current_dir(&self.dir)
            .env("BBBCM_CONFIG", self.dir.join("config.toml"))
            .env("BBBCM_SERVER_ENVIRONMENT", "test")
            .env("BBBCM_DATABASE_SQLITE_PATH", &self.cfg.sqlite_path)
            .env("BBBCM_WORKER_EMBEDDED", "false")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn shared_scheduler_consumes_persistent_events_and_deduplicates_on_restart() {
    let f = Fixture::new().await;
    let event = f.enqueue(&["local.probe"]).await;
    let local = Arc::new(LocalSink::default());
    let scheduler = f.scheduler(vec![local.clone()]);
    assert_eq!(scheduler.tick(&CancellationToken::new()).await.unwrap(), 1);
    assert_eq!(f.state(&event).await.0, "delivered");
    assert_eq!(local.consumed(), 1);
    let restarted = f.scheduler(vec![local.clone()]);
    assert_eq!(restarted.tick(&CancellationToken::new()).await.unwrap(), 0);
    assert_eq!(local.consumed(), 1);
    f.pool.close().await;
}
struct FaultSink {
    calls: AtomicUsize,
    retryable: bool,
    external: bool,
}
impl Sink for FaultSink {
    fn name(&self) -> &str {
        "local.fault"
    }
    fn is_external(&self) -> bool {
        self.external
    }
    fn deliver<'a>(&'a self, _: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(if self.retryable {
                SinkError::Retryable
            } else {
                SinkError::Permanent
            })
        })
    }
}
#[tokio::test]
async fn scheduler_applies_backoff_retry_budget_dlq_and_external_stop_check() {
    let f = Fixture::new().await;
    let event = f.enqueue(&["local.fault"]).await;
    let faults = Arc::new(FaultSink {
        calls: AtomicUsize::new(0),
        retryable: true,
        external: false,
    });
    let scheduler = f.scheduler(vec![faults.clone()]);
    let cancel = CancellationToken::new();
    scheduler.tick(&cancel).await.unwrap();
    assert_eq!(f.state(&event).await.0, "ready");
    assert_eq!(scheduler.tick(&cancel).await.unwrap(), 0);
    tokio::time::sleep(Duration::from_millis(12)).await;
    scheduler.tick(&cancel).await.unwrap();
    tokio::time::sleep(Duration::from_millis(22)).await;
    scheduler.tick(&cancel).await.unwrap();
    assert_eq!(f.state(&event).await, ("dlq".into(), 3));
    assert_eq!(faults.calls.load(Ordering::SeqCst), 3);
    let disabled = f.enqueue(&["local.fault"]).await;
    let external = Arc::new(FaultSink {
        calls: AtomicUsize::new(0),
        retryable: false,
        external: true,
    });
    f.scheduler(vec![external.clone()])
        .tick(&cancel)
        .await
        .unwrap();
    assert_eq!(external.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.state(&disabled).await, ("ready".into(), 0));
    f.pool.close().await;
}
struct SlowSink {
    entered: tokio::sync::Notify,
    finish: bool,
}
impl Sink for SlowSink {
    fn name(&self) -> &str {
        "local.slow"
    }
    fn is_external(&self) -> bool {
        false
    }
    fn deliver<'a>(&'a self, _: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            self.entered.notify_one();
            if self.finish {
                tokio::time::sleep(Duration::from_millis(240)).await;
                Ok(())
            } else {
                std::future::pending().await
            }
        })
    }
}
#[tokio::test]
async fn renews_long_running_batch_and_cancellation_releases_unfinished_work() {
    let f = Fixture::new().await;
    let first = f.enqueue(&["local.slow"]).await;
    let second = f.enqueue(&["local.slow"]).await;
    let slow = Arc::new(SlowSink {
        entered: tokio::sync::Notify::new(),
        finish: true,
    });
    f.scheduler(vec![slow])
        .tick(&CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(f.state(&first).await.0, "delivered");
    assert_eq!(f.state(&second).await.0, "delivered");
    let unfinished = f.enqueue(&["local.slow"]).await;
    let blocked = Arc::new(SlowSink {
        entered: tokio::sync::Notify::new(),
        finish: false,
    });
    let scheduler = f.scheduler(vec![blocked.clone()]);
    let cancel = CancellationToken::new();
    let run_cancel = cancel.clone();
    let task = tokio::spawn(async move { scheduler.run(run_cancel).await });
    tokio::time::timeout(Duration::from_secs(1), blocked.entered.notified())
        .await
        .unwrap();
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(f.state(&unfinished).await, ("ready".into(), 0));
    let resumed = Arc::new(SlowSink {
        entered: tokio::sync::Notify::new(),
        finish: true,
    });
    f.scheduler(vec![resumed])
        .tick(&CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(f.state(&unfinished).await.0, "delivered");
    f.pool.close().await;
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
async fn wait_state(f: &Fixture, event: &OutboxEvent, expected: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if f.state(event).await.0 == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn actual_worker_process_consumes_and_os_lock_rejects_other_host_then_recovers() {
    let f = Fixture::new().await;
    let first = f.enqueue(&["local.probe"]).await;
    let mut child = Process(f.command().spawn().unwrap());
    wait_state(&f, &first, "delivered").await;
    assert!(child.0.try_wait().unwrap().is_none());
    let second_host = f.command().output().unwrap();
    assert!(!second_host.status.success());
    assert!(
        HostLock::acquire(&f.cfg).is_err(),
        "an embedded host must use this same lock"
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let lock = HostLock::acquire(&f.cfg).unwrap();
    drop(lock);
    let second = f.enqueue(&["local.probe"]).await;
    let mut restarted = Process(f.command().spawn().unwrap());
    wait_state(&f, &second, "delivered").await;
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-TERM", &restarted.0.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let exited = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(status) = restarted.0.try_wait().unwrap() {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            exited.success(),
            "SIGTERM should complete graceful cancellation"
        );
    }
    f.pool.close().await;
}
#[test]
fn retry_backoff_is_bounded_and_registry_rejects_duplicate_names() {
    let settings = Settings::default();
    assert_eq!(settings.backoff(1), Duration::from_secs(1));
    assert_eq!(settings.backoff(4), Duration::from_secs(8));
    assert_eq!(settings.backoff(i64::MAX), settings.retry_max);
    let mut registry = SinkRegistry::default();
    registry.register(Arc::new(LocalSink::default())).unwrap();
    assert!(registry.register(Arc::new(LocalSink::default())).is_err());
}

struct OnceFailingSink(AtomicUsize);
impl Sink for OnceFailingSink {
    fn name(&self) -> &str {
        "local.once"
    }
    fn is_external(&self) -> bool {
        false
    }
    fn deliver<'a>(&'a self, _: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(SinkError::Permanent)
            } else {
                Ok(())
            }
        })
    }
}
struct SyntheticReplay(SchemaRenderer);
impl outbox_repo::ReplayPolicy for SyntheticReplay {
    fn authorize<'a, 'c>(
        &'a self,
        _: &'a mut platform_db::transaction::TransactionConnection<'c>,
        request: &'a platform::outbox::ReplayRequest,
    ) -> BoxFuture<'a, Result<(), platform::outbox::OutboxError>> {
        Box::pin(async move {
            if request.actor_id == "1" && request.reason_code == "local.retry" {
                Ok(())
            } else {
                Err(platform::outbox::OutboxError::ReplayDenied)
            }
        })
    }
    fn append_audit<'a, 'c>(
        &'a self,
        tx: &'a mut platform_db::transaction::TransactionConnection<'c>,
        request: &'a platform::outbox::ReplayRequest,
    ) -> BoxFuture<'a, Result<(), platform::outbox::OutboxError>> {
        Box::pin(async move {
            let mut audit = platform::audit::AuditEvent::synthetic(1, request.operation_id.clone());
            audit.action = platform::audit::AuditAction::OutboxReplay;
            audit.reason = platform::audit::AuditReason::Recovery;
            audit.target = platform::audit::AuditTarget {
                kind: platform::audit::TargetKind::Outbox,
                id: request.event_id.clone(),
            };
            platform_db::audit_repo::append_audit(tx, &self.0, &audit)
                .await
                .map_err(|_| platform::outbox::OutboxError::Unavailable)
        })
    }
}
#[tokio::test]
async fn local_sink_executes_failure_dlq_authorized_audited_replay_and_success() {
    let f = Fixture::new().await;
    let event = f.enqueue(&["local.once"]).await;
    let sink = Arc::new(OnceFailingSink(AtomicUsize::new(0)));
    let replica = Arc::new(platform_audit::ReplicaSink::new(
        Arc::new(platform_db::audit_repo::DbAuditExport::new(
            f.pool.clone(),
            f.renderer.clone(),
        )),
        Arc::new(platform_audit::DisabledReplica),
    ));
    let scheduler = f.scheduler(vec![sink.clone(), replica]);
    scheduler.tick(&CancellationToken::new()).await.unwrap();
    assert_eq!(f.state(&event).await.0, "dlq");
    let request = platform::outbox::ReplayRequest {
        event_id: event.event_id.clone(),
        actor_id: "1".into(),
        reason_code: "local.retry".into(),
        operation_id: platform::transaction::new_operation_id(),
    };
    let store = DbOutboxStore::new(f.pool.clone(), f.renderer.clone());
    store
        .replay(&request, &SyntheticReplay(f.renderer.clone()), Utc::now())
        .await
        .unwrap();
    scheduler.tick(&CancellationToken::new()).await.unwrap();
    assert_eq!(f.state(&event).await.0, "delivered");
    assert_eq!(sink.0.load(Ordering::SeqCst), 2);
    assert!(
        store
            .replay(&request, &SyntheticReplay(f.renderer.clone()), Utc::now())
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(f.state(&event).await.0, "delivered");
    f.pool.close().await;
}
