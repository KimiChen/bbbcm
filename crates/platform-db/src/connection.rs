//! 专用数据库连接。迁移与种子不借业务池；事务引擎仍属于 A04。

use platform::config::DatabaseConfig;
use platform::db::Backend;
use platform::identifiers::SchemaRenderer;
use sqlx::{ConnectOptions, Connection};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("数据库配置非法或后端尚未接入")]
    InvalidConfiguration,
    #[error("数据库操作失败；检查连接、角色、对象权限及本机数据库状态（底层信息已脱敏）")]
    Operation,
    #[error("PostgreSQL 需要 UTF8 且 lc_ctype 不能为 C/POSIX")]
    UnsupportedLocale,
    #[error("数据库角色不符合最小权限策略；请按数据库初始化手册修复")]
    UnsafeRole,
}
impl From<sqlx::Error> for DatabaseError {
    fn from(_: sqlx::Error) -> Self {
        Self::Operation
    }
}
impl From<platform::identifiers::SchemaError> for DatabaseError {
    fn from(_: platform::identifiers::SchemaError) -> Self {
        Self::InvalidConfiguration
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Purpose {
    App,
    Worker,
    Migrator,
}
impl Purpose {
    pub fn role(self) -> &'static str {
        match self {
            Self::App => "app_rw",
            Self::Worker => "worker_rw",
            Self::Migrator => "migrator",
        }
    }
}

pub enum DatabaseConnection {
    Postgres(sqlx::PgConnection),
    Sqlite(sqlx::SqliteConnection),
}

pub(crate) fn sqlite_options(
    cfg: &DatabaseConfig,
    create: bool,
) -> Result<sqlx::sqlite::SqliteConnectOptions, DatabaseError> {
    if cfg.sqlite_path.trim().is_empty()
        || cfg.sqlite_path.starts_with("file:")
        || cfg.sqlite_path == ":memory:"
    {
        return Err(DatabaseError::InvalidConfiguration);
    }
    if create
        && let Some(parent) = std::path::Path::new(&cfg.sqlite_path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|_| DatabaseError::Operation)?;
    }
    Ok(sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&cfg.sqlite_path)
        .create_if_missing(create)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_millis(5000))
        .foreign_keys(true)
        .disable_statement_logging())
}

pub(crate) fn postgres_options(
    cfg: &DatabaseConfig,
) -> Result<sqlx::postgres::PgConnectOptions, DatabaseError> {
    use platform::config::{
        decode_url_component, reject_postgres_driver_environment, validate_postgres_url,
    };
    use sqlx::postgres::{PgConnectOptions, PgSslMode};
    // new/default/from_str 会读 .pgpass；new_without_pgpass 仍读 PG*，必须先拒绝。
    reject_postgres_driver_environment().map_err(|_| DatabaseError::InvalidConfiguration)?;
    let text = cfg
        .postgres_url
        .as_deref()
        .ok_or(DatabaseError::InvalidConfiguration)?;
    validate_postgres_url(text).map_err(|_| DatabaseError::InvalidConfiguration)?;
    let url = url::Url::parse(text).map_err(|_| DatabaseError::InvalidConfiguration)?;
    let host = match url.host().ok_or(DatabaseError::InvalidConfiguration)? {
        url::Host::Domain(host) => host.to_owned(),
        url::Host::Ipv4(host) => host.to_string(),
        url::Host::Ipv6(host) => host.to_string(),
    };
    let username =
        decode_url_component(url.username()).ok_or(DatabaseError::InvalidConfiguration)?;
    let database = decode_url_component(url.path().trim_start_matches('/'))
        .ok_or(DatabaseError::InvalidConfiguration)?;
    let mut options = PgConnectOptions::new_without_pgpass()
        .host(&host)
        .port(url.port().unwrap_or(5432))
        .username(&username)
        .database(&database)
        .ssl_mode(PgSslMode::Prefer)
        .statement_cache_capacity(100)
        .application_name("bbbcm")
        .disable_statement_logging();
    if let Some(password) = url.password() {
        let password = decode_url_component(password).ok_or(DatabaseError::InvalidConfiguration)?;
        options = options.password(&password);
    }
    for (key, value) in url.query_pairs() {
        options = match key.as_ref() {
            "sslmode" | "ssl-mode" => options.ssl_mode(
                value
                    .parse()
                    .map_err(|_| DatabaseError::InvalidConfiguration)?,
            ),
            "sslrootcert" | "ssl-root-cert" | "ssl-ca" => options.ssl_root_cert(value.as_ref()),
            "sslcert" | "ssl-cert" => options.ssl_client_cert(value.as_ref()),
            "sslkey" | "ssl-key" => options.ssl_client_key(value.as_ref()),
            "application_name" => options.application_name(&value),
            "statement-cache-capacity" => options.statement_cache_capacity(
                value
                    .parse()
                    .map_err(|_| DatabaseError::InvalidConfiguration)?,
            ),
            _ => return Err(DatabaseError::InvalidConfiguration),
        };
    }
    Ok(options)
}

/// 以组角色执行，杜绝迁移对象落到登录角色名下，或运行时误用 migrator。
/// PostgreSQL 16 membership 有独立的 SET/INHERIT 选项，不能只验证 NOINHERIT。
pub async fn prepare_postgres(
    conn: &mut sqlx::PgConnection,
    renderer: &SchemaRenderer,
    purpose: Purpose,
) -> Result<(), DatabaseError> {
    let locale: (String, String) = sqlx::query_as("SELECT pg_encoding_to_char(encoding), datctype FROM pg_database WHERE datname = current_database()")
        .fetch_one(&mut *conn).await?;
    if locale.0 != "UTF8" || matches!(locale.1.as_str(), "C" | "POSIX") {
        return Err(DatabaseError::UnsupportedLocale);
    }
    for name in platform::identifiers::DATABASE_ROLES {
        let role = renderer.role(name)?;
        let safe: Option<bool> = sqlx::query_scalar("SELECT NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolinherit OR rolreplication OR rolbypassrls OR rolcanlogin) FROM pg_roles WHERE rolname = $1")
            .bind(&role).fetch_optional(&mut *conn).await?;
        if safe != Some(true) {
            return Err(DatabaseError::UnsafeRole);
        }
        let memberships: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.member WHERE r.rolname = $1")
            .bind(&role).fetch_one(&mut *conn).await?;
        if memberships != 0 {
            return Err(DatabaseError::UnsafeRole);
        }
    }
    // IF NOT EXISTS 不修正所有权，启动时不能把同名 schema 当作可信对象。
    // 迁移器还会在 advisory lock 内复查全部 schema，避免迁移间隙的所有权漂移。
    for schema in platform::identifiers::LOGICAL_SCHEMAS {
        let physical = renderer.schema(schema)?;
        let expected_owner: Option<bool> = sqlx::query_scalar("SELECT nspowner = (SELECT oid FROM pg_roles WHERE rolname = $2) FROM pg_namespace WHERE nspname = $1")
            .bind(&physical).bind(renderer.role("migrator")?).fetch_optional(&mut *conn).await?;
        if expected_owner == Some(false) {
            return Err(DatabaseError::UnsafeRole);
        }
    }
    if !matches!(purpose, Purpose::Migrator) {
        // SET ROLE 不是权限沙箱：登录角色可 RESET ROLE，因此必须同时限制登录者。
        let safe_login: bool = sqlx::query_scalar("SELECT rolcanlogin AND NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolinherit OR rolreplication OR rolbypassrls) FROM pg_roles WHERE rolname = session_user")
            .fetch_one(&mut *conn).await?;
        let memberships: Vec<(String, bool, bool, bool)> = sqlx::query_as("SELECT r.rolname, m.admin_option, m.inherit_option, m.set_option FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.roleid WHERE m.member = (SELECT oid FROM pg_roles WHERE rolname = session_user)")
            .fetch_all(&mut *conn).await?;
        if !safe_login || memberships != vec![(renderer.role(purpose.role())?, false, false, true)]
        {
            return Err(DatabaseError::UnsafeRole);
        }
        let dangerous_database: bool = sqlx::query_scalar("SELECT datdba = (SELECT oid FROM pg_roles WHERE rolname = session_user) OR has_database_privilege(session_user, oid, 'CREATE,TEMPORARY') FROM pg_database WHERE datname = current_database()")
            .fetch_one(&mut *conn).await?;
        let dangerous_schema: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspowner = (SELECT oid FROM pg_roles WHERE rolname = session_user) OR has_schema_privilege(session_user, oid, 'CREATE'))")
            .fetch_one(&mut *conn).await?;
        if dangerous_database || dangerous_schema {
            return Err(DatabaseError::UnsafeRole);
        }
        for schema in platform::identifiers::LOGICAL_SCHEMAS {
            let physical = renderer.schema(schema)?;
            // NOINHERIT 登录者应只获 CONNECT + SET ROLE，没有绕过组 policy 的对象授权。
            let direct_objects: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND CASE WHEN c.relkind = 'S' THEN has_sequence_privilege(session_user, c.oid, 'USAGE,SELECT,UPDATE') WHEN c.relkind IN ('r', 'p', 'v', 'm', 'f') THEN has_table_privilege(session_user, c.oid, 'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') OR has_any_column_privilege(session_user, c.oid, 'SELECT,INSERT,UPDATE,REFERENCES') ELSE false END)")
                .bind(&physical).fetch_one(&mut *conn).await?;
            if direct_objects {
                return Err(DatabaseError::UnsafeRole);
            }
        }
        // runtime 组自身也不得通过额外的数据库/schema ACL 获得 CREATE。
        let group_create: bool = sqlx::query_scalar("SELECT has_database_privilege($1, current_database(), 'CREATE,TEMPORARY') OR EXISTS (SELECT 1 FROM pg_namespace WHERE has_schema_privilege($1, oid, 'CREATE'))")
            .bind(renderer.role(purpose.role())?).fetch_one(&mut *conn).await?;
        if group_create {
            return Err(DatabaseError::UnsafeRole);
        }
    }
    sqlx::query(&format!("SET ROLE {}", renderer.role(purpose.role())?))
        .execute(&mut *conn)
        .await?;
    sqlx::query("SET search_path = pg_catalog")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

impl DatabaseConnection {
    pub async fn connect(
        cfg: &DatabaseConfig,
        purpose: Purpose,
        create: bool,
    ) -> Result<Self, DatabaseError> {
        let renderer = SchemaRenderer::new(&cfg.schema_prefix)?;
        match cfg.backend {
            Backend::Mysql8 => Err(DatabaseError::InvalidConfiguration),
            Backend::Sqlite => Ok(Self::Sqlite(
                sqlx::SqliteConnection::connect_with(&sqlite_options(cfg, create)?).await?,
            )),
            Backend::Postgres => {
                let mut conn = sqlx::PgConnection::connect_with(&postgres_options(cfg)?).await?;
                prepare_postgres(&mut conn, &renderer, purpose).await?;
                Ok(Self::Postgres(conn))
            }
        }
    }
    pub fn backend(&self) -> Backend {
        match self {
            Self::Postgres(_) => Backend::Postgres,
            Self::Sqlite(_) => Backend::Sqlite,
        }
    }
    pub async fn close(self) -> Result<(), DatabaseError> {
        match self {
            Self::Postgres(c) => c.close().await?,
            Self::Sqlite(c) => c.close().await?,
        }
        Ok(())
    }
}

/// 当前服务骨架的完整模式启动探测；SQLite 夹具继续不依赖业务池。
/// 搜索 Flag 始终关闭，缺扩展只报告明确受限能力，不切换到其它检索路径。
pub async fn check_startup(cfg: &DatabaseConfig, purpose: Purpose) -> Result<(), DatabaseError> {
    if cfg.backend != Backend::Postgres {
        return Ok(());
    }
    let mut conn = DatabaseConnection::connect(cfg, purpose, false).await?;
    let DatabaseConnection::Postgres(c) = &mut conn else {
        unreachable!()
    };
    let installed: Vec<String> = sqlx::query_scalar("SELECT extname FROM pg_extension")
        .fetch_all(&mut *c)
        .await?;
    if !installed.iter().any(|s| s == "pg_jieba") || !installed.iter().any(|s| s == "pg_trgm") {
        tracing::warn!("缺少 pg_jieba/pg_trgm；search Flag 保持关闭，不使用未评审的检索路径");
    }
    conn.close().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform::config::Config;
    use sqlx::postgres::PgSslMode;

    fn options(value: &str) -> Result<sqlx::postgres::PgConnectOptions, DatabaseError> {
        let mut cfg = Config::default().database;
        cfg.backend = Backend::Postgres;
        cfg.postgres_url = Some(value.to_string().into());
        postgres_options(&cfg)
    }

    #[test]
    fn postgres_url_constructs_explicit_options_without_pgpass() {
        let options =
            options("postgres://synthetic%2Duser:synthetic-secret@127.0.0.1/test%2Ddatabase")
                .unwrap();
        assert_eq!(options.get_host(), "127.0.0.1");
        assert_eq!(options.get_port(), 5432);
        assert_eq!(options.get_username(), "synthetic-user");
        assert_eq!(options.get_database(), Some("test-database"));
        assert_eq!(options.get_application_name(), Some("bbbcm"));
        assert!(options.get_options().is_none());
        assert!(options.get_socket().is_none());
        assert!(matches!(options.get_ssl_mode(), PgSslMode::Prefer));
    }

    #[test]
    fn postgres_ipv6_and_whitelisted_parameters_are_applied() {
        let options = options("postgresql://synthetic@[::1]:5440/example?sslmode=verify-full&application_name=synthetic-worker&statement-cache-capacity=0&sslrootcert=synthetic-ca.pem&sslcert=synthetic-cert.pem&sslkey=synthetic-key.pem").unwrap();
        assert_eq!(options.get_host(), "::1");
        assert_eq!(options.get_port(), 5440);
        assert_eq!(options.get_application_name(), Some("synthetic-worker"));
        assert!(matches!(options.get_ssl_mode(), PgSslMode::VerifyFull));
    }

    #[test]
    fn raw_driver_entry_rejects_unknown_and_override_parameters_without_secrets() {
        for url in [
            "postgres://synthetic@localhost/example?unknown=PRIVATE_SENTINEL",
            "postgres://synthetic@localhost/example?password=PRIVATE_SENTINEL",
            "postgres://synthetic@localhost/example?options=PRIVATE_SENTINEL",
            "postgres://synthetic@localhost/example?sslmode=PRIVATE_SENTINEL",
        ] {
            // 不对 PgConnectOptions 使用 expect_err：其 Debug 由驱动定义且可能包含凭证。
            let Err(error) = options(url) else {
                panic!("应拒绝非法驱动参数")
            };
            assert!(!format!("{error:?} {error}").contains("PRIVATE_SENTINEL"));
        }
    }

    #[test]
    fn sqlite_driver_entry_refuses_nonpersistent_uri_paths() {
        for path in [
            "",
            ":memory:",
            "file::memory:",
            "file:synthetic?mode=memory",
        ] {
            let mut cfg = Config::default().database;
            cfg.sqlite_path = path.into();
            assert!(sqlite_options(&cfg, false).is_err());
        }
    }
}
