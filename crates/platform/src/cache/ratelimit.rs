//! In-memory token buckets are explicitly for tests / local injection. The runtime
//! assembler never chooses them: no Redis means no rate-limiting capability (DEC-024).
use super::{BoxFuture, CacheError, DEFAULT_MAX_ENTRIES, RateLimitKey, RateLimiter};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitPolicy {
    pub(super) capacity: u32,
    pub(super) refill: Duration,
}
impl RateLimitPolicy {
    /// One token is restored per refill interval. Product thresholds are selected by
    /// trusted callers; no registration/login limits are invented by this platform.
    pub fn new(capacity: u32, refill: Duration) -> Result<Self, CacheError> {
        super::ttl_millis(refill)?;
        if capacity == 0
            || capacity > 1_000_000
            || refill.as_millis().saturating_mul(u128::from(capacity)) > 86_400_000
        {
            return Err(CacheError::InvalidInput);
        }
        Ok(Self { capacity, refill })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitDecision {
    pub allowed: bool,
    pub retry_after: Duration,
}

pub struct DisabledRateLimiter;
impl RateLimiter for DisabledRateLimiter {
    fn check<'a>(
        &'a self,
        _key: &'a RateLimitKey,
        _policy: RateLimitPolicy,
    ) -> BoxFuture<'a, Result<RateLimitDecision, CacheError>> {
        Box::pin(async { Err(CacheError::Disabled) })
    }
}
#[derive(Default)]
pub struct TokenBucketLimiter {
    buckets: Mutex<HashMap<RateLimitKey, Bucket>>,
    max_entries: Option<usize>,
}
struct Bucket {
    tokens: f64,
    updated: Instant,
    policy: RateLimitPolicy,
}
impl TokenBucketLimiter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = Some(max_entries.max(1));
        self
    }
    pub fn tracked_keys(&self) -> Result<usize, CacheError> {
        Ok(self
            .buckets
            .lock()
            .map_err(|_| CacheError::Unavailable)?
            .len())
    }
}
impl RateLimiter for TokenBucketLimiter {
    fn check<'a>(
        &'a self,
        key: &'a RateLimitKey,
        policy: RateLimitPolicy,
    ) -> BoxFuture<'a, Result<RateLimitDecision, CacheError>> {
        Box::pin(async move {
            let now = Instant::now();
            let mut buckets = self.buckets.lock().map_err(|_| CacheError::Unavailable)?;
            if !buckets.contains_key(key)
                && buckets.len() >= self.max_entries.unwrap_or(DEFAULT_MAX_ENTRIES)
            {
                buckets.retain(|_, bucket| {
                    replenished(bucket, now) < f64::from(bucket.policy.capacity)
                });
                if buckets.len() >= self.max_entries.unwrap_or(DEFAULT_MAX_ENTRIES) {
                    return Ok(RateLimitDecision {
                        allowed: false,
                        retry_after: policy.refill,
                    });
                }
            }
            let bucket = buckets.entry(key.clone()).or_insert(Bucket {
                tokens: f64::from(policy.capacity),
                updated: now,
                policy,
            });
            // A caller changing policy under the same dimension cannot reset a bucket.
            if bucket.policy != policy {
                return Err(CacheError::InvalidInput);
            }
            bucket.tokens = replenished(bucket, now).min(f64::from(policy.capacity));
            bucket.updated = now;
            let allowed = bucket.tokens >= 1.0;
            if allowed {
                bucket.tokens -= 1.0;
            }
            let retry_after = if allowed {
                Duration::ZERO
            } else {
                Duration::from_secs_f64((1.0 - bucket.tokens) * policy.refill.as_secs_f64())
            };
            Ok(RateLimitDecision {
                allowed,
                retry_after,
            })
        })
    }
}
fn replenished(bucket: &Bucket, now: Instant) -> f64 {
    bucket.tokens
        + now.duration_since(bucket.updated).as_secs_f64() / bucket.policy.refill.as_secs_f64()
}
