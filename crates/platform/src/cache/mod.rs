//! 缓存/唤醒/租约/限流抽象：Redis 与进程内存两套实现按 `features.redis` 装配。
//!
//! 限流由 Redis 承载（`DEC-024`）：`features.redis = false` 时不提供限流能力，
//! SQLite 体验模式默认即此配置。[`ratelimit::TokenBucketLimiter`] 只供测试与本机
//! 开发使用，不是生产限流器；Redis 实现随 `WP-MVP-A05` 接入。
//!
//! trait 方法当前为同步签名。`WP-MVP-A05` 接入 Redis 时统一改为返回 Boxed Future
//! 并带 `Result`；现在零调用点，是改签名最便宜的时刻（`02` 第 2 节）。

pub mod ratelimit;

pub use ratelimit::TokenBucketLimiter;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 进程内结构的默认条目上限。key 常含 IP、用户名等客户端可控成分，
/// 无上限的 map 等于把内存交给调用方支配。
pub const DEFAULT_MAX_ENTRIES: usize = 100_000;

/// 可重建缓存：只存放可丢失、可从主库重建的数据（公开片段缓存、非权威在线状态）。
pub trait CacheStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: String, ttl: Duration);
}

/// Worker 唤醒：低延迟通知有新 Outbox 事件。丢失只会退化为轮询，不影响正确性。
pub trait Wakeup: Send + Sync {
    /// 通知指定通道有新工作。
    fn notify(&self, channel: &str);
    /// 等待通知，最多 `timeout`；返回是否收到通知（超时返回 false）。
    fn wait(&self, channel: &str, timeout: Duration) -> bool;
}

/// 短租约：用于「同一时刻只有一个执行者」的协调，不承载业务事实。
pub trait Lease: Send + Sync {
    /// 尝试取得租约，成功返回租约令牌。
    fn acquire(&self, key: &str, ttl: Duration) -> Option<String>;
    /// 续租；令牌不匹配返回 false。
    fn renew(&self, key: &str, token: &str, ttl: Duration) -> bool;
    /// 释放；令牌不匹配则忽略。
    fn release(&self, key: &str, token: &str);
}

/// 进程内存缓存实现（单实例；多实例部署时改用 Redis 实现）。
///
/// 有条目上限并按过期时间淘汰：先清过期项，仍超限则丢弃最早过期的条目。
#[derive(Debug)]
pub struct MemoryCache {
    entries: Mutex<HashMap<String, (String, Instant)>>,
    max_entries: usize,
}

impl Default for MemoryCache {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_MAX_ENTRIES)
    }
}

impl MemoryCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_entries: max_entries.max(1),
        }
    }

    /// 当前条目数，用于测试与指标。
    pub fn len(&self) -> usize {
        self.entries.lock().expect("缓存锁中毒").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl CacheStore for MemoryCache {
    fn get(&self, key: &str) -> Option<String> {
        let mut entries = self.entries.lock().expect("缓存锁中毒");
        let (value, expires_at) = entries.get(key)?.clone();
        if Instant::now() >= expires_at {
            entries.remove(key);
            None
        } else {
            Some(value)
        }
    }

    fn set(&self, key: &str, value: String, ttl: Duration) {
        let now = Instant::now();
        let expires_at = now + ttl;
        let mut entries = self.entries.lock().expect("缓存锁中毒");
        if entries.len() >= self.max_entries && !entries.contains_key(key) {
            entries.retain(|_, (_, exp)| *exp > now);
            while entries.len() >= self.max_entries {
                let victim = entries
                    .iter()
                    .min_by_key(|(_, (_, exp))| *exp)
                    .map(|(k, _)| k.clone());
                match victim {
                    Some(k) => {
                        entries.remove(&k);
                    }
                    None => break,
                }
            }
        }
        entries.insert(key.to_string(), (value, expires_at));
    }
}

/// 进程内唤醒：单进程内嵌 worker（`worker.embedded = true`）使用。
#[derive(Debug, Default)]
pub struct MemoryWakeup {
    pending: Mutex<HashMap<String, bool>>,
}

impl MemoryWakeup {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Wakeup for MemoryWakeup {
    fn notify(&self, channel: &str) {
        self.pending
            .lock()
            .expect("唤醒锁中毒")
            .insert(channel.to_string(), true);
    }

    fn wait(&self, channel: &str, _timeout: Duration) -> bool {
        // 内存实现不阻塞：调用方本就按固定间隔轮询（`02` 第 12 节）。
        self.pending
            .lock()
            .expect("唤醒锁中毒")
            .remove(channel)
            .unwrap_or(false)
    }
}

/// 进程内租约：单进程内有效，多实例部署必须换 Redis 实现。
#[derive(Debug, Default)]
pub struct MemoryLease {
    held: Mutex<HashMap<String, (String, Instant)>>,
}

impl MemoryLease {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Lease for MemoryLease {
    fn acquire(&self, key: &str, ttl: Duration) -> Option<String> {
        let now = Instant::now();
        let mut held = self.held.lock().expect("租约锁中毒");
        held.retain(|_, (_, exp)| *exp > now);
        if held.contains_key(key) {
            return None;
        }
        // 令牌只需在进程内唯一：用单调递增计数即可，不引入随机源。
        let token = format!("{key}:{}", held.len() + 1);
        held.insert(key.to_string(), (token.clone(), now + ttl));
        Some(token)
    }

    fn renew(&self, key: &str, token: &str, ttl: Duration) -> bool {
        let mut held = self.held.lock().expect("租约锁中毒");
        match held.get_mut(key) {
            Some((t, exp)) if t == token => {
                *exp = Instant::now() + ttl;
                true
            }
            _ => false,
        }
    }

    fn release(&self, key: &str, token: &str) {
        let mut held = self.held.lock().expect("租约锁中毒");
        if held.get(key).is_some_and(|(t, _)| t == token) {
            held.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_cache_expires_after_ttl() {
        let cache = MemoryCache::new();
        cache.set("k", "v".into(), Duration::from_millis(1));
        assert_eq!(cache.get("k").as_deref(), Some("v"));
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(cache.get("k"), None);
    }

    #[test]
    fn memory_cache_is_bounded() {
        let cache = MemoryCache::with_capacity(4);
        for i in 0..50 {
            cache.set(&format!("k{i}"), "v".into(), Duration::from_secs(60));
        }
        assert!(
            cache.len() <= 4,
            "客户端可控 key 不得无限增长，当前 {}",
            cache.len()
        );
    }

    #[test]
    fn wakeup_consumes_notification_once() {
        let w = MemoryWakeup::new();
        assert!(!w.wait("outbox", Duration::from_millis(1)));
        w.notify("outbox");
        assert!(w.wait("outbox", Duration::from_millis(1)));
        assert!(!w.wait("outbox", Duration::from_millis(1)));
    }

    #[test]
    fn lease_is_exclusive_and_token_bound() {
        let l = MemoryLease::new();
        let token = l
            .acquire("job", Duration::from_secs(60))
            .expect("首次应取得");
        assert!(
            l.acquire("job", Duration::from_secs(60)).is_none(),
            "已被持有"
        );
        assert!(
            !l.renew("job", "wrong", Duration::from_secs(60)),
            "令牌不匹配不得续租"
        );
        assert!(l.renew("job", &token, Duration::from_secs(60)));
        l.release("job", "wrong");
        assert!(
            l.acquire("job", Duration::from_secs(60)).is_none(),
            "错误令牌不得释放"
        );
        l.release("job", &token);
        assert!(l.acquire("job", Duration::from_secs(60)).is_some());
    }

    #[test]
    fn lease_expires() {
        let l = MemoryLease::new();
        l.acquire("job", Duration::from_millis(1))
            .expect("首次应取得");
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            l.acquire("job", Duration::from_secs(60)).is_some(),
            "过期后应可重取"
        );
    }
}
