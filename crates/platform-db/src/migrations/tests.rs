use super::*;
use platform::config::Config;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    root: PathBuf,
    cfg: DatabaseConfig,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-migrations-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = dir.join("migrations");
        for track in TRACKS {
            fs::create_dir_all(root.join(track)).unwrap();
        }
        let mut cfg = Config::default().database;
        cfg.sqlite_path = dir.join("database.sqlite3").to_str().unwrap().to_owned();
        Self { dir, root, cfg }
    }
    fn pair(&self, name: &str, sql: &str) {
        for track in ["postgres", "sqlite"] {
            fs::write(self.root.join(track).join(name), sql).unwrap();
        }
    }
    fn initial(&self) {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations");
        for track in ["postgres", "sqlite"] {
            fs::copy(
                repository.join(track).join("0001_init.sql"),
                self.root.join(track).join("0001_init.sql"),
            )
            .unwrap();
        }
    }
    fn renderer(&self) -> SchemaRenderer {
        SchemaRenderer::new(&self.cfg.schema_prefix).unwrap()
    }
    async fn connect(&self) -> DatabaseConnection {
        DatabaseConnection::connect(&self.cfg, Purpose::Migrator, true)
            .await
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn alignment_rejects_empty_active_tracks_and_accepts_only_mysql_placeholder() {
    let f = Fixture::new();
    assert!(matches!(
        verify_repository(&f.root),
        Err(MigrationError::Misaligned { .. })
    ));
    fs::write(f.root.join("postgres/0001_init.sql"), "SELECT 1;\n").unwrap();
    assert!(matches!(
        verify_repository(&f.root),
        Err(MigrationError::Misaligned { .. })
    ));
    f.pair("0001_init.sql", "SELECT 1;\n");
    assert_eq!(verify_repository(&f.root).unwrap(), ["0001"]);
    fs::write(f.root.join("mysql8/0001_init.sql"), "SELECT 1;\n").unwrap();
    assert!(verify_repository(&f.root).is_ok());
    fs::write(f.root.join("mysql8/0002_extra.sql"), "SELECT 1;\n").unwrap();
    assert!(matches!(
        verify_repository(&f.root),
        Err(MigrationError::Misaligned { .. })
    ));
}

#[test]
fn alignment_rejects_duplicate_gap_zero_and_invalid_name() {
    for invalid in [
        "0000_zero.sql",
        "0003_gap.sql",
        "not_numbered.sql",
        "0001_duplicate.sql",
    ] {
        let f = Fixture::new();
        f.pair("0001_init.sql", "SELECT 1;\n");
        f.pair(invalid, "SELECT 1;\n");
        assert!(verify_repository(&f.root).is_err(), "{invalid}");
    }
}

#[test]
fn migration_content_checks_encoding_transactions_and_checksum() {
    for sql in [
        "",
        "-- empty\n",
        "SELECT 1;\r\n",
        "SELECT 1;",
        "COMMIT;\n",
        "/*comment*/ PRAGMA foreign_keys=OFF;\n",
        "SELECT 1; BEGIN;\n",
        "SELECT 'unclosed;\n",
        "CREATE TRIGGER wrong;\n",
    ] {
        let f = Fixture::new();
        f.pair("0001_init.sql", sql);
        assert!(
            matches!(
                MigrationSet::discover(&f.root, Backend::Sqlite),
                Err(MigrationError::InvalidContent { .. })
            ),
            "{sql}"
        );
    }
    let f = Fixture::new();
    f.pair(
        "0001_init.sql",
        "-- BEGIN in comment\nSELECT 'COMMIT;''PRAGMA';\n",
    );
    let first = MigrationSet::discover(&f.root, Backend::Sqlite).unwrap();
    assert_eq!(first.migrations[0].checksum.len(), 64);
    f.pair(
        "0001_init.sql",
        "-- changed comment\nSELECT 'COMMIT;''PRAGMA';\n",
    );
    let second = MigrationSet::discover(&f.root, Backend::Sqlite).unwrap();
    assert_ne!(first.migrations[0].checksum, second.migrations[0].checksum);
    assert_eq!(
        MigrationSet::discover(&f.root, Backend::Mysql8).unwrap_err(),
        MigrationError::UnsupportedBackend
    );
}

#[test]
fn repository_migrations_are_real_and_aligned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations");
    for backend in [Backend::Postgres, Backend::Sqlite] {
        let set = MigrationSet::discover(&root, backend).unwrap();
        assert!(!set.migrations.is_empty());
        for migration in set.migrations {
            SchemaRenderer::new("isolated_")
                .unwrap()
                .migration(backend, &migration.sql)
                .unwrap();
        }
    }
}

#[tokio::test]
async fn status_does_not_create_database_and_both_prefixes_migrate_once() {
    for prefix in ["bcm_", "round_two_"] {
        let mut f = Fixture::new();
        f.cfg.schema_prefix = prefix.to_owned();
        f.initial();
        let before = status(&f.cfg, &f.root).await.unwrap();
        assert_eq!(before[0].state, "pending");
        assert!(!Path::new(&f.cfg.sqlite_path).exists());
        let first = migrate(&f.cfg, &f.root).await.unwrap();
        assert_eq!(first.applied, [1]);
        assert!(first.skipped.is_empty());
        let states = status(&f.cfg, &f.root).await.unwrap();
        assert_eq!(states[0].state, "applied");
        assert_eq!(states[0].executed_at.as_ref().unwrap().len(), 27);
        let repeated = migrate(&f.cfg, &f.root).await.unwrap();
        assert_eq!(repeated.skipped, [1]);
        assert!(repeated.applied.is_empty());
        assert_eq!(states, status(&f.cfg, &f.root).await.unwrap());
        let mut conn = f.connect().await;
        let DatabaseConnection::Sqlite(c) = &mut conn else {
            unreachable!()
        };
        let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_schema WHERE type='table' AND name != 'sqlite_sequence' ORDER BY name").fetch_all(&mut *c).await.unwrap();
        assert_eq!(tables.len(), 3);
        assert!(tables.iter().all(|t| t.starts_with(prefix)));
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut *c)
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        let busy: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&mut *c)
            .await
            .unwrap();
        assert_eq!(busy, 5000);
        conn.close().await.unwrap();
    }
}

#[tokio::test]
async fn changed_applied_migration_and_metadata_drift_are_refused() {
    let f = Fixture::new();
    f.initial();
    migrate(&f.cfg, &f.root).await.unwrap();
    let path = f.root.join("sqlite/0001_init.sql");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{original}-- drift\n")).unwrap();
    assert_eq!(
        status(&f.cfg, &f.root).await.unwrap_err(),
        MigrationError::ContentDrift { number: 1 }
    );
    assert_eq!(
        migrate(&f.cfg, &f.root).await.unwrap_err(),
        MigrationError::ContentDrift { number: 1 }
    );
    fs::write(&path, original).unwrap();
    let mut conn = f.connect().await;
    execute(
        &mut conn,
        &f.renderer()
            .migration(
                Backend::Sqlite,
                "UPDATE {{table:platform.schema_migrations}} SET number=2;",
            )
            .unwrap(),
    )
    .await
    .unwrap();
    conn.close().await.unwrap();
    assert_eq!(
        status(&f.cfg, &f.root).await.unwrap_err(),
        MigrationError::InvalidMetadata
    );
}

#[tokio::test]
async fn unknown_metadata_shape_is_not_adopted() {
    let f = Fixture::new();
    f.initial();
    let mut conn = f.connect().await;
    execute(
        &mut conn,
        &f.renderer()
            .migration(
                Backend::Sqlite,
                "CREATE TABLE {{table:platform.schema_migrations}} (number INTEGER);",
            )
            .unwrap(),
    )
    .await
    .unwrap();
    conn.close().await.unwrap();
    assert_eq!(
        migrate(&f.cfg, &f.root).await.unwrap_err(),
        MigrationError::InvalidMetadata
    );
}

#[tokio::test]
async fn invalid_registered_identifier_fails_before_database_creation() {
    let f = Fixture::new();
    f.pair(
        "0001_init.sql",
        "CREATE TABLE {{table:community.unregistered}} (id INTEGER);\n",
    );
    assert_eq!(
        migrate(&f.cfg, &f.root).await.unwrap_err(),
        MigrationError::InvalidConfiguration
    );
    assert!(!Path::new(&f.cfg.sqlite_path).exists());
}

#[tokio::test]
async fn failed_migration_rolls_back_changes_and_records_safe_failure() {
    let f = Fixture::new();
    f.initial();
    f.pair("0002_failed.sql", "INSERT INTO {{table:community.forums}} (slug,name,position,status,version) VALUES ('transient','secret-do-not-log',1,'draft',1); SELECT nonexistent_column FROM {{table:community.forums}};\n");
    let error = migrate(&f.cfg, &f.root).await.unwrap_err();
    assert_eq!(error, MigrationError::ExecutionFailed { number: 2 });
    assert!(!format!("{error:?} {error}").contains("secret-do-not-log"));
    let states = status(&f.cfg, &f.root).await.unwrap();
    assert_eq!(
        states.iter().map(|s| s.state.as_str()).collect::<Vec<_>>(),
        ["applied", "failed"]
    );
    let mut conn = f.connect().await;
    let sql = f
        .renderer()
        .migration(
            Backend::Sqlite,
            "SELECT COUNT(*) FROM {{table:community.forums}}",
        )
        .unwrap();
    let DatabaseConnection::Sqlite(c) = &mut conn else {
        unreachable!()
    };
    let count: i64 = sqlx::query_scalar(&sql).fetch_one(c).await.unwrap();
    assert_eq!(count, 0);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn failed_identical_content_can_retry_after_external_precondition_recovers() {
    let f = Fixture::new();
    f.initial();
    migrate(&f.cfg, &f.root).await.unwrap();
    f.pair("0002_rules.sql", "INSERT INTO {{table:moderation.rule_versions}} (forum_id,version,body,status) VALUES (1,1,'placeholder','placeholder');\n");
    assert_eq!(
        migrate(&f.cfg, &f.root).await.unwrap_err(),
        MigrationError::ExecutionFailed { number: 2 }
    );
    let mut conn = f.connect().await;
    execute(&mut conn, &f.renderer().migration(Backend::Sqlite, "INSERT INTO {{table:community.forums}} (slug,name,position,status,version) VALUES ('ready','ready',1,'draft',1);").unwrap()).await.unwrap();
    conn.close().await.unwrap();
    assert_eq!(migrate(&f.cfg, &f.root).await.unwrap().applied, [2]);
    assert!(
        status(&f.cfg, &f.root)
            .await
            .unwrap()
            .iter()
            .all(|s| s.state == "applied")
    );
}

async fn same_connection_rebuild(sql: &str, expected: Option<MigrationError>) {
    let f = Fixture::new();
    f.initial();
    migrate(&f.cfg, &f.root).await.unwrap();
    f.pair("0002_rebuild.sql", sql);
    let set = MigrationSet::discover(&f.root, Backend::Sqlite).unwrap();
    let mut conn = f.connect().await;
    let result = apply_one(&mut conn, &f.renderer(), &set, &set.migrations[1]).await;
    if let Some(err) = expected {
        assert_eq!(result.unwrap_err(), err);
    } else {
        assert!(result.unwrap());
        assert!(
            !apply_one(&mut conn, &f.renderer(), &set, &set.migrations[1])
                .await
                .unwrap()
        );
    }
    let DatabaseConnection::Sqlite(c) = &mut conn else {
        unreachable!()
    };
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(fk, 1);
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *c)
        .await
        .unwrap();
    assert!(violations.is_empty());
    let insert = f.renderer().migration(Backend::Sqlite, "INSERT INTO {{table:moderation.rule_versions}} (forum_id,version,body,status) VALUES (99999,9,'x','placeholder')").unwrap();
    assert!(
        sqlx::query(&insert).execute(c).await.is_err(),
        "外键必须在同一连接上重新生效"
    );
    conn.close().await.unwrap();
}

#[tokio::test]
async fn rebuild_foreign_key_violation_rolls_back_and_restores_same_connection() {
    same_connection_rebuild("-- rebuild\nINSERT INTO {{table:moderation.rule_versions}} (forum_id,version,body,status) VALUES (999,1,'bad','placeholder');\n", Some(MigrationError::ForeignKeyViolation { number: 2 })).await;
}

#[tokio::test]
async fn rebuild_statement_failure_restores_same_connection() {
    same_connection_rebuild(
        "-- rebuild\nSELECT nonexistent_column FROM {{table:community.forums}};\n",
        Some(MigrationError::ExecutionFailed { number: 2 }),
    )
    .await;
}

#[tokio::test]
async fn rebuild_success_and_skip_restore_same_connection() {
    same_connection_rebuild("-- rebuild\nALTER TABLE {{table:community.forums}} ADD COLUMN description TEXT NOT NULL DEFAULT '';\n", None).await;
}

#[tokio::test]
async fn metadata_constraints_reject_null_number_bad_checksum_and_unknown_state() {
    let f = Fixture::new();
    f.initial();
    migrate(&f.cfg, &f.root).await.unwrap();
    let mut conn = f.connect().await;
    let sql = f.renderer().migration(Backend::Sqlite, "INSERT INTO {{table:platform.schema_migrations}} (number,name,checksum,state,executed_at) VALUES (?, ?, ?, ?, ?)").unwrap();
    let DatabaseConnection::Sqlite(c) = &mut conn else {
        unreachable!()
    };
    for (number, checksum, state) in [
        (None, "a".repeat(64), "applied"),
        (Some(2_i64), "bad".to_owned(), "applied"),
        (Some(2), "a".repeat(64), "running"),
    ] {
        assert!(
            sqlx::query(&sql)
                .bind(number)
                .bind("0002_probe.sql")
                .bind(checksum)
                .bind(state)
                .bind("2026-09-05T00:00:00.000000Z")
                .execute(&mut *c)
                .await
                .is_err()
        );
    }
    conn.close().await.unwrap();
}

#[test]
fn nested_block_comments_cannot_hide_sqlite_transaction_control() {
    let f = Fixture::new();
    f.pair(
        "0001_init.sql",
        "/* outer /* inner */ COMMIT; -- */\nSELECT 1;\n",
    );
    assert!(matches!(
        MigrationSet::discover(&f.root, Backend::Sqlite),
        Err(MigrationError::InvalidContent { .. })
    ));
}

#[tokio::test]
async fn status_preserves_existing_delete_journal_mode() {
    let f = Fixture::new();
    f.initial();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&f.cfg.sqlite_path)
        .create_if_missing(true);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE unrelated (id INTEGER)")
        .execute(&mut conn)
        .await
        .unwrap();
    let before: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(before, "delete");
    conn.close().await.unwrap();
    assert_eq!(status(&f.cfg, &f.root).await.unwrap()[0].state, "pending");
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    let after: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(before, after);
    conn.close().await.unwrap();
}

#[tokio::test]
async fn actual_parent_rebuild_preserves_rows_and_restrict_relationship() {
    let f = Fixture::new();
    f.initial();
    migrate(&f.cfg, &f.root).await.unwrap();
    let mut conn = f.connect().await;
    execute(&mut conn, &f.renderer().migration(Backend::Sqlite, "INSERT INTO {{table:community.forums}} (slug,name,position,status,version) VALUES ('preserved','preserved',1,'draft',1); INSERT INTO {{table:moderation.rule_versions}} (forum_id,version,body,status) VALUES (1,1,'preserved','placeholder');").unwrap()).await.unwrap();
    conn.close().await.unwrap();
    // 替代表名由已注册 base 派生，不能用任意物理名绕过 renderer。
    f.pair("0002_rebuild.sql", "-- rebuild\nCREATE TABLE {{rebuild:community.forums}} (id INTEGER PRIMARY KEY AUTOINCREMENT, slug TEXT NOT NULL COLLATE BINARY UNIQUE, name TEXT NOT NULL, position INTEGER NOT NULL CHECK (position > 0), status TEXT NOT NULL CHECK (status IN ('draft','active','closed')), version INTEGER NOT NULL CHECK (version > 0), description TEXT NOT NULL DEFAULT '');\nINSERT INTO {{rebuild:community.forums}} (id,slug,name,position,status,version) SELECT id,slug,name,position,status,version FROM {{table:community.forums}};\nDROP TABLE {{table:community.forums}};\nALTER TABLE {{rebuild:community.forums}} RENAME TO {{table:community.forums}};\n");
    assert_eq!(migrate(&f.cfg, &f.root).await.unwrap().applied, [2]);
    let mut conn = f.connect().await;
    let sql = f.renderer().migration(Backend::Sqlite, "SELECT f.slug,f.description,r.body FROM {{table:community.forums}} f JOIN {{table:moderation.rule_versions}} r ON r.forum_id=f.id").unwrap();
    let DatabaseConnection::Sqlite(c) = &mut conn else {
        unreachable!()
    };
    let row: (String, String, String) = sqlx::query_as(&sql).fetch_one(&mut *c).await.unwrap();
    assert_eq!(
        row,
        (
            "preserved".to_owned(),
            String::new(),
            "preserved".to_owned()
        )
    );
    let deletion = f
        .renderer()
        .migration(
            Backend::Sqlite,
            "DELETE FROM {{table:community.forums}} WHERE id=1",
        )
        .unwrap();
    assert!(sqlx::query(&deletion).execute(&mut *c).await.is_err());
    conn.close().await.unwrap();
}

#[tokio::test]
async fn migration_source_cannot_rewrite_applied_history() {
    let f = Fixture::new();
    f.initial();
    migrate(&f.cfg, &f.root).await.unwrap();
    let first = status(&f.cfg, &f.root).await.unwrap().remove(0);
    f.pair(
        "0002_tamper.sql",
        "DELETE FROM {{table:platform.schema_migrations}} WHERE number=1;\n",
    );
    assert_eq!(
        migrate(&f.cfg, &f.root).await.unwrap_err(),
        MigrationError::MetadataMutation { number: 2 }
    );
    let after = status(&f.cfg, &f.root).await.unwrap();
    assert_eq!(after[0], first);
    assert_eq!(after[1].state, "failed");
}

#[tokio::test]
async fn hidden_generated_metadata_column_is_rejected() {
    let f = Fixture::new();
    f.initial();
    migrate(&f.cfg, &f.root).await.unwrap();
    let mut conn = f.connect().await;
    execute(&mut conn, &f.renderer().migration(Backend::Sqlite, "ALTER TABLE {{table:platform.schema_migrations}} ADD COLUMN hidden_value TEXT AS (name) VIRTUAL;").unwrap()).await.unwrap();
    conn.close().await.unwrap();
    assert_eq!(
        status(&f.cfg, &f.root).await.unwrap_err(),
        MigrationError::InvalidMetadata
    );
}

#[test]
fn postgres_escape_literals_and_transaction_aliases_are_rejected() {
    let f = Fixture::new();
    for sql in [
        r"SELECT E'\''; COMMIT; -- '
SELECT broken;
",
        "ABORT;\n",
        "PREPARE TRANSACTION 'x';\n",
    ] {
        f.pair("0001_init.sql", sql);
        assert!(matches!(
            MigrationSet::discover(&f.root, Backend::Postgres),
            Err(MigrationError::InvalidContent { .. })
        ));
    }
}

#[tokio::test]
async fn upgrades_a03_database_to_engine_and_flags_without_changing_old_data() {
    for prefix in ["bcm_", "upgrade42_"] {
        let mut f = Fixture::new();
        f.cfg.schema_prefix = prefix.into();
        f.initial();
        assert_eq!(migrate(&f.cfg, &f.root).await.unwrap().applied, [1]);
        let config = Config {
            database: f.cfg.clone(),
            ..Config::default()
        };
        crate::seed::seed(&config).await.unwrap();
        let before = status(&f.cfg, &f.root).await.unwrap();
        let current = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations");
        let result = migrate(&f.cfg, &current).await.unwrap();
        assert_eq!(result.applied, [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]);
        assert_eq!(result.skipped, [1]);
        let after = status(&f.cfg, &current).await.unwrap();
        assert_eq!(before[0].checksum, after[0].checksum);
        assert_eq!(before[0].executed_at, after[0].executed_at);
        let seed = crate::seed::seed(&config).await.unwrap();
        assert_eq!(seed.forums_created, 0);
        assert_eq!(seed.rules_created, 0);
        let pool = crate::Pool::connect_existing(&f.cfg).await.unwrap();
        let reader = crate::flags_repo::DbFlagReader::new(pool.clone(), f.renderer());
        use platform::flags::FlagReader;
        assert!(
            reader
                .load()
                .await
                .unwrap()
                .values()
                .iter()
                .all(|v| !v.enabled && v.version == Some(1))
        );
        pool.close().await;
    }
}
