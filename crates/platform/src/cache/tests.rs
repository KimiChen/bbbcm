use super::*;
use crate::config::{Environment, SecretString};

fn key(id: &str, version: u64) -> PublicCacheKey {
    PublicCacheKey::new(
        CacheEntity::new("probe", id).unwrap(),
        version,
        CacheVisibility::Public,
    )
    .unwrap()
}
fn policy() -> RateLimitPolicy {
    RateLimitPolicy::new(2, Duration::from_secs(1)).unwrap()
}

#[test]
fn controlled_keys_reject_namespace_injection_private_and_unversioned_content() {
    let namespace = Namespace::new(Environment::Test);
    let a = namespace.cache_key(&key("1", 1));
    let b = namespace.cache_key(&key("1", 2));
    assert!(a.starts_with("bcm:test:v1:cache:probe:"));
    assert!(a.contains(":public:1"));
    assert_ne!(a, b);
    assert!(!Namespace::new(Environment::Production).contains(&a));
    for bad in ["", "*", "probe:other", "../probe", "UPPER"] {
        assert!(CacheEntity::new(bad, "1").is_err());
    }
    for bad in ["0", "-1", "01", "secret@example.invalid", "*"] {
        assert!(CacheEntity::new("probe", bad).is_err());
    }
    assert!(
        PublicCacheKey::new(
            CacheEntity::new("probe", "1").unwrap(),
            0,
            CacheVisibility::Public
        )
        .is_err()
    );
    assert!(
        PublicCacheKey::new(
            CacheEntity::new("probe", "1").unwrap(),
            1,
            CacheVisibility::Private
        )
        .is_err()
    );
    let rate_key = namespace
        .rate_limit_key(&RateLimitKey::new("synthetic", "synthetic@example.invalid").unwrap());
    assert!(rate_key.starts_with("bcm:test:v1:ratelimit:synthetic:"));
    assert!(!rate_key.contains("example.invalid"));
    assert!(RateLimitKey::new("x:*", "1").is_err());
    assert!(LeaseKey::new("x", "\n").is_err());
}

#[tokio::test]
async fn memory_cache_ttl_lru_all_version_invalidation_and_input_bounds() {
    let cache = MemoryCache::with_capacity(2);
    cache
        .set(&key("1", 1), "a".into(), DEFAULT_PUBLIC_TTL)
        .await
        .unwrap();
    cache
        .set(&key("2", 1), "b".into(), DEFAULT_PUBLIC_TTL)
        .await
        .unwrap();
    assert_eq!(cache.get(&key("1", 1)).await.unwrap().as_deref(), Some("a"));
    cache
        .set(&key("3", 1), "c".into(), DEFAULT_PUBLIC_TTL)
        .await
        .unwrap();
    assert_eq!(
        cache.get(&key("2", 1)).await.unwrap(),
        None,
        "least recently used key must be evicted"
    );
    assert_eq!(cache.len().unwrap(), 2);
    cache
        .set(&key("1", 2), "v2".into(), DEFAULT_PUBLIC_TTL)
        .await
        .unwrap();
    assert_eq!(cache.invalidate(key("1", 1).entity()).await.unwrap(), 1);
    assert_eq!(cache.get(&key("1", 2)).await.unwrap(), None);
    cache
        .set(&key("1", 3), "short".into(), Duration::from_millis(1))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(cache.get(&key("1", 3)).await.unwrap(), None);
    for ttl in [Duration::ZERO, Duration::from_secs(31), Duration::MAX] {
        assert_eq!(
            cache.set(&key("1", 1), "x".into(), ttl).await,
            Err(CacheError::InvalidInput)
        );
    }
    assert!(
        cache
            .set(
                &key("1", 1),
                "x".repeat(MAX_CACHE_VALUE_BYTES + 1),
                DEFAULT_PUBLIC_TTL
            )
            .await
            .is_err()
    );
    let cache = MemoryCache::default();
    for version in 1..=3 {
        cache
            .set(&key("1", version), "x".into(), DEFAULT_PUBLIC_TTL)
            .await
            .unwrap();
    }
    assert_eq!(cache.invalidate(key("1", 1).entity()).await.unwrap(), 3);
    assert!(cache.is_empty().unwrap());
}

#[tokio::test]
async fn leases_reject_expiry_and_old_owner_after_reacquire() {
    let lease = MemoryLease::default();
    let key = LeaseKey::new("probe", "1").unwrap();
    assert_eq!(
        lease.acquire(&key, Duration::ZERO).await,
        Err(CacheError::InvalidInput)
    );
    let token = lease
        .acquire(&key, Duration::from_millis(1))
        .await
        .unwrap()
        .unwrap();
    assert!(!format!("{token:?}").contains(&token.0));
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        !lease
            .renew(&key, &token, Duration::from_secs(1))
            .await
            .unwrap()
    );
    assert!(!lease.release(&key, &token).await.unwrap());
    let next = lease
        .acquire(&key, Duration::from_secs(1))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(token, next);
    assert!(
        lease
            .acquire(&key, Duration::from_secs(1))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !lease
            .renew(&key, &token, Duration::from_secs(1))
            .await
            .unwrap()
    );
    assert!(!lease.release(&key, &token).await.unwrap());
    assert!(
        lease
            .renew(&key, &next, Duration::from_secs(1))
            .await
            .unwrap()
    );
    assert!(lease.release(&key, &next).await.unwrap());
}

#[tokio::test]
async fn async_wakeup_timeout_notify_and_cancellation() {
    let wakeup = MemoryWakeup::new();
    assert!(!wakeup.wait(Duration::from_millis(1)).await.unwrap());
    wakeup.notify().await.unwrap();
    assert!(wakeup.wait(Duration::from_millis(5)).await.unwrap());
    assert!(!wakeup.wait(Duration::from_millis(1)).await.unwrap());
    assert!(
        tokio::time::timeout(
            Duration::from_millis(5),
            wakeup.wait(Duration::from_secs(5))
        )
        .await
        .is_err()
    );
    wakeup.notify().await.unwrap();
    assert!(wakeup.wait(Duration::from_millis(5)).await.unwrap());
    let polling = PollingWakeup;
    polling.notify().await.unwrap();
    assert!(!polling.wait(Duration::from_millis(1)).await.unwrap());
}

#[tokio::test]
async fn test_limiter_replenishes_is_bounded_and_does_not_reset_on_policy_change() {
    let limiter = TokenBucketLimiter::new().with_max_entries(2);
    let key = RateLimitKey::new("synthetic", "1").unwrap();
    let policy = RateLimitPolicy::new(1, Duration::from_millis(30)).unwrap();
    assert!(limiter.check(&key, policy).await.unwrap().allowed);
    let denied = limiter.check(&key, policy).await.unwrap();
    assert!(!denied.allowed);
    assert!(denied.retry_after > Duration::ZERO);
    assert_eq!(
        limiter
            .check(
                &key,
                RateLimitPolicy::new(2, Duration::from_secs(1)).unwrap()
            )
            .await,
        Err(CacheError::InvalidInput)
    );
    for n in 2..=30 {
        let key = RateLimitKey::new("synthetic", &n.to_string()).unwrap();
        limiter.check(&key, policy).await.unwrap();
    }
    assert_eq!(limiter.tracked_keys().unwrap(), 2);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(limiter.check(&key, policy).await.unwrap().allowed);
    assert!(
        limiter
            .check(&RateLimitKey::new("synthetic", "new").unwrap(), policy)
            .await
            .unwrap()
            .allowed
    );
    for (capacity, refill) in [
        (0, Duration::from_secs(1)),
        (1, Duration::ZERO),
        (1, Duration::from_micros(1500)),
        (u32::MAX, Duration::from_secs(1)),
        (100, Duration::from_secs(86_400)),
    ] {
        assert!(RateLimitPolicy::new(capacity, refill).is_err());
    }
}

#[tokio::test]
async fn assembly_distinguishes_disabled_and_enabled_but_unavailable_without_secret_errors() {
    let mut config = Config::default();
    let disabled = RedisRuntime::from_config(&config).unwrap();
    let rate_key = RateLimitKey::new("synthetic", "1").unwrap();
    assert!(disabled.redis.is_none());
    assert_eq!(
        disabled.rate_limiter.check(&rate_key, policy()).await,
        Err(CacheError::Disabled)
    );
    disabled
        .cache
        .set(&key("1", 1), "public".into(), DEFAULT_PUBLIC_TTL)
        .await
        .unwrap();
    assert_eq!(
        disabled.cache.get(&key("1", 1)).await.unwrap().as_deref(),
        Some("public")
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // Controlled TCP peer rejects every connection. This tests an actual socket
    // failure, not Redis protocol/atomicity (the latter is the ignored integration).
    let reject = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            drop(stream);
        }
    });
    config.features.redis = true;
    config.redis.url = Some(SecretString::new(format!(
        "redis://:synthetic-secret@127.0.0.1:{port}/0"
    )));
    let enabled = RedisRuntime::from_config(&config).unwrap();
    let error = enabled
        .rate_limiter
        .check(&rate_key, policy())
        .await
        .unwrap_err();
    assert_eq!(error, CacheError::Unavailable);
    assert!(!format!("{error:?} {error}").contains("synthetic-secret"));
    assert!(!format!("{:?}", enabled.redis).contains("synthetic-secret"));
    assert_eq!(
        enabled.cache.get(&key("1", 1)).await,
        Err(CacheError::Unavailable)
    );
    assert_eq!(
        enabled.wakeup.wait(Duration::from_millis(20)).await,
        Err(CacheError::Unavailable)
    );
    assert!(enabled.redis.as_ref().unwrap().metrics().dependency_errors >= 3);
    reject.abort();
}

#[tokio::test]
async fn invalidation_sink_retries_failures_and_rejects_unrelated_events() {
    let cache = Arc::new(MemoryCache::default());
    cache
        .set(&key("1", 1), "public".into(), DEFAULT_PUBLIC_TTL)
        .await
        .unwrap();
    let sink = CacheInvalidationSink::new(cache.clone());
    let mut event = OutboxEvent {
        event_id: crate::transaction::new_operation_id(),
        aggregate_type: "probe".into(),
        aggregate_id: "1".into(),
        event_type: "cache.invalidate".into(),
        version: 1,
        occurred_at: chrono::Utc::now(),
        payload: crate::outbox::SafePayload::empty(),
        trace_id: String::new(),
    };
    sink.deliver(&event).await.unwrap();
    sink.deliver(&event).await.unwrap();
    assert_eq!(cache.get(&key("1", 1)).await.unwrap(), None);
    event.event_type = "other".into();
    assert_eq!(sink.deliver(&event).await, Err(SinkError::Permanent));
    struct Failing;
    impl CacheStore for Failing {
        fn get<'a>(
            &'a self,
            _key: &'a PublicCacheKey,
        ) -> BoxFuture<'a, Result<Option<String>, CacheError>> {
            Box::pin(async { Err(CacheError::Unavailable) })
        }
        fn set<'a>(
            &'a self,
            _key: &'a PublicCacheKey,
            _value: String,
            _ttl: Duration,
        ) -> BoxFuture<'a, Result<(), CacheError>> {
            Box::pin(async { Err(CacheError::Unavailable) })
        }
        fn invalidate<'a>(
            &'a self,
            _entity: &'a CacheEntity,
        ) -> BoxFuture<'a, Result<u64, CacheError>> {
            Box::pin(async { Err(CacheError::Unavailable) })
        }
    }
    event.event_type = "cache.invalidate".into();
    assert_eq!(
        CacheInvalidationSink::new(Arc::new(Failing))
            .deliver(&event)
            .await,
        Err(SinkError::Retryable)
    );
}

#[tokio::test]
async fn synthetic_high_risk_fails_closed_public_reads_return_real_origin_and_disabled_is_explicit()
{
    struct Unavailable;
    impl RateLimiter for Unavailable {
        fn check<'a>(
            &'a self,
            _key: &'a RateLimitKey,
            _policy: RateLimitPolicy,
        ) -> BoxFuture<'a, Result<RateLimitDecision, CacheError>> {
            Box::pin(async { Err(CacheError::Unavailable) })
        }
    }
    impl CacheStore for Unavailable {
        fn get<'a>(
            &'a self,
            _key: &'a PublicCacheKey,
        ) -> BoxFuture<'a, Result<Option<String>, CacheError>> {
            Box::pin(async { Err(CacheError::Unavailable) })
        }
        fn set<'a>(
            &'a self,
            _key: &'a PublicCacheKey,
            _value: String,
            _ttl: Duration,
        ) -> BoxFuture<'a, Result<(), CacheError>> {
            Box::pin(async { Err(CacheError::Unavailable) })
        }
        fn invalidate<'a>(
            &'a self,
            _entity: &'a CacheEntity,
        ) -> BoxFuture<'a, Result<u64, CacheError>> {
            Box::pin(async { Err(CacheError::Unavailable) })
        }
    }
    let rate_key = RateLimitKey::new("synthetic", "1").unwrap();
    assert_eq!(
        check_rate_limit(&Unavailable, &rate_key, policy(), RateLimitUse::HighRisk).await,
        Err(CacheError::Unavailable)
    );
    assert_eq!(
        check_rate_limit(&Unavailable, &rate_key, policy(), RateLimitUse::PublicRead).await,
        Ok(RateLimitOutcome::OriginFallback)
    );
    for usage in [RateLimitUse::HighRisk, RateLimitUse::PublicRead] {
        assert_eq!(
            check_rate_limit(&DisabledRateLimiter, &rate_key, policy(), usage).await,
            Ok(RateLimitOutcome::NoProtection)
        );
    }
    let mut reads = 0;
    let result = read_public(&Unavailable, &key("1", 1), || async {
        reads += 1;
        Ok::<_, CacheError>("authoritative synthetic public value".into())
    })
    .await
    .unwrap();
    assert_eq!(reads, 1);
    assert_eq!(result.value, "authoritative synthetic public value");
    assert_eq!(result.source, PublicReadSource::Origin);
    assert_eq!(result.cache_read, CacheReadStatus::Unavailable);
    assert_eq!(result.cache_write, CacheWriteStatus::Unavailable);
    assert_eq!(
        read_public(&Unavailable, &key("1", 1), || async {
            Err::<String, _>(CacheError::Unavailable)
        })
        .await,
        Err(CacheError::Unavailable)
    );
    let cache = MemoryCache::new();
    let result = read_public(&cache, &key("1", 1), || async {
        Ok::<_, CacheError>("from-db".into())
    })
    .await
    .unwrap();
    assert_eq!(result.cache_read, CacheReadStatus::Miss);
    assert_eq!(result.cache_write, CacheWriteStatus::Stored);
    let result = read_public(&cache, &key("1", 1), || async {
        Err::<String, _>(CacheError::Unavailable)
    })
    .await
    .unwrap();
    assert_eq!(result.source, PublicReadSource::Cache);
    assert_eq!(result.cache_read, CacheReadStatus::Hit);
}
