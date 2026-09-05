//! Embedded host of the same persistent Outbox scheduler used by bbbcm-worker.
use platform::config::Config;
use platform_outbox::{
    ClosedExternalEffects, HostLock, LocalSink, Scheduler, Settings, SinkRegistry,
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub struct PreparedJobs {
    scheduler: Scheduler,
    pool: platform_db::Pool,
    _host_lock: HostLock,
}
/// Run before opening the HTTP listener: lock/connection errors must fail startup.
pub async fn prepare(config: &Config) -> Result<PreparedJobs, String> {
    let pool =
        platform_db::Pool::connect_for(&config.database, platform_db::connection::Purpose::App)
            .await
            .map_err(|error| error.to_string())?;
    let host_lock = HostLock::acquire(&config.database).map_err(|error| error.to_string())?;
    let renderer = platform::identifiers::SchemaRenderer::new(&config.database.schema_prefix)
        .map_err(|error| error.to_string())?;
    let store = Arc::new(platform_db::outbox_repo::DbOutboxStore::new(
        pool.clone(),
        renderer.clone(),
    ));
    let mut registry = SinkRegistry::default();
    registry
        .register(Arc::new(LocalSink::default()))
        .map_err(|error| error.to_string())?;
    registry
        .register(Arc::new(platform_audit::ReplicaSink::new(
            Arc::new(platform_db::audit_repo::DbAuditExport::new(
                pool.clone(),
                renderer,
            )),
            Arc::new(platform_audit::DisabledReplica),
        )))
        .map_err(|error| error.to_string())?;
    let scheduler = Scheduler::new(
        store,
        registry,
        Arc::new(ClosedExternalEffects),
        Settings::default(),
    )
    .map_err(|error| error.to_string())?;
    Ok(PreparedJobs {
        scheduler,
        pool,
        _host_lock: host_lock,
    })
}
impl PreparedJobs {
    pub async fn run(self, cancel: CancellationToken) -> Result<(), String> {
        tracing::info!("内嵌 Outbox 轮询启动，外部副作用关闭");
        let result = self
            .scheduler
            .run(cancel.clone())
            .await
            .map_err(|error| error.to_string());
        if result.is_err() {
            cancel.cancel();
        }
        self.pool.close().await;
        result
    }
}
