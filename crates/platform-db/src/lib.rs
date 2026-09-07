//! 连接管理、能力探测与数据库初始化；业务事务引擎归 A04。

use platform::config::DatabaseConfig;
use platform::db::{Backend, Capabilities};
use platform::observability::{DatabasePurpose, PoolObserver};
use std::sync::Arc;
use thiserror::Error;

mod pool_observation;

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("后端 {0} 尚未接入（DEC-022 规范预留值）")]
    BackendNotImplemented(Backend),
    #[error(transparent)]
    Database(#[from] connection::DatabaseError),
}
impl From<sqlx::Error> for ConnectError {
    fn from(_: sqlx::Error) -> Self {
        Self::Database(connection::DatabaseError::Operation)
    }
}

/// 双后端连接池封装；repo 经 `platform::db::render_sql` + SchemaRenderer 使用。
/// `sqlx` 的连接池本身是 `Arc` 语义，`Clone` 只增加引用计数，便于放入 axum 状态。
#[derive(Clone, Debug)]
pub enum Pool {
    Postgres(sqlx::PgPool, Arc<dyn PoolObserver>),
    Sqlite(sqlx::SqlitePool, Arc<dyn PoolObserver>),
}

impl Pool {
    pub fn backend(&self) -> Backend {
        match self {
            Pool::Postgres(..) => Backend::Postgres,
            Pool::Sqlite(..) => Backend::Sqlite,
        }
    }

    /// Construction context, never inferred from a URL or database credentials.
    pub fn purpose(&self) -> DatabasePurpose {
        match self {
            Self::Postgres(_, observer) | Self::Sqlite(_, observer) => observer.purpose(),
        }
    }

    /// 按配置建立连接池；SQLite 自动启用 WAL、busy_timeout 与外键。
    ///
    /// SQLite 的连接数取自配置且被校验为 1：多写连接只会把单写者冲突变成
    /// `SQLITE_BUSY`，而不会提高吞吐（`02` 第 6.1 节）。
    pub async fn connect(cfg: &DatabaseConfig) -> Result<Pool, ConnectError> {
        Self::connect_for(cfg, connection::Purpose::App).await
    }

    pub async fn connect_for(
        cfg: &DatabaseConfig,
        purpose: connection::Purpose,
    ) -> Result<Pool, ConnectError> {
        if cfg.max_connections == 0 || (cfg.backend == Backend::Sqlite && cfg.max_connections != 1)
        {
            return Err(connection::DatabaseError::InvalidConfiguration.into());
        }
        match cfg.backend {
            Backend::Mysql8 => Err(ConnectError::BackendNotImplemented(cfg.backend)),
            Backend::Sqlite => {
                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(cfg.max_connections)
                    .connect_with(connection::sqlite_options(cfg, true)?)
                    .await?;
                let observer = pool_observation::register(&pool, cfg.backend, purpose);
                Ok(Pool::Sqlite(pool, observer))
            }
            Backend::Postgres => {
                let renderer = platform::identifiers::SchemaRenderer::new(&cfg.schema_prefix)
                    .map_err(connection::DatabaseError::from)?;
                let pool = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(cfg.max_connections)
                    .after_connect(move |conn, _| {
                        let renderer = renderer.clone();
                        Box::pin(async move {
                            connection::prepare_postgres(conn, &renderer, purpose)
                                .await
                                .map_err(|_| {
                                    sqlx::Error::Protocol("数据库启动校验失败（已脱敏）".into())
                                })
                        })
                    })
                    .connect_with(connection::postgres_options(cfg)?)
                    .await?;
                let observer = pool_observation::register(&pool, cfg.backend, purpose);
                Ok(Pool::Postgres(pool, observer))
            }
        }
    }

    /// 只读查询入口连接已有存储；SQLite 不创建目录或数据库，不修复结构。
    pub async fn connect_existing(cfg: &DatabaseConfig) -> Result<Pool, ConnectError> {
        if cfg.backend != Backend::Sqlite {
            return Self::connect(cfg).await;
        }
        if cfg.max_connections != 1 {
            return Err(connection::DatabaseError::InvalidConfiguration.into());
        }
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(connection::sqlite_options(cfg, false)?)
            .await?;
        let observer = pool_observation::register(&pool, cfg.backend, connection::Purpose::App);
        Ok(Pool::Sqlite(pool, observer))
    }

    /// 探测后端能力集。
    ///
    /// SQLite 的 FTS5 探针建在 `temp` 库上，而 `temp` 是**连接级**的：建表与删表
    /// 必须落在同一条连接上，否则探针表会留在池中某条连接里。
    /// PostgreSQL 查询 `pg_extension` 确认 `pg_jieba`/`pg_trgm` 是否装好——
    /// 二者是完整模式全文检索的前提，缺失必须能被上层发现（`02` 第 4 节）。
    pub async fn capabilities(&self) -> Capabilities {
        match self {
            Pool::Sqlite(pool, _) => {
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
                    Err(_) => {
                        tracing::warn!("取连接探测 FTS5 失败，按不可用处理");
                    }
                }
                caps
            }
            Pool::Postgres(pool, _) => {
                // 连同**安装 schema**一起查。运行连接的 `search_path` 被固定为
                // `pg_catalog`（见 `prepare_postgres`），因此检索配置名必须限定
                // 到 schema。项目只支持装在 `public` 的常规位置；装在别处时这里
                // 按「扩展缺失」处理——降级为标题检索并如实告知，比在每次查询时
                // 抛 `text search configuration does not exist` 好得多。
                let installed: Vec<String> = sqlx::query_scalar(
                    "SELECT e.extname FROM pg_extension e \
                     JOIN pg_namespace n ON n.oid = e.extnamespace WHERE n.nspname = 'public'",
                )
                .fetch_all(pool)
                .await
                .unwrap_or_else(|_| {
                    tracing::warn!("查询 pg_extension 失败，按扩展缺失处理");
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
            Pool::Postgres(pool, _) => pool.close().await,
            Pool::Sqlite(pool, _) => pool.close().await,
        }
    }
}

pub mod acl;
pub mod audit_repo;
pub mod command;
pub mod connection;
pub mod flags_repo;
pub mod idempotency_repo;
pub mod migrations;
pub mod reserved_slug;
pub mod seed;
mod seed_repo;
pub mod transaction;

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

        let Pool::Sqlite(ref sqlite, _) = pool else {
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

pub mod outbox_repo;

pub mod analytics_repo;
pub mod mail_repo;
pub mod policy;
pub mod status_repo;
