//! 配置加载：`config/default.toml` 提供安全默认，环境变量注入密钥类配置。
//!
//! 所有结构使用 `deny_unknown_fields`：拼错的键必须让启动失败，而不是静默取默认值
//! ——一个拼错的 `featurs.redis` 会让限流在生产静默消失（`02` 第 4 节）。
//! 本地覆盖文件与更完整的环境变量映射随 `WP-MVP-A02` 接入。

use crate::db::Backend;
use serde::Deserialize;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("读取配置失败 {path:?}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("解析配置失败 {path:?}: {source}")]
    Parse {
        path: std::path::PathBuf,
        source: toml::de::Error,
    },
    #[error("配置非法: {0}")]
    Invalid(String),
}

/// 运行环境。只允许四个值，`02` 第 4 节要求非法环境拒绝启动。
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
    /// staging 与 production 拒绝演示开关、弱默认值与仅供开发的实现。
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub features: FeaturesConfig,
    pub worker: WorkerConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub environment: Environment,
    /// HTTP 监听地址；生产只监听本机，由反向代理转发。
    pub bind: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            environment: Environment::default(),
            bind: "127.0.0.1:8080".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    pub backend: Backend,
    /// 物理对象名前缀；默认及生产固定 `bcm_`（`01` 第 7.2 节）。
    pub schema_prefix: String,
    /// SQLite 数据库文件路径（backend = sqlite 时使用）。
    pub sqlite_path: String,
    /// 连接池上限。SQLite 是单写者，超过 1 的写连接只会制造 `SQLITE_BUSY`。
    pub max_connections: u32,
    /// PostgreSQL 连接串：只从环境变量注入，禁止写入配置文件。
    #[serde(skip)]
    pub postgres_url: Option<String>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            backend: Backend::default(),
            schema_prefix: "bcm_".to_string(),
            sqlite_path: "data/bbbcm.db".to_string(),
            max_connections: 1,
            postgres_url: None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeaturesConfig {
    /// false 时缓存/唤醒/租约使用进程内存实现，且**不提供限流**（`DEC-024`）。
    pub redis: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkerConfig {
    /// true 时任务循环内嵌进 bbbcm-web 进程（体验模式默认）。
    pub embedded: bool,
}

impl Config {
    /// 读取指定配置文件；文件不存在时报错而不是静默回退，避免生产误用默认值。
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// 应用环境变量覆盖；目前仅 PostgreSQL 连接串。
    pub fn apply_env(&mut self) {
        if let Ok(url) = std::env::var("BBBCM_DATABASE_POSTGRES_URL") {
            self.database.postgres_url = Some(url);
        }
    }

    /// 装配前的一致性校验。非法或缺失配置一律拒绝启动（`02` 第 4 节）。
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !self.database.backend.is_implemented() {
            return Err(ConfigError::Invalid(format!(
                "database.backend = \"{}\" 是规范预留值，该后端尚未接入（DEC-022）",
                self.database.backend
            )));
        }
        crate::identifiers::SchemaRenderer::new(&self.database.schema_prefix)
            .map_err(|e| ConfigError::Invalid(format!("database.schema_prefix 非法: {e}")))?;
        if self.database.backend == Backend::Postgres && self.database.postgres_url.is_none() {
            return Err(ConfigError::Invalid(
                "backend = postgres 时必须经环境变量 BBBCM_DATABASE_POSTGRES_URL 注入连接串".into(),
            ));
        }
        if self.database.backend == Backend::Sqlite && self.database.max_connections != 1 {
            return Err(ConfigError::Invalid(format!(
                "SQLite 是单写者，database.max_connections 必须为 1，收到 {}",
                self.database.max_connections
            )));
        }
        if self.database.max_connections == 0 {
            return Err(ConfigError::Invalid(
                "database.max_connections 必须大于 0".into(),
            ));
        }
        if self.server.bind.parse::<std::net::SocketAddr>().is_err() {
            return Err(ConfigError::Invalid(format!(
                "server.bind 不是合法监听地址: {}",
                self.server.bind
            )));
        }
        if self.server.environment.is_production_like() && !self.features.redis {
            return Err(ConfigError::Invalid(format!(
                "{} 环境必须启用 Redis：限流由 Redis 承载，关闭即无速率保护（DEC-024）",
                self.server.environment
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str::<Config>(toml_text)
    }

    #[test]
    fn defaults_are_experience_mode() {
        let c = Config::default();
        assert_eq!(c.database.backend, Backend::Sqlite);
        assert_eq!(c.database.schema_prefix, "bcm_");
        assert_eq!(c.database.max_connections, 1);
        assert_eq!(c.server.environment, Environment::Development);
        assert!(!c.features.redis);
    }

    #[test]
    fn unknown_key_is_rejected() {
        let err = parse("[features]\nrediss = true\n").expect_err("拼错的键必须报错");
        assert!(
            err.to_string().contains("rediss") || err.to_string().contains("unknown"),
            "错误信息应指出未知键: {err}"
        );
        assert!(
            parse("[featurez]\nredis = true\n").is_err(),
            "未知表也应报错"
        );
    }

    #[test]
    fn unknown_environment_is_rejected() {
        assert!(parse("[server]\nenvironment = \"prod\"\n").is_err());
        assert!(parse("[server]\nenvironment = \"production\"\n").is_ok());
    }

    #[test]
    fn mysql8_is_reserved_and_refused_at_startup() {
        let c = parse("[database]\nbackend = \"mysql8\"\n").expect("应能解析预留值");
        let err = c.validate().expect_err("预留后端必须拒绝启动");
        assert!(err.to_string().contains("尚未接入"), "{err}");
    }

    #[test]
    fn postgres_requires_url_from_env() {
        let c =
            parse("[database]\nbackend = \"postgres\"\nmax_connections = 8\n").expect("应能解析");
        assert!(c.validate().is_err(), "缺连接串必须拒绝启动");
    }

    #[test]
    fn sqlite_rejects_multiple_connections() {
        let c = parse("[database]\nmax_connections = 10\n").expect("应能解析");
        let err = c.validate().expect_err("SQLite 单写者不得配多连接");
        assert!(err.to_string().contains("单写者"), "{err}");
    }

    #[test]
    fn bad_schema_prefix_is_rejected() {
        let c = parse("[database]\nschema_prefix = \"BCM\"\n").expect("应能解析");
        assert!(c.validate().is_err());
    }

    #[test]
    fn bad_bind_is_rejected() {
        let c = parse("[server]\nbind = \"not-an-address\"\n").expect("应能解析");
        assert!(c.validate().is_err());
    }

    #[test]
    fn production_without_redis_is_rejected() {
        let c = parse("[server]\nenvironment = \"production\"\n").expect("应能解析");
        let err = c.validate().expect_err("生产环境无 Redis 即无限流");
        assert!(err.to_string().contains("Redis"), "{err}");
    }

    #[test]
    fn env_override_injects_postgres_url() {
        let mut c = Config::default();
        assert!(c.database.postgres_url.is_none());
        // SAFETY: 单线程测试内设置进程环境变量。
        unsafe { std::env::set_var("BBBCM_DATABASE_POSTGRES_URL", "postgres://x/y") };
        c.apply_env();
        unsafe { std::env::remove_var("BBBCM_DATABASE_POSTGRES_URL") };
        assert_eq!(c.database.postgres_url.as_deref(), Some("postgres://x/y"));
    }

    #[test]
    fn loads_repository_default_config() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/default.toml");
        let c = Config::load(&path).expect("仓库默认配置必须可加载");
        c.validate().expect("仓库默认配置必须通过校验");
    }
}
