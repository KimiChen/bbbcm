use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-config-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn variables(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
fn parse(text: &str) -> Result<Config, ConfigError> {
    let fixture = Fixture::new();
    Config::load(&fixture.file("config.toml", text))
}
fn postgres_secrets() -> SecretOverrides {
    SecretOverrides {
        postgres_url: Some(
            "postgres://synthetic@localhost/example?sslmode=require"
                .to_string()
                .into(),
        ),
        ..Default::default()
    }
}

#[test]
fn defaults_are_safe_experience_mode() {
    let config = Config::default();
    assert_eq!(config.database.backend, Backend::Sqlite);
    assert_eq!(config.database.schema_prefix, "bcm_");
    assert_eq!(config.server.environment, Environment::Development);
    assert!(config.worker.embedded);
    assert!(!config.features.redis);
    config.validate_for(ProcessKind::Web).unwrap();
    config.validate_for(ProcessKind::Cli).unwrap();
}
#[test]
fn precedence_defaults_file_local_environment_and_secret() {
    let fixture = Fixture::new();
    let base = fixture.file(
        "base.toml",
        "[server]\nbind='127.0.0.1:18001'\n[database]\nbackend='postgres'\nmax_connections=4\n",
    );
    let local = fixture.file(
        "local.toml",
        "[server]\nbind='127.0.0.1:18002'\n[database]\nmax_connections=5\n",
    );
    let variables = variables(&[
        ("BBBCM_SERVER_BIND", "127.0.0.1:18003"),
        (
            "BBBCM_DATABASE_POSTGRES_URL",
            "postgres://ignored@localhost/ignored",
        ),
    ]);
    let config = Config::load_layers(&base, Some(&local), &variables, postgres_secrets()).unwrap();
    assert_eq!(config.server.bind, "127.0.0.1:18003");
    assert_eq!(config.database.max_connections, 5);
    assert_eq!(config.database.schema_prefix, "bcm_");
    assert_eq!(
        config.database.postgres_url.as_deref(),
        Some("postgres://synthetic@localhost/example?sslmode=require")
    );
    let local_only =
        Config::load_layers(&base, Some(&local), &BTreeMap::new(), postgres_secrets()).unwrap();
    assert_eq!(local_only.server.bind, "127.0.0.1:18002");
    assert_eq!(Config::load(&base).unwrap().database.max_connections, 4);
}
#[test]
fn unknown_keys_types_environments_and_secret_files_are_rejected() {
    for text in [
        "[features]\nrediss=true",
        "[featurez]\nredis=true",
        "[features]\nredis='true'",
        "[database]\nmax_connections='4'",
        "[server]\nenvironment='prod'",
        "[database]\npostgres_url='postgres://synthetic:do-not-print@localhost/example'",
        "[redis]\nurl='redis://:do-not-print@localhost/0'",
        "[auth]\nweak_cookie=true",
        "[provider]\napi_key='do-not-print'",
    ] {
        let error = parse(text).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("do-not-print"));
    }
}
#[test]
fn invalid_layer_is_not_hidden_by_later_overrides() {
    let fixture = Fixture::new();
    let base = fixture.file("base.toml", "[features]\nredis='not-a-bool'");
    let local = fixture.file("local.toml", "[features]\nredis=false");
    assert!(
        Config::load_layers(
            &base,
            Some(&local),
            &BTreeMap::new(),
            SecretOverrides::default()
        )
        .is_err()
    );
    let base = fixture.file("base.toml", "");
    let local = fixture.file("local.toml", "[features]\nrediss=false");
    assert!(
        Config::load_layers(
            &base,
            Some(&local),
            &BTreeMap::new(),
            SecretOverrides::default()
        )
        .is_err()
    );
}
#[test]
fn explicit_file_missing_fails_optional_local_missing_succeeds() {
    let fixture = Fixture::new();
    let missing = fixture.0.join("missing.toml");
    assert!(Config::load(&missing).is_err());
    let base = fixture.file("base.toml", "");
    Config::load_layers(
        &base,
        Some(&missing),
        &BTreeMap::new(),
        SecretOverrides::default(),
    )
    .unwrap();
}
#[test]
fn environment_mappings_are_strict_and_allow_asset_isolation() {
    let fixture = Fixture::new();
    let base = fixture.file("base.toml", "");
    for (key, value) in [
        ("BBBCM_FEATURES_REDISS", "false"),
        ("BBBCM_FEATURES_REDIS", "1"),
        ("BBBCM_WORKER_EMBEDDED", "TRUE"),
        ("BBBCM_DATABASE_MAX_CONNECTIONS", "-1"),
        ("BBBCM_SERVER_ENVIRONMENT", "prod"),
        ("BBBCM_DATABASE_BACKEND", "psql"),
        ("PGPASSWORD", "do-not-print"),
        ("PGOPTIONS", "do-not-print"),
        ("PGPASSFILE", "do-not-print"),
    ] {
        assert!(
            Config::load_layers(
                &base,
                None,
                &variables(&[(key, value)]),
                SecretOverrides::default()
            )
            .is_err(),
            "{key}"
        );
    }
    let config = Config::load_layers(
        &base,
        None,
        &variables(&[
            ("BBBCM_SERVER_ENVIRONMENT", "test"),
            ("BBBCM_DATABASE_BACKEND", "sqlite"),
            ("BBBCM_DATABASE_SCHEMA_PREFIX", "trial_"),
            ("BBBCM_DATABASE_SQLITE_PATH", "data/isolated.db"),
            ("BBBCM_DATABASE_MAX_CONNECTIONS", "1"),
            ("BBBCM_WORKER_EMBEDDED", "false"),
            ("BBBCM_FEATURES_REDIS", "false"),
            ("BBBCM_ASSET_DIST", "/isolated/assets"),
        ]),
        SecretOverrides::default(),
    )
    .unwrap();
    assert_eq!(config.server.environment, Environment::Test);
    assert_eq!(config.database.schema_prefix, "trial_");
    assert_eq!(config.database.sqlite_path, "data/isolated.db");
    assert!(!config.worker.embedded);
}
#[test]
fn secret_debug_and_parse_errors_never_disclose_values() {
    let variables = variables(&[
        (
            "BBBCM_DATABASE_POSTGRES_URL",
            "postgres://synthetic:do-not-print@localhost/example",
        ),
        ("BBBCM_REDIS_URL", "redis://:do-not-print@localhost/0"),
    ]);
    let secrets = SecretOverrides::from_variables(&variables);
    assert!(!format!("{secrets:?}").contains("do-not-print"));
    let fixture = Fixture::new();
    let base = fixture.file("base.toml", "[database]\nbackend='postgres'");
    let config = Config::load_layers(&base, None, &variables, secrets).unwrap();
    assert!(!format!("{config:?}").contains("do-not-print"));
    for text in [
        "this is invalid do-not-print",
        "[server]\nbind='do-not-print'",
        "[database]\nschema_prefix='do-not-print'",
    ] {
        let base = fixture.file("bad.toml", text);
        let error = Config::load_layers(&base, None, &BTreeMap::new(), SecretOverrides::default())
            .unwrap_err();
        assert!(!format!("{error:?} {error}").contains("do-not-print"));
        assert!(std::error::Error::source(&error).is_none());
    }
}
#[test]
fn backend_connections_prefix_path_and_bind_validation() {
    for text in [
        "[database]\nbackend='mysql8'",
        "[database]\nbackend='postgres'",
        "[database]\nmax_connections=0",
        "[database]\nmax_connections=2",
        "[database]\nschema_prefix='BCM'",
        "[database]\nsqlite_path=''",
        "[database]\nsqlite_path=':memory:'",
        "[database]\nsqlite_path='file:memdb?mode=memory&cache=shared'",
        "[database]\nsqlite_path='file::memory:'",
        "[database]\nsqlite_path='sqlite://data/example.db'",
        "[database]\nsqlite_path='${DATA_PATH}/example.db'",
        "[server]\nbind='not-an-address'",
        "[server]\nbind='127.0.0.1:0'",
        "[server]\nbind='0.0.0.0:18080'",
        "[features]\nredis=true",
    ] {
        assert!(parse(text).unwrap().validate().is_err(), "{text}");
    }
}
#[test]
fn urls_require_real_connection_parameters_without_disclosing_them() {
    for value in [
        "http://synthetic@localhost/example",
        "postgres://synthetic@localhost/",
        "postgres:///example",
        "postgres://localhost/example",
        "postgres://synthetic:do-not-print@localhost:99999/example",
        "postgres://synthetic:do-not-print@localhost:0/example",
        "postgres://synthetic:do-not-print@localhost/example#fragment",
        "postgres://synthetic:${SECRET}@localhost/example",
        "postgres://synthetic:%24%7BSECRET%7D@localhost/example",
        "postgres://synthetic:invalid%ZZ@localhost/example",
        "postgres://synthetic@localhost/example?secret=do-not-print",
        "postgres://synthetic@localhost/example?password=do-not-print",
        "postgres://synthetic@localhost/example?sslmode=invalid",
    ] {
        let error = validate_url(value, UrlKind::Postgres, Environment::Development).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("do-not-print"));
    }
    for value in [
        "https://localhost/0",
        "redis:///0",
        "redis://localhost/not-a-db",
        "redis://localhost/0/1",
        "redis://localhost:0/0",
    ] {
        assert!(
            validate_url(value, UrlKind::Redis, Environment::Development).is_err(),
            "{value}"
        );
    }
    validate_url(
        "postgresql://synthetic@localhost/example?sslmode=require",
        UrlKind::Postgres,
        Environment::Test,
    )
    .unwrap();
    validate_url(
        "rediss://:synthetic-test@localhost:6379/2",
        UrlKind::Redis,
        Environment::Test,
    )
    .unwrap();
}
#[test]
fn all_four_environments_and_production_boundaries() {
    for environment in [
        Environment::Development,
        Environment::Test,
        Environment::Staging,
        Environment::Production,
    ] {
        let mut config = Config::default();
        config.server.environment = environment;
        if environment.is_production_like() {
            assert!(config.validate().is_err());
            config.database.backend = Backend::Postgres;
            config.database.postgres_url = postgres_secrets().postgres_url;
            config.features.redis = true;
            config.redis.url = Some("redis://localhost/0".to_string().into());
            config.worker.embedded = false;
        }
        config.validate().unwrap();
        config.database.schema_prefix = "trial_".into();
        assert_eq!(
            config.validate().is_ok(),
            environment != Environment::Production
        );
    }
    assert!(
        validate_url(
            "postgres://synthetic:password@localhost/example?sslmode=require",
            UrlKind::Postgres,
            Environment::Production
        )
        .is_err()
    );
    assert!(
        validate_url(
            "redis://:default@localhost/0",
            UrlKind::Redis,
            Environment::Staging
        )
        .is_err()
    );
    assert!(
        validate_url(
            "postgres://synthetic:p%61ssword@localhost/example?sslmode=require",
            UrlKind::Postgres,
            Environment::Production
        )
        .is_err()
    );
}
#[test]
fn sqlite_worker_mode_is_mutually_exclusive() {
    let mut config = Config::default();
    assert!(config.validate_for(ProcessKind::Worker).is_err());
    config.worker.embedded = false;
    for process in [ProcessKind::Web, ProcessKind::Worker, ProcessKind::Cli] {
        config.validate_for(process).unwrap();
    }
}
#[test]
fn repository_templates_cover_four_environments_and_require_deployment_values() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config");
    Config::load(&root.join("default.toml"))
        .unwrap()
        .validate()
        .unwrap();
    for (file, environment) in [
        ("development", Environment::Development),
        ("test", Environment::Test),
        ("staging", Environment::Staging),
        ("production", Environment::Production),
    ] {
        let config = Config::load(&root.join(format!("{file}.toml.example"))).unwrap();
        assert_eq!(config.server.environment, environment);
        assert_eq!(config.validate().is_err(), environment.is_production_like());
        if environment.is_production_like() {
            let variables = variables(&[
                ("BBBCM_SERVER_BIND", "127.0.0.1:18080"),
                ("BBBCM_DATABASE_MAX_CONNECTIONS", "2"),
            ]);
            Config::load_layers(
                &root.join(format!("{file}.toml.example")),
                None,
                &variables,
                SecretOverrides {
                    postgres_url: postgres_secrets().postgres_url,
                    redis_url: Some("redis://localhost/0".to_string().into()),
                },
            )
            .unwrap();
        }
    }
}

#[test]
fn production_postgres_requires_tls_without_plaintext_fallback() {
    for environment in [Environment::Staging, Environment::Production] {
        for suffix in [
            "",
            "?sslmode=disable",
            "?sslmode=allow",
            "?sslmode=prefer",
            "?sslmode=require&ssl-mode=disable",
        ] {
            let value = format!("postgres://synthetic@localhost/example{suffix}");
            assert!(validate_url(&value, UrlKind::Postgres, environment).is_err());
        }
        for suffix in [
            "?sslmode=require",
            "?sslmode=verify-ca",
            "?sslmode=verify-full",
            "?ssl-mode=require",
        ] {
            let value = format!("postgres://synthetic@localhost/example{suffix}");
            validate_url(&value, UrlKind::Postgres, environment).unwrap();
        }
    }
    validate_url(
        "postgres://synthetic@localhost/example",
        UrlKind::Postgres,
        Environment::Development,
    )
    .unwrap();
}
