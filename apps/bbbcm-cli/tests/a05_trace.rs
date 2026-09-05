//! A05 integration: generated request/task-local ID -> transaction -> persisted Outbox
//! -> real scheduler -> sink. SQLite and synthetic data only, no business HTTP write.
use platform::observability::{REQUEST_ID, new_request_id};
use platform::outbox::{BoxFuture, OutboxEvent, SafePayload, Sink, SinkError};
use platform::transaction::{
    CommandIdentity, CommitOutcome, LockPlan, StoredResponse, new_operation_id,
};
use platform_db::{
    Pool,
    transaction::{DbTransaction, TransactionError},
};
use platform_outbox::{ClosedExternalEffects, Scheduler, Settings, SinkRegistry};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct TraceSink {
    seen: Mutex<Vec<(String, String)>>,
}
impl Sink for TraceSink {
    fn name(&self) -> &str {
        "local.trace"
    }
    fn is_external(&self) -> bool {
        false
    }
    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            assert_eq!(
                platform::observability::current_trace_id().as_deref(),
                Some(event.trace_id.as_str())
            );
            let downstream = OutboxEvent {
                event_id: new_operation_id(),
                trace_id: String::new(),
                ..event.clone()
            }
            .with_inherited_trace()
            .unwrap();
            assert_eq!(downstream.trace_id, event.trace_id);
            self.seen
                .lock()
                .unwrap()
                .push((event.event_id.clone(), event.trace_id.clone()));
            Ok(())
        })
    }
}
fn event() -> OutboxEvent {
    OutboxEvent {
        event_id: new_operation_id(),
        aggregate_type: "synthetic".into(),
        aggregate_id: "1".into(),
        event_type: "synthetic.trace".into(),
        version: 1,
        occurred_at: chrono::Utc::now(),
        payload: SafePayload::empty(),
        trace_id: String::new(),
    }
}

#[tokio::test]
async fn generated_request_trace_reaches_committed_event_and_real_sink_with_background_fallback() {
    let dir = std::env::temp_dir().join(format!("bbbcm-a05-trace-{}", new_operation_id()));
    std::fs::create_dir(&dir).unwrap();
    let mut config = platform::config::Config::default().database;
    config.sqlite_path = dir.join("trace.sqlite3").to_string_lossy().into();
    let renderer = platform::identifiers::SchemaRenderer::new(&config.schema_prefix).unwrap();
    platform_db::migrations::migrate(
        &config,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
    )
    .await
    .unwrap();
    let pool = Pool::connect(&config).await.unwrap();
    let request_id = new_request_id();
    let generated = event();
    let event_id = generated.event_id.clone();
    let command = CommandIdentity {
        scope: "synthetic.trace".into(),
        actor_id: 1,
        idempotency_key: new_operation_id(),
        request_hash: platform::transaction::request_hash(b"synthetic trace"),
        operation_id: new_operation_id(),
    };
    let outcome = REQUEST_ID
        .scope(
            request_id.clone(),
            platform_db::command::execute_command(
                &pool,
                &renderer,
                &command,
                &LockPlan::default(),
                chrono::Utc::now(),
                |connection| {
                    let renderer = renderer.clone();
                    let event = generated.clone();
                    Box::pin(async move {
                        platform_db::outbox_repo::enqueue(
                            connection,
                            &renderer,
                            &event,
                            &["local.trace"],
                        )
                        .await
                        .map_err(|_| TransactionError::InvalidInput)?;
                        Ok(StoredResponse {
                            status_code: 201,
                            body: SafePayload::empty().to_json().unwrap(),
                        })
                    })
                },
            ),
        )
        .await
        .unwrap();
    assert!(matches!(outcome, CommitOutcome::Committed(_)));
    let background = event();
    let background_id = background.event_id.clone();
    let mut tx = DbTransaction::begin(&pool).await.unwrap();
    platform_db::outbox_repo::enqueue(
        &mut tx.connection(),
        &renderer,
        &background,
        &["local.trace"],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let sink = Arc::new(TraceSink::default());
    let mut registry = SinkRegistry::default();
    registry.register(sink.clone()).unwrap();
    let scheduler = Scheduler::new(
        Arc::new(platform_db::outbox_repo::DbOutboxStore::new(
            pool.clone(),
            renderer,
        )),
        registry,
        Arc::new(ClosedExternalEffects),
        Settings::default(),
    )
    .unwrap();
    assert_eq!(scheduler.tick(&CancellationToken::new()).await.unwrap(), 2);
    let seen = sink.seen.lock().unwrap().clone();
    assert!(seen.contains(&(event_id, request_id)));
    assert!(seen.contains(&(background_id.clone(), background_id)));
    assert_eq!(scheduler.tick(&CancellationToken::new()).await.unwrap(), 0);
    pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn upstream_trace_is_preserved_and_invalid_task_ids_never_enter_events() {
    let mut explicit = event();
    explicit.trace_id = new_request_id();
    let expected = explicit.trace_id.clone();
    let normalized = REQUEST_ID
        .scope(new_request_id(), async { explicit.with_inherited_trace() })
        .await
        .unwrap();
    assert_eq!(normalized.trace_id, expected);
    assert!(
        REQUEST_ID
            .scope("secret-invalid-client-header".into(), async {
                event().with_inherited_trace()
            })
            .await
            .is_err()
    );
}
