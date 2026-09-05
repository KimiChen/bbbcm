use platform::{
    config::Config,
    observability::{DatabaseBackend, DatabasePurpose, Measurement, PoolSample, metrics},
};
use platform_db::{Pool, connection::Purpose};

fn sample(backend: DatabaseBackend, purpose: DatabasePurpose) -> Measurement<PoolSample> {
    metrics()
        .pools
        .snapshot()
        .into_iter()
        .find(|group| group.backend == backend && group.purpose == purpose)
        .expect("closed backend/purpose group")
        .observation
}
fn sqlite(purpose: DatabasePurpose) -> Measurement<PoolSample> {
    sample(DatabaseBackend::Sqlite, purpose)
}

// A separate integration-test process gives the global registry no unrelated pools.
#[tokio::test]
async fn actual_pool_gauges_preserve_purpose_clone_identity_and_lifetime() {
    assert_eq!(sqlite(DatabasePurpose::App), Measurement::NotConnected);
    assert_eq!(sqlite(DatabasePurpose::Worker), Measurement::NotConnected);
    let dir = std::env::temp_dir().join(format!(
        "bbbcm-pool-observations-{}",
        platform::transaction::new_operation_id()
    ));
    let mut cfg = Config::default().database;
    cfg.sqlite_path = dir.join("pool.sqlite").to_str().unwrap().into();
    let app = Pool::connect(&cfg).await.unwrap();
    let app_clone = app.clone();
    let worker = Pool::connect_for(&cfg, Purpose::Worker).await.unwrap();
    assert_eq!(app.purpose(), DatabasePurpose::App);
    assert_eq!(worker.purpose(), DatabasePurpose::Worker);
    let idle = PoolSample {
        pools: 1,
        open: 1,
        idle: 1,
        in_use: 0,
        max: 1,
    };
    assert_eq!(sqlite(DatabasePurpose::App), Measurement::Collected(idle));
    assert_eq!(
        sqlite(DatabasePurpose::Worker),
        Measurement::Collected(idle)
    );
    assert_eq!(sqlite(DatabasePurpose::Migrator), Measurement::NotConnected);
    assert_eq!(
        sample(DatabaseBackend::Postgres, DatabasePurpose::App),
        Measurement::NotConnected
    );

    let Pool::Sqlite(driver, _) = &worker else {
        unreachable!()
    };
    let mut checkout = driver.acquire().await.unwrap();
    assert_eq!(
        sqlite(DatabasePurpose::Worker),
        Measurement::Collected(PoolSample {
            idle: 0,
            in_use: 1,
            ..idle
        })
    );
    assert_eq!(sqlite(DatabasePurpose::App), Measurement::Collected(idle));
    checkout.return_to_pool().await;
    assert_eq!(
        sqlite(DatabasePurpose::Worker),
        Measurement::Collected(idle)
    );

    let app_second = Pool::connect(&cfg).await.unwrap();
    assert_eq!(
        sqlite(DatabasePurpose::App),
        Measurement::Collected(PoolSample {
            pools: 2,
            open: 2,
            idle: 2,
            in_use: 0,
            max: 2
        })
    );
    app_second.close().await;
    drop(app_second);
    drop(app);
    assert_eq!(
        sqlite(DatabasePurpose::App),
        Measurement::Collected(idle),
        "clone retains the single registration"
    );
    app_clone.close().await;
    assert_eq!(
        sqlite(DatabasePurpose::App),
        Measurement::Collected(PoolSample {
            open: 0,
            idle: 0,
            ..idle
        }),
        "closed pool is a collected zero while its wrapper exists"
    );
    drop(app_clone);
    assert_eq!(
        sqlite(DatabasePurpose::App),
        Measurement::NotConnected,
        "weak registry does not keep the pool alive"
    );
    worker.close().await;
    drop(worker);
    assert_eq!(sqlite(DatabasePurpose::Worker), Measurement::NotConnected);
    let exported = serde_json::to_string(&metrics().pools.snapshot()).unwrap();
    assert!(!exported.contains(cfg.sqlite_path.as_str()));
    assert!(!exported.contains("pool_id"));
    std::fs::remove_dir_all(dir).unwrap();
}
