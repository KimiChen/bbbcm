//! Embedded host of the same persistent Outbox scheduler used by bbbcm-worker.
use platform::config::Config;
use platform_outbox::{HostLock, Scheduler};
use tokio_util::sync::CancellationToken;

pub struct PreparedJobs {
    scheduler: Scheduler,
    pool: platform_db::Pool,
    _host_lock: HostLock,
    config: Config,
}
/// Run before opening the HTTP listener: lock/connection errors must fail startup.
pub async fn prepare(
    config: &Config,
    runtime: &platform::cache::RedisRuntime,
) -> Result<PreparedJobs, String> {
    let pool =
        platform_db::Pool::connect_for(&config.database, platform_db::connection::Purpose::Worker)
            .await
            .map_err(|error| error.to_string())?;
    let host_lock = HostLock::acquire(&config.database).map_err(|error| error.to_string())?;
    let scheduler = crate::worker_services::scheduler(config, pool.clone(), runtime)?;
    Ok(PreparedJobs {
        scheduler,
        config: config.clone(),
        pool,
        _host_lock: host_lock,
    })
}
impl PreparedJobs {
    pub async fn run(self, cancel: CancellationToken) -> Result<(), String> {
        tracing::info!(adapter_mode=?self.config.adapters.mode, "内嵌 Outbox 调度启动");
        let observer = tokio::spawn(crate::worker_services::observe_mail(
            self.config,
            self.pool.clone(),
            cancel.clone(),
        ));
        let result = self
            .scheduler
            .run(cancel.clone())
            .await
            .map_err(|error| error.to_string());
        if result.is_err() {
            cancel.cancel();
        }
        cancel.cancel();
        let _ = observer.await;
        self.pool.close().await;
        result
    }
}
