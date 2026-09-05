//! A04 application composition tests: real isolated SQLite, no real accounts or providers.
use chrono::Utc;
use platform::audit::*;
use platform::config::Config;
use platform::identifiers::SchemaRenderer;
use platform::outbox::{OutboxEvent, SafePayload};
use platform::transaction::{CommandIdentity, StoredResponse, new_operation_id};
use platform_db::Pool;
use platform_db::audit_repo::{DbAuditExport, DbSensitiveSession, ReadPolicy, append_audit};
use platform_db::transaction::{DbTransaction, TransactionConnection, TransactionError};
use std::sync::Arc;

struct Fixture {
    dir: std::path::PathBuf,
    pool: Pool,
    renderer: SchemaRenderer,
}
impl Fixture {
    async fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("bbbcm-engine-{}", new_operation_id()));
        std::fs::create_dir(&dir).unwrap();
        let mut cfg = Config::default().database;
        cfg.sqlite_path = dir.join("test.db").to_str().unwrap().into();
        cfg.schema_prefix = prefix.into();
        platform_db::migrations::migrate(
            &cfg,
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        Self {
            dir,
            pool: Pool::connect(&cfg).await.unwrap(),
            renderer: SchemaRenderer::new(prefix).unwrap(),
        }
    }
    async fn count(&self, schema: &str, name: &str) -> i64 {
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        let table = self
            .renderer
            .table(self.pool.backend(), schema, name)
            .unwrap();
        let count = tx
            .connection()
            .fetch_optional(&format!("SELECT COUNT(*) AS n FROM {table}"), &[])
            .await
            .unwrap()
            .unwrap()
            .i64("n")
            .unwrap();
        tx.rollback().await.unwrap();
        count
    }
    async fn close(self) {
        self.pool.close().await;
        std::fs::remove_dir_all(self.dir).unwrap();
    }
}
fn command() -> CommandIdentity {
    CommandIdentity {
        scope: "synthetic.write".into(),
        actor_id: 1,
        idempotency_key: new_operation_id(),
        request_hash: platform_db::idempotency_repo::request_hash(b"synthetic"),
        operation_id: new_operation_id(),
    }
}
fn event() -> OutboxEvent {
    OutboxEvent {
        event_id: new_operation_id(),
        aggregate_type: "synthetic".into(),
        aggregate_id: "1".into(),
        event_type: "synthetic.created".into(),
        version: 1,
        occurred_at: Utc::now(),
        payload: SafePayload::empty(),
        trace_id: String::new(),
    }
}
#[tokio::test]
async fn business_idempotency_outbox_audit_commit_or_wholly_rollback() {
    for prefix in ["bcm_", "engine42_"] {
        let f = Fixture::new(prefix).await;
        for fail_at in 0..=5 {
            let identity = command();
            let audit = AuditEvent::synthetic(1, identity.operation_id.clone());
            let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
            let result=async {
                let mut c=tx.connection();
                platform_db::idempotency_repo::reserve(&mut c,&f.renderer,&identity,Utc::now()).await?;
                if fail_at==0 {return Err(TransactionError::InvalidInput);}
                let table=f.renderer.table(c.backend(),"community","forums")?;
                c.execute(&format!("INSERT INTO {table} (slug,name,position,status,version) VALUES (?, ?, ?, ?, ?)"),&[identity.operation_id.clone().into(),"synthetic".into(),1_i64.into(),"draft".into(),1_i64.into()]).await?;
                assert!(c.last_insert_id().await?>0);
                if fail_at==1 {return Err(TransactionError::InvalidInput);}
                platform_db::outbox_repo::enqueue(&mut c,&f.renderer,&event(),&["local.probe"]).await.map_err(|_|TransactionError::InvalidInput)?;
                if fail_at==2 {return Err(TransactionError::InvalidInput);}
                append_audit(&mut c,&f.renderer,&audit).await?;
                if fail_at==3 { // Actual unique violation at the audit store must roll everything back.
                    append_audit(&mut c,&f.renderer,&audit).await?;
                }
                platform_db::idempotency_repo::complete(&mut c,&f.renderer,&identity,&StoredResponse {status_code:201,body:SafePayload::empty().to_json().unwrap()},Utc::now()).await?;
                if fail_at==4 {return Err(TransactionError::InvalidInput);}
                Ok::<_,TransactionError>(())
            }.await;
            if fail_at < 5 {
                assert!(result.is_err());
                tx.rollback().await.unwrap();
            } else {
                result.unwrap();
                tx.commit().await.unwrap();
            }
            let n = i64::from(fail_at == 5);
            assert_eq!(f.count("community", "forums").await, n);
            assert_eq!(f.count("platform", "idempotency_records").await, n);
            assert_eq!(f.count("platform", "outbox_events").await, 2 * n);
            assert_eq!(f.count("audit", "audit_events").await, n);
        }
        f.close().await;
    }
}

struct SyntheticPolicy {
    allow: bool,
}
impl ReadPolicy<String> for SyntheticPolicy {
    fn authorize<'a>(
        &'a mut self,
        mut conn: TransactionConnection<'a>,
        event: &'a SensitiveAccessEvent,
    ) -> AuditFuture<'a, ()> {
        Box::pin(async move {
            // Synthetic actor context only. Query proves auth runs inside the supplied transaction.
            conn.fetch_optional("SELECT 1 AS n", &[])
                .await
                .map_err(|_| AuditError::Unavailable)?;
            if self.allow && event.0.actor_id == 1 {
                Ok(())
            } else {
                Err(AuditError::Denied)
            }
        })
    }
    fn read<'a>(&'a mut self, mut conn: TransactionConnection<'a>) -> AuditFuture<'a, String> {
        Box::pin(async move {
            conn.fetch_optional("SELECT 1 AS n", &[])
                .await
                .map_err(|_| AuditError::Unavailable)?;
            Ok("restricted synthetic projection".into())
        })
    }
}
#[tokio::test]
async fn sensitive_read_requires_authorization_and_committed_access_audit() {
    let f = Fixture::new("audit42_").await;
    let mut record = AuditEvent::synthetic(1, new_operation_id());
    record.action = AuditAction::SensitiveRead;
    let event = SensitiveAccessEvent(record);
    for (index, allow) in [false, true, true].into_iter().enumerate() {
        let mut session = DbSensitiveSession::new(
            DbTransaction::begin(&f.pool).await.unwrap(),
            f.renderer.clone(),
            SyntheticPolicy { allow },
        );
        let result = platform_audit::read_sensitive(&mut session, &event).await;
        drop(session);
        if !allow {
            assert_eq!(result, Err(AuditError::Denied));
            assert_eq!(f.count("audit", "sensitive_access_events").await, 0);
        } else {
            assert_eq!(f.count("audit", "sensitive_access_events").await, 1);
            if index == 1 {
                assert_eq!(result.as_deref(), Ok("restricted synthetic projection"));
            } else {
                assert_eq!(result, Err(AuditError::Unavailable));
            }
        }
    }
    let mut session = DbSensitiveSession::new(
        DbTransaction::begin(&f.pool).await.unwrap(),
        f.renderer.clone(),
        SyntheticPolicy { allow: true },
    );
    assert!(matches!(session.stage().await, Err(AuditError::Denied)));
    assert!(session.commit().await.is_err());
    session.rollback().await.unwrap();
    for result in [AuditResult::Denied, AuditResult::Failed] {
        let mut invalid = event.clone();
        invalid.0.result = result;
        let mut session = DbSensitiveSession::new(
            DbTransaction::begin(&f.pool).await.unwrap(),
            f.renderer.clone(),
            SyntheticPolicy { allow: true },
        );
        assert_eq!(
            platform_audit::read_sensitive(&mut session, &invalid).await,
            Err(AuditError::Invalid)
        );
        session.rollback().await.unwrap();
    }
    // A staged projection has no public accessor. Premature commit refuses it and rollback discards it.
    let mut session = DbSensitiveSession::new(
        DbTransaction::begin(&f.pool).await.unwrap(),
        f.renderer.clone(),
        SyntheticPolicy { allow: true },
    );
    session.authorize(&event).await.unwrap();
    session.stage().await.unwrap();
    assert_eq!(session.commit().await, Err(AuditError::Denied));
    session.rollback().await.unwrap();
    f.close().await;
}

struct AllowLocalEffects;
impl platform_outbox::ExternalEffects for AllowLocalEffects {
    fn allowed(&self) -> bool {
        true
    }
}
#[tokio::test]
async fn audit_redacts_secret_appends_correction_and_copies_only_with_explicit_test_adapter() {
    use platform_outbox::{ClosedExternalEffects, Scheduler, Settings, SinkRegistry};
    let f = Fixture::new("copy42_").await;
    let mut original = AuditEvent::synthetic(1, new_operation_id());
    original.details.before = Some(AuditSummary::from_value(
        Classification::Secret,
        b"SYNTHETIC_SECRET_SENTINEL",
    ));
    original.details.after = Some(AuditSummary::from_value(
        Classification::C3,
        b"SYNTHETIC_C3_SENTINEL",
    ));
    assert!(
        !serde_json::to_string(&original)
            .unwrap()
            .contains("SENTINEL")
    );
    let mut correction = AuditEvent::synthetic(1, new_operation_id());
    correction.action = AuditAction::Correction;
    correction.correction_of = Some(original.event_id.clone());
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    append_audit(&mut tx.connection(), &f.renderer, &original)
        .await
        .unwrap();
    append_audit(&mut tx.connection(), &f.renderer, &correction)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(f.count("audit", "audit_events").await, 2);
    let export = Arc::new(DbAuditExport::new(f.pool.clone(), f.renderer.clone()));
    let fetched = export.load(&original.event_id).await.unwrap();
    assert!(fetched.correction_of.is_none());
    assert!(
        !serde_json::to_string(&fetched)
            .unwrap()
            .contains("SENTINEL")
    );
    assert_eq!(
        platform_audit::DisabledReplica.append(&fetched).await,
        Err(AuditError::ReplicaDisabled)
    );
    let local = Arc::new(platform_audit::LocalReplica::default());
    let sink = Arc::new(platform_audit::ReplicaSink::new(export, local.clone()));
    let store = Arc::new(platform_db::outbox_repo::DbOutboxStore::new(
        f.pool.clone(),
        f.renderer.clone(),
    ));
    let mut registry = SinkRegistry::default();
    registry.register(sink.clone()).unwrap();
    let closed = Scheduler::new(
        store.clone(),
        registry,
        Arc::new(ClosedExternalEffects),
        Settings::default(),
    )
    .unwrap();
    closed
        .tick(&tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert!(local.is_empty());
    // Advance availability only in the isolated test store; runtime never has a flag bypass.
    let table = f
        .renderer
        .table(f.pool.backend(), "platform", "outbox_events")
        .unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {table} SET available_at = ?"),
            &[platform_db::transaction::DbValue::Time(Utc::now())],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut registry = SinkRegistry::default();
    registry.register(sink).unwrap();
    let scheduler = Scheduler::new(
        store,
        registry,
        Arc::new(AllowLocalEffects),
        Settings::default(),
    )
    .unwrap();
    scheduler
        .tick(&tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(local.len(), 2);
    scheduler
        .tick(&tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(local.len(), 2);
    f.close().await;
}

struct FailingReadPolicy;
impl ReadPolicy<String> for FailingReadPolicy {
    fn authorize<'a>(
        &'a mut self,
        _: TransactionConnection<'a>,
        _: &'a SensitiveAccessEvent,
    ) -> AuditFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn read<'a>(&'a mut self, _: TransactionConnection<'a>) -> AuditFuture<'a, String> {
        Box::pin(async { Err(AuditError::Unavailable) })
    }
}
#[tokio::test]
async fn failed_projection_cannot_commit_a_successful_access_record() {
    let f = Fixture::new("readfail_").await;
    let mut record = AuditEvent::synthetic(1, new_operation_id());
    record.action = AuditAction::SensitiveRead;
    let event = SensitiveAccessEvent(record);
    let mut session = DbSensitiveSession::new(
        DbTransaction::begin(&f.pool).await.unwrap(),
        f.renderer.clone(),
        FailingReadPolicy,
    );
    session.authorize(&event).await.unwrap();
    assert_eq!(session.stage().await, Err(AuditError::Unavailable));
    assert_eq!(session.append(&event).await, Err(AuditError::Denied));
    assert_eq!(session.commit().await, Err(AuditError::Denied));
    session.rollback().await.unwrap();
    assert_eq!(f.count("audit", "sensitive_access_events").await, 0);
    f.close().await;
}

struct CommitRejectPolicy;
impl ReadPolicy<String> for CommitRejectPolicy {
    fn authorize<'a>(
        &'a mut self,
        _: TransactionConnection<'a>,
        _: &'a SensitiveAccessEvent,
    ) -> AuditFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn read<'a>(&'a mut self, mut conn: TransactionConnection<'a>) -> AuditFuture<'a, String> {
        Box::pin(async move {
            conn.execute(
                "INSERT INTO synthetic_deferred_child (parent_id) VALUES (?)",
                &[1_i64.into()],
            )
            .await
            .map_err(|_| AuditError::Unavailable)?;
            Ok("must never leave before commit".into())
        })
    }
}
#[tokio::test]
async fn actual_sqlite_commit_rejection_returns_no_sensitive_projection() {
    let f = Fixture::new("commitfail_").await;
    let Pool::Sqlite(sqlite) = &f.pool else {
        unreachable!()
    };
    // A test-only deferred constraint creates a real COMMIT failure after the access append succeeds.
    sqlx::query("CREATE TABLE synthetic_deferred_parent (id INTEGER PRIMARY KEY)")
        .execute(sqlite)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE synthetic_deferred_child (parent_id INTEGER REFERENCES synthetic_deferred_parent(id) DEFERRABLE INITIALLY DEFERRED)").execute(sqlite).await.unwrap();
    let mut record = AuditEvent::synthetic(1, new_operation_id());
    record.action = AuditAction::SensitiveRead;
    let event = SensitiveAccessEvent(record);
    let mut session = DbSensitiveSession::new(
        DbTransaction::begin(&f.pool).await.unwrap(),
        f.renderer.clone(),
        CommitRejectPolicy,
    );
    assert_eq!(
        platform_audit::read_sensitive(&mut session, &event).await,
        Err(AuditError::Unavailable)
    );
    assert_eq!(f.count("audit", "sensitive_access_events").await, 0);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM synthetic_deferred_child")
        .fetch_one(sqlite)
        .await
        .unwrap();
    assert_eq!(count, 0);
    f.close().await;
}
