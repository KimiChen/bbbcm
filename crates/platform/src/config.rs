//! 安全默认 < 配置文件 < 本机覆盖 < 环境变量 < Secret 注入。
//! 每层拒绝未知键和非法类型；错误不携带配置原文、输入值或 Secret。

use crate::db::Backend;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("读取{layer}失败（{kind:?}）")]
    Io {
        layer: &'static str,
        kind: std::io::ErrorKind,
    },
    #[error("{layer}包含非法 TOML、未知键或非法类型；为保护 Secret 不显示原文")]
    Parse { layer: &'static str },
    #[error("配置非法: {0}")]
    Invalid(&'static str),
}

/// 仅可由运行环境/部署注入；Config 的 Debug 也不会输出内容。
#[derive(Clone)]
pub struct SecretString(String);
impl SecretString {
    pub fn new(value: String) -> Self {
        Self(value)
    }
}
impl std::ops::Deref for SecretString {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// 独立部署注入层；测试不需要修改全局进程环境。
#[derive(Debug, Default)]
pub struct SecretOverrides {
    pub postgres_url: Option<SecretString>,
    pub redis_url: Option<SecretString>,
}
impl SecretOverrides {
    fn from_variables(variables: &BTreeMap<String, String>) -> Self {
        Self {
            postgres_url: variables
                .get("BBBCM_DATABASE_POSTGRES_URL")
                .cloned()
                .map(Into::into),
            redis_url: variables.get("BBBCM_REDIS_URL").cloned().map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Environment {
    #[default]
    Development,
    Test,
    Staging,
    Production,
}
impl Environment {
    pub fn is_production_like(self) -> bool {
        matches!(self, Environment::Staging | Environment::Production)
    }
}
impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Environment::Development => "development",
            Environment::Test => "test",
            Environment::Staging => "staging",
            Environment::Production => "production",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKind {
    Web,
    Worker,
    Cli,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub features: FeaturesConfig,
    pub redis: RedisConfig,
    pub worker: WorkerConfig,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub environment: Environment,
    pub bind: String,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            environment: Environment::default(),
            bind: "127.0.0.1:8080".into(),
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    pub backend: Backend,
    pub schema_prefix: String,
    pub sqlite_path: String,
    pub max_connections: u32,
    /// skip 配合 deny_unknown_fields：文件中出现此键也会拒绝。
    #[serde(skip)]
    pub postgres_url: Option<SecretString>,
}
impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            backend: Backend::default(),
            schema_prefix: "bcm_".into(),
            sqlite_path: "data/bbbcm.db".into(),
            max_connections: 1,
            postgres_url: None,
        }
    }
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeaturesConfig {
    /// false 时无 Redis 限流；运行时实现仍归 A05。
    pub redis: bool,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RedisConfig {
    /// URL 可能含凭证，只能经 BBBCM_REDIS_URL 注入。
    #[serde(skip)]
    pub url: Option<SecretString>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkerConfig {
    pub embedded: bool,
}
impl Default for WorkerConfig {
    fn default() -> Self {
        Self { embedded: true }
    }
}

impl Config {
    /// 只解析单文件及安全默认，不隐式读取环境；启动使用 from_environment。
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        parse_document(&read_file(path, "配置文件")?, "配置文件")?
            .try_into()
            .map_err(|_| ConfigError::Parse {
                layer: "配置文件"
            })
    }

    /// 显式 BBBCM_CONFIG 或 config/default.toml 必须存在；config/local.toml 可选。
    /// 相对路径相对于进程工作目录，冒烟测试使用隔离工作目录。
    pub fn from_environment() -> Result<Self, ConfigError> {
        reject_postgres_driver_environment()?;
        let mut variables = BTreeMap::new();
        for (key, value) in std::env::vars_os() {
            let Some(key) = key.to_str() else { continue };
            if key.starts_with("BBBCM_") {
                let value = value
                    .to_str()
                    .ok_or(ConfigError::Invalid("BBBCM 环境变量必须为 UTF-8 文本"))?;
                variables.insert(key.to_owned(), value.to_owned());
            }
        }
        let path = variables
            .get("BBBCM_CONFIG")
            .map(String::as_str)
            .unwrap_or("config/default.toml");
        Self::load_layers(
            Path::new(path),
            Some(Path::new("config/local.toml")),
            &variables,
            SecretOverrides::from_variables(&variables),
        )
    }

    /// 确定性接口：明确提供每层输入，不触碰全局环境变量。
    pub fn load_layers(
        path: &Path,
        local_path: Option<&Path>,
        variables: &BTreeMap<String, String>,
        secrets: SecretOverrides,
    ) -> Result<Self, ConfigError> {
        let mut merged = parse_document(&read_file(path, "配置文件")?, "配置文件")?;
        if let Some(local_path) = local_path {
            match std::fs::read_to_string(local_path) {
                Ok(text) => merge(&mut merged, parse_document(&text, "本机覆盖文件")?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(ConfigError::Io {
                        layer: "本机覆盖文件",
                        kind: error.kind(),
                    });
                }
            }
        }
        merge(&mut merged, environment_document(variables)?);
        let mut config: Self = merged.try_into().map_err(|_| ConfigError::Parse {
            layer: "合并后配置",
        })?;
        config.database.postgres_url = secrets.postgres_url;
        config.redis.url = secrets.redis_url;
        config.validate()?;
        Ok(config)
    }

    pub fn validate_for(&self, process: ProcessKind) -> Result<(), ConfigError> {
        self.validate()?;
        if self.database.backend == Backend::Sqlite
            && self.worker.embedded
            && process == ProcessKind::Worker
        {
            return Err(ConfigError::Invalid(
                "SQLite 的 worker.embedded=true 与独立 bbbcm-worker 互斥；请关闭内嵌 Worker",
            ));
        }
        Ok(())
    }

    /// 校验当前接入组件，不提前开发认证和外部 provider。
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !self.database.backend.is_implemented() {
            return Err(ConfigError::Invalid(
                "database.backend=mysql8 是规范预留值，该后端尚未接入（DEC-022）",
            ));
        }
        crate::identifiers::SchemaRenderer::new(&self.database.schema_prefix)
            .map_err(|_| ConfigError::Invalid("database.schema_prefix 不符合受信前缀规则"))?;
        if self.server.environment == Environment::Production
            && self.database.schema_prefix != "bcm_"
        {
            return Err(ConfigError::Invalid(
                "production 的 database.schema_prefix 固定为 bcm_",
            ));
        }
        if self.database.max_connections == 0 {
            return Err(ConfigError::Invalid(
                "database.max_connections 必须显式配置为正整数",
            ));
        }
        if self.database.backend == Backend::Sqlite {
            if self.database.max_connections != 1 {
                return Err(ConfigError::Invalid(
                    "SQLite 是单写者，database.max_connections 必须为 1",
                ));
            }
            let path = &self.database.sqlite_path;
            if path.trim().is_empty()
                || path.contains(['\0', '\r', '\n'])
                || path.contains("://")
                || path.starts_with("file:")
                || path == ":memory:"
                || unresolved(path)
                || Path::new(path).file_name().is_none()
                || Path::new(path).is_dir()
            {
                return Err(ConfigError::Invalid(
                    "database.sqlite_path 必须是非空、可持久化的数据库文件路径",
                ));
            }
        }
        if self.database.backend == Backend::Postgres && self.database.postgres_url.is_none() {
            return Err(ConfigError::Invalid(
                "backend=postgres 时必须经 BBBCM_DATABASE_POSTGRES_URL 注入连接串",
            ));
        }
        if let Some(value) = self.database.postgres_url.as_deref() {
            validate_url(value, UrlKind::Postgres, self.server.environment)?;
        }
        if self.features.redis && self.redis.url.is_none() {
            return Err(ConfigError::Invalid(
                "features.redis=true 时必须经 BBBCM_REDIS_URL 注入连接串",
            ));
        }
        if let Some(value) = self.redis.url.as_deref() {
            validate_url(value, UrlKind::Redis, self.server.environment)?;
        }
        let bind = self
            .server
            .bind
            .parse::<std::net::SocketAddr>()
            .map_err(|_| ConfigError::Invalid("server.bind 必须是有效 IP:端口"))?;
        if bind.port() == 0 {
            return Err(ConfigError::Invalid("server.bind 不允许使用端口 0"));
        }
        if self.database.backend == Backend::Sqlite && !bind.ip().is_loopback() {
            return Err(ConfigError::Invalid("SQLite 体验模式只能监听本机回环地址"));
        }
        if self.server.environment.is_production_like() {
            if !self.features.redis {
                return Err(ConfigError::Invalid(
                    "staging/production 必须启用 Redis，关闭即无速率保护（DEC-024）",
                ));
            }
            if self.database.backend != Backend::Postgres || self.worker.embedded {
                return Err(ConfigError::Invalid(
                    "staging/production 必须使用 PostgreSQL 和独立 Worker，禁止体验模式",
                ));
            }
            if !bind.ip().is_loopback() {
                return Err(ConfigError::Invalid(
                    "staging/production 只允许监听本机回环地址，由反向代理接入",
                ));
            }
        }
        Ok(())
    }
}

fn read_file(path: &Path, layer: &'static str) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|error| ConfigError::Io {
        layer,
        kind: error.kind(),
    })
}
/// 逐层校验，后续覆盖不能掩盖此前的未知键或非法类型。
fn parse_document(text: &str, layer: &'static str) -> Result<toml::Value, ConfigError> {
    let value: toml::Value = toml::from_str(text).map_err(|_| ConfigError::Parse { layer })?;
    value
        .clone()
        .try_into::<Config>()
        .map_err(|_| ConfigError::Parse { layer })?;
    Ok(value)
}
fn merge(target: &mut toml::Value, overlay: toml::Value) {
    match (target, overlay) {
        (toml::Value::Table(target), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                match target.get_mut(&key) {
                    Some(existing) => merge(existing, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (target, overlay) => *target = overlay,
    }
}
fn environment_document(variables: &BTreeMap<String, String>) -> Result<toml::Value, ConfigError> {
    let mut table = toml::Table::new();
    for (key, value) in variables {
        let (section, name, typed) = match key.as_str() {
            "BBBCM_SERVER_ENVIRONMENT" => {
                ("server", "environment", toml::Value::String(value.clone()))
            }
            "BBBCM_SERVER_BIND" => ("server", "bind", toml::Value::String(value.clone())),
            "BBBCM_DATABASE_BACKEND" => ("database", "backend", toml::Value::String(value.clone())),
            "BBBCM_DATABASE_SCHEMA_PREFIX" => (
                "database",
                "schema_prefix",
                toml::Value::String(value.clone()),
            ),
            "BBBCM_DATABASE_SQLITE_PATH" => (
                "database",
                "sqlite_path",
                toml::Value::String(value.clone()),
            ),
            "BBBCM_DATABASE_MAX_CONNECTIONS" => (
                "database",
                "max_connections",
                toml::Value::Integer(value.parse::<u32>().map_err(|_| {
                    ConfigError::Invalid("BBBCM_DATABASE_MAX_CONNECTIONS 必须为无符号整数")
                })? as i64),
            ),
            "BBBCM_FEATURES_REDIS" => ("features", "redis", parse_boolean(value)?),
            "BBBCM_WORKER_EMBEDDED" => ("worker", "embedded", parse_boolean(value)?),
            // A01 资产目录由资产组件处理；Secret 由最后注入层处理。
            "BBBCM_CONFIG"
            | "BBBCM_ASSET_DIST"
            | "BBBCM_DATABASE_POSTGRES_URL"
            | "BBBCM_REDIS_URL" => continue,
            _ if key.starts_with("BBBCM_") => {
                return Err(ConfigError::Invalid(
                    "未知 BBBCM 环境变量；请检查配置变量拼写",
                ));
            }
            _ if key.starts_with("PG") => return Err(driver_environment_error()),
            _ => continue,
        };
        table
            .entry(section)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .expect("环境映射只创建 TOML table")
            .insert(name.into(), typed);
    }
    let value = toml::Value::Table(table);
    value
        .clone()
        .try_into::<Config>()
        .map_err(|_| ConfigError::Parse {
            layer: "环境变量"
        })?;
    Ok(value)
}
fn parse_boolean(value: &str) -> Result<toml::Value, ConfigError> {
    value
        .parse::<bool>()
        .map(toml::Value::Boolean)
        .map_err(|_| ConfigError::Invalid("布尔配置环境变量只接受 true 或 false"))
}
fn unresolved(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains(['<', '>'])
        || value.contains("${")
        || value.contains("{{")
        || [
            "change_me",
            "change-me",
            "changeme",
            "replace_me",
            "replace-me",
            "placeholder",
        ]
        .iter()
        .any(|marker| value.contains(marker))
}
enum UrlKind {
    Postgres,
    Redis,
}

/// sqlx/libpq 风格的 PG* 配置属于隐式输入，不得绕过 BBBCM Secret 注入层。
/// 只检查变量名；不读取 .pgpass、PGPASSFILE 或其它凭证文件。
pub fn reject_postgres_driver_environment() -> Result<(), ConfigError> {
    if std::env::vars_os().any(|(key, _)| key.to_str().is_some_and(|key| key.starts_with("PG"))) {
        return Err(driver_environment_error());
    }
    Ok(())
}

fn driver_environment_error() -> ConfigError {
    ConfigError::Invalid(
        "禁止隐式 PG* 驱动环境变量；连接参数仅使用 BBBCM_DATABASE_POSTGRES_URL，不读取 .pgpass",
    )
}

/// 数据库层构造驱动选项前再次校验，防止绕过 Config 的库调用泄露 URL 查询值。
pub fn validate_postgres_url(value: &str) -> Result<(), ConfigError> {
    validate_url(value, UrlKind::Postgres, Environment::Development)
}

fn validate_url(value: &str, kind: UrlKind, environment: Environment) -> Result<(), ConfigError> {
    let invalid = || {
        ConfigError::Invalid(match kind {
            UrlKind::Postgres => "BBBCM_DATABASE_POSTGRES_URL 格式或连接参数非法（内容已隐藏）",
            UrlKind::Redis => "BBBCM_REDIS_URL 格式或连接参数非法（内容已隐藏）",
        })
    };
    if value.chars().any(char::is_whitespace) || value.contains('\0') || unresolved(value) {
        return Err(invalid());
    }
    let decoded = decode_url_component(value).ok_or_else(invalid)?;
    if decoded.chars().any(char::is_control) || unresolved(&decoded) {
        return Err(invalid());
    }
    let url = url::Url::parse(value).map_err(|_| invalid())?;
    if url.host_str().is_none_or(str::is_empty) || url.fragment().is_some() || url.port() == Some(0)
    {
        return Err(invalid());
    }
    match kind {
        UrlKind::Postgres => {
            if !matches!(url.scheme(), "postgres" | "postgresql")
                || url.username().is_empty()
                || url.path().trim_matches('/').is_empty()
                || url.path()[1..].contains('/')
            {
                return Err(invalid());
            }
            // sqlx 对未知查询参数会记录原始 key/value，必须在交给驱动前拒绝。
            // 不开放 URL 查询中的 host/user/password/db/options 覆盖，确保检查的
            // 连接对象与最终连接一致；搜索路径和角色由数据库层配置。
            let mut tls_required = false;
            for (key, value) in url.query_pairs() {
                if matches!(key.as_ref(), "sslmode" | "ssl-mode") {
                    tls_required =
                        matches!(value.as_ref(), "require" | "verify-ca" | "verify-full");
                    if environment.is_production_like() && !tls_required {
                        return Err(ConfigError::Invalid(
                            "staging/production 的 PostgreSQL 必须强制 TLS：sslmode=require/verify-ca/verify-full",
                        ));
                    }
                }
                let valid = match key.as_ref() {
                    "sslmode" | "ssl-mode" => matches!(
                        value.as_ref(),
                        "disable" | "allow" | "prefer" | "require" | "verify-ca" | "verify-full"
                    ),
                    "sslrootcert" | "ssl-root-cert" | "ssl-ca" | "sslcert" | "ssl-cert"
                    | "sslkey" | "ssl-key" | "application_name" => !value.is_empty(),
                    "statement-cache-capacity" => value.parse::<usize>().is_ok(),
                    _ => false,
                };
                if !valid {
                    return Err(invalid());
                }
            }
            if environment.is_production_like() && !tls_required {
                return Err(ConfigError::Invalid(
                    "staging/production 的 PostgreSQL 必须显式配置 sslmode=require/verify-ca/verify-full",
                ));
            }
        }
        UrlKind::Redis => {
            let database = url.path().strip_prefix('/').unwrap_or(url.path());
            if !matches!(url.scheme(), "redis" | "rediss")
                || (!database.is_empty() && database.parse::<u32>().is_err())
                || url.query().is_some()
            {
                return Err(invalid());
            }
        }
    }
    if environment.is_production_like()
        && url
            .password()
            .and_then(decode_url_component)
            .is_some_and(|password| {
                matches!(
                    password.to_ascii_lowercase().as_str(),
                    "password" | "default" | "secret" | "postgres" | "redis"
                )
            })
    {
        return Err(ConfigError::Invalid(
            "staging/production 拒绝已知演示默认密钥（内容已隐藏）",
        ));
    }
    Ok(())
}

/// 拒绝畸形编码，且使百分号编码不能绕过占位符和演示密钥检查。
pub fn decode_url_component(value: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests;
