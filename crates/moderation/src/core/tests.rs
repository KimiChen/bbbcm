use super::*;
use chrono::{DateTime, Utc};
use platform::config::Config;
use platform_db::{
    Pool,
    transaction::{DbTransaction, DbValue},
};
use std::path::{Path, PathBuf};

struct Fixture {
    dir: PathBuf,
    pool: Pool,
    renderer: SchemaRenderer,
}
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-policy-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = Config::default().database;
        config.sqlite_path = dir.join("policies.sqlite3").to_string_lossy().into();
        config.schema_prefix = "policy_probe_".into();
        platform_db::migrations::migrate(
            &config,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        Self {
            dir,
            pool: Pool::connect(&config).await.unwrap(),
            renderer: SchemaRenderer::new(&config.schema_prefix).unwrap(),
        }
    }
    async fn insert(
        &self,
        tx: &mut TransactionConnection<'_>,
        kind: &str,
        version: i64,
        status: &str,
    ) -> i64 {
        let table = self
            .renderer
            .table(tx.backend(), "moderation", "policy_versions")
            .unwrap();
        tx.execute(&format!("INSERT INTO {table} (kind, version, body, status, created_at) VALUES (?, ?, ?, ?, ?)"),
            &[DbValue::Text(kind.into()), DbValue::I64(version), DbValue::Text("明确标注：仅供本机测试的合成政策，无法律效力。".into()), DbValue::Text(status.into()), DbValue::Time(DateTime::<Utc>::UNIX_EPOCH)]).await.unwrap();
        tx.last_insert_id().await.unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn current_consent_requires_all_three_kinds_and_exact_current_ids() {
    let f = Fixture::new().await;
    let reader = CurrentPolicyReader::new(Environment::Test);
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    assert_eq!(
        reader.current(&mut tx.connection(), &f.renderer).await,
        Err(PolicyError::NotReady)
    );
    f.insert(&mut tx.connection(), "terms", 1, "synthetic")
        .await;
    f.insert(&mut tx.connection(), "privacy", 1, "synthetic")
        .await;
    assert_eq!(
        reader.current(&mut tx.connection(), &f.renderer).await,
        Err(PolicyError::NotReady)
    );
    f.insert(&mut tx.connection(), "community", 1, "synthetic")
        .await;
    let current = reader
        .current(&mut tx.connection(), &f.renderer)
        .await
        .unwrap();
    reader
        .require_current(&mut tx.connection(), &f.renderer, &current)
        .await
        .unwrap();
    let swapped = PolicySet {
        terms: current.privacy,
        privacy: current.terms,
        community: current.community,
    };
    assert_eq!(
        reader
            .require_current(&mut tx.connection(), &f.renderer, &swapped)
            .await,
        Err(PolicyError::Changed)
    );
    f.insert(&mut tx.connection(), "terms", 2, "synthetic")
        .await;
    assert_eq!(
        reader
            .require_current(&mut tx.connection(), &f.renderer, &current)
            .await,
        Err(PolicyError::Changed)
    );
    tx.rollback().await.unwrap();
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    assert_eq!(
        reader.current(&mut tx.connection(), &f.renderer).await,
        Err(PolicyError::NotReady)
    );
    tx.rollback().await.unwrap();
    f.pool.close().await;
}

#[tokio::test]
async fn synthetic_versions_never_satisfy_production_and_retired_versions_are_ignored() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    for kind in ["terms", "privacy", "community"] {
        f.insert(&mut tx.connection(), kind, 1, "synthetic").await;
    }
    for environment in [Environment::Staging, Environment::Production] {
        assert_eq!(
            CurrentPolicyReader::new(environment)
                .current(&mut tx.connection(), &f.renderer)
                .await,
            Err(PolicyError::NotReady)
        );
    }
    for kind in ["terms", "privacy", "community"] {
        f.insert(&mut tx.connection(), kind, 2, "published").await;
        f.insert(&mut tx.connection(), kind, 3, "retired").await;
    }
    let docs = CurrentPolicyReader::new(Environment::Production)
        .documents(&mut tx.connection(), &f.renderer)
        .await
        .unwrap();
    assert!(docs.iter().all(|v| v.version == 2 && !v.synthetic));
    tx.commit().await.unwrap();
    f.pool.close().await;
}
