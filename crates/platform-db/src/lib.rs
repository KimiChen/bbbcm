//! 连接管理与后端能力探测；事务模板与迁移执行随后续工作包接入。

use platform::config::DatabaseConfig;
use platform::db::{Backend, Capabilities};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("缺少 PostgreSQL 连接串：请设置环境变量 BBBCM_DATABASE_POSTGRES_URL")]
    MissingPostgresUrl,
    #[error("后端 {0} 尚未接入（DEC-022 规范预留值）")]
    BackendNotImplemented(Backend),
    #[error("数据库连接失败: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("创建数据目录失败: {0}")]
    Io(#[from] std::io::Error),
}

/// 双后端连接池封装；repo 经 `platform::db::render_sql` + SchemaRenderer 使用。
/// `sqlx` 的连接池本身是 `Arc` 语义，`Clone` 只增加引用计数，便于放入 axum 状态。
#[derive(Clone, Debug)]
pub enum Pool {
    Postgres(sqlx::PgPool),
    Sqlite(sqlx::SqlitePool),
}

impl Pool {
    pub fn backend(&self) -> Backend {
        match self {
            Pool::Postgres(_) => Backend::Postgres,
            Pool::Sqlite(_) => Backend::Sqlite,
        }
    }

    /// 按配置建立连接池；SQLite 自动启用 WAL、busy_timeout 与外键。
    ///
    /// SQLite 的连接数取自配置且被校验为 1：多写连接只会把单写者冲突变成
    /// `SQLITE_BUSY`，而不会提高吞吐（`02` 第 6.1 节）。
    pub async fn connect(cfg: &DatabaseConfig) -> Result<Pool, ConnectError> {
        match cfg.backend {
            Backend::Mysql8 => Err(ConnectError::BackendNotImplemented(cfg.backend)),
            Backend::Sqlite => {
                if let Some(dir) = std::path::Path::new(&cfg.sqlite_path).parent()
                    && !dir.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(dir)?;
                }
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(cfg.sqlite_path.clone())
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .busy_timeout(Duration::from_millis(5000))
                    .foreign_keys(true);
                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(cfg.max_connections.max(1))
                    .connect_with(options)
                    .await?;
                Ok(Pool::Sqlite(pool))
            }
            Backend::Postgres => {
                let url = cfg
                    .postgres_url
                    .as_deref()
                    .ok_or(ConnectError::MissingPostgresUrl)?;
                let pool = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(cfg.max_connections.max(1))
                    .connect(url)
                    .await?;
                Ok(Pool::Postgres(pool))
            }
        }
    }

    /// 探测后端能力集。
    ///
    /// SQLite 的 FTS5 探针建在 `temp` 库上，而 `temp` 是**连接级**的：建表与删表
    /// 必须落在同一条连接上，否则探针表会留在池中某条连接里。
    /// PostgreSQL 查询 `pg_extension` 确认 `pg_jieba`/`pg_trgm` 是否装好——
    /// 二者是完整模式全文检索的前提，缺失必须能被上层发现（`02` 第 4 节）。
    pub async fn capabilities(&self) -> Capabilities {
        match self {
            Pool::Sqlite(pool) => {
                let mut caps = Capabilities::default();
                match pool.acquire().await {
                    Ok(mut conn) => {
                        let probe = sqlx::query(
                            "CREATE VIRTUAL TABLE IF NOT EXISTS temp.fts5_probe USING fts5(x)",
                        )
                        .execute(conn.as_mut())
                        .await;
                        caps.sqlite_fts5 = probe.is_ok();
                        if caps.sqlite_fts5 {
                            // 与建表同一连接，探针表不会残留。
                            let _ = sqlx::query("DROP TABLE IF EXISTS temp.fts5_probe")
                                .execute(conn.as_mut())
                                .await;
                        } else {
                            tracing::info!("SQLite 未编译 FTS5，标题搜索降级为规范化列 LIKE");
                        }
                    }
                    Err(err) => {
                        tracing::warn!(%err, "取连接探测 FTS5 失败，按不可用处理");
                    }
                }
                caps
            }
            Pool::Postgres(pool) => {
                let installed: Vec<String> = sqlx::query_scalar("SELECT extname FROM pg_extension")
                    .fetch_all(pool)
                    .await
                    .unwrap_or_else(|err| {
                        tracing::warn!(%err, "查询 pg_extension 失败，按扩展缺失处理");
                        Vec::new()
                    });
                let caps = Capabilities {
                    sqlite_fts5: false,
                    pg_jieba: installed.iter().any(|e| e == "pg_jieba"),
                    pg_trgm: installed.iter().any(|e| e == "pg_trgm"),
                };
                if !caps.pg_jieba || !caps.pg_trgm {
                    tracing::warn!(
                        pg_jieba = caps.pg_jieba,
                        pg_trgm = caps.pg_trgm,
                        "完整模式缺少全文检索扩展，搜索能力须按降级矩阵关闭"
                    );
                }
                caps
            }
        }
    }

    pub async fn close(&self) {
        match self {
            Pool::Postgres(pool) => pool.close().await,
            Pool::Sqlite(pool) => pool.close().await,
        }
    }
}

pub mod migrations;

#[cfg(test)]
mod tests {
    use super::*;
    use platform::config::Config;

    fn sqlite_cfg(path: &str) -> DatabaseConfig {
        let mut cfg = Config::default().database;
        cfg.sqlite_path = path.to_string();
        cfg
    }

    #[tokio::test]
    async fn mysql8_is_refused() {
        let mut cfg = sqlite_cfg("data/unused.db");
        cfg.backend = Backend::Mysql8;
        let err = Pool::connect(&cfg).await.expect_err("预留后端必须拒绝");
        assert!(matches!(err, ConnectError::BackendNotImplemented(_)));
    }

    #[tokio::test]
    async fn sqlite_applies_pragmas_and_probes_fts5() {
        let dir = std::env::temp_dir().join(format!("bbbcm-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录失败");
        let path = dir.join("probe.db");
        let pool = Pool::connect(&sqlite_cfg(path.to_str().expect("路径应为 UTF-8")))
            .await
            .expect("SQLite 应可连接");
        assert_eq!(pool.backend(), Backend::Sqlite);

        let Pool::Sqlite(ref sqlite) = pool else {
            panic!("应为 SQLite 池");
        };
        let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(sqlite)
            .await
            .expect("应能读 journal_mode");
        assert_eq!(journal.to_lowercase(), "wal", "必须启用 WAL");
        let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(sqlite)
            .await
            .expect("应能读 foreign_keys");
        assert_eq!(fk, 1, "必须启用外键");

        let caps = pool.capabilities().await;
        // 探针表必须不残留：再建一次同名表应当成功。
        if caps.sqlite_fts5 {
            sqlx::query("CREATE VIRTUAL TABLE temp.fts5_probe USING fts5(x)")
                .execute(sqlite)
                .await
                .expect("探针表应已被清理，可重新创建");
        }
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
