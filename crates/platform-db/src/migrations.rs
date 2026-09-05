//! N 轨迁移：专用连接、前缀锁、内容身份与失败恢复；不提供 A04 业务事务引擎。

mod discovery;
pub use discovery::{Migration, MigrationSet, TRACKS, verify_alignment, verify_repository};

use crate::connection::{DatabaseConnection, Purpose};
use chrono::{DateTime, Utc};
use platform::config::DatabaseConfig;
use platform::db::{Backend, encode_time, render_sql};
use platform::identifiers::SchemaRenderer;
use sqlx::{ConnectOptions, Connection};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MigrationError {
    #[error("无法读取迁移目录或文件；检查本机路径及文件权限")]
    Unreadable,
    #[error("迁移文件名必须为连续正编号 NNNN_name.sql：{track}/{name}")]
    BadFileName { track: String, name: String },
    #[error("{track} 轨存在重复编号 {number}")]
    DuplicateNumber { track: String, number: String },
    #[error("迁移编号不对齐或不连续：{detail}")]
    Misaligned { detail: String },
    #[error("迁移 {number:04} 内容非法：{reason}")]
    InvalidContent { number: i64, reason: &'static str },
    #[error("mysql8 后端尚未接入，不能执行迁移")]
    UnsupportedBackend,
    #[error("数据库配置或受信迁移标识符非法")]
    InvalidConfiguration,
    #[error("数据库迁移操作失败；底层 SQL、参数与连接信息已脱敏")]
    Database,
    #[error("迁移元数据结构、顺序、状态或所有权不符合约定；拒绝接管")]
    InvalidMetadata,
    #[error("迁移 {number:04} 已登记内容与文件不一致；必须使用新的前向迁移")]
    ContentDrift { number: i64 },
    #[error("迁移 {number:04} 执行失败且已回滚，失败状态已记录；原 SQL 与参数已脱敏")]
    ExecutionFailed { number: i64 },
    #[error("迁移 {number:04} 重建后存在外键违规，已回滚并记录失败")]
    ForeignKeyViolation { number: i64 },
    #[error("迁移 {number:04} 试图改变 runner 元数据；已回滚并记录失败")]
    MetadataMutation { number: i64 },
    #[error("事务回滚、外键恢复或失败状态记录未完成；连接已丢弃，须 db status 核查")]
    RecoveryFailed,
    #[error("提交结果未知；禁止假定成功或回滚，请重新 db status 核查")]
    CommitUnknown,
}

impl From<sqlx::Error> for MigrationError {
    fn from(_: sqlx::Error) -> Self {
        Self::Database
    }
}
impl From<crate::connection::DatabaseError> for MigrationError {
    fn from(_: crate::connection::DatabaseError) -> Self {
        Self::Database
    }
}
impl From<platform::identifiers::SchemaError> for MigrationError {
    fn from(_: platform::identifiers::SchemaError) -> Self {
        Self::InvalidConfiguration
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStatus {
    pub number: i64,
    pub name: String,
    pub checksum: String,
    /// pending 仅由源文件集合派生；数据库只保存 applied/failed。
    pub state: String,
    pub executed_at: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationSummary {
    pub applied: Vec<i64>,
    pub skipped: Vec<i64>,
}

fn render(
    renderer: &SchemaRenderer,
    backend: Backend,
    sql: &str,
) -> Result<String, MigrationError> {
    render_sql(backend, &renderer.migration(backend, sql)?)
        .map_err(|_| MigrationError::InvalidConfiguration)
}

async fn execute(conn: &mut DatabaseConnection, sql: &str) -> Result<(), MigrationError> {
    match conn {
        DatabaseConnection::Postgres(c) => {
            sqlx::raw_sql(sql).execute(c).await?;
        }
        DatabaseConnection::Sqlite(c) => {
            sqlx::raw_sql(sql).execute(c).await?;
        }
    }
    Ok(())
}

async fn begin(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
) -> Result<(), MigrationError> {
    match conn {
        DatabaseConnection::Postgres(c) => {
            sqlx::raw_sql("BEGIN").execute(&mut *c).await?;
            if sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
                .bind(renderer.prefix())
                .execute(&mut *c)
                .await
                .is_err()
            {
                let _ = sqlx::raw_sql("ROLLBACK").execute(c).await;
                return Err(MigrationError::Database);
            }
        }
        DatabaseConnection::Sqlite(c) => {
            sqlx::raw_sql("BEGIN IMMEDIATE").execute(c).await?;
        }
    }
    Ok(())
}

async fn restore_foreign_keys(conn: &mut DatabaseConnection) -> Result<(), MigrationError> {
    if let DatabaseConnection::Sqlite(c) = conn {
        sqlx::raw_sql("PRAGMA foreign_keys = ON")
            .execute(&mut *c)
            .await
            .map_err(|_| MigrationError::RecoveryFailed)?;
        let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(c)
            .await
            .map_err(|_| MigrationError::RecoveryFailed)?;
        if enabled != 1 {
            return Err(MigrationError::RecoveryFailed);
        }
    }
    Ok(())
}

async fn rollback(conn: &mut DatabaseConnection) -> Result<(), MigrationError> {
    let rolled_back = execute(conn, "ROLLBACK").await;
    let restored = restore_foreign_keys(conn).await;
    rolled_back.map_err(|_| MigrationError::RecoveryFailed)?;
    restored
}

async fn metadata_exists(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
) -> Result<bool, MigrationError> {
    match conn {
        DatabaseConnection::Postgres(c) => {
            let found: Option<(bool, String)> = sqlx::query_as(
                "SELECT c.relkind = 'r', r.rolname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace JOIN pg_roles r ON r.oid = c.relowner WHERE n.nspname = $1 AND c.relname = $2")
                .bind(renderer.schema("platform")?).bind("schema_migrations")
                .fetch_optional(c).await?;
            if let Some((kind, owner)) = found {
                if !kind || owner != renderer.role("migrator")? {
                    return Err(MigrationError::InvalidMetadata);
                }
                Ok(true)
            } else {
                Ok(false)
            }
        }
        DatabaseConnection::Sqlite(c) => {
            let found: Option<String> =
                sqlx::query_scalar("SELECT type FROM main.sqlite_schema WHERE name = ?")
                    .bind(renderer.table(Backend::Sqlite, "platform", "schema_migrations")?)
                    .fetch_optional(c)
                    .await?;
            match found.as_deref() {
                Some("table") => Ok(true),
                None => Ok(false),
                _ => Err(MigrationError::InvalidMetadata),
            }
        }
    }
}

async fn bootstrap_inner(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
) -> Result<(), MigrationError> {
    validate_schema_owners(conn, renderer).await?;
    if let DatabaseConnection::Postgres(c) = conn {
        let owner: Option<String> = sqlx::query_scalar("SELECT r.rolname FROM pg_namespace n JOIN pg_roles r ON r.oid = n.nspowner WHERE n.nspname = $1")
            .bind(renderer.schema("platform")?).fetch_optional(&mut *c).await?;
        match owner {
            Some(owner) if owner != renderer.role("migrator")? => {
                return Err(MigrationError::InvalidMetadata);
            }
            Some(_) => {}
            None => {
                let ddl = renderer.migration(
                    Backend::Postgres,
                    "CREATE SCHEMA {{schema:platform}} AUTHORIZATION {{role:migrator}};",
                )?;
                sqlx::raw_sql(&ddl).execute(&mut *c).await?;
            }
        }
    }
    if metadata_exists(conn, renderer).await? {
        validate_shape(conn, renderer).await?;
        return Ok(());
    }
    let time_type = if conn.backend() == Backend::Postgres {
        "TIMESTAMPTZ"
    } else {
        "TEXT"
    };
    let ddl = renderer.migration(conn.backend(), &format!(
        "CREATE TABLE {{{{table:platform.schema_migrations}}}} (number BIGINT NOT NULL PRIMARY KEY CHECK (number > 0), name TEXT NOT NULL, checksum TEXT NOT NULL CHECK (length(checksum) = 64), state TEXT NOT NULL CHECK (state IN ('applied', 'failed')), executed_at {time_type} NOT NULL);"))?;
    execute(conn, &ddl).await?;
    if conn.backend() == Backend::Postgres {
        let acl = renderer.migration(Backend::Postgres,
            "REVOKE ALL ON SCHEMA {{schema:platform}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}}; REVOKE ALL ON TABLE {{table:platform.schema_migrations}} FROM PUBLIC, {{role:app_rw}}, {{role:worker_rw}}, {{role:report_ro}};")?;
        execute(conn, &acl).await?;
    }
    Ok(())
}

/// IF NOT EXISTS 不会修正已有 schema 的 owner，必须在事务锁内逐一核对。
async fn validate_schema_owners(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
) -> Result<(), MigrationError> {
    if let DatabaseConnection::Postgres(c) = conn {
        for logical in platform::identifiers::LOGICAL_SCHEMAS {
            let owner: Option<String> = sqlx::query_scalar(
                "SELECT r.rolname FROM pg_namespace n JOIN pg_roles r ON r.oid = n.nspowner WHERE n.nspname = $1",
            )
            .bind(renderer.schema(logical)?)
            .fetch_optional(&mut *c)
            .await?;
            if owner.is_some_and(|name| name != renderer.role("migrator").expect("受信角色")) {
                return Err(MigrationError::InvalidMetadata);
            }
        }
    }
    Ok(())
}

/// 元数据不接受“同名就兼容”：校验精确列、类型、NOT NULL 与主键。
type SqliteColumnInfo = (i64, String, String, i64, Option<String>, i64, i64);

async fn validate_shape(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
) -> Result<(), MigrationError> {
    let expected = ["number", "name", "checksum", "state", "executed_at"];
    let valid = match conn {
        DatabaseConnection::Sqlite(c) => {
            let sql = format!(
                "PRAGMA main.table_xinfo({})",
                renderer.table(Backend::Sqlite, "platform", "schema_migrations")?
            );
            let columns: Vec<SqliteColumnInfo> = sqlx::query_as(&sql).fetch_all(&mut *c).await?;
            columns.len() == expected.len()
                && columns.iter().enumerate().all(|(i, col)| {
                    col.1 == expected[i]
                        && col.2 == if i == 0 { "BIGINT" } else { "TEXT" }
                        && col.3 == 1
                        && col.5 == i64::from(i == 0)
                        && col.6 == 0
                })
        }
        DatabaseConnection::Postgres(c) => {
            let columns: Vec<(String, String, bool, bool)> = sqlx::query_as(
                "SELECT a.attname, t.typname, a.attnotnull, EXISTS(SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid AND i.indisprimary AND a.attnum = ANY(i.indkey)) FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace JOIN pg_type t ON t.oid = a.atttypid WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum")
                .bind(renderer.schema("platform")?).bind("schema_migrations").fetch_all(c).await?;
            columns.len() == expected.len()
                && columns.iter().enumerate().all(|(i, col)| {
                    let ty = if i == 0 {
                        "int8"
                    } else if i == 4 {
                        "timestamptz"
                    } else {
                        "text"
                    };
                    col.0 == expected[i] && col.1 == ty && col.2 && col.3 == (i == 0)
                })
        }
    };
    if valid {
        Ok(())
    } else {
        Err(MigrationError::InvalidMetadata)
    }
}

async fn bootstrap(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
) -> Result<(), MigrationError> {
    begin(conn, renderer).await?;
    match bootstrap_inner(conn, renderer).await {
        Ok(()) => execute(conn, "COMMIT")
            .await
            .map_err(|_| MigrationError::CommitUnknown),
        Err(err) => {
            rollback(conn).await?;
            Err(err)
        }
    }
}

async fn read_history(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
) -> Result<Vec<MigrationStatus>, MigrationError> {
    if !metadata_exists(conn, renderer).await? {
        return Ok(Vec::new());
    }
    validate_shape(conn, renderer).await?;
    let sql = renderer.migration(conn.backend(), "SELECT number, name, checksum, state, executed_at FROM {{table:platform.schema_migrations}} ORDER BY number")?;
    match conn {
        DatabaseConnection::Postgres(c) => {
            let rows: Vec<(i64, String, String, String, DateTime<Utc>)> =
                sqlx::query_as(&sql).fetch_all(c).await?;
            Ok(rows
                .into_iter()
                .map(
                    |(number, name, checksum, state, executed_at)| MigrationStatus {
                        number,
                        name,
                        checksum,
                        state,
                        executed_at: Some(encode_time(executed_at)),
                    },
                )
                .collect())
        }
        DatabaseConnection::Sqlite(c) => {
            let rows: Vec<(i64, String, String, String, String)> =
                sqlx::query_as(&sql).fetch_all(c).await?;
            rows.into_iter()
                .map(|(number, name, checksum, state, executed_at)| {
                    let time = platform::db::decode_time(&executed_at)
                        .map_err(|_| MigrationError::InvalidMetadata)?;
                    if encode_time(time) != executed_at {
                        return Err(MigrationError::InvalidMetadata);
                    }
                    Ok(MigrationStatus {
                        number,
                        name,
                        checksum,
                        state,
                        executed_at: Some(executed_at),
                    })
                })
                .collect()
        }
    }
}

fn reconcile(
    set: &MigrationSet,
    history: &[MigrationStatus],
) -> Result<Vec<MigrationStatus>, MigrationError> {
    let mut failed = false;
    for (index, record) in history.iter().enumerate() {
        if record.number != index as i64 + 1
            || failed
            || !matches!(record.state.as_str(), "applied" | "failed")
        {
            return Err(MigrationError::InvalidMetadata);
        }
        let source = set
            .migrations
            .get(index)
            .ok_or(MigrationError::ContentDrift {
                number: record.number,
            })?;
        if source.name != record.name || source.checksum != record.checksum {
            return Err(MigrationError::ContentDrift {
                number: record.number,
            });
        }
        failed = record.state == "failed";
    }
    Ok(set
        .migrations
        .iter()
        .enumerate()
        .map(|(index, source)| {
            history.get(index).cloned().unwrap_or(MigrationStatus {
                number: source.number,
                name: source.name.clone(),
                checksum: source.checksum.clone(),
                state: "pending".to_owned(),
                executed_at: None,
            })
        })
        .collect())
}

async fn write_state(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
    migration: &Migration,
    state: &str,
    exists: bool,
) -> Result<(), MigrationError> {
    let template = if exists {
        "UPDATE {{table:platform.schema_migrations}} SET name = ?, checksum = ?, state = ?, executed_at = ? WHERE number = ?"
    } else {
        "INSERT INTO {{table:platform.schema_migrations}} (name, checksum, state, executed_at, number) VALUES (?, ?, ?, ?, ?)"
    };
    let sql = render(renderer, conn.backend(), template)?;
    let now = Utc::now();
    let affected = match conn {
        DatabaseConnection::Postgres(c) => sqlx::query(&sql)
            .bind(&migration.name)
            .bind(&migration.checksum)
            .bind(state)
            .bind(now)
            .bind(migration.number)
            .execute(c)
            .await?
            .rows_affected(),
        DatabaseConnection::Sqlite(c) => sqlx::query(&sql)
            .bind(&migration.name)
            .bind(&migration.checksum)
            .bind(state)
            .bind(encode_time(now))
            .bind(migration.number)
            .execute(c)
            .await?
            .rows_affected(),
    };
    if affected != 1 {
        return Err(MigrationError::InvalidMetadata);
    }
    Ok(())
}

async fn record_failure(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
    set: &MigrationSet,
    migration: &Migration,
) -> Result<(), MigrationError> {
    begin(conn, renderer).await?;
    let result = async {
        let history = read_history(conn, renderer).await?;
        let states = reconcile(set, &history)?;
        // 在原失败事务释放锁后，另一 runner 可能已成功。绝不降级 applied。
        let state = &states[migration.number as usize - 1];
        if state.state != "applied" {
            write_state(conn, renderer, migration, "failed", state.state == "failed").await?;
        }
        Ok::<(), MigrationError>(())
    }
    .await;
    match result {
        Ok(()) => execute(conn, "COMMIT")
            .await
            .map_err(|_| MigrationError::CommitUnknown),
        Err(err) => {
            rollback(conn).await?;
            Err(err)
        }
    }
}

async fn apply_one(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
    set: &MigrationSet,
    migration: &Migration,
) -> Result<bool, MigrationError> {
    let rebuild = migration.rebuild && conn.backend() == Backend::Sqlite;
    if rebuild {
        execute(conn, "PRAGMA foreign_keys = OFF").await?;
    }
    if let Err(err) = begin(conn, renderer).await {
        restore_foreign_keys(conn).await?;
        return Err(err);
    }
    let result = async {
        validate_schema_owners(conn, renderer).await?;
        let original_history = read_history(conn, renderer).await?;
        let states = reconcile(set, &original_history)?;
        let state = &states[migration.number as usize - 1];
        if state.state == "applied" {
            return Ok(false);
        }
        let sql = renderer.migration(conn.backend(), &migration.sql)?;
        execute(conn, &sql).await?;
        let unchanged_history = match read_history(conn, renderer).await {
            Ok(after) => original_history == after,
            Err(_) => false,
        };
        if !unchanged_history {
            return Err(MigrationError::MetadataMutation {
                number: migration.number,
            });
        }
        if rebuild
            && let DatabaseConnection::Sqlite(c) = &mut *conn
            && sqlx::query("PRAGMA foreign_key_check")
                .fetch_optional(c)
                .await?
                .is_some()
        {
            return Err(MigrationError::ForeignKeyViolation {
                number: migration.number,
            });
        }
        write_state(
            conn,
            renderer,
            migration,
            "applied",
            state.state == "failed",
        )
        .await?;
        reconcile(set, &read_history(conn, renderer).await?)?;
        Ok(true)
    }
    .await;
    match result {
        Ok(applied) => {
            if execute(conn, "COMMIT").await.is_err() {
                // COMMIT 错误不能证明回滚；尝试恢复连接后丢弃，绝不记录 failed。
                let _ = execute(conn, "ROLLBACK").await;
                let _ = restore_foreign_keys(conn).await;
                return Err(MigrationError::CommitUnknown);
            }
            restore_foreign_keys(conn).await?;
            Ok(applied)
        }
        Err(err) => {
            rollback(conn).await?;
            if matches!(
                err,
                MigrationError::Database
                    | MigrationError::ForeignKeyViolation { .. }
                    | MigrationError::MetadataMutation { .. }
            ) {
                record_failure(conn, renderer, set, migration)
                    .await
                    .map_err(|_| MigrationError::RecoveryFailed)?;
                if matches!(
                    err,
                    MigrationError::ForeignKeyViolation { .. }
                        | MigrationError::MetadataMutation { .. }
                ) {
                    return Err(err);
                }
                return Err(MigrationError::ExecutionFailed {
                    number: migration.number,
                });
            }
            Err(err)
        }
    }
}

/// 发现、完整历史预校验、专用连接初始化和逐迁移事务执行。
pub async fn migrate(
    cfg: &DatabaseConfig,
    root: &Path,
) -> Result<MigrationSummary, MigrationError> {
    let set = MigrationSet::discover(root, cfg.backend)?;
    let renderer = SchemaRenderer::new(&cfg.schema_prefix)?;
    for migration in &set.migrations {
        renderer.migration(cfg.backend, &migration.sql)?;
    }
    let mut conn = DatabaseConnection::connect(cfg, Purpose::Migrator, true).await?;
    let result = async {
        reconcile(&set, &read_history(&mut conn, &renderer).await?)?;
        bootstrap(&mut conn, &renderer).await?;
        let mut summary = MigrationSummary::default();
        for migration in &set.migrations {
            if apply_one(&mut conn, &renderer, &set, migration).await? {
                summary.applied.push(migration.number);
            } else {
                summary.skipped.push(migration.number);
            }
        }
        Ok::<MigrationSummary, MigrationError>(summary)
    }
    .await;
    let closed = conn.close().await;
    let summary = result?;
    closed?;
    Ok(summary)
}

/// 只读状态；SQLite 不存在时不建文件或目录，不调用 bootstrap。
pub async fn status(
    cfg: &DatabaseConfig,
    root: &Path,
) -> Result<Vec<MigrationStatus>, MigrationError> {
    let set = MigrationSet::discover(root, cfg.backend)?;
    let renderer = SchemaRenderer::new(&cfg.schema_prefix)?;
    if cfg.backend == Backend::Sqlite && cfg.sqlite_path.trim().is_empty() {
        return Err(MigrationError::InvalidConfiguration);
    }
    for migration in &set.migrations {
        renderer.migration(cfg.backend, &migration.sql)?;
    }
    if cfg.backend == Backend::Sqlite && !Path::new(&cfg.sqlite_path).exists() {
        return reconcile(&set, &[]);
    }
    let mut conn = if cfg.backend == Backend::Sqlite {
        // status 不得经通用 WAL 写连接修改既有 DELETE 模式文件的 header。
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&cfg.sqlite_path)
            .create_if_missing(false)
            .read_only(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(5000))
            .disable_statement_logging();
        DatabaseConnection::Sqlite(sqlx::SqliteConnection::connect_with(&options).await?)
    } else {
        DatabaseConnection::connect(cfg, Purpose::Migrator, false).await?
    };
    let result = match read_history(&mut conn, &renderer).await {
        Ok(history) => reconcile(&set, &history),
        Err(err) => Err(err),
    };
    let closed = conn.close().await;
    let states = result?;
    closed?;
    Ok(states)
}

#[cfg(test)]
mod tests;
