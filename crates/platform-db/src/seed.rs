//! development/test 专用初始化命令；没有账号、生产 IAM 或业务事件。
use crate::connection::{DatabaseConnection, DatabaseError, Purpose};
pub use crate::seed_repo::{FORUMS, SeedSummary};
use platform::config::Config;
use platform::identifiers::SchemaRenderer;

#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("db seed 只允许 development/test；staging/production 禁止种子写入")]
    Environment,
    #[error(transparent)]
    Database(#[from] DatabaseError),
}

pub async fn seed(cfg: &Config) -> Result<SeedSummary, SeedError> {
    // 在连接/建目录之前拒绝，不能借 --backend 覆盖绕过环境边界。
    if cfg.server.environment.is_production_like() {
        return Err(SeedError::Environment);
    }
    cfg.validate()
        .map_err(|_| DatabaseError::InvalidConfiguration)?;
    let renderer = SchemaRenderer::new(&cfg.database.schema_prefix).map_err(DatabaseError::from)?;
    let mut conn = DatabaseConnection::connect(&cfg.database, Purpose::Migrator, false).await?;
    let result = run(&mut conn, &renderer).await;
    let closed = conn.close().await;
    let result = result?;
    closed?;
    Ok(result)
}

async fn run(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
) -> Result<SeedSummary, DatabaseError> {
    match conn {
        DatabaseConnection::Postgres(c) => {
            sqlx::raw_sql("BEGIN").execute(&mut *c).await?;
            if sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
                .bind(renderer.prefix())
                .execute(&mut *c)
                .await
                .is_err()
            {
                let _ = sqlx::raw_sql("ROLLBACK").execute(&mut *c).await;
                return Err(DatabaseError::Operation);
            }
        }
        DatabaseConnection::Sqlite(c) => {
            sqlx::raw_sql("BEGIN IMMEDIATE").execute(&mut *c).await?;
        }
    }
    let result = crate::seed_repo::insert_missing(conn, renderer).await;
    let end = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
    match conn {
        DatabaseConnection::Postgres(c) => {
            sqlx::raw_sql(end).execute(&mut *c).await?;
        }
        DatabaseConnection::Sqlite(c) => {
            sqlx::raw_sql(end).execute(&mut *c).await?;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform::config::Environment;
    use platform::db::Backend;
    use std::path::Path;

    #[tokio::test]
    async fn seeds_are_idempotent_restrict_references_and_preserve_edits() {
        let dir = std::env::temp_dir().join(format!("bbbcm-seed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for prefix in ["bcm_", "seed42_"] {
            let mut cfg = Config::default();
            cfg.server.environment = Environment::Test;
            cfg.database.schema_prefix = prefix.into();
            cfg.database.sqlite_path = dir.join(format!("{prefix}.db")).to_str().unwrap().into();
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations");
            crate::migrations::migrate(&cfg.database, &root)
                .await
                .unwrap();
            assert_eq!(
                seed(&cfg).await.unwrap(),
                SeedSummary {
                    forums_created: 4,
                    rules_created: 4
                }
            );
            let mut conn = DatabaseConnection::connect(&cfg.database, Purpose::Migrator, false)
                .await
                .unwrap();
            let DatabaseConnection::Sqlite(c) = &mut conn else {
                panic!("SQLite")
            };
            let renderer = SchemaRenderer::new(prefix).unwrap();
            let table = renderer
                .table(Backend::Sqlite, "community", "forums")
                .unwrap();
            let rules = renderer
                .table(Backend::Sqlite, "moderation", "rule_versions")
                .unwrap();
            sqlx::query(&format!("UPDATE {table} SET name = ? WHERE slug = ?"))
                .bind("本机编辑保留")
                .bind("ai")
                .execute(&mut *c)
                .await
                .unwrap();
            assert!(
                sqlx::query(&format!("DELETE FROM {table} WHERE slug = ?"))
                    .bind("ai")
                    .execute(&mut *c)
                    .await
                    .is_err()
            );
            let count: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {rules} WHERE status = ?"))
                    .bind("placeholder")
                    .fetch_one(&mut *c)
                    .await
                    .unwrap();
            assert_eq!(count, 4);
            conn.close().await.unwrap();
            assert_eq!(seed(&cfg).await.unwrap(), SeedSummary::default());
            let mut conn = DatabaseConnection::connect(&cfg.database, Purpose::Migrator, false)
                .await
                .unwrap();
            let DatabaseConnection::Sqlite(c) = &mut conn else {
                panic!("SQLite")
            };
            let name: String =
                sqlx::query_scalar(&format!("SELECT name FROM {table} WHERE slug = ?"))
                    .bind("ai")
                    .fetch_one(&mut *c)
                    .await
                    .unwrap();
            assert_eq!(name, "本机编辑保留");
            let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name").fetch_all(&mut *c).await.unwrap();
            let mut expected: Vec<_> = platform::identifiers::TABLES
                .iter()
                .map(|t| renderer.table(Backend::Sqlite, t.schema, t.table).unwrap())
                .collect();
            expected.sort();
            assert_eq!(
                tables, expected,
                "种子只使用已迁移受信对象，不创建账户或IAM表"
            );
            let flags = renderer
                .table(Backend::Sqlite, "platform", "feature_flags")
                .unwrap();
            let enabled: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {flags} WHERE enabled <> 0"))
                    .fetch_one(&mut *c)
                    .await
                    .unwrap();
            assert_eq!(enabled, 0, "种子不能启用Flag");
            conn.close().await.unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[tokio::test]
    async fn seed_denies_both_production_like_environments_before_validation_or_io() {
        for environment in [Environment::Staging, Environment::Production] {
            let mut cfg = Config::default();
            cfg.server.environment = environment;
            assert!(matches!(seed(&cfg).await, Err(SeedError::Environment)));
        }
    }
}
