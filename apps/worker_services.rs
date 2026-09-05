//! Application composition shared by both hosts, preserving platform crate direction.
use platform::{
    cache::RedisRuntime,
    config::Config,
    identifiers::SchemaRenderer,
    integrations::{AdapterMode, AdapterRuntime, UnwiredEffectSource},
    mail::{MailAlertSink, MailSink, UnwiredMailSource},
};
use platform_db::Pool;
use platform_outbox::{ExternalEffects, LocalSink, Scheduler, Settings, SinkRegistry};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct LocalOnlyEffects(bool);
impl ExternalEffects for LocalOnlyEffects {
    fn allowed(&self) -> bool {
        self.0
    }
}
pub fn scheduler(config: &Config, pool: Pool, runtime: &RedisRuntime) -> Result<Scheduler, String> {
    let renderer =
        SchemaRenderer::new(&config.database.schema_prefix).map_err(|e| e.to_string())?;
    let adapters = AdapterRuntime::assemble(&config.adapters, config.server.environment)
        .map_err(|e| e.to_string())?;
    let mail = Arc::new(platform_db::mail_repo::DbMailStore::new(
        pool.clone(),
        renderer.clone(),
    ));
    let mut registry = SinkRegistry::default();
    registry
        .register(Arc::new(LocalSink::default()))
        .map_err(|e| e.to_string())?;
    registry
        .register(Arc::new(platform_audit::ReplicaSink::new(
            Arc::new(platform_db::audit_repo::DbAuditExport::new(
                pool.clone(),
                renderer.clone(),
            )),
            Arc::new(platform_audit::DisabledReplica),
        )))
        .map_err(|e| e.to_string())?;
    registry
        .register(Arc::new(platform::cache::CacheInvalidationSink::new(
            runtime.cache.clone(),
        )))
        .map_err(|e| e.to_string())?;
    registry
        .register(Arc::new(platform::analytics::AnalyticsSink::new(Arc::new(
            platform_db::analytics_repo::DbAnalyticsStore::new(pool.clone(), renderer.clone()),
        ))))
        .map_err(|e| e.to_string())?;
    registry
        .register(Arc::new(MailSink {
            source: Arc::new(UnwiredMailSource),
            store: mail.clone(),
            provider: adapters.mail.clone(),
        }))
        .map_err(|e| e.to_string())?;
    registry
        .register(Arc::new(MailAlertSink {
            source: mail,
            tracker: adapters.errors.clone(),
        }))
        .map_err(|e| e.to_string())?;
    registry
        .register(Arc::new(
            adapters.effects_sink(Arc::new(UnwiredEffectSource)),
        ))
        .map_err(|e| e.to_string())?;
    let local = config.adapters.mode == AdapterMode::LocalTest
        && !config.server.environment.is_production_like();
    Scheduler::new(
        Arc::new(platform_db::outbox_repo::DbOutboxStore::new(pool, renderer)),
        registry,
        Arc::new(LocalOnlyEffects(local)),
        Settings::default(),
    )
    .map(|s| {
        s.with_wakeup(runtime.wakeup.clone())
            .with_backend(config.database.backend)
    })
    .map_err(|e| e.to_string())
}

/// The worker's own role observes its mail data. Web never borrows Worker credentials.
pub async fn observe_mail(config: Config, pool: Pool, cancel: CancellationToken) {
    let Ok(renderer) = SchemaRenderer::new(&config.database.schema_prefix) else {
        return;
    };
    let store = platform_db::mail_repo::DbMailStore::new(pool, renderer);
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tokio::select! { biased; ()=cancel.cancelled()=>return, _=interval.tick()=>{} }
        let observations = platform::observability::metrics();
        tracing::info!(
            process_instance=%observations.instance_id(),
            pools=?observations.pools.snapshot(),
            series=?observations.grouped.snapshot(),
            worker_active_jobs=?observations.grouped.active_jobs_observation(),
            "worker observations"
        );
        let end = platform::db::decode_time(&platform::db::encode_time(chrono::Utc::now()))
            .expect("canonical UTC microseconds");
        let start = end
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("UTC midnight")
            .and_utc();
        if start >= end {
            continue;
        }
        for simulated in [false, true] {
            let result = tokio::select! { biased; ()=cancel.cancelled()=>return, result=store.metric_snapshot(start,end,simulated)=>result };
            match result {
                Ok(sample) => {
                    tracing::info!(process_instance=%observations.instance_id(),provider_mode=?config.adapters.mode, source_state="not_connected", simulated, submitted=sample.submitted,delivered=sample.delivered,delayed=sample.delayed,bounced=sample.bounced,complained=sample.complained,max_delay_ms=?sample.max_delay_ms,window_start=%start,window_end=%end,"mail metrics");
                    if let Ok(alerts) = config.mail_alerts.evaluate(&sample) {
                        for alert in alerts {
                            tracing::warn!(simulated,alert=?alert,"mail metric alert");
                        }
                    }
                }
                Err(_) => {
                    platform::observability::metrics()
                        .increment(platform::observability::Metric::MailError);
                    tracing::warn!(
                        dependency = "mail_metrics",
                        outcome = "unavailable",
                        "mail observations unavailable"
                    );
                }
            }
        }
    }
}
