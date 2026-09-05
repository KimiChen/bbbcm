//! Shared dependency-failure policy, exercised with synthetic contexts before identity
//! handlers exist. This module grants no user permission or Feature Flag bypass.
use super::*;
use std::future::Future;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitUse {
    PublicRead,
    HighRisk,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitOutcome {
    Allowed,
    Limited {
        retry_after: Duration,
    },
    /// The caller must retain this explicit absence of protection, not call it allowed.
    NoProtection,
    /// Public reads may consult the authoritative store when optional Redis fails.
    OriginFallback,
}
pub async fn check_rate_limit(
    limiter: &dyn RateLimiter,
    key: &RateLimitKey,
    policy: RateLimitPolicy,
    usage: RateLimitUse,
) -> Result<RateLimitOutcome, CacheError> {
    match limiter.check(key, policy).await {
        Ok(RateLimitDecision { allowed: true, .. }) => Ok(RateLimitOutcome::Allowed),
        Ok(RateLimitDecision { retry_after, .. }) => Ok(RateLimitOutcome::Limited { retry_after }),
        Err(CacheError::Disabled) => Ok(RateLimitOutcome::NoProtection),
        Err(CacheError::Unavailable) if usage == RateLimitUse::PublicRead => {
            Ok(RateLimitOutcome::OriginFallback)
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicReadSource {
    Cache,
    Origin,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheReadStatus {
    Hit,
    Miss,
    Disabled,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheWriteStatus {
    NotAttempted,
    Stored,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicReadResult {
    pub value: String,
    pub source: PublicReadSource,
    pub cache_read: CacheReadStatus,
    pub cache_write: CacheWriteStatus,
}

/// Read only public data under a trusted current version. The origin callback is
/// responsible for database permission/version checks and returns its actual error;
/// optional cache read/write failure never fabricates an origin response.
pub async fn read_public<F, Fut, E>(
    cache: &dyn CacheStore,
    key: &PublicCacheKey,
    origin: F,
) -> Result<PublicReadResult, E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<String, E>>,
{
    let cache_read = match cache.get(key).await {
        Ok(Some(value)) => {
            return Ok(PublicReadResult {
                value,
                source: PublicReadSource::Cache,
                cache_read: CacheReadStatus::Hit,
                cache_write: CacheWriteStatus::NotAttempted,
            });
        }
        Ok(None) => CacheReadStatus::Miss,
        Err(CacheError::Disabled) => CacheReadStatus::Disabled,
        Err(_) => CacheReadStatus::Unavailable,
    };
    let value = origin().await?;
    let cache_write = match cache.set(key, value.clone(), DEFAULT_PUBLIC_TTL).await {
        Ok(()) => CacheWriteStatus::Stored,
        Err(_) => CacheWriteStatus::Unavailable,
    };
    Ok(PublicReadResult {
        value,
        source: PublicReadSource::Origin,
        cache_read,
        cache_write,
    })
}
