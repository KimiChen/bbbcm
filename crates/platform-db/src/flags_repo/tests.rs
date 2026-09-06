use super::*;
use platform::config::Config;
use platform::flags::Flag;
use std::path::Path;

#[tokio::test]
async fn flags_snapshot_migration_registry_restart_missing_and_corruption() {
    for prefix in ["bcm_", "flags42_"] {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-flags-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut cfg = Config::default().database;
        cfg.schema_prefix = prefix.into();
        cfg.sqlite_path = dir.join("test.db").to_str().unwrap().into();
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations");
        assert_eq!(
            crate::migrations::migrate(&cfg, &migrations)
                .await
                .unwrap()
                .applied,
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
        );
        assert_eq!(
            crate::migrations::migrate(&cfg, &migrations)
                .await
                .unwrap()
                .skipped,
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
        );
        let r = SchemaRenderer::new(prefix).unwrap();
        let pool = Pool::connect(&cfg).await.unwrap();
        let reader = DbFlagReader::new(pool.clone(), r.clone());
        let snapshot = reader.load().await.unwrap();
        assert_eq!(snapshot.values().len(), Flag::ALL.len());
        assert!(
            snapshot
                .values()
                .iter()
                .all(|v| !v.enabled && v.version == Some(1))
        );
        let table = r
            .table(pool.backend(), "platform", "feature_flags")
            .unwrap();
        let Pool::Sqlite(sqlite, _) = &pool else {
            unreachable!()
        };
        let keys: Vec<String> =
            sqlx::query_scalar(&format!("SELECT flag_key FROM {table} ORDER BY flag_key"))
                .fetch_all(sqlite)
                .await
                .unwrap();
        let mut expected: Vec<_> = Flag::ALL.iter().map(|v| v.key()).collect();
        expected.sort();
        assert_eq!(keys, expected);
        // Test-only direct writes represent a later approved writer; no runtime writer is exposed.
        sqlx::query(&format!(
            "UPDATE {table} SET enabled = ?, version = ? WHERE flag_key = ?"
        ))
        .bind(1)
        .bind(2)
        .bind("public_read")
        .execute(sqlite)
        .await
        .unwrap();
        pool.close().await;
        assert!(matches!(
            reader.load().await,
            Err(FlagReadError::Unavailable)
        ));
        let pool = Pool::connect(&cfg).await.unwrap();
        let reader = DbFlagReader::new(pool.clone(), r);
        assert!(reader.load().await.unwrap().get(Flag::PublicRead).enabled);
        assert_eq!(
            reader.load().await.unwrap().get(Flag::PublicRead).version,
            Some(2)
        );
        let Pool::Sqlite(sqlite, _) = &pool else {
            unreachable!()
        };
        sqlx::query(&format!("DELETE FROM {table} WHERE flag_key = ?"))
            .bind("public_read")
            .execute(sqlite)
            .await
            .unwrap();
        let missing = reader.load().await.unwrap().get(Flag::PublicRead);
        assert!(!missing.enabled && missing.version.is_none());
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COUNT(*) FROM {table} WHERE flag_key = ?"
            ))
            .bind("public_read")
            .fetch_one(sqlite)
            .await
            .unwrap(),
            0,
            "read never repairs"
        );
        sqlx::query(&format!(
            "INSERT INTO {table} (flag_key, enabled, version) VALUES (?, ?, ?)"
        ))
        .bind("unregistered")
        .bind(0)
        .bind(1)
        .execute(sqlite)
        .await
        .unwrap();
        assert!(matches!(reader.load().await, Err(FlagReadError::Corrupt)));
        sqlx::query(&format!("DELETE FROM {table} WHERE flag_key = ?"))
            .bind("unregistered")
            .execute(sqlite)
            .await
            .unwrap();
        sqlx::query("PRAGMA ignore_check_constraints=ON")
            .execute(sqlite)
            .await
            .unwrap();
        for (enabled, version) in [(2, 1), (0, 0)] {
            sqlx::query(&format!(
                "UPDATE {table} SET enabled = ?, version = ? WHERE flag_key = ?"
            ))
            .bind(enabled)
            .bind(version)
            .bind("search")
            .execute(sqlite)
            .await
            .unwrap();
            assert!(matches!(reader.load().await, Err(FlagReadError::Corrupt)));
        }
        sqlx::query(&format!(
            "UPDATE {table} SET enabled = ?, version = ? WHERE flag_key = ?"
        ))
        .bind("broken")
        .bind(1)
        .bind("search")
        .execute(sqlite)
        .await
        .unwrap();
        assert!(matches!(reader.load().await, Err(FlagReadError::Corrupt)));
        pool.close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
#[test]
fn postgres_flag_migration_snapshot_matches_registry_and_is_read_only() {
    let source = include_str!("../../../../db/migrations/postgres/0003_flags.sql");
    let keys: Vec<_> = source
        .lines()
        .filter_map(|l| l.split("VALUES ('").nth(1))
        .map(|v| v.split("'").next().unwrap())
        .collect();
    assert_eq!(keys, Flag::ALL.iter().map(|f| f.key()).collect::<Vec<_>>());
    assert_eq!(source.matches("FALSE, 1").count(), Flag::ALL.len());
    assert!(source.contains("GRANT SELECT ON TABLE"));
    assert!(!source.contains("GRANT UPDATE"));
    assert!(!source.contains("GRANT INSERT"));
}
