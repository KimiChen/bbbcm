//! Explicit hardening entry: needs an isolated, disposable loopback Redis 8 server.
//! Unit tests do not invoke this test or claim Redis atomicity has been exercised.
use platform::cache::*;
use platform::config::{Environment, SecretString};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
#[ignore = "requires explicit isolated Redis 8 endpoint; see docs/development/redis.md"]
async fn redis8_contract_atomicity_namespace_and_recovery() {
    assert_eq!(
        std::env::var("REDIS_CONTRACT_ISOLATED").ok().as_deref(),
        Some("yes"),
        "explicit isolation acknowledgement is required"
    );
    let raw = std::env::var("REDIS_CONTRACT_URL")
        .unwrap_or_else(|_| panic!("REDIS_CONTRACT_URL is required"));
    let parsed = url::Url::parse(&raw).unwrap_or_else(|_| panic!("invalid isolated endpoint"));
    assert!(
        parsed.host_str().is_some_and(|host| host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())),
        "only loopback IPs are allowed"
    );
    let secret = SecretString::new(raw);
    let backend = Arc::new(RedisBackend::new(Environment::Test, &secret).unwrap());
    let other = RedisBackend::new(Environment::Development, &secret).unwrap();
    let client = redis::Client::open(&*secret)
        .unwrap_or_else(|_| panic!("isolated connection configuration failed"));
    let mut connection = client
        .get_multiplexed_async_connection()
        .await
        .unwrap_or_else(|_| panic!("isolated Redis connection failed"));
    let info: String = redis::cmd("INFO")
        .arg("server")
        .query_async(&mut connection)
        .await
        .unwrap_or_else(|_| panic!("isolated version probe failed"));
    assert!(
        info.lines()
            .any(|line| line.starts_with("redis_version:8.")),
        "this contract must run against Redis 8"
    );
    let id = platform::transaction::new_operation_id();
    let entity = CacheEntity::new("integration", &id).unwrap();
    let first = PublicCacheKey::new(entity.clone(), 1, CacheVisibility::Public).unwrap();
    let second = PublicCacheKey::new(entity.clone(), 2, CacheVisibility::Public).unwrap();
    backend.health().await.unwrap();
    backend
        .set(&first, "synthetic-public-v1".into(), DEFAULT_PUBLIC_TTL)
        .await
        .unwrap();
    backend
        .set(&second, "synthetic-public-v2".into(), DEFAULT_PUBLIC_TTL)
        .await
        .unwrap();
    assert_eq!(
        backend.get(&first).await.unwrap().as_deref(),
        Some("synthetic-public-v1")
    );
    assert_eq!(backend.invalidate(&entity).await.unwrap(), 2);
    assert_eq!(backend.get(&second).await.unwrap(), None);
    backend
        .set(&first, "expires".into(), Duration::from_millis(20))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(backend.get(&first).await.unwrap(), None);

    let lease_key = LeaseKey::new("integration", &id).unwrap();
    let old = backend
        .acquire(&lease_key, Duration::from_millis(20))
        .await
        .unwrap()
        .unwrap();
    assert!(
        backend
            .acquire(&lease_key, Duration::from_secs(1))
            .await
            .unwrap()
            .is_none()
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !backend
            .renew(&lease_key, &old, Duration::from_secs(1))
            .await
            .unwrap()
    );
    assert!(!backend.release(&lease_key, &old).await.unwrap());
    let current = backend
        .acquire(&lease_key, Duration::from_secs(1))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(old, current);
    assert!(
        !backend
            .renew(&lease_key, &old, Duration::from_secs(1))
            .await
            .unwrap()
    );
    assert!(!backend.release(&lease_key, &old).await.unwrap());
    assert!(
        backend
            .renew(&lease_key, &current, Duration::from_secs(1))
            .await
            .unwrap()
    );
    assert!(backend.release(&lease_key, &current).await.unwrap());

    let rate_key = RateLimitKey::new("integration", &id).unwrap();
    let policy = RateLimitPolicy::new(10, Duration::from_secs(60)).unwrap();
    let mut requests = Vec::new();
    for _ in 0..64 {
        let backend = backend.clone();
        let key = rate_key.clone();
        requests.push(tokio::spawn(async move {
            backend.check(&key, policy).await.unwrap().allowed
        }));
    }
    let mut allowed = 0;
    for request in requests {
        allowed += usize::from(request.await.unwrap());
    }
    assert_eq!(
        allowed, 10,
        "atomic shared bucket must allow exactly its initial capacity"
    );
    assert!(!backend.check(&rate_key, policy).await.unwrap().allowed);
    assert_eq!(
        backend
            .check(
                &rate_key,
                RateLimitPolicy::new(11, Duration::from_secs(60)).unwrap()
            )
            .await,
        Err(CacheError::InvalidInput)
    );

    let wait_backend = backend.clone();
    let wait = tokio::spawn(async move { wait_backend.wait(Duration::from_secs(1)).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    backend.notify().await.unwrap();
    assert!(wait.await.unwrap().unwrap());
    // A hint before subscription is intentionally lossy; the application scheduler
    // must recover by polling its DB, as covered by the scheduler SQLite tests.
    backend.notify().await.unwrap();
    assert!(!backend.wait(Duration::from_millis(20)).await.unwrap());
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            backend.wait(Duration::from_secs(10))
        )
        .await
        .is_err()
    );
    backend.health().await.unwrap();
    let length: u64 = redis::cmd("XLEN")
        .arg(backend.namespace().wakeup_key())
        .query_async(&mut connection)
        .await
        .unwrap_or_else(|_| panic!("isolated stream probe failed"));
    assert_eq!(length, 1);

    backend
        .set(&first, "test".into(), DEFAULT_PUBLIC_TTL)
        .await
        .unwrap();
    other
        .set(&first, "other-environment".into(), DEFAULT_PUBLIC_TTL)
        .await
        .unwrap();
    let unrelated = format!("unrelated:integration:{id}");
    let _: () = redis::cmd("SET")
        .arg(&unrelated)
        .arg("synthetic")
        .query_async(&mut connection)
        .await
        .unwrap_or_else(|_| panic!("isolated sentinel write failed"));
    assert!(backend.clear_namespace().await.unwrap() >= 3);
    assert_eq!(backend.get(&first).await.unwrap(), None);
    assert_eq!(
        other.get(&first).await.unwrap().as_deref(),
        Some("other-environment")
    );
    let sentinel: Option<String> = redis::cmd("GET")
        .arg(&unrelated)
        .query_async(&mut connection)
        .await
        .unwrap_or_else(|_| panic!("isolated sentinel read failed"));
    assert_eq!(sentinel.as_deref(), Some("synthetic"));
    backend
        .set(
            &first,
            "rebuilt-from-synthetic-source".into(),
            DEFAULT_PUBLIC_TTL,
        )
        .await
        .unwrap();
    assert_eq!(
        backend.get(&first).await.unwrap().as_deref(),
        Some("rebuilt-from-synthetic-source")
    );
    backend.invalidate(&entity).await.unwrap();
    other.invalidate(&entity).await.unwrap();
    let _: u64 = redis::cmd("UNLINK")
        .arg(&unrelated)
        .query_async(&mut connection)
        .await
        .unwrap_or_else(|_| panic!("isolated sentinel cleanup failed"));
    let metrics = backend.metrics();
    assert!(
        metrics.cache_hits >= 2
            && metrics.cache_misses >= 3
            && metrics.rate_allowed == 10
            && metrics.rate_denied >= 54
    );
}
