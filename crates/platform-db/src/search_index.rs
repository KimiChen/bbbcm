//! 完整模式搜索索引轨（`postgres-search`）的发现与执行。
//!
//! **为什么不放进常规迁移轨**：`02` §6.5 允许缺 `pg_jieba` 的 PostgreSQL 实例
//! 正常启动、只关闭搜索能力，不回退到未评审的检索路径。而 GIN 表达式索引必须写
//! `to_tsvector('public.jiebaqry', …)`，缺扩展的实例执行该 DDL 会直接失败——把它
//! 放进常规轨等于把 `pg_jieba` 升格为**迁移期硬依赖**，那条降级路径就没了。
//!
//! 因此索引单独成轨，由显式命令执行；扩展不在时**跳过而不是报错**，与 `02` §6.5
//! 的降级语义一致。轨的执行历史记在独立的 `platform.search_migrations` 表，不与
//! 三库基线轨的 `schema_migrations` 混用——两者的适用实例集合本就不同，混在一张
//! 表里会让「基线已就绪」与「搜索已就绪」无法区分。
use crate::connection::{DatabaseConnection, Purpose};
use crate::migrations::{MigrationError, MigrationSet, SEARCH_TRACK};
use platform::config::DatabaseConfig;
use platform::db::Backend;
use platform::identifiers::SchemaRenderer;
use std::path::Path;

/// 本轨要求的扩展。缺任意一个即整轨跳过。
const REQUIRED_EXTENSIONS: &[&str] = &["pg_jieba"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchIndexOutcome {
    /// 已应用与已跳过的迁移编号。
    Applied {
        applied: Vec<i64>,
        skipped: Vec<i64>,
    },
    /// 扩展不在位，整轨未执行。**这不是失败**：`02` §6.5 允许这种实例运行，
    /// 只是搜索能力关闭。附上缺失的扩展名，便于运维判断该装什么。
    ExtensionMissing { missing: Vec<String> },
}

async fn execute(conn: &mut DatabaseConnection, sql: &str) -> Result<(), MigrationError> {
    match conn {
        DatabaseConnection::Postgres(c) => {
            sqlx::raw_sql(sql).execute(c).await?;
        }
        DatabaseConnection::Sqlite(_) => return Err(MigrationError::UnsupportedBackend),
    }
    Ok(())
}

/// 检查所需扩展是否已安装。只读 `pg_extension`，不尝试 `CREATE EXTENSION`——
/// 全项目没有任何迁移创建扩展，扩展由镜像与运维提供（`99` §5.24）。
async fn missing_extensions(conn: &mut DatabaseConnection) -> Result<Vec<String>, MigrationError> {
    let DatabaseConnection::Postgres(c) = conn else {
        return Err(MigrationError::UnsupportedBackend);
    };
    let mut missing = Vec::new();
    for name in REQUIRED_EXTENSIONS {
        // 类型必须是 i32：PostgreSQL 的字面量 `1` 是 int4，sqlx 按声明类型严格
        // 解码，写成 i64 会在运行期变成一个被脱敏成「数据库操作失败」的解码错误。
        let present: Option<i32> =
            sqlx::query_scalar("SELECT 1 FROM pg_extension WHERE extname = $1")
                .bind(name)
                .fetch_optional(&mut *c)
                .await?;
        if present.is_none() {
            missing.push((*name).to_owned());
        }
    }
    Ok(missing)
}

/// 建历史表。列与 `schema_migrations` 同形，便于运维用同一套查询习惯读它，
/// 但**是另一张表**——见模块头的理由。
async fn bootstrap(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
) -> Result<(), MigrationError> {
    let ddl = renderer.migration(
        Backend::Postgres,
        "CREATE TABLE IF NOT EXISTS {{table:platform.search_migrations}} (number BIGINT NOT NULL PRIMARY KEY CHECK (number > 0), name TEXT NOT NULL, checksum TEXT NOT NULL CHECK (length(checksum) = 64), state TEXT NOT NULL CHECK (state IN ('applied', 'failed')), executed_at TIMESTAMPTZ NOT NULL);",
    )?;
    execute(conn, &ddl).await?;
    let acl = renderer.migration(
        Backend::Postgres,
        "REVOKE ALL ON TABLE {{table:platform.search_migrations}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};",
    )?;
    execute(conn, &acl).await
}

/// 应用搜索索引轨。
///
/// 幂等：已记录且校验和相同的迁移直接跳过；不同则报 `ContentDrift` 并拒绝执行。
pub async fn migrate_search(
    cfg: &DatabaseConfig,
    root: &Path,
) -> Result<SearchIndexOutcome, MigrationError> {
    if cfg.backend != Backend::Postgres {
        return Err(MigrationError::UnsupportedBackend);
    }
    let set = MigrationSet::discover_track(root, Backend::Postgres, SEARCH_TRACK)?;
    let renderer = SchemaRenderer::new(&cfg.schema_prefix)?;
    // 先在连库之前把模板渲染一遍：非受信标识符要在碰数据库之前就失败。
    for migration in &set.migrations {
        renderer.migration(Backend::Postgres, &migration.sql)?;
    }
    let mut conn = DatabaseConnection::connect(cfg, Purpose::Migrator, true).await?;
    let result = async {
        let missing = missing_extensions(&mut conn).await?;
        if !missing.is_empty() {
            return Ok(SearchIndexOutcome::ExtensionMissing { missing });
        }
        bootstrap(&mut conn, &renderer).await?;
        let table = renderer.table(Backend::Postgres, "platform", "search_migrations")?;
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        for migration in &set.migrations {
            let DatabaseConnection::Postgres(c) = &mut conn else {
                return Err(MigrationError::UnsupportedBackend);
            };
            let existing: Option<(String, String)> = sqlx::query_as(&format!(
                "SELECT checksum, state FROM {table} WHERE number = $1"
            ))
            .bind(migration.number)
            .fetch_optional(&mut *c)
            .await?;
            if let Some((checksum, state)) = existing {
                if checksum != migration.checksum {
                    // 与常规轨同一语义：登记内容与文件不一致时拒绝执行，
                    // 必须用新的前向迁移。索引定义改了却沿用同一编号，会让
                    // 不同实例上的索引悄悄不一致，而查询计划的差异不报错。
                    return Err(MigrationError::ContentDrift {
                        number: migration.number,
                    });
                }
                if state == "applied" {
                    skipped.push(migration.number);
                    continue;
                }
            }
            let sql = renderer.migration(Backend::Postgres, &migration.sql)?;
            execute(&mut conn, &sql).await?;
            let DatabaseConnection::Postgres(c) = &mut conn else {
                return Err(MigrationError::UnsupportedBackend);
            };
            sqlx::query(&format!(
                "INSERT INTO {table} (number, name, checksum, state, executed_at) VALUES ($1, $2, $3, 'applied', now()) ON CONFLICT (number) DO UPDATE SET checksum = EXCLUDED.checksum, name = EXCLUDED.name, state = 'applied', executed_at = EXCLUDED.executed_at"
            ))
            .bind(migration.number)
            .bind(&migration.name)
            .bind(&migration.checksum)
            .execute(&mut *c)
            .await?;
            applied.push(migration.number);
        }
        Ok(SearchIndexOutcome::Applied { applied, skipped })
    }
    .await;
    let closed = conn.close().await;
    let outcome = result?;
    closed?;
    Ok(outcome)
}
