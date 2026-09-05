//! Finite, privacy-safe observation vocabulary. No caller-supplied metric labels.
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

mod grouped;
pub use grouped::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    HttpRead,
    HttpWrite,
    HttpError,
    HttpLatencyMs,
    TransactionRetry,
    TransactionCommitted,
    TransactionUnknown,
    DatabaseAcquireMs,
    DatabaseAcquireFailed,
    DatabaseHoldMs,
    DatabaseTransactionMs,
    DatabaseAttempts,
    OutboxClaimed,
    OutboxCompleted,
    OutboxFailed,
    OutboxLeaseRenewed,
    OutboxLeaseLost,
    SinkSuccess,
    SinkRetryable,
    SinkPermanent,
    SinkDisabled,
    SinkLatencyMs,
    DatabaseError,
    RedisError,
    MailError,
    MediaError,
    SearchError,
    AnalyticsError,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum Measurement<T> {
    Collected(T),
    Unsupported,
    NotConnected,
    Unavailable,
}
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Sample {
    pub count: u64,
    pub sum: f64,
    /// Cumulative finite buckets; units/bounds are published with each collection.
    /// `count` is the +Inf bucket. Exact p95 remains an E02 measurement.
    pub buckets: Option<[u64; 8]>,
}
pub const LATENCY_BOUNDS_MS: [f64; 8] = [1., 5., 10., 50., 100., 400., 800., 5000.];
#[derive(Debug)]
pub struct Metrics {
    values: Mutex<BTreeMap<Metric, Sample>>,
    pub grouped: GroupedMetrics,
    pub pools: PoolRegistry,
    instance_id: String,
}
impl Default for Metrics {
    fn default() -> Self {
        Self {
            values: Mutex::default(),
            grouped: GroupedMetrics::default(),
            pools: PoolRegistry::default(),
            instance_id: crate::transaction::new_operation_id(),
        }
    }
}
impl Metrics {
    /// Reset segment identity, emitted as snapshot metadata and never a metric label.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
    pub fn increment(&self, metric: Metric) {
        self.record(metric, 1., false);
    }
    pub fn duration(&self, metric: Metric, elapsed: Duration) {
        self.record(metric, elapsed.as_secs_f64() * 1000., true);
    }
    fn record(&self, metric: Metric, value: f64, histogram: bool) {
        let Ok(mut values) = self.values.lock() else {
            return;
        };
        let sample = values.entry(metric).or_default();
        sample.count = sample.count.saturating_add(1);
        sample.sum += value;
        if histogram {
            let buckets = sample.buckets.get_or_insert([0; 8]);
            for (i, bound) in LATENCY_BOUNDS_MS.iter().enumerate() {
                if value <= *bound {
                    buckets[i] = buckets[i].saturating_add(1);
                }
            }
        }
    }
    pub fn snapshot(&self) -> BTreeMap<Metric, Sample> {
        self.values.lock().map(|v| v.clone()).unwrap_or_default()
    }
}
pub fn metrics() -> Arc<Metrics> {
    static METRICS: OnceLock<Arc<Metrics>> = OnceLock::new();
    METRICS.get_or_init(|| Arc::new(Metrics::default())).clone()
}

/// Client headers never become trace fields, even if formatted as a UUID.
pub fn new_request_id() -> String {
    crate::transaction::new_operation_id()
}

tokio::task_local! { pub static REQUEST_ID: String; }
pub fn current_trace_id() -> Option<String> {
    REQUEST_ID.try_with(Clone::clone).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_measurements_are_not_zero_and_labels_are_closed() {
        let metrics = Metrics::default();
        assert!(metrics.snapshot().is_empty());
        metrics.duration(Metric::HttpLatencyMs, Duration::from_millis(400));
        let sample = metrics.snapshot().remove(&Metric::HttpLatencyMs).unwrap();
        assert_eq!(sample.count, 1);
        assert_eq!(sample.sum, 400.);
        assert_eq!(sample.buckets, Some([0, 0, 0, 0, 0, 1, 1, 1]));
        assert_ne!(
            serde_json::to_string(&Measurement::<u64>::NotConnected).unwrap(),
            serde_json::to_string(&Measurement::Collected(0u64)).unwrap()
        );
        assert_ne!(new_request_id(), new_request_id());
    }
}
