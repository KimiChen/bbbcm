use super::*;
use crate::migrations;
use platform::config::Config;
use std::path::PathBuf;

pub(crate) struct Fixture {
    pub pool: Pool,
    pub renderer: SchemaRenderer,
    pub dir: PathBuf,
}
impl Fixture {
    pub async fn new() -> Self {
        Self::with_prefix("tx_test_").await
    }
    pub async fn with_prefix(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-engine-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default().database;
        cfg.sqlite_path = dir.join("engine.sqlite3").to_str().unwrap().into();
        cfg.schema_prefix = prefix.into();
        migrations::migrate(
            &cfg,
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&cfg).await.unwrap();
        Self {
            pool,
            renderer: SchemaRenderer::new(prefix).unwrap(),
            dir,
        }
    }
    pub fn forums(&self) -> String {
        self.renderer
            .table(Backend::Sqlite, "community", "forums")
            .unwrap()
    }
    pub async fn count(&self, schema: &str, name: &str) -> i64 {
        let table = self.renderer.table(Backend::Sqlite, schema, name).unwrap();
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
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
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn savepoint_recovers_unique_conflict_then_version_update_commits() {
    for prefix in ["bcm_", "tx_alt_"] {
        let f = Fixture::with_prefix(prefix).await;
        let table = f.forums();
        let insert = format!(
            "INSERT INTO {table} (slug, name, position, status, version) VALUES (?, ?, ?, ?, ?)"
        );
        let params = [
            "synthetic".into(),
            "fixture".into(),
            1_i64.into(),
            "draft".into(),
            1_i64.into(),
        ];
        let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
        let mut conn = tx.connection();
        conn.execute(&insert, &params).await.unwrap();
        let id = conn.last_insert_id().await.unwrap();
        assert!(id > 0);
        let update = format!(
            "UPDATE {table} SET name = ?, version = version + 1 WHERE id = ? AND version = ?"
        );
        assert_eq!(
            conn.insert_or_update(
                &insert,
                &params,
                &update,
                &["updated".into(), id.into(), 1_i64.into()]
            )
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            conn.execute_versioned(&update, &["stale".into(), id.into(), 1_i64.into()])
                .await,
            Err(TransactionError::VersionConflict)
        );
        tx.commit().await.unwrap();
        let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
        let row = tx
            .connection()
            .fetch_optional(
                &format!("SELECT id, name, version FROM {table} WHERE id = ?"),
                &[id.into()],
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.text("name").unwrap(), "updated");
        assert_eq!(row.i64("version").unwrap(), 2);
        tx.rollback().await.unwrap();
    }
}

#[tokio::test]
async fn drop_and_explicit_rollback_remove_all_writes_and_borrower_cannot_commit() {
    let f = Fixture::new().await;
    let table = f.forums();
    let insert = format!(
        "INSERT INTO {table} (slug, name, position, status, version) VALUES (?, ?, ?, ?, ?)"
    );
    for explicit in [false, true] {
        let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
        let mut conn = tx.connection();
        conn.execute(
            &insert,
            &[
                "synthetic".into(),
                "fixture".into(),
                1_i64.into(),
                "draft".into(),
                1_i64.into(),
            ],
        )
        .await
        .unwrap();
        for control in [
            "COMMIT",
            "ROLLBACK",
            "BEGIN",
            "SAVEPOINT unsafe",
            "SELECT 1; COMMIT",
            "/* hidden */ COMMIT",
        ] {
            assert_eq!(
                conn.execute(control, &[]).await,
                Err(TransactionError::InvalidInput)
            );
        }
        if explicit {
            tx.rollback().await.unwrap();
        } else {
            drop(tx);
        }
        assert_eq!(f.count("community", "forums").await, 0);
    }
}

#[tokio::test]
async fn sqlite_begin_is_immediate_and_locks_reject_reverse_order() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(f.dir.join("engine.sqlite3"))
        .busy_timeout(std::time::Duration::ZERO);
    use sqlx::Connection;
    let mut competing = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    let busy = sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut competing)
        .await
        .unwrap_err();
    assert_eq!(
        classify_error(Backend::Sqlite, &busy),
        TransactionError::Database(DbErrorClass::Busy)
    );
    let table = f.forums();
    let mut conn = tx.connection();
    for id in [1_i64, 2] {
        conn.execute(&format!("INSERT INTO {table} (id, slug, name, position, status, version) VALUES (?, ?, ?, ?, ?, ?)"), &[id.into(), format!("fixture-{id}").into(), "fixture".into(), id.into(), "draft".into(), 1_i64.into()]).await.unwrap();
    }
    conn.lock(
        &f.renderer,
        &LockTarget::new("community", "forums", 2).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        conn.lock(
            &f.renderer,
            &LockTarget::new("community", "forums", 1).unwrap()
        )
        .await,
        Err(TransactionError::LockOrder)
    );
    conn.enter_append_phase();
    assert_eq!(
        conn.lock(
            &f.renderer,
            &LockTarget::new("community", "forums", 2).unwrap()
        )
        .await,
        Err(TransactionError::LockOrder)
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn malformed_time_and_boolean_rows_fail_closed() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let row = tx
        .connection()
        .fetch_optional(
            "SELECT ? AS timestamp, ? AS enabled",
            &["2026-09-05T01:00:00Z".into(), 2_i64.into()],
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.time("timestamp"), Err(TransactionError::CorruptData));
    assert_eq!(row.boolean("enabled"), Err(TransactionError::CorruptData));
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn cancellation_discards_connection_but_confirmed_completion_reuses_it() {
    let f = Fixture::new().await;
    let Pool::Sqlite(sqlite, _) = &f.pool else {
        panic!("SQLite fixture");
    };
    sqlx::query("CREATE TEMP TABLE connection_marker (id INTEGER)")
        .execute(sqlite)
        .await
        .unwrap();
    // 成功提交/回滚保持同一连接，取消则丢弃，即使底层连接仍然健康。
    DbTransaction::begin(&f.pool)
        .await
        .unwrap()
        .commit()
        .await
        .unwrap();
    DbTransaction::begin(&f.pool)
        .await
        .unwrap()
        .rollback()
        .await
        .unwrap();
    let marker: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_temp_master WHERE name = 'connection_marker'",
    )
    .fetch_one(sqlite)
    .await
    .unwrap();
    assert_eq!(marker, 1);
    let pool = f.pool.clone();
    let table = f.forums();
    let (ready, started) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut tx = DbTransaction::begin(&pool).await.unwrap();
        tx.connection().execute(&format!("INSERT INTO {table} (slug, name, position, status, version) VALUES (?, ?, ?, ?, ?)"), &["cancelled".into(), "fixture".into(), 1_i64.into(), "draft".into(), 1_i64.into()]).await.unwrap();
        ready.send(()).unwrap();
        std::future::pending::<()>().await;
        drop(tx);
    });
    started.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(f.count("community", "forums").await, 0);
    let marker: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_temp_master WHERE name = 'connection_marker'",
    )
    .fetch_one(sqlite)
    .await
    .unwrap();
    assert_eq!(marker, 0);
}

#[tokio::test]
async fn actual_transaction_outcomes_and_acquire_failures_remain_separate() {
    let f = Fixture::new().await;
    let observed = Arc::new(Metrics::default());
    DbTransaction::begin_observed(&f.pool, false, observed.clone())
        .await
        .unwrap()
        .commit()
        .await
        .unwrap();
    DbTransaction::begin_observed(&f.pool, true, observed.clone())
        .await
        .unwrap()
        .rollback()
        .await
        .unwrap();
    DbTransaction::begin_observed(&f.pool, false, observed.clone())
        .await
        .unwrap()
        .rollback_after(&TransactionError::Database(DbErrorClass::UniqueConflict))
        .await
        .unwrap();
    assert_eq!(
        DbTransaction::begin_observed(&f.pool, false, observed.clone())
            .await
            .unwrap()
            .simulate_lost_ack(true)
            .await,
        Err(TransactionError::CommitUnknown)
    );

    let pool = f.pool.clone();
    let observations = observed.clone();
    let table = f.forums();
    let (ready, started) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut tx = DbTransaction::begin_observed(&pool, false, observations)
            .await
            .unwrap();
        tx.connection().execute(&format!("INSERT INTO {table} (slug, name, position, status, version) VALUES (?, ?, ?, ?, ?)"), &["metric-cancel".into(), "fixture".into(), 1_i64.into(), "draft".into(), 1_i64.into()]).await.unwrap();
        ready.send(()).unwrap();
        std::future::pending::<()>().await;
        drop(tx);
    });
    started.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(f.count("community", "forums").await, 0);

    let held = DbTransaction::begin(&f.pool).await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(10),
            DbTransaction::begin_observed(&f.pool, false, observed.clone())
        )
        .await
        .is_err()
    );
    held.rollback().await.unwrap();
    f.pool.close().await;
    assert!(
        DbTransaction::begin_observed(&f.pool, false, observed.clone())
            .await
            .is_err()
    );

    let series = observed.grouped.snapshot();
    for (outcome, error_class) in [
        (TransactionOutcome::Committed, ErrorClass::None),
        (TransactionOutcome::RolledBack, ErrorClass::None),
        (TransactionOutcome::RolledBack, ErrorClass::UniqueConflict),
        (TransactionOutcome::CommitUnknown, ErrorClass::Connection),
        (
            TransactionOutcome::CancelledOrAbandoned,
            ErrorClass::NotObserved,
        ),
    ] {
        let sample = series
            .iter()
            .find(|s| {
                s.dimensions
                    == SeriesKey::DbTransactionSeconds {
                        backend: DatabaseBackend::Sqlite,
                        purpose: DatabasePurpose::App,
                        outcome,
                        error_class,
                    }
            })
            .expect("each actual outcome has its own series");
        assert_eq!(sample.sample.count, 1);
        assert!(sample.sample.sum > 0.);
    }
    for (outcome, error_class) in [
        (AcquireOutcome::Cancelled, ErrorClass::NotObserved),
        (AcquireOutcome::Failed, ErrorClass::Connection),
    ] {
        let sample = series
            .iter()
            .find(|s| {
                s.dimensions
                    == SeriesKey::DbPoolAcquireSeconds {
                        backend: DatabaseBackend::Sqlite,
                        purpose: DatabasePurpose::App,
                        outcome,
                        error_class,
                    }
            })
            .unwrap();
        assert_eq!(sample.sample.count, 1);
        if outcome == AcquireOutcome::Cancelled {
            assert!(sample.sample.sum >= 0.01);
        }
    }
    let attempts = series
        .iter()
        .find(|s| matches!(s.dimensions, SeriesKey::DbTransactionAttemptsTotal { .. }))
        .unwrap();
    assert_eq!(
        attempts.sample.count, 5,
        "failed/cancelled acquire never started BEGIN"
    );
    assert_ne!(observed.instance_id(), Metrics::default().instance_id());
}
