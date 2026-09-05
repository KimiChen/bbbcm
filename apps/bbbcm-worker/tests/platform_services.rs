//! Real application composition: committed SQLite Outbox -> Scheduler -> local files.
use chrono::{DateTime, Utc};
use platform::config::{Config, Environment};
use platform::identifiers::SchemaRenderer;
use platform::integrations::{
    AdapterConfig, AdapterError, AdapterMode, AdapterRuntime, Effect, EffectSource, ObjectArea,
    ObjectKey,
};
use platform::mail::{MailClass, MailMessage, MailMessageSource, MailSink};
use platform::outbox::{BoxFuture, OutboxEvent, SafePayload, Sink};
use platform_db::{
    Pool,
    mail_repo::DbMailStore,
    outbox_repo::{self, DbOutboxStore},
    transaction::{DbTransaction, DbValue},
};
use platform_outbox::{ExternalEffects, Scheduler, Settings, SinkRegistry};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Fixture {
    dir: PathBuf,
    pool: Pool,
    renderer: SchemaRenderer,
    keep: bool,
}
impl Fixture {
    async fn new() -> Self {
        let keep = std::env::var("BBBCM_KEEP_ADAPTER_OUTPUTS").as_deref() == Ok("1");
        let id = platform::transaction::new_operation_id();
        let dir = if keep {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../data/adapter-probe")
                .join(&id);
            std::fs::create_dir_all(&path).unwrap();
            eprintln!("adapter artifacts retained: data/adapter-probe/{id}");
            path.canonicalize().unwrap()
        } else {
            std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("bbbcm-adapter-worker-{id}"))
        };
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default().database;
        cfg.sqlite_path = dir.join("probe.sqlite3").to_string_lossy().into();
        cfg.schema_prefix = "adapter_probe_".into();
        platform_db::migrations::migrate(
            &cfg,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&cfg).await.unwrap();
        let renderer = SchemaRenderer::new(&cfg.schema_prefix).unwrap();
        Self {
            dir,
            pool,
            renderer,
            keep,
        }
    }
    async fn enqueue(&self, event_type: &str, resource_id: &str, sink: &str) -> OutboxEvent {
        let event = OutboxEvent {
            event_id: platform::transaction::new_operation_id(),
            aggregate_type: "local_adapter".into(),
            aggregate_id: resource_id.into(),
            event_type: event_type.into(),
            version: 1,
            occurred_at: clock(),
            payload: SafePayload::empty()
                .with_reference("resource_id", resource_id)
                .unwrap(),
            trace_id: String::new(),
        };
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        outbox_repo::enqueue(&mut tx.connection(), &self.renderer, &event, &[sink])
            .await
            .unwrap();
        tx.commit().await.unwrap();
        event
    }
    async fn state(&self, event: &OutboxEvent) -> (String, i64, String) {
        let name = self
            .renderer
            .table(self.pool.backend(), "platform", "outbox_events")
            .unwrap();
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        let row = tx
            .connection()
            .fetch_optional(
                &format!("SELECT state, attempts, error_code FROM {name} WHERE event_id = ?"),
                &[DbValue::Text(event.event_id.clone())],
            )
            .await
            .unwrap()
            .unwrap();
        tx.rollback().await.unwrap();
        (
            row.text("state").unwrap(),
            row.i64("attempts").unwrap(),
            row.text("error_code").unwrap(),
        )
    }
    fn scheduler(&self, sinks: Vec<Arc<dyn Sink>>) -> Scheduler {
        let mut registry = SinkRegistry::default();
        for sink in sinks {
            registry.register(sink).unwrap();
        }
        Scheduler::new(
            Arc::new(DbOutboxStore::new(self.pool.clone(), self.renderer.clone())),
            registry,
            Arc::new(LocalTestEffects),
            Settings {
                poll_interval: Duration::from_millis(5),
                lease_duration: Duration::from_secs(2),
                renew_interval: Duration::from_millis(100),
                retry_base: Duration::from_millis(5),
                retry_max: Duration::from_millis(20),
                ..Default::default()
            },
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}
/// Deliberate test-only policy; production hosts retain ClosedExternalEffects.
struct LocalTestEffects;
impl ExternalEffects for LocalTestEffects {
    fn allowed(&self) -> bool {
        true
    }
}
fn clock() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-05T00:00:00Z").unwrap()
}
#[derive(Clone)]
struct Source(MailMessage);
impl MailMessageSource for Source {
    fn load<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<MailMessage, AdapterError>> {
        Box::pin(async move {
            if self.0.message_id == id {
                Ok(self.0.clone())
            } else {
                Err(AdapterError::NotFound)
            }
        })
    }
}
struct ObjectSource {
    reference: String,
    key: ObjectKey,
}
impl EffectSource for ObjectSource {
    fn load<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Effect, AdapterError>> {
        Box::pin(async move {
            if id != self.reference {
                return Err(AdapterError::NotFound);
            }
            Ok(Effect::Store {
                key: self.key.clone(),
                bytes: b"synthetic quarantine object through committed Outbox".to_vec(),
                max_bytes: 1024,
            })
        })
    }
}
fn message() -> MailMessage {
    MailMessage {
        message_id: platform::transaction::new_operation_id(),
        recipient_ref: platform::transaction::new_operation_id(),
        class: MailClass::Security,
        from: "noreply@example.test".into(),
        to: "synthetic@example.test".into(),
        subject: "草莓论坛本地测试".into(),
        text: "本邮件由临时 SQLite 已提交 Outbox 和真实 Scheduler 驱动，仅写本地文件。".into(),
        unsubscribe_url: None,
        queued_at: clock(),
    }
}

#[tokio::test]
async fn committed_mail_and_object_effects_produce_files_and_survive_scheduler_restart() {
    let f = Fixture::new().await;
    let mail = message();
    let adapters = AdapterRuntime::assemble(
        &AdapterConfig {
            mode: AdapterMode::LocalTest,
            local_root: Some(f.dir.join("spool").to_string_lossy().into()),
        },
        Environment::Test,
    )
    .unwrap();
    let mail_event = f
        .enqueue("mail.requested", &mail.message_id, "mail.delivery")
        .await;
    let object_ref = platform::transaction::new_operation_id();
    let key = ObjectKey::new(
        ObjectArea::Quarantine,
        &platform::transaction::new_operation_id(),
    )
    .unwrap();
    let object_event = f
        .enqueue(
            "integrations.requested",
            &object_ref,
            "integrations.effects",
        )
        .await;
    let mail_sink = Arc::new(MailSink {
        source: Arc::new(Source(mail)),
        store: Arc::new(DbMailStore::new(f.pool.clone(), f.renderer.clone())),
        provider: adapters.mail.clone(),
    });
    let effects = Arc::new(adapters.effects_sink(Arc::new(ObjectSource {
        reference: object_ref,
        key: key.clone(),
    })));
    let scheduler = f.scheduler(vec![mail_sink.clone(), effects.clone()]);
    assert_eq!(scheduler.tick(&CancellationToken::new()).await.unwrap(), 2);
    assert_eq!(f.state(&mail_event).await.0, "delivered");
    assert_eq!(f.state(&object_event).await.0, "delivered");
    let mime = std::fs::read_to_string(
        f.dir
            .join("spool/mail")
            .join(format!("{}.eml", mail_event.event_id)),
    )
    .unwrap();
    assert!(mime.contains("X-BBBcm-Delivery: local-test-only"));
    assert!(mime.contains("草莓论坛"));
    let bytes = adapters.objects.read(&key, 1024).await.unwrap();
    assert_eq!(
        bytes,
        b"synthetic quarantine object through committed Outbox"
    );
    let restarted = f.scheduler(vec![mail_sink, effects]);
    assert_eq!(restarted.tick(&CancellationToken::new()).await.unwrap(), 0);
    assert_eq!(
        std::fs::read_dir(f.dir.join("spool/mail")).unwrap().count(),
        1
    );
    assert_eq!(
        std::fs::read_dir(f.dir.join("spool/quarantine"))
            .unwrap()
            .count(),
        1
    );
    f.pool.close().await;
}

#[tokio::test]
async fn adapter_disabled_keeps_queued_without_attempt_budget_and_scheduler_cancels() {
    let f = Fixture::new().await;
    let mail = message();
    let event = f
        .enqueue("mail.requested", &mail.message_id, "mail.delivery")
        .await;
    let adapters = AdapterRuntime::assemble(&AdapterConfig::default(), Environment::Test).unwrap();
    let scheduler = f.scheduler(vec![Arc::new(MailSink {
        source: Arc::new(Source(mail)),
        store: Arc::new(DbMailStore::new(f.pool.clone(), f.renderer.clone())),
        provider: adapters.mail,
    })]);
    let cancel = CancellationToken::new();
    scheduler.tick(&cancel).await.unwrap();
    assert_eq!(
        f.state(&event).await,
        ("ready".into(), 0, "external_disabled".into())
    );
    assert!(!f.dir.join("spool").exists());
    cancel.cancel();
    assert_eq!(scheduler.tick(&cancel).await.unwrap(), 0);
    tokio::time::timeout(Duration::from_secs(1), scheduler.run(cancel))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f.state(&event).await.1, 0);
    f.pool.close().await;
}

#[tokio::test]
async fn disabled_audit_replica_preserves_attempt_budget_when_local_effects_gate_is_open() {
    let f = Fixture::new().await;
    let audit =
        platform::audit::AuditEvent::synthetic(1, platform::transaction::new_operation_id());
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    platform_db::audit_repo::append_audit(&mut tx.connection(), &f.renderer, &audit)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let scheduler = f.scheduler(vec![Arc::new(platform_audit::ReplicaSink::new(
        Arc::new(platform_db::audit_repo::DbAuditExport::new(
            f.pool.clone(),
            f.renderer.clone(),
        )),
        Arc::new(platform_audit::DisabledReplica),
    ))]);
    assert_eq!(scheduler.tick(&CancellationToken::new()).await.unwrap(), 1);
    let name = f
        .renderer
        .table(f.pool.backend(), "platform", "outbox_events")
        .unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let row = tx
        .connection()
        .fetch_optional(
            &format!("SELECT state, attempts, error_code FROM {name} WHERE aggregate_id = ?"),
            &[DbValue::Text(audit.event_id)],
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.text("state").unwrap(), "ready");
    assert_eq!(row.i64("attempts").unwrap(), 0);
    assert_eq!(row.text("error_code").unwrap(), "external_disabled");
    tx.rollback().await.unwrap();
    f.pool.close().await;
}

/// Controlled failed/dropped hints; authoritative events still live in actual SQLite.
struct LostWakeup {
    unavailable: bool,
    wait_calls: std::sync::atomic::AtomicUsize,
    entered_wait: tokio::sync::Notify,
}
impl platform::cache::Wakeup for LostWakeup {
    fn notify(&self) -> BoxFuture<'_, Result<(), platform::cache::CacheError>> {
        Box::pin(async move {
            if self.unavailable {
                Err(platform::cache::CacheError::Unavailable)
            } else {
                Ok(())
            }
        })
    }
    fn wait(&self, timeout: Duration) -> BoxFuture<'_, Result<bool, platform::cache::CacheError>> {
        Box::pin(async move {
            self.wait_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.entered_wait.notify_one();
            if self.unavailable {
                Err(platform::cache::CacheError::Unavailable)
            } else {
                // A successful notify is deliberately dropped; only the timeout completes.
                tokio::time::sleep(timeout).await;
                Ok(false)
            }
        })
    }
}

#[tokio::test]
async fn failed_or_lost_wakeup_recovers_new_commits_by_database_polling_and_cancels() {
    use platform::cache::Wakeup;
    use platform_outbox::LocalSink;
    for unavailable in [true, false] {
        let f = Fixture::new().await;
        let sink = Arc::new(LocalSink::default());
        let wakeup = Arc::new(LostWakeup {
            unavailable,
            wait_calls: std::sync::atomic::AtomicUsize::new(0),
            entered_wait: tokio::sync::Notify::new(),
        });
        let scheduler = f.scheduler(vec![sink.clone()]).with_wakeup(wakeup.clone());
        let cancel = CancellationToken::new();
        let running_cancel = cancel.clone();
        let running = tokio::spawn(async move { scheduler.run(running_cancel).await });
        // Reaching Wakeup::wait proves run's first empty database tick has completed.
        tokio::time::timeout(Duration::from_secs(2), wakeup.entered_wait.notified())
            .await
            .unwrap();
        assert_eq!(sink.consumed(), 0);
        let event = f
            .enqueue(
                "local.probe",
                &platform::transaction::new_operation_id(),
                "local.probe",
            )
            .await;
        let hint = wakeup.notify().await;
        assert_eq!(
            hint,
            if unavailable {
                Err(platform::cache::CacheError::Unavailable)
            } else {
                Ok(())
            }
        );
        // No successful hint is delivered in either mode. Scheduler must discover
        // the event after it was committed, through its real database polling loop.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if f.state(&event).await.0 == "delivered" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(sink.consumed(), 1);
        assert!(wakeup.wait_calls.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(f.state(&event).await.0, "delivered");
        f.pool.close().await;
    }
}
