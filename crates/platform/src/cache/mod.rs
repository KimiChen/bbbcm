//! A05 asynchronous contracts. All state here is disposable; database transactions,
//! permissions, idempotency and Outbox delivery remain the authority.
mod keys;
mod memory;
mod policy;
pub mod ratelimit;
mod redis;

pub use keys::{CacheEntity, CacheVisibility, LeaseKey, Namespace, PublicCacheKey, RateLimitKey};
pub use memory::{MemoryCache, MemoryLease, MemoryWakeup, PollingWakeup};
pub use policy::{
    CacheReadStatus, CacheWriteStatus, PublicReadResult, PublicReadSource, RateLimitOutcome,
    RateLimitUse, check_rate_limit, read_public,
};
pub use ratelimit::{DisabledRateLimiter, RateLimitDecision, RateLimitPolicy, TokenBucketLimiter};
pub use redis::{RedisBackend, RedisMetricsSnapshot};

use crate::config::Config;
pub use crate::outbox::BoxFuture;
use crate::outbox::{OutboxEvent, Sink, SinkError};
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_MAX_ENTRIES: usize = 100_000;
pub const DEFAULT_PUBLIC_TTL: Duration = Duration::from_secs(30);
pub const MAX_CACHE_VALUE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CacheError {
    #[error("此依赖未启用，不提供限流能力")]
    Disabled,
    #[error("可重建依赖不可用（底层信息已脱敏）")]
    Unavailable,
    #[error("可重建依赖参数非法")]
    InvalidInput,
}

/// Keys require a trusted current version and public visibility. Permission tightening,
/// deletion and high-risk reads must consult the database, never this cache.
pub trait CacheStore: Send + Sync {
    fn get<'a>(
        &'a self,
        key: &'a PublicCacheKey,
    ) -> BoxFuture<'a, Result<Option<String>, CacheError>>;
    fn set<'a>(
        &'a self,
        key: &'a PublicCacheKey,
        value: String,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<(), CacheError>>;
    /// Call only after a confirmed commit or from an already committed Outbox event.
    fn invalidate<'a>(&'a self, entity: &'a CacheEntity) -> BoxFuture<'a, Result<u64, CacheError>>;
}

/// The only channel is the worker's Outbox hint. Futures are cancellation-safe;
/// timeout, missed notifications and dependency errors must fall back to DB polling.
pub trait Wakeup: Send + Sync {
    fn notify(&self) -> BoxFuture<'_, Result<(), CacheError>>;
    fn wait(&self, timeout: Duration) -> BoxFuture<'_, Result<bool, CacheError>>;
}

#[derive(Clone, PartialEq, Eq)]
pub struct LeaseToken(String);
impl LeaseToken {
    fn fresh() -> Self {
        Self(crate::transaction::new_operation_id())
    }
}
impl std::fmt::Debug for LeaseToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LeaseToken([REDACTED])")
    }
}

/// Short coordination leases never replace authoritative database locks or constraints.
pub trait Lease: Send + Sync {
    fn acquire<'a>(
        &'a self,
        key: &'a LeaseKey,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<Option<LeaseToken>, CacheError>>;
    fn renew<'a>(
        &'a self,
        key: &'a LeaseKey,
        token: &'a LeaseToken,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<bool, CacheError>>;
    fn release<'a>(
        &'a self,
        key: &'a LeaseKey,
        token: &'a LeaseToken,
    ) -> BoxFuture<'a, Result<bool, CacheError>>;
}

pub trait RateLimiter: Send + Sync {
    /// Disabled is different from allowed. Redis errors must fail closed at high-risk
    /// operations; ordinary public reads may proceed to the authoritative store.
    fn check<'a>(
        &'a self,
        key: &'a RateLimitKey,
        policy: RateLimitPolicy,
    ) -> BoxFuture<'a, Result<RateLimitDecision, CacheError>>;
}

#[derive(Clone)]
pub struct RedisRuntime {
    pub cache: Arc<dyn CacheStore>,
    pub wakeup: Arc<dyn Wakeup>,
    pub lease: Arc<dyn Lease>,
    pub rate_limiter: Arc<dyn RateLimiter>,
    pub redis: Option<Arc<RedisBackend>>,
}
impl RedisRuntime {
    /// Assembly checks configuration but connects lazily. Optional Redis outages do
    /// not turn public reads or the whole process into a startup failure.
    pub fn from_config(config: &Config) -> Result<Self, CacheError> {
        config.validate().map_err(|_| CacheError::InvalidInput)?;
        if config.features.redis {
            let backend = Arc::new(RedisBackend::new(
                config.server.environment,
                config.redis.url.as_ref().ok_or(CacheError::InvalidInput)?,
            )?);
            Ok(Self {
                cache: backend.clone(),
                wakeup: backend.clone(),
                lease: backend.clone(),
                rate_limiter: backend.clone(),
                redis: Some(backend),
            })
        } else {
            Ok(Self {
                cache: Arc::new(MemoryCache::default()),
                wakeup: Arc::new(PollingWakeup),
                lease: Arc::new(MemoryLease::default()),
                rate_limiter: Arc::new(DisabledRateLimiter),
                redis: None,
            })
        }
    }
}

/// Applications register this sink on their existing worker. No direct HTTP write
/// endpoint exists. Outbox ownership/commit/delivery deduplication is the worker's job.
pub struct CacheInvalidationSink {
    cache: Arc<dyn CacheStore>,
}
impl CacheInvalidationSink {
    pub fn new(cache: Arc<dyn CacheStore>) -> Self {
        Self { cache }
    }
}
impl Sink for CacheInvalidationSink {
    fn name(&self) -> &str {
        "cache.invalidate"
    }
    fn is_external(&self) -> bool {
        false
    }
    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            event.validate().map_err(|_| SinkError::Permanent)?;
            if event.event_type != "cache.invalidate" {
                return Err(SinkError::Permanent);
            }
            let entity = CacheEntity::new(&event.aggregate_type, &event.aggregate_id)
                .map_err(|_| SinkError::Permanent)?;
            self.cache
                .invalidate(&entity)
                .await
                .map_err(|error| match error {
                    CacheError::InvalidInput => SinkError::Permanent,
                    _ => SinkError::Retryable,
                })?;
            Ok(())
        })
    }
}

fn ttl_millis(ttl: Duration) -> Result<u64, CacheError> {
    // Bounds also prevent overflowing Instant / Redis's signed millisecond clock.
    if ttl < Duration::from_millis(1)
        || ttl > Duration::from_secs(86_400)
        || !ttl.subsec_nanos().is_multiple_of(1_000_000)
    {
        return Err(CacheError::InvalidInput);
    }
    Ok(ttl.as_millis() as u64)
}
fn validate_cache_write(value: &str, ttl: Duration) -> Result<(), CacheError> {
    ttl_millis(ttl)?;
    if ttl > DEFAULT_PUBLIC_TTL || value.len() > MAX_CACHE_VALUE_BYTES {
        return Err(CacheError::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
