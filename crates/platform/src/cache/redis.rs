//! Redis 8 adapter: bounded commands, sanitized failures, lazy reconnect and atomic
//! token-fenced scripts. Blocking wakeups use their own disposable connection.
use super::*;
use crate::config::{Environment, SecretString};
use ::redis::{Client, FromRedisValue, aio::MultiplexedConnection};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

const IO_TIMEOUT: Duration = Duration::from_secs(2);
const RENEW: &str = "if redis.call('GET', KEYS[1]) == ARGV[1] and redis.call('PTTL', KEYS[1]) > 0 then return redis.call('PEXPIRE', KEYS[1], ARGV[2]) else return 0 end";
const RELEASE: &str = "if redis.call('GET', KEYS[1]) == ARGV[1] and redis.call('PTTL', KEYS[1]) > 0 then return redis.call('UNLINK', KEYS[1]) else return 0 end";
const RATE: &str = r#"
local capacity = tonumber(ARGV[1])
local refill = tonumber(ARGV[2])
local clock = redis.call('TIME')
local now = tonumber(clock[1]) * 1000 + math.floor(tonumber(clock[2]) / 1000)
local stored = redis.call('HMGET', KEYS[1], 'tokens', 'updated', 'capacity', 'refill')
local tokens = capacity
local updated = now
if stored[1] then
  if tonumber(stored[3]) ~= capacity or tonumber(stored[4]) ~= refill then return {-1, 0} end
  if not tonumber(stored[1]) or not tonumber(stored[2]) then return {-1, 0} end
  updated = math.max(now, tonumber(stored[2]))
  tokens = math.min(capacity, tonumber(stored[1]) + math.max(0, now - tonumber(stored[2])) / refill)
end
local allowed = 0
local retry = math.ceil((1 - tokens) * refill)
if tokens >= 1 then tokens = tokens - 1; allowed = 1; retry = 0 end
redis.call('HSET', KEYS[1], 'tokens', tokens, 'updated', updated, 'capacity', capacity, 'refill', refill)
-- Preserve the bucket through a backwards server clock jump; expiry must never
-- manufacture a full bucket earlier than the stored refill clock.
redis.call('PEXPIRE', KEYS[1], math.ceil((capacity - tokens) * refill) + math.max(0, updated - now) + 1)
return {allowed, retry}
"#;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct RedisMetricsSnapshot {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_errors: u64,
    pub cache_invalidations: u64,
    pub lease_acquired: u64,
    pub lease_contended: u64,
    pub lease_renewed: u64,
    pub lease_lost: u64,
    pub lease_errors: u64,
    pub wakeup_notified: u64,
    pub wakeup_received: u64,
    pub wakeup_timeouts: u64,
    pub wakeup_errors: u64,
    pub rate_allowed: u64,
    pub rate_denied: u64,
    pub rate_errors: u64,
    pub dependency_errors: u64,
}
#[derive(Default)]
struct Metrics {
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    cache_errors: AtomicU64,
    cache_invalidations: AtomicU64,
    lease_acquired: AtomicU64,
    lease_contended: AtomicU64,
    lease_renewed: AtomicU64,
    lease_lost: AtomicU64,
    lease_errors: AtomicU64,
    wakeup_notified: AtomicU64,
    wakeup_received: AtomicU64,
    wakeup_timeouts: AtomicU64,
    wakeup_errors: AtomicU64,
    rate_allowed: AtomicU64,
    rate_denied: AtomicU64,
    rate_errors: AtomicU64,
    dependency_errors: AtomicU64,
}
fn increment(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

pub struct RedisBackend {
    client: Client,
    namespace: Namespace,
    connection: Mutex<Option<CachedConnection>>,
    next_generation: AtomicU64,
    metrics: Metrics,
}
#[derive(Clone)]
struct CachedConnection {
    connection: MultiplexedConnection,
    generation: u64,
}
impl std::fmt::Debug for RedisBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisBackend")
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}
impl RedisBackend {
    pub fn new(environment: Environment, url: &SecretString) -> Result<Self, CacheError> {
        // Enforce the same URL validation at direct adapter construction as config
        // assembly; callers cannot bypass query/TLS/placeholder restrictions.
        crate::config::validate_redis_connection(url, environment)
            .map_err(|_| CacheError::InvalidInput)?;
        let client = Client::open(&**url).map_err(|_| CacheError::InvalidInput)?;
        Ok(Self {
            client,
            namespace: Namespace::new(environment),
            connection: Mutex::new(None),
            next_generation: AtomicU64::new(1),
            metrics: Metrics::default(),
        })
    }
    pub fn namespace(&self) -> Namespace {
        self.namespace
    }
    pub fn metrics(&self) -> RedisMetricsSnapshot {
        let m = &self.metrics;
        RedisMetricsSnapshot {
            cache_hits: m.cache_hits.load(Ordering::Relaxed),
            cache_misses: m.cache_misses.load(Ordering::Relaxed),
            cache_errors: m.cache_errors.load(Ordering::Relaxed),
            cache_invalidations: m.cache_invalidations.load(Ordering::Relaxed),
            lease_acquired: m.lease_acquired.load(Ordering::Relaxed),
            lease_contended: m.lease_contended.load(Ordering::Relaxed),
            lease_renewed: m.lease_renewed.load(Ordering::Relaxed),
            lease_lost: m.lease_lost.load(Ordering::Relaxed),
            lease_errors: m.lease_errors.load(Ordering::Relaxed),
            wakeup_notified: m.wakeup_notified.load(Ordering::Relaxed),
            wakeup_received: m.wakeup_received.load(Ordering::Relaxed),
            wakeup_timeouts: m.wakeup_timeouts.load(Ordering::Relaxed),
            wakeup_errors: m.wakeup_errors.load(Ordering::Relaxed),
            rate_allowed: m.rate_allowed.load(Ordering::Relaxed),
            rate_denied: m.rate_denied.load(Ordering::Relaxed),
            rate_errors: m.rate_errors.load(Ordering::Relaxed),
            dependency_errors: m.dependency_errors.load(Ordering::Relaxed),
        }
    }
    async fn fresh_connection(&self) -> Result<MultiplexedConnection, CacheError> {
        let config = ::redis::AsyncConnectionConfig::new()
            .set_connection_timeout(Some(IO_TIMEOUT))
            .set_response_timeout(Some(IO_TIMEOUT));
        tokio::time::timeout(
            IO_TIMEOUT,
            self.client
                .get_multiplexed_async_connection_with_config(&config),
        )
        .await
        .map_err(|_| CacheError::Unavailable)?
        .map_err(|_| CacheError::Unavailable)
    }
    async fn connection(&self) -> Result<CachedConnection, CacheError> {
        let mut stored = self.connection.lock().await;
        if let Some(connection) = stored.as_ref() {
            return Ok(connection.clone());
        }
        let connection = CachedConnection {
            connection: self.fresh_connection().await?,
            generation: self.next_generation.fetch_add(1, Ordering::Relaxed),
        };
        *stored = Some(connection.clone());
        Ok(connection)
    }
    async fn query<T: FromRedisValue>(&self, command: &::redis::Cmd) -> Result<T, CacheError> {
        let mut generation = None;
        let result = tokio::time::timeout(IO_TIMEOUT, async {
            let mut connection = self.connection().await?;
            generation = Some(connection.generation);
            command
                .query_async(&mut connection.connection)
                .await
                .map_err(|_| CacheError::Unavailable)
        })
        .await
        .unwrap_or(Err(CacheError::Unavailable));
        if result.is_err() {
            increment(&self.metrics.dependency_errors);
            // A timed-out command may have executed. Never retry it here; dropping
            // the shared connection permits a fresh attempt by the semantic owner.
            // Cleanup cannot extend the operation timeout, and an old failed clone
            // must not evict a newer connection created by another operation.
            if let Ok(mut stored) = self.connection.try_lock()
                && stored
                    .as_ref()
                    .is_some_and(|current| Some(current.generation) == generation)
            {
                *stored = None;
            }
        }
        result
    }
    pub async fn health(&self) -> Result<(), CacheError> {
        let pong: String = self.query(&::redis::cmd("PING")).await?;
        if pong == "PONG" {
            Ok(())
        } else {
            Err(CacheError::Unavailable)
        }
    }
    /// Explicit operator capability. The backend never accepts a raw prefix and
    /// never uses database-wide deletion. Concurrent repopulation remains possible.
    pub async fn clear_namespace(&self) -> Result<u64, CacheError> {
        self.unlink_prefix(&self.namespace.prefix()).await
    }
    async fn unlink_prefix(&self, prefix: &str) -> Result<u64, CacheError> {
        if !prefix.starts_with(&self.namespace.prefix()) {
            return Err(CacheError::InvalidInput);
        }
        let mut cursor = 0u64;
        let mut deleted = 0;
        loop {
            let (next, keys): (u64, Vec<String>) = self
                .query(
                    ::redis::cmd("SCAN")
                        .arg(cursor)
                        .arg("MATCH")
                        .arg(format!("{prefix}*"))
                        .arg("COUNT")
                        .arg(100),
                )
                .await?;
            // Treat server responses defensively: do not delete an out-of-scope key
            // even if a faulty adapter / server returns one despite MATCH.
            if keys
                .iter()
                .any(|key| !key.starts_with(prefix) || !self.namespace.contains(key))
            {
                return Err(CacheError::Unavailable);
            }
            for batch in keys.chunks(100) {
                let count: u64 = self.query(::redis::cmd("UNLINK").arg(batch)).await?;
                deleted += count;
            }
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        Ok(deleted)
    }
}
impl CacheStore for RedisBackend {
    fn get<'a>(
        &'a self,
        key: &'a PublicCacheKey,
    ) -> BoxFuture<'a, Result<Option<String>, CacheError>> {
        Box::pin(async move {
            let result = self
                .query(::redis::cmd("GET").arg(self.namespace.cache_key(key)))
                .await;
            match &result {
                Ok(Some(_)) => increment(&self.metrics.cache_hits),
                Ok(None) => increment(&self.metrics.cache_misses),
                Err(_) => increment(&self.metrics.cache_errors),
            }
            result
        })
    }
    fn set<'a>(
        &'a self,
        key: &'a PublicCacheKey,
        value: String,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<(), CacheError>> {
        Box::pin(async move {
            validate_cache_write(&value, ttl)?;
            let result = self
                .query(
                    ::redis::cmd("SET")
                        .arg(self.namespace.cache_key(key))
                        .arg(value)
                        .arg("PX")
                        .arg(ttl_millis(ttl)?),
                )
                .await;
            if result.is_err() {
                increment(&self.metrics.cache_errors);
            }
            result
        })
    }
    fn invalidate<'a>(&'a self, entity: &'a CacheEntity) -> BoxFuture<'a, Result<u64, CacheError>> {
        Box::pin(async move {
            let result = self
                .unlink_prefix(&self.namespace.cache_entity_prefix(entity))
                .await;
            if result.is_ok() {
                increment(&self.metrics.cache_invalidations);
            } else {
                increment(&self.metrics.cache_errors);
            }
            result
        })
    }
}
impl Lease for RedisBackend {
    fn acquire<'a>(
        &'a self,
        key: &'a LeaseKey,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<Option<LeaseToken>, CacheError>> {
        Box::pin(async move {
            let ttl = ttl_millis(ttl)?;
            let token = LeaseToken::fresh();
            let result: Result<Option<String>, _> = self
                .query(
                    ::redis::cmd("SET")
                        .arg(self.namespace.lease_key(key))
                        .arg(&token.0)
                        .arg("NX")
                        .arg("PX")
                        .arg(ttl),
                )
                .await;
            match result {
                Ok(Some(_)) => {
                    increment(&self.metrics.lease_acquired);
                    Ok(Some(token))
                }
                Ok(None) => {
                    increment(&self.metrics.lease_contended);
                    Ok(None)
                }
                Err(error) => {
                    increment(&self.metrics.lease_errors);
                    Err(error)
                }
            }
        })
    }
    fn renew<'a>(
        &'a self,
        key: &'a LeaseKey,
        token: &'a LeaseToken,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<bool, CacheError>> {
        Box::pin(async move {
            let result: Result<i64, _> = self
                .query(
                    ::redis::cmd("EVAL")
                        .arg(RENEW)
                        .arg(1)
                        .arg(self.namespace.lease_key(key))
                        .arg(&token.0)
                        .arg(ttl_millis(ttl)?),
                )
                .await;
            match result {
                Ok(1) => {
                    increment(&self.metrics.lease_renewed);
                    Ok(true)
                }
                Ok(_) => {
                    increment(&self.metrics.lease_lost);
                    Ok(false)
                }
                Err(error) => {
                    increment(&self.metrics.lease_errors);
                    Err(error)
                }
            }
        })
    }
    fn release<'a>(
        &'a self,
        key: &'a LeaseKey,
        token: &'a LeaseToken,
    ) -> BoxFuture<'a, Result<bool, CacheError>> {
        Box::pin(async move {
            let result: Result<i64, _> = self
                .query(
                    ::redis::cmd("EVAL")
                        .arg(RELEASE)
                        .arg(1)
                        .arg(self.namespace.lease_key(key))
                        .arg(&token.0),
                )
                .await;
            match result {
                Ok(1) => Ok(true),
                Ok(_) => {
                    increment(&self.metrics.lease_lost);
                    Ok(false)
                }
                Err(error) => {
                    increment(&self.metrics.lease_errors);
                    Err(error)
                }
            }
        })
    }
}
impl RateLimiter for RedisBackend {
    fn check<'a>(
        &'a self,
        key: &'a RateLimitKey,
        policy: RateLimitPolicy,
    ) -> BoxFuture<'a, Result<RateLimitDecision, CacheError>> {
        Box::pin(async move {
            let result: Result<(i64, u64), _> = self
                .query(
                    ::redis::cmd("EVAL")
                        .arg(RATE)
                        .arg(1)
                        .arg(self.namespace.rate_limit_key(key))
                        .arg(policy.capacity)
                        .arg(policy.refill.as_millis() as u64),
                )
                .await;
            match result {
                Ok((allowed @ 0..=1, retry)) => {
                    if allowed == 1 {
                        increment(&self.metrics.rate_allowed);
                    } else {
                        increment(&self.metrics.rate_denied);
                    }
                    Ok(RateLimitDecision {
                        allowed: allowed == 1,
                        retry_after: Duration::from_millis(retry),
                    })
                }
                Ok(_) => {
                    increment(&self.metrics.rate_errors);
                    Err(CacheError::InvalidInput)
                }
                Err(error) => {
                    increment(&self.metrics.rate_errors);
                    Err(error)
                }
            }
        })
    }
}
impl Wakeup for RedisBackend {
    fn notify(&self) -> BoxFuture<'_, Result<(), CacheError>> {
        Box::pin(async move {
            let result: Result<String, _> = self
                .query(
                    ::redis::cmd("XADD")
                        .arg(self.namespace.wakeup_key())
                        .arg("MAXLEN")
                        .arg(1)
                        .arg("*")
                        .arg("hint")
                        .arg("1"),
                )
                .await;
            match result {
                Ok(_) => {
                    increment(&self.metrics.wakeup_notified);
                    Ok(())
                }
                Err(error) => {
                    increment(&self.metrics.wakeup_errors);
                    Err(error)
                }
            }
        })
    }
    fn wait(&self, timeout: Duration) -> BoxFuture<'_, Result<bool, CacheError>> {
        Box::pin(async move {
            if timeout.is_zero() {
                return Ok(false);
            }
            let millis = ttl_millis(timeout)?;
            let mut connected = false;
            let result = tokio::time::timeout(timeout, async {
                // A separate connection avoids head-of-line blocking of cache/lease
                // commands. Dropping this future closes that connection's driver.
                let mut connection = self.fresh_connection().await?;
                connected = true;
                connection.set_response_timeout(timeout + IO_TIMEOUT);
                let value: ::redis::Value = ::redis::cmd("XREAD")
                    .arg("BLOCK")
                    .arg(millis)
                    .arg("COUNT")
                    .arg(1)
                    .arg("STREAMS")
                    .arg(self.namespace.wakeup_key())
                    .arg("$")
                    .query_async(&mut connection)
                    .await
                    .map_err(|_| CacheError::Unavailable)?;
                Ok(!matches!(value, ::redis::Value::Nil))
            })
            .await
            .unwrap_or({
                if connected {
                    Ok(false)
                } else {
                    Err(CacheError::Unavailable)
                }
            });
            match &result {
                Ok(true) => increment(&self.metrics.wakeup_received),
                Ok(false) => increment(&self.metrics.wakeup_timeouts),
                Err(_) => {
                    increment(&self.metrics.wakeup_errors);
                    increment(&self.metrics.dependency_errors);
                }
            }
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn timed_out_operation_does_not_block_on_connection_cleanup() {
        let backend = RedisBackend::new(
            Environment::Test,
            &SecretString::new("redis://127.0.0.1:1/0".into()),
        )
        .unwrap();
        // A concurrent connection attempt owns the mutex. No Redis socket is used.
        // Holding it across the outer deadline reproduces the former cleanup deadlock.
        let _connecting = backend.connection.lock().await;
        let result =
            tokio::time::timeout(IO_TIMEOUT + Duration::from_millis(200), backend.health()).await;
        assert_eq!(result.unwrap(), Err(CacheError::Unavailable));
    }

    #[tokio::test]
    async fn wakeup_deadline_includes_connection_setup() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let stalled = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
            drop(stream);
        });
        let backend = RedisBackend::new(
            Environment::Test,
            &SecretString::new(format!("redis://127.0.0.1:{port}/0")),
        )
        .unwrap();
        let result = tokio::time::timeout(
            Duration::from_millis(200),
            backend.wait(Duration::from_millis(20)),
        )
        .await;
        assert_eq!(result.unwrap(), Err(CacheError::Unavailable));
        stalled.abort();
    }
}
