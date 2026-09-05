use super::*;
use platform::analytics::{Device, Dimensions, EventKind, TrustedExclusions};
use platform::config::{Config, DatabaseConfig};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

struct Fixture {
    dir: PathBuf,
    config: DatabaseConfig,
    pool: Pool,
    renderer: SchemaRenderer,
}
impl Fixture {
    async fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-analytics-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = Config::default().database;
        config.sqlite_path = dir.join("analytics.sqlite3").to_string_lossy().into();
        config.schema_prefix = prefix.into();
        crate::migrations::migrate(
            &config,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&config).await.unwrap();
        Self {
            dir,
            config,
            pool,
            renderer: SchemaRenderer::new(prefix).unwrap(),
        }
    }
    fn store(&self) -> DbAnalyticsStore {
        DbAnalyticsStore::new(self.pool.clone(), self.renderer.clone())
    }
    async fn enqueue(&self, fact: &AnalyticsFact, received_at: DateTime<Utc>) -> OutboxEvent {
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        let event = enqueue(
            &mut tx.connection(),
            &self.renderer,
            fact,
            received_at,
            String::new(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        event
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn time(value: &str) -> DateTime<Utc> {
    platform::db::decode_time(value).unwrap()
}
fn at() -> DateTime<Utc> {
    time("2026-09-05T00:00:00Z")
}
fn fact(actor: &str, when: DateTime<Utc>, exclusions: [bool; 4]) -> AnalyticsFact {
    AnalyticsFact::from_authority(
        platform::transaction::new_operation_id(),
        EventKind::Read,
        when,
        actor.into(),
        Some(1),
        Dimensions {
            forum_id: Some(1),
            tag_ids: BTreeSet::from([2, 3]),
            channel_id: Some(4),
            device: Device::Mobile,
        },
        TrustedExclusions::from_authority(
            exclusions[0],
            exclusions[1],
            exclusions[2],
            exclusions[3],
        ),
    )
    .unwrap()
}

#[tokio::test]
async fn committed_source_dedup_exclusions_late_arrival_and_restart() {
    for prefix in ["bcm_", "analytics_test_"] {
        let f = Fixture::new(prefix).await;
        let actor = platform::transaction::new_operation_id();
        let first = fact(&actor, at(), [false; 4]);
        let event = f.enqueue(&first, at()).await;
        assert_eq!(
            f.store()
                .process(&event, at() + chrono::Duration::seconds(5))
                .await
                .unwrap(),
            ProcessOutcome::Applied
        );
        assert_eq!(
            f.store()
                .process(&event, at() + chrono::Duration::seconds(6))
                .await
                .unwrap(),
            ProcessOutcome::Duplicate
        );
        for index in 0..4 {
            let mut flags = [false; 4];
            flags[index] = true;
            let excluded = fact(&actor, at(), flags);
            let event = f.enqueue(&excluded, at()).await;
            assert_eq!(
                f.store()
                    .process(&event, at() + chrono::Duration::seconds(7))
                    .await
                    .unwrap(),
                ProcessOutcome::Excluded
            );
        }
        let late = fact(&actor, at() - chrono::Duration::milliseconds(1), [false; 4]);
        let late_event = f.enqueue(&late, at() + chrono::Duration::seconds(8)).await;
        assert_eq!(
            f.store()
                .process(&late_event, at() + chrono::Duration::seconds(10))
                .await
                .unwrap(),
            ProcessOutcome::Applied
        );
        let (watermark, aggregates) = f.store().snapshot().await.unwrap();
        assert_eq!(watermark.processed, 6);
        assert_eq!(watermark.excluded, 4);
        assert_eq!(watermark.max_occurred_at, Some(at()));
        assert_eq!(
            watermark.max_received_at,
            Some(at() + chrono::Duration::seconds(8))
        );
        assert_eq!(watermark.maximum_delay_ms, Some(10001));
        let all = aggregates
            .iter()
            .filter(|a| a.dimension == "all")
            .collect::<Vec<_>>();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|a| a.count == 1));
        assert_eq!(all[0].utc_day, at() - chrono::Duration::days(1));
        assert!(!format!("{watermark:?} {aggregates:?}").contains(&actor));
        let summary = f
            .store()
            .window_summary(StatisticsWindow::natural_day(at()).unwrap())
            .await
            .unwrap();
        assert_eq!(summary.unique_actors, 1);
        assert_eq!(summary.event_counts[&EventKind::Read], 1);
        f.pool.close().await;
        let pool = Pool::connect(&f.config).await.unwrap();
        let restarted = DbAnalyticsStore::new(pool.clone(), f.renderer.clone());
        assert_eq!(
            restarted
                .process(&event, at() + chrono::Duration::seconds(20))
                .await
                .unwrap(),
            ProcessOutcome::Duplicate
        );
        assert_eq!(restarted.snapshot().await.unwrap(), (watermark, aggregates));
        pool.close().await;
    }
}

#[tokio::test]
async fn rolled_back_source_and_forged_envelope_cannot_be_projected() {
    let f = Fixture::new("bcm_").await;
    let actor = platform::transaction::new_operation_id();
    let source = fact(&actor, at(), [false; 4]);
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let aborted = enqueue(
        &mut tx.connection(),
        &f.renderer,
        &source,
        at(),
        String::new(),
    )
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        f.store().process(&aborted, at()).await,
        Err(AnalyticsError::SourceMismatch)
    );
    let mut event = f.enqueue(&source, at()).await;
    event.event_type = "client.forged".into();
    assert_eq!(
        f.store().process(&event, at()).await,
        Err(AnalyticsError::SourceMismatch)
    );
    event.event_type = EVENT_TYPE.into();
    event.trace_id = platform::transaction::new_operation_id();
    assert_eq!(
        f.store().process(&event, at()).await,
        Err(AnalyticsError::SourceMismatch)
    );
    let (watermark, aggregates) = f.store().snapshot().await.unwrap();
    assert_eq!(watermark.processed, 0);
    assert_eq!(watermark.maximum_delay_ms, None);
    assert_eq!(watermark.max_occurred_at, None);
    assert!(aggregates.is_empty());
    f.pool.close().await;
}

#[tokio::test]
async fn interrupted_projection_and_rebuild_rollback_do_not_leave_receipts_or_partial_counts() {
    let f = Fixture::new("bcm_").await;
    let actor = platform::transaction::new_operation_id();
    let source = fact(&actor, at(), [false; 4]);
    let event = f.enqueue(&source, at()).await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    assert_eq!(
        process_committed(&mut tx.connection(), &f.renderer, &event, at())
            .await
            .unwrap(),
        ProcessOutcome::Applied
    );
    tx.rollback().await.unwrap();
    assert_eq!(f.store().snapshot().await.unwrap().0.processed, 0);
    assert_eq!(
        f.store().process(&event, at()).await.unwrap(),
        ProcessOutcome::Applied
    );
    let before = f.store().snapshot().await.unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    rebuild(&mut tx.connection(), &f.renderer, at())
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(f.store().snapshot().await.unwrap(), before);
    let rebuilt = f
        .store()
        .rebuild(at() + chrono::Duration::seconds(1))
        .await
        .unwrap();
    assert_eq!(rebuilt.processed, before.0.processed);
    assert_eq!(rebuilt.version, before.0.version + 1);
    assert_eq!(f.store().snapshot().await.unwrap().1, before.1);
    assert_eq!(
        f.store()
            .process(&event, at() + chrono::Duration::seconds(2))
            .await
            .unwrap(),
        ProcessOutcome::Duplicate
    );
    assert_eq!(f.store().snapshot().await.unwrap().0.processed, 1);
    f.pool.close().await;
}

#[tokio::test]
async fn sink_uses_durable_processor_and_disabled_is_an_error() {
    use platform::analytics::{AnalyticsSink, DisabledAnalytics};
    use platform::outbox::{Sink, SinkError};
    use std::sync::Arc;
    let f = Fixture::new("bcm_").await;
    let source = fact(&platform::transaction::new_operation_id(), at(), [false; 4]);
    let event = f.enqueue(&source, at()).await;
    let sink = AnalyticsSink::new(Arc::new(f.store()));
    assert_eq!(sink.name(), SINK_NAME);
    assert!(!sink.is_external());
    sink.deliver(&event).await.unwrap();
    sink.deliver(&event).await.unwrap();
    assert_eq!(f.store().snapshot().await.unwrap().0.processed, 1);
    let disabled = AnalyticsSink::new(Arc::new(DisabledAnalytics));
    assert_eq!(disabled.deliver(&event).await, Err(SinkError::Disabled));
    f.pool.close().await;
}

#[tokio::test]
async fn failure_after_receipt_insert_rolls_back_and_retry_applies_once() {
    let f = Fixture::new("bcm_").await;
    let actor = platform::transaction::new_operation_id();
    let first = f.enqueue(&fact(&actor, at(), [false; 4]), at()).await;
    f.store().process(&first, at()).await.unwrap();
    let second = f.enqueue(&fact(&actor, at(), [false; 4]), at()).await;
    let aggregates = table(&f.renderer, f.pool.backend(), "aggregates").unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {aggregates} SET event_count = ? WHERE dimension = ?"),
            &[DbValue::I64(i64::MAX), text("all")],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        f.store().process(&second, at()).await,
        Err(AnalyticsError::CorruptData)
    );
    assert_eq!(f.store().snapshot().await.unwrap().0.processed, 1);
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {aggregates} SET event_count = 1 WHERE dimension = ?"),
            &[text("all")],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        f.store().process(&second, at()).await.unwrap(),
        ProcessOutcome::Applied
    );
    assert_eq!(
        f.store().process(&second, at()).await.unwrap(),
        ProcessOutcome::Duplicate
    );
    let (watermark, aggregates) = f.store().snapshot().await.unwrap();
    assert_eq!(watermark.processed, 2);
    assert!(aggregates.iter().all(|aggregate| aggregate.count == 2));
    f.pool.close().await;
}

#[test]
fn utc_windows_and_typed_facts_reject_sensitive_or_invalid_inputs() {
    let actor = platform::transaction::new_operation_id();
    let source = fact(&actor, at(), [false; 4]);
    assert!(!format!("{source:?}").contains(&actor));
    assert_eq!(
        AnalyticsFact::from_storage_json(&source.to_json().unwrap()).unwrap(),
        source
    );
    assert!(
        AnalyticsFact::from_storage_json(
            &source
                .to_json()
                .unwrap()
                .replace("\"exclusion_mask\":0", "\"exclusion_mask\":32")
        )
        .is_err()
    );
    let month = StatisticsWindow::natural_month(time("2024-02-29T23:59:59Z")).unwrap();
    assert_eq!(month.from, time("2024-02-01T00:00:00Z"));
    assert_eq!(month.until, time("2024-03-01T00:00:00Z"));
    assert!(!month.contains(month.until));
    assert!(month.contains(month.from));
    let last = StatisticsWindow::natural_month(time("2026-12-31T23:59:59Z")).unwrap();
    assert_eq!(last.until, time("2027-01-01T00:00:00Z"));
    let too_precise = StatisticsWindow {
        from: at() - chrono::Duration::seconds(1),
        until: at() + chrono::Duration::nanoseconds(500),
    };
    assert_eq!(too_precise.validate(), Err(AnalyticsError::InvalidInput));
    let summary = platform::analytics::summarize(
        [&source, &source],
        StatisticsWindow::natural_day(at()).unwrap(),
    )
    .unwrap();
    assert_eq!(summary.event_counts[&EventKind::Read], 1);
    assert_eq!(summary.unique_actors, 1);
}
