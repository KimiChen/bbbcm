//! 进程内令牌桶：**仅供测试与本机开发**，不是生产限流器。
//!
//! 按 `DEC-024`，生产限流由 Redis 承载（键在 `bcm:<env>:v1:ratelimit:` 下，多实例共享
//! 计数），`features.redis = false` 时不提供限流能力。本实现按实例计数、进程重启即清零，
//! 不满足限流的安全语义，不得装配进 staging/production。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use super::DEFAULT_MAX_ENTRIES;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LimiterError {
    #[error("补充速率必须是有限的非负数，收到 {0}")]
    BadRefillRate(String),
}

pub struct TokenBucketLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
    capacity: f64,
    refill_per_second: f64,
    max_entries: usize,
}

struct Bucket {
    tokens: f64,
    updated: Instant,
}

impl TokenBucketLimiter {
    /// `capacity` 为桶容量（突发上限），`refill_per_second` 为每秒补充速率。
    pub fn new(capacity: u32, refill_per_second: f64) -> Result<Self, LimiterError> {
        if !refill_per_second.is_finite() || refill_per_second < 0.0 {
            return Err(LimiterError::BadRefillRate(refill_per_second.to_string()));
        }
        Ok(Self {
            buckets: Mutex::new(HashMap::new()),
            capacity: capacity.max(1) as f64,
            refill_per_second,
            max_entries: DEFAULT_MAX_ENTRIES,
        })
    }

    /// 覆盖条目上限；超限时新 key 一律拒绝（fail-closed）。
    pub fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = max_entries.max(1);
        self
    }

    /// 命中返回 true；key 建议按 `维度:标识` 组合（如 `login:user:42`）。
    ///
    /// key 通常含 IP 或用户名等客户端可控成分。满桶条目等价于「不存在」，
    /// 先清掉它们；仍然超过上限时**拒绝**新 key，而不是继续增长——
    /// 限流器被打爆内存比拒绝请求更糟。
    pub fn try_acquire(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().expect("限流锁中毒");

        if !buckets.contains_key(key) && buckets.len() >= self.max_entries {
            self.sweep_full(&mut buckets, now);
            if buckets.len() >= self.max_entries {
                return false;
            }
        }

        let capacity = self.capacity;
        let refill = self.refill_per_second;
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: capacity,
            updated: now,
        });
        let elapsed = now.duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill).min(capacity);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// 当前跟踪的 key 数，用于测试与指标。
    pub fn tracked_keys(&self) -> usize {
        self.buckets.lock().expect("限流锁中毒").len()
    }

    fn sweep_full(&self, buckets: &mut HashMap<String, Bucket>, now: Instant) {
        let capacity = self.capacity;
        let refill = self.refill_per_second;
        buckets.retain(|_, b| {
            let elapsed = now.duration_since(b.updated).as_secs_f64();
            (b.tokens + elapsed * refill) < capacity
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(capacity: u32, refill: f64) -> TokenBucketLimiter {
        TokenBucketLimiter::new(capacity, refill).expect("参数应合法")
    }

    #[test]
    fn burst_then_throttle() {
        let l = limiter(1, 0.0);
        assert!(l.try_acquire("k"));
        assert!(!l.try_acquire("k"), "容量 1 且不补充时第二次必须被拒");
    }

    #[test]
    fn rejects_invalid_refill_rate() {
        assert!(TokenBucketLimiter::new(1, f64::NAN).is_err());
        assert!(TokenBucketLimiter::new(1, f64::INFINITY).is_err());
        assert!(TokenBucketLimiter::new(1, -1.0).is_err());
    }

    #[test]
    fn distinct_keys_do_not_grow_unbounded() {
        // 容量 1 且不补充：每个 key 用掉唯一令牌后不再是满桶，无法被清扫，
        // 因此必须靠上限拒绝，而不是继续分配。
        let l = limiter(1, 0.0).with_max_entries(8);
        for i in 0..1000 {
            l.try_acquire(&format!("ip:{i}"));
        }
        assert!(l.tracked_keys() <= 8, "当前 {}", l.tracked_keys());
    }

    #[test]
    fn full_buckets_are_swept_after_refill() {
        // 清扫只回收「已回满」的桶，而回满需要时间流逝。紧循环内没有时间流逝，
        // 因此上限内的拒绝是正确的保守行为；等补充完成后，旧 key 应被回收让位。
        let l = limiter(4, 1000.0).with_max_entries(4);
        for i in 0..4 {
            assert!(l.try_acquire(&format!("old:{i}")));
        }
        assert!(
            !l.try_acquire("new:1"),
            "上限内且旧桶未回满时应 fail-closed"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(l.try_acquire("new:1"), "旧桶回满后应被清扫，新 key 放行");
        assert!(l.tracked_keys() <= 4);
    }
}
