//! Polling scheduler shared by standalone and embedded hosts. Delivery is at least once;
//! an external provider still needs its own idempotency key or reconciliation contract.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use platform::config::DatabaseConfig;
use platform::db::Backend;
use platform::observability::{
    DatabaseBackend, OutboxOutcome, SeriesKey, SinkGroup, SinkOutcome, WorkerOutcome,
};
use platform::observability::{Metric, Metrics, metrics};
pub use platform::outbox::{BoxFuture, OutboxEvent, Sink, SinkError};
use platform::outbox::{Claim, Failure, FailureCode, LeasedEvent, OutboxError, OutboxStore};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

#[derive(Default)]
pub struct SinkRegistry {
    sinks: BTreeMap<String, Arc<dyn Sink>>,
}
impl SinkRegistry {
    pub fn register(&mut self, sink: Arc<dyn Sink>) -> Result<(), OutboxError> {
        if !platform::outbox::is_machine_name(sink.name()) || self.sinks.contains_key(sink.name()) {
            return Err(OutboxError::InvalidInput);
        }
        self.sinks.insert(sink.name().into(), sink);
        Ok(())
    }
}
/// This is independent of the Flag write path. Applications inject the allowed policy;
/// the current hosts always use ClosedExternalEffects until provider and access work lands.
pub trait ExternalEffects: Send + Sync {
    fn allowed(&self) -> bool;
}
pub struct ClosedExternalEffects;
impl ExternalEffects for ClosedExternalEffects {
    fn allowed(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub poll_interval: Duration,
    pub lease_duration: Duration,
    pub renew_interval: Duration,
    pub batch_size: u32,
    pub max_attempts: i64,
    pub retry_base: Duration,
    pub retry_max: Duration,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            lease_duration: Duration::from_secs(30),
            renew_interval: Duration::from_secs(10),
            batch_size: 16,
            max_attempts: 5,
            retry_base: Duration::from_secs(1),
            retry_max: Duration::from_secs(300),
        }
    }
}
impl Settings {
    fn validate(&self) -> Result<(), OutboxError> {
        if self.poll_interval.is_zero()
            || self.renew_interval.is_zero()
            || self.lease_duration <= self.renew_interval
            || self.batch_size == 0
            || self.batch_size > 100
            || self.max_attempts < 1
            || self.retry_base.is_zero()
            || self.retry_max < self.retry_base
            || chrono::Duration::from_std(self.lease_duration).is_err()
            || chrono::Duration::from_std(self.retry_max).is_err()
        {
            return Err(OutboxError::InvalidInput);
        }
        Ok(())
    }
    pub fn backoff(&self, attempts: i64) -> Duration {
        self.retry_base
            .saturating_mul(
                1u32.checked_shl(attempts.saturating_sub(1).clamp(0, 31) as u32)
                    .unwrap_or(u32::MAX),
            )
            .min(self.retry_max)
    }
}

pub struct Scheduler {
    store: Arc<dyn OutboxStore>,
    registry: SinkRegistry,
    effects: Arc<dyn ExternalEffects>,
    settings: Settings,
    owner: String,
    wakeup: Option<Arc<dyn platform::cache::Wakeup>>,
    observations: Arc<Metrics>,
    backend: DatabaseBackend,
}
impl Scheduler {
    pub fn new(
        store: Arc<dyn OutboxStore>,
        registry: SinkRegistry,
        effects: Arc<dyn ExternalEffects>,
        settings: Settings,
    ) -> Result<Self, OutboxError> {
        settings.validate()?;
        // Hostname is diagnostic only and never enters event payloads.
        let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".into());
        let owner = format!(
            "{}:{}:{}",
            hostname
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'))
                .take(64)
                .collect::<String>(),
            std::process::id(),
            platform::transaction::new_operation_id()
        );
        Ok(Self {
            store,
            registry,
            effects,
            settings,
            owner,
            wakeup: None,
            observations: metrics(),
            backend: DatabaseBackend::Unknown,
        })
    }
    pub fn with_wakeup(mut self, wakeup: Arc<dyn platform::cache::Wakeup>) -> Self {
        self.wakeup = Some(wakeup);
        self
    }
    /// The host supplies its already validated database backend; injected test stores
    /// without a known backend retain an explicit Unknown observation.
    pub fn with_backend(mut self, backend: Backend) -> Self {
        self.backend = backend.into();
        self
    }
    pub async fn run(&self, cancel: CancellationToken) -> Result<(), OutboxError> {
        self.observations.grouped.observe_worker();
        let mut exit = WorkerExitObservation {
            observations: self.observations.clone(),
            outcome: WorkerOutcome::Aborted,
            backend: self.backend,
        };
        let mut previous_poll: Option<std::time::Instant> = None;
        loop {
            if cancel.is_cancelled() {
                exit.outcome = WorkerOutcome::Cancelled;
                return Ok(());
            }
            let poll = std::time::Instant::now();
            if let Some(previous) = previous_poll.replace(poll) {
                self.observations.grouped.duration(
                    SeriesKey::WorkerPollSeconds {
                        backend: self.backend,
                    },
                    poll.duration_since(previous),
                );
            }
            if let Err(error) = self.tick(&cancel).await {
                exit.outcome = WorkerOutcome::Failed;
                return Err(error);
            }
            let wait = async {
                if let Some(wakeup) = &self.wakeup {
                    if wakeup.wait(self.settings.poll_interval).await.is_err() {
                        self.observations.increment(Metric::RedisError);
                        tokio::time::sleep(self.settings.poll_interval).await;
                    }
                } else {
                    tokio::time::sleep(self.settings.poll_interval).await;
                }
            };
            tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    exit.outcome = WorkerOutcome::Cancelled;
                    return Ok(());
                }
                () = wait => {}
            }
        }
    }
    pub async fn tick(&self, cancel: &CancellationToken) -> Result<usize, OutboxError> {
        if cancel.is_cancelled() {
            return Ok(0);
        }
        let now = Utc::now();
        let until = now
            + chrono::Duration::from_std(self.settings.lease_duration)
                .map_err(|_| OutboxError::InvalidInput)?;
        let leases = self
            .store
            .claim(Claim {
                owner: self.owner.clone(),
                batch_id: platform::transaction::new_operation_id(),
                now,
                lease_until: until,
                limit: self.settings.batch_size,
            })
            .await
            .inspect_err(|error| self.observe_store_error(SinkGroup::All, error))?;
        // Claims count the complete acquired batch, including leases released before
        // execution because cancellation arrived after the database claim.
        for _ in &leases {
            self.observations.increment(Metric::OutboxClaimed);
            self.observe_outbox(SinkGroup::All, OutboxOutcome::Claimed);
        }
        for (index, lease) in leases.iter().enumerate() {
            if cancel.is_cancelled() {
                self.release(&leases[index..]).await?;
                break;
            }
            let span = tracing::info_span!("outbox", trace_id = %lease.event.trace_id, event_id = %lease.event.event_id);
            match platform::observability::REQUEST_ID
                .scope(
                    lease.event.trace_id.clone(),
                    self.consume(lease, &leases[index..], cancel)
                        .instrument(span),
                )
                .await
            {
                Ok(()) | Err(OutboxError::LeaseLost) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(leases.len())
    }
    async fn renew_batch(&self, batch: &[LeasedEvent]) -> Result<(), OutboxError> {
        let now = Utc::now();
        let until = now
            + chrono::Duration::from_std(self.settings.lease_duration)
                .map_err(|_| OutboxError::InvalidInput)?;
        for lease in batch {
            match self.store.renew(lease, now, until).await {
                Ok(()) => {
                    self.observations.increment(Metric::OutboxLeaseRenewed);
                    self.observe_outbox(SinkGroup::All, OutboxOutcome::Renewed);
                }
                Err(error) => {
                    self.observe_store_error(SinkGroup::All, &error);
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    async fn release(&self, leases: &[LeasedEvent]) -> Result<(), OutboxError> {
        for lease in leases {
            let now = Utc::now();
            match self
                .store
                .fail(
                    lease,
                    Failure {
                        code: FailureCode::Cancelled,
                        now,
                        available_at: now,
                        dead_letter: false,
                    },
                )
                .await
            {
                Ok(()) => self.observe_outbox(SinkGroup::All, OutboxOutcome::Cancelled),
                Err(error) => {
                    self.observe_store_error(SinkGroup::All, &error);
                    if error != OutboxError::LeaseLost {
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }
    async fn consume(
        &self,
        lease: &LeasedEvent,
        batch: &[LeasedEvent],
        cancel: &CancellationToken,
    ) -> Result<(), OutboxError> {
        let _active_job = self.observations.grouped.active_job();
        self.renew_batch(batch).await?;
        for name in &lease.pending_sinks {
            let group = SinkGroup::from_name(name);
            let Some(sink) = self.registry.sinks.get(name) else {
                return self
                    .fail(lease, FailureCode::UnregisteredSink, true, group)
                    .await;
            };
            let mut observation = SinkObservation {
                observations: self.observations.clone(),
                sink: group,
                started: std::time::Instant::now(),
                outcome: SinkOutcome::Cancelled,
                backend: self.backend,
            };
            if sink.is_external() && !self.effects.allowed() {
                observation.outcome = SinkOutcome::Disabled;
                self.observations.increment(Metric::SinkDisabled);
                drop(observation);
                return self
                    .fail(lease, FailureCode::ExternalDisabled, false, group)
                    .await;
            }
            let delivered = sink
                .deliver(&lease.event)
                .instrument(tracing::info_span!("sink", kind = %safe_sink_kind(name)));
            tokio::pin!(delivered);
            let mut renewal = tokio::time::interval(self.settings.renew_interval);
            renewal.tick().await;
            let result = loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => {
                        drop(observation);
                        self.release(std::slice::from_ref(lease)).await?;
                        return Ok(());
                    }
                    _ = renewal.tick() => {
                        if let Err(error) = self.renew_batch(batch).await {
                            observation.outcome = if error == OutboxError::LeaseLost {
                                SinkOutcome::LeaseLost
                            } else {
                                SinkOutcome::StoreError
                            };
                            return Err(error);
                        }
                        if sink.is_external() && !self.effects.allowed() {
                            observation.outcome = SinkOutcome::Disabled;
                            self.observations.increment(Metric::SinkDisabled);
                            drop(observation);
                            return self.fail(lease, FailureCode::ExternalDisabled, false, group).await;
                        }
                    }
                    result = &mut delivered => break result,
                }
            };
            observation.outcome = match result {
                Ok(()) => SinkOutcome::Success,
                Err(SinkError::Disabled) => SinkOutcome::Disabled,
                Err(SinkError::Permanent) => SinkOutcome::Permanent,
                Err(SinkError::Retryable) => SinkOutcome::Retryable,
            };
            // Sink timing ends at the provider result; durable Outbox acknowledgement
            // has its own outcome and cannot turn a failed DB write into completion.
            drop(observation);
            match result {
                Ok(()) => {
                    self.observations.increment(Metric::SinkSuccess);
                    tracing::info!(outcome = "success", "sink finished");
                    self.store
                        .delivery_done(lease, name, Utc::now())
                        .await
                        .inspect_err(|error| self.observe_store_error(group, error))?;
                }
                Err(SinkError::Disabled) => {
                    self.observations.increment(Metric::SinkDisabled);
                    return self
                        .fail(lease, FailureCode::ExternalDisabled, false, group)
                        .await;
                }
                Err(SinkError::Permanent) => {
                    self.observations.increment(Metric::SinkPermanent);
                    return self.fail(lease, FailureCode::Permanent, true, group).await;
                }
                Err(SinkError::Retryable) => {
                    self.observations.increment(Metric::SinkRetryable);
                    return self
                        .fail(
                            lease,
                            FailureCode::Retryable,
                            lease.attempts >= self.settings.max_attempts,
                            group,
                        )
                        .await;
                }
            }
        }
        self.store
            .complete(lease, Utc::now())
            .await
            .inspect_err(|error| self.observe_store_error(SinkGroup::All, error))?;
        self.observations.increment(Metric::OutboxCompleted);
        self.observe_outbox(SinkGroup::All, OutboxOutcome::Completed);
        Ok(())
    }
    async fn fail(
        &self,
        lease: &LeasedEvent,
        code: FailureCode,
        dead_letter: bool,
        sink: SinkGroup,
    ) -> Result<(), OutboxError> {
        let now = Utc::now();
        tracing::warn!(
            failure = code.as_str(),
            "outbox delivery deferred or failed"
        );
        let backoff = chrono::Duration::from_std(self.settings.backoff(lease.attempts))
            .map_err(|_| OutboxError::InvalidInput)?;
        self.store
            .fail(
                lease,
                Failure {
                    code,
                    now,
                    available_at: now + backoff,
                    dead_letter,
                },
            )
            .await
            .inspect_err(|error| self.observe_store_error(sink, error))?;
        self.observations.increment(Metric::OutboxFailed);
        self.observe_outbox(
            sink,
            match code {
                FailureCode::Retryable if dead_letter => OutboxOutcome::RetryExhausted,
                FailureCode::Retryable => OutboxOutcome::Retryable,
                FailureCode::Permanent => OutboxOutcome::Permanent,
                FailureCode::UnregisteredSink => OutboxOutcome::Unregistered,
                FailureCode::ExternalDisabled => OutboxOutcome::ExternalDisabled,
                FailureCode::Cancelled => OutboxOutcome::Cancelled,
            },
        );
        Ok(())
    }
    fn observe_outbox(&self, sink: SinkGroup, outcome: OutboxOutcome) {
        self.observations.grouped.increment(SeriesKey::OutboxTotal {
            backend: self.backend,
            sink,
            outcome,
        });
    }
    fn observe_store_error(&self, sink: SinkGroup, error: &OutboxError) {
        let outcome = if *error == OutboxError::LeaseLost {
            self.observations.increment(Metric::OutboxLeaseLost);
            OutboxOutcome::LeaseLost
        } else {
            OutboxOutcome::StoreError
        };
        self.observe_outbox(sink, outcome);
    }
}

struct SinkObservation {
    observations: Arc<Metrics>,
    sink: SinkGroup,
    started: std::time::Instant,
    outcome: SinkOutcome,
    backend: DatabaseBackend,
}
impl Drop for SinkObservation {
    fn drop(&mut self) {
        let duration = self.started.elapsed();
        self.observations.duration(Metric::SinkLatencyMs, duration);
        self.observations.grouped.duration(
            SeriesKey::SinkSeconds {
                backend: self.backend,
                sink: self.sink,
                outcome: self.outcome,
            },
            duration,
        );
    }
}
struct WorkerExitObservation {
    observations: Arc<Metrics>,
    outcome: WorkerOutcome,
    backend: DatabaseBackend,
}
impl Drop for WorkerExitObservation {
    fn drop(&mut self) {
        self.observations
            .grouped
            .increment(SeriesKey::WorkerExitTotal {
                backend: self.backend,
                outcome: self.outcome,
            });
    }
}

fn safe_sink_kind(name: &str) -> &'static str {
    match name {
        "local.probe" => "local",
        "cache.invalidate" => "cache",
        "analytics.project" => "analytics",
        "mail.delivery" | "mail.alert" => "mail",
        "audit.replica" => "audit",
        _ => "other",
    }
}

/// Synthetic local sink. It performs no network I/O and retains only opaque IDs.
#[derive(Default)]
pub struct LocalSink {
    seen: Mutex<BTreeSet<String>>,
}
impl LocalSink {
    pub fn consumed(&self) -> usize {
        self.seen.lock().expect("local sink lock").len()
    }
}
impl Sink for LocalSink {
    fn name(&self) -> &str {
        "local.probe"
    }
    fn is_external(&self) -> bool {
        false
    }
    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            self.seen
                .lock()
                .map_err(|_| SinkError::Retryable)?
                .insert(event.event_id.clone());
            Ok(())
        })
    }
}

/// OS-owned advisory lock: both host modes take the same canonical database lock.
/// Lock files intentionally persist; the OS releases ownership even on process death.
#[derive(Debug)]
pub struct HostLock {
    _file: Option<File>,
}
impl HostLock {
    pub fn acquire(config: &DatabaseConfig) -> Result<Self, HostLockError> {
        if config.backend != Backend::Sqlite {
            return Ok(Self { _file: None });
        }
        Self::acquire_path(Path::new(&config.sqlite_path))
    }
    pub fn acquire_path(database: &Path) -> Result<Self, HostLockError> {
        let canonical = database
            .canonicalize()
            .map_err(|_| HostLockError::Unavailable)?;
        let mut name = canonical.into_os_string();
        name.push(".worker.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(name)
            .map_err(|_| HostLockError::Unavailable)?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|_| HostLockError::AlreadyRunning)?;
        Ok(Self { _file: Some(file) })
    }
}
#[derive(Debug, thiserror::Error)]
pub enum HostLockError {
    #[error("SQLite Worker 宿主锁不可用")]
    Unavailable,
    #[error("此 SQLite 数据库已有 Worker 宿主；内嵌与独立模式互斥")]
    AlreadyRunning,
}

#[cfg(test)]
mod observation_tests;
