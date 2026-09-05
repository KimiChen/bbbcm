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
pub use platform::outbox::{BoxFuture, OutboxEvent, Sink, SinkError};
use platform::outbox::{Claim, Failure, FailureCode, LeasedEvent, OutboxError, OutboxStore};
use tokio_util::sync::CancellationToken;

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
        })
    }
    pub async fn run(&self, cancel: CancellationToken) -> Result<(), OutboxError> {
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            self.tick(&cancel).await?;
            tokio::select! { () = cancel.cancelled() => return Ok(()), () = tokio::time::sleep(self.settings.poll_interval) => {} }
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
            .await?;
        for (index, lease) in leases.iter().enumerate() {
            if cancel.is_cancelled() {
                self.release(&leases[index..]).await?;
                break;
            }
            match self.consume(lease, &leases[index..], cancel).await {
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
            self.store.renew(lease, now, until).await?;
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
                Ok(()) | Err(OutboxError::LeaseLost) => {}
                Err(error) => return Err(error),
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
        self.renew_batch(batch).await?;
        for name in &lease.pending_sinks {
            let Some(sink) = self.registry.sinks.get(name) else {
                return self.fail(lease, FailureCode::UnregisteredSink, true).await;
            };
            if sink.is_external() && !self.effects.allowed() {
                return self.fail(lease, FailureCode::ExternalDisabled, false).await;
            }
            let delivered = sink.deliver(&lease.event);
            tokio::pin!(delivered);
            let mut renewal = tokio::time::interval(self.settings.renew_interval);
            renewal.tick().await;
            let result = loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => { self.release(std::slice::from_ref(lease)).await?; return Ok(()); }
                    _ = renewal.tick() => {
                        self.renew_batch(batch).await?;
                        if sink.is_external() && !self.effects.allowed() { return self.fail(lease, FailureCode::ExternalDisabled, false).await; }
                    }
                    result = &mut delivered => break result,
                }
            };
            match result {
                Ok(()) => self.store.delivery_done(lease, name, Utc::now()).await?,
                Err(SinkError::Permanent) => {
                    return self.fail(lease, FailureCode::Permanent, true).await;
                }
                Err(SinkError::Retryable) => {
                    return self
                        .fail(
                            lease,
                            FailureCode::Retryable,
                            lease.attempts >= self.settings.max_attempts,
                        )
                        .await;
                }
            }
        }
        self.store.complete(lease, Utc::now()).await
    }
    async fn fail(
        &self,
        lease: &LeasedEvent,
        code: FailureCode,
        dead_letter: bool,
    ) -> Result<(), OutboxError> {
        let now = Utc::now();
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
