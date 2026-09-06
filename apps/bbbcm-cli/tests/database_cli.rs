//! 真实二进制、隔离 cwd/config/local.toml 与临时 SQLite；不需要独立构建步骤。
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-cli-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(
            dir.join("config.toml"),
            format!(
                "[server]\nenvironment='test'\n[database]\nsqlite_path='{}'\n",
                dir.join("test.db").display()
            ),
        )
        .unwrap();
        Self { dir }
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bbbcm-cli"));
        command
            .env_clear()
            .current_dir(&self.dir)
            .env("BBBCM_CONFIG", self.dir.join("config.toml"));
        command
    }
    fn db(&self, args: &[&str]) -> Output {
        self.command()
            .args(["db", "--migrations-dir"])
            .arg(repository_migrations())
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn repository_migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations")
}
fn succeeds(output: Output) -> String {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn cli_migrates_reports_and_seeds_repeatedly() {
    let f = Fixture::new();
    assert!(succeeds(f.db(&["status"])).contains("pending"));
    assert!(!f.dir.join("test.db").exists(), "status不能创建数据库");
    assert!(!f.db(&["seed"]).status.success());
    assert!(
        succeeds(f.db(&["migrate", "--backend", "sqlite"]))
            .contains("应用 [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]")
    );
    assert!(
        succeeds(f.db(&["migrate"]))
            .contains("已应用跳过 [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]")
    );
    assert!(succeeds(f.db(&["status"])).contains("applied"));
    assert!(succeeds(f.db(&["seed"])).contains("新增 4 个版块、4 个规则"));
    assert!(succeeds(f.db(&["seed"])).contains("新增 0 个版块、0 个规则"));
    assert!(!f.db(&["migrate", "--backend", "mysql8"]).status.success());
    assert!(
        !f.run(&["flag", "set", "public_read", "true"])
            .status
            .success()
    );
}
#[test]
fn cli_uses_local_override_and_rejects_secrets_in_files() {
    let f = Fixture::new();
    std::fs::create_dir(f.dir.join("config")).unwrap();
    std::fs::write(
        f.dir.join("config/local.toml"),
        "[database]\nschema_prefix='cli42_'\n",
    )
    .unwrap();
    assert!(succeeds(f.db(&["status"])).contains("cli42_"));
    let sql = succeeds(f.db(&["bootstrap-sql"]));
    assert!(sql.contains("CREATE ROLE cli42_migrator NOLOGIN"));
    std::fs::write(
        f.dir.join("config/local.toml"),
        "[database]\npostgres_url='PRIVATE_SENTINEL'\n",
    )
    .unwrap();
    let output = f.db(&["status"]);
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("PRIVATE_SENTINEL"));
}
#[test]
fn production_seed_rejected_before_database_connection() {
    let f = Fixture::new();
    std::fs::write(f.dir.join("config.toml"), "[server]\nenvironment='production'\nbind='127.0.0.1:8080'\n[database]\nbackend='postgres'\nmax_connections=1\n[features]\nredis=true\n[worker]\nembedded=false\n").unwrap();
    let output = f
        .command()
        .env(
            "BBBCM_DATABASE_POSTGRES_URL",
            "postgres://synthetic:nondefault@127.0.0.1:1/synthetic?sslmode=require",
        )
        .env("BBBCM_REDIS_URL", "redis://127.0.0.1:1/0")
        .args(["db", "seed"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("staging/production 禁止种子"));
    assert!(!f.dir.join("test.db").exists());
}

#[test]
fn implicit_postgres_environment_cannot_bypass_shared_config() {
    let f = Fixture::new();
    for key in [
        "PGHOST",
        "PGHOSTADDR",
        "PGPORT",
        "PGUSER",
        "PGDATABASE",
        "PGPASSWORD",
        "PGSSLMODE",
        "PGSSLROOTCERT",
        "PGSSLCERT",
        "PGSSLKEY",
        "PGAPPNAME",
        "PGOPTIONS",
        "PGPASSFILE",
    ] {
        let output = f
            .command()
            .env(key, "PRIVATE_DRIVER_SENTINEL")
            .args(["flag", "list"])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{key}必须拒绝隐式驱动配置");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("PRIVATE_DRIVER_SENTINEL"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_DRIVER_SENTINEL"));
    }
}

#[test]
fn flag_list_reads_storage_and_fails_without_schema() {
    let f = Fixture::new();
    assert!(!f.run(&["flag", "list"]).status.success());
    assert!(
        !f.dir.join("test.db").exists(),
        "只读Flag查询不能创建数据库"
    );
    succeeds(f.db(&["migrate"]));
    let text = succeeds(f.run(&["flag", "list"]));
    assert!(text.contains("数据库快照"));
    assert_eq!(
        text.matches("默认 关 当前 关 版本 1 来源 数据库").count(),
        14
    );
}

#[tokio::test]
async fn flag_list_distinguishes_persisted_value_and_missing_row() {
    let f = Fixture::new();
    succeeds(f.db(&["migrate"]));
    let mut config = platform::config::Config::default().database;
    config.sqlite_path = f.dir.join("test.db").to_str().unwrap().into();
    let pool = platform_db::Pool::connect_existing(&config).await.unwrap();
    let renderer = platform::identifiers::SchemaRenderer::new(&config.schema_prefix).unwrap();
    let table = renderer
        .table(pool.backend(), "platform", "feature_flags")
        .unwrap();
    let platform_db::Pool::Sqlite(sqlite, _) = &pool else {
        unreachable!()
    };
    sqlx::query(&format!(
        "UPDATE {table} SET enabled = ?, version = ? WHERE flag_key = ?"
    ))
    .bind(1)
    .bind(2)
    .bind("public_read")
    .execute(sqlite)
    .await
    .unwrap();
    sqlx::query(&format!("DELETE FROM {table} WHERE flag_key = ?"))
        .bind("search")
        .execute(sqlite)
        .await
        .unwrap();
    pool.close().await;
    let text = succeeds(f.run(&["flag", "list"]));
    assert!(
        text.lines().any(|line| line.contains("public_read")
            && line.contains("默认 关 当前 开 版本 2 来源 数据库"))
    );
    assert!(
        text.lines().any(|line| line.contains("search")
            && line.contains("默认 关 当前 关 版本 - 来源 缺项安全关闭"))
    );
}
