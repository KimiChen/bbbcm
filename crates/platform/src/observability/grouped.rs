//! Closed dimensions and seconds-based series for the M0-05 observation contract.
use super::{Measurement, Sample};
use crate::db::Backend;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseBackend {
    Unknown,
    Postgres,
    Sqlite,
    Mysql8,
}
impl From<Backend> for DatabaseBackend {
    fn from(value: Backend) -> Self {
        match value {
            Backend::Postgres => Self::Postgres,
            Backend::Sqlite => Self::Sqlite,
            Backend::Mysql8 => Self::Mysql8,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabasePurpose {
    App,
    Worker,
    Migrator,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    None,
    UniqueConflict,
    Serialization,
    Deadlock,
    Busy,
    BusySnapshot,
    LockTimeout,
    Connection,
    Other,
    NotObserved,
}
impl From<crate::transaction::DbErrorClass> for ErrorClass {
    fn from(value: crate::transaction::DbErrorClass) -> Self {
        use crate::transaction::DbErrorClass as E;
        match value {
            E::UniqueConflict => Self::UniqueConflict,
            E::Serialization => Self::Serialization,
            E::Deadlock => Self::Deadlock,
            E::Busy => Self::Busy,
            E::BusySnapshot => Self::BusySnapshot,
            E::LockTimeout => Self::LockTimeout,
            E::Connection => Self::Connection,
            E::Other => Self::Other,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionOutcome {
    Committed,
    RolledBack,
    CommitUnknown,
    CommitRejected,
    RollbackFailed,
    BeginFailed,
    Cancelled,
    CancelledOrAbandoned,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquireOutcome {
    Acquired,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpRoute {
    Home,
    Status,
    Health,
    Diagnostics,
    Fixture,
    Asset,
    Auth,
    AccountSettings,
    Profile,
    Policy,
    Invitation,
    ContentList,
    ContentDetail,
    ContentEditor,
    ContentOperation,
    Other,
}
impl HttpRoute {
    pub fn from_path(path: &str) -> Self {
        match path {
            "/" => Self::Home,
            "/new" | "/pinned" | "/featured" | "/me/topics" | "/me/posts" | "/me/replies" => {
                Self::ContentList
            }
            "/topics/new" => Self::ContentEditor,
            "/content/operations" => Self::ContentOperation,
            "/status" => Self::Status,
            "/health" | "/health/live" | "/health/ready" => Self::Health,
            "/__diagnostics/metrics" => Self::Diagnostics,
            "/login"
            | "/register"
            | "/verify-email"
            | "/resend-verification"
            | "/forgot-password"
            | "/reset-password"
            | "/confirm-email"
            | "/reauth"
            | "/logout"
            | "/auth/operations" => Self::Auth,
            "/settings/profile"
            | "/settings/email"
            | "/settings/username"
            | "/settings/password"
            | "/settings/sessions"
            | "/settings/sessions/revoke" => Self::AccountSettings,
            "/terms" | "/privacy" | "/rules" => Self::Policy,
            "/me/invites"
            | "/me/invites/revoke"
            | "/settings/invitations"
            | "/settings/invitations/revoke" => Self::Invitation,
            // Match the route shape only. The opaque profile identifier never becomes
            // a metric dimension, even for an invalid or missing user.
            p if p
                .strip_prefix("/u/")
                .is_some_and(|id| !id.is_empty() && !id.contains(['/', '?', '#'])) =>
            {
                Self::Profile
            }
            p if content_route(p).is_some() => content_route(p).unwrap_or(Self::Other),
            p if p.starts_with("/__fixtures/a00/") => Self::Fixture,
            p if p.starts_with("/assets/") => Self::Asset,
            _ => Self::Other,
        }
    }
}
fn content_route(path: &str) -> Option<HttpRoute> {
    let segments: Vec<_> = path.strip_prefix('/')?.split('/').collect();
    let valid = |s: &str| !s.is_empty() && !s.contains(['?', '#']);
    match segments.as_slice() {
        ["forums" | "tags", id] if valid(id) => Some(HttpRoute::ContentList),
        ["u", id, "topics" | "posts" | "replies"] if valid(id) => Some(HttpRoute::ContentList),
        ["topics" | "posts", id] if valid(id) => Some(HttpRoute::ContentDetail),
        ["topics", id, "reply"] | ["posts", id, "edit" | "delete"] if valid(id) => {
            Some(HttpRoute::ContentEditor)
        }
        _ => None,
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Other,
}
impl HttpMethod {
    pub fn from_method(method: &str) -> Self {
        match method {
            "GET" => Self::Get,
            "HEAD" => Self::Head,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "PATCH" => Self::Patch,
            "DELETE" => Self::Delete,
            _ => Self::Other,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpStatusClass {
    Informational,
    Success,
    Redirection,
    ClientError,
    ServerError,
    Other,
}
impl HttpStatusClass {
    pub fn from_status(status: u16) -> Self {
        match status {
            100..200 => Self::Informational,
            200..300 => Self::Success,
            300..400 => Self::Redirection,
            400..500 => Self::ClientError,
            500..600 => Self::ServerError,
            _ => Self::Other,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    NotConnected,
    Hit,
    Miss,
    Bypass,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkGroup {
    All,
    Local,
    AuditReplica,
    CacheInvalidation,
    Analytics,
    Mail,
    MailAlert,
    AdapterEffects,
    Other,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkOutcome {
    Success,
    Retryable,
    Permanent,
    Disabled,
    Cancelled,
    LeaseLost,
    StoreError,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxOutcome {
    Claimed,
    Renewed,
    LeaseLost,
    Completed,
    Retryable,
    RetryExhausted,
    Permanent,
    ExternalDisabled,
    Unregistered,
    Cancelled,
    StoreError,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerOutcome {
    Cancelled,
    Failed,
    Aborted,
}
impl SinkGroup {
    pub fn from_name(name: &str) -> Self {
        match name {
            "local.probe" => Self::Local,
            "audit.replica" => Self::AuditReplica,
            "cache.invalidate" => Self::CacheInvalidation,
            "analytics.project" => Self::Analytics,
            "mail.delivery" => Self::Mail,
            "mail.alert" => Self::MailAlert,
            "integrations.effects" => Self::AdapterEffects,
            _ => Self::Other,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "metric", rename_all = "snake_case")]
pub enum SeriesKey {
    SinkSeconds {
        backend: DatabaseBackend,
        sink: SinkGroup,
        outcome: SinkOutcome,
    },
    OutboxTotal {
        backend: DatabaseBackend,
        sink: SinkGroup,
        outcome: OutboxOutcome,
    },
    WorkerPollSeconds {
        backend: DatabaseBackend,
    },
    WorkerExitTotal {
        backend: DatabaseBackend,
        outcome: WorkerOutcome,
    },
    HttpHeadersSeconds {
        backend: DatabaseBackend,
        route: HttpRoute,
        method: HttpMethod,
        status_class: HttpStatusClass,
        cache_state: CacheState,
    },
    DbPoolAcquireSeconds {
        backend: DatabaseBackend,
        purpose: DatabasePurpose,
        outcome: AcquireOutcome,
        error_class: ErrorClass,
    },
    DbPoolAcquireFailuresTotal {
        backend: DatabaseBackend,
        purpose: DatabasePurpose,
        error_class: ErrorClass,
    },
    DbConnectionHoldSeconds {
        backend: DatabaseBackend,
        purpose: DatabasePurpose,
    },
    DbTransactionSeconds {
        backend: DatabaseBackend,
        purpose: DatabasePurpose,
        outcome: TransactionOutcome,
        error_class: ErrorClass,
    },
    DbTransactionAttemptsTotal {
        backend: DatabaseBackend,
        purpose: DatabasePurpose,
    },
    DbTransactionRetriesTotal {
        backend: DatabaseBackend,
        purpose: DatabasePurpose,
        error_class: ErrorClass,
    },
}
#[derive(Debug, Clone, Serialize)]
pub struct Series {
    pub dimensions: SeriesKey,
    pub sample: Sample,
}
pub const LATENCY_BOUNDS_SECONDS: [f64; 8] = [0.001, 0.005, 0.01, 0.05, 0.1, 0.4, 0.8, 5.];
#[derive(Debug, Default)]
pub struct GroupedMetrics(Mutex<BTreeMap<SeriesKey, Sample>>, AtomicU64, AtomicBool);
pub struct ActiveJob<'a>(&'a AtomicU64);
impl Drop for ActiveJob<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
impl GroupedMetrics {
    pub fn observe_worker(&self) {
        self.2.store(true, Ordering::Relaxed);
    }
    pub fn active_job(&self) -> ActiveJob<'_> {
        self.observe_worker();
        self.1.fetch_add(1, Ordering::Relaxed);
        ActiveJob(&self.1)
    }
    pub fn active_jobs(&self) -> u64 {
        self.1.load(Ordering::Relaxed)
    }
    pub fn active_jobs_observation(&self) -> Measurement<u64> {
        if self.2.load(Ordering::Relaxed) {
            Measurement::Collected(self.active_jobs())
        } else {
            Measurement::NotConnected
        }
    }
    pub fn increment(&self, key: SeriesKey) {
        self.record(key, 1., false);
    }
    pub fn duration(&self, key: SeriesKey, elapsed: Duration) {
        self.record(key, elapsed.as_secs_f64(), true);
    }
    fn record(&self, key: SeriesKey, value: f64, histogram: bool) {
        let Ok(mut values) = self.0.lock() else {
            return;
        };
        let sample = values.entry(key).or_default();
        sample.count = sample.count.saturating_add(1);
        sample.sum += value;
        if histogram {
            let buckets = sample.buckets.get_or_insert([0; 8]);
            for (count, bound) in buckets.iter_mut().zip(LATENCY_BOUNDS_SECONDS) {
                if value <= bound {
                    *count = count.saturating_add(1);
                }
            }
        }
    }
    pub fn snapshot(&self) -> Vec<Series> {
        self.0
            .lock()
            .map(|values| {
                values
                    .iter()
                    .map(|(&dimensions, sample)| Series {
                        dimensions,
                        sample: sample.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct PoolSample {
    pub pools: u64,
    pub open: u64,
    pub idle: u64,
    pub in_use: u64,
    pub max: u64,
}
/// Adapters expose actual driver gauges; no SQL, credentials, path or pool ID escapes.
pub trait PoolObserver: Send + Sync + std::fmt::Debug {
    fn backend(&self) -> DatabaseBackend;
    fn purpose(&self) -> DatabasePurpose;
    fn sample(&self) -> PoolSample;
}
#[derive(Debug, Clone, Serialize)]
pub struct PoolGroup {
    pub backend: DatabaseBackend,
    pub purpose: DatabasePurpose,
    pub observation: Measurement<PoolSample>,
}
#[derive(Debug, Default)]
pub struct PoolRegistry(Mutex<Vec<Weak<dyn PoolObserver>>>);
impl PoolRegistry {
    pub fn register(&self, observer: &Arc<dyn PoolObserver>) {
        if let Ok(mut pools) = self.0.lock() {
            pools.retain(|p| p.strong_count() > 0);
            pools.push(Arc::downgrade(observer));
        }
    }
    pub fn snapshot(&self) -> Vec<PoolGroup> {
        let mut values = BTreeMap::<_, PoolSample>::new();
        let mut unavailable = false;
        if let Ok(mut pools) = self.0.lock() {
            pools.retain(|p| {
                let Some(pool) = p.upgrade() else {
                    return false;
                };
                let value = values.entry((pool.backend(), pool.purpose())).or_default();
                let sample = pool.sample();
                value.pools += sample.pools;
                value.open += sample.open;
                value.idle += sample.idle;
                value.in_use += sample.in_use;
                value.max += sample.max;
                true
            });
        } else {
            unavailable = true;
        }
        [DatabaseBackend::Postgres, DatabaseBackend::Sqlite]
            .into_iter()
            .flat_map(|backend| {
                let values = &values;
                [
                    DatabasePurpose::App,
                    DatabasePurpose::Worker,
                    DatabasePurpose::Migrator,
                ]
                .into_iter()
                .map(move |purpose| PoolGroup {
                    backend,
                    purpose,
                    observation: if unavailable {
                        Measurement::Unavailable
                    } else {
                        values
                            .get(&(backend, purpose))
                            .copied()
                            .map(Measurement::Collected)
                            .unwrap_or(Measurement::NotConnected)
                    },
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_metrics_never_carry_titles_slugs_owners_or_queries() {
        for (path, expected) in [
            ("/new", HttpRoute::ContentList),
            ("/pinned", HttpRoute::ContentList),
            ("/featured", HttpRoute::ContentList),
            ("/me/topics", HttpRoute::ContentList),
            ("/forums/PRIVATE_SENTINEL", HttpRoute::ContentList),
            ("/tags/PRIVATE_SENTINEL", HttpRoute::ContentList),
            ("/u/PRIVATE_SENTINEL/topics", HttpRoute::ContentList),
            ("/topics/PRIVATE_SENTINEL", HttpRoute::ContentDetail),
            ("/posts/PRIVATE_SENTINEL", HttpRoute::ContentDetail),
            ("/topics/new", HttpRoute::ContentEditor),
            ("/topics/PRIVATE_SENTINEL/reply", HttpRoute::ContentEditor),
            ("/posts/PRIVATE_SENTINEL/edit", HttpRoute::ContentEditor),
            ("/posts/PRIVATE_SENTINEL/delete", HttpRoute::ContentEditor),
            ("/content/operations", HttpRoute::ContentOperation),
            ("/topics/PRIVATE_SENTINEL/arbitrary", HttpRoute::Other),
            ("/topics/PRIVATE_SENTINEL?cursor=secret", HttpRoute::Other),
        ] {
            let actual = HttpRoute::from_path(path);
            assert_eq!(actual, expected);
            assert!(
                !serde_json::to_string(&actual)
                    .unwrap()
                    .contains("PRIVATE_SENTINEL")
            );
        }
    }

    #[test]
    fn identity_routes_are_closed_and_dynamic_identifiers_never_escape() {
        let metrics = GroupedMetrics::default();
        for (path, expected) in [
            ("/register", HttpRoute::Auth),
            ("/login", HttpRoute::Auth),
            ("/verify-email", HttpRoute::Auth),
            ("/resend-verification", HttpRoute::Auth),
            ("/forgot-password", HttpRoute::Auth),
            ("/reset-password", HttpRoute::Auth),
            ("/confirm-email", HttpRoute::Auth),
            ("/reauth", HttpRoute::Auth),
            ("/logout", HttpRoute::Auth),
            ("/auth/operations", HttpRoute::Auth),
            ("/settings/profile", HttpRoute::AccountSettings),
            ("/settings/email", HttpRoute::AccountSettings),
            ("/settings/username", HttpRoute::AccountSettings),
            ("/settings/password", HttpRoute::AccountSettings),
            ("/settings/sessions", HttpRoute::AccountSettings),
            ("/settings/sessions/revoke", HttpRoute::AccountSettings),
            ("/me/invites", HttpRoute::Invitation),
            ("/me/invites/revoke", HttpRoute::Invitation),
            ("/settings/invitations", HttpRoute::Invitation),
            ("/settings/invitations/revoke", HttpRoute::Invitation),
            ("/terms", HttpRoute::Policy),
            ("/privacy", HttpRoute::Policy),
            ("/rules", HttpRoute::Policy),
            ("/u/PRIVATE_SENTINEL", HttpRoute::Profile),
            ("/u/another_private_identifier", HttpRoute::Profile),
            ("/u/", HttpRoute::Other),
            ("/u/PRIVATE_SENTINEL/devices", HttpRoute::Other),
            ("/settings/PRIVATE_SENTINEL", HttpRoute::Other),
            ("/auth/PRIVATE_SENTINEL", HttpRoute::Other),
            ("/verify-email/PRIVATE_SENTINEL", HttpRoute::Other),
            ("/resend-verification/PRIVATE_SENTINEL", HttpRoute::Other),
            ("/unknown/PRIVATE_SENTINEL", HttpRoute::Other),
        ] {
            let route = HttpRoute::from_path(path);
            assert_eq!(route, expected);
            metrics.duration(
                SeriesKey::HttpHeadersSeconds {
                    backend: DatabaseBackend::Sqlite,
                    route,
                    method: HttpMethod::Get,
                    status_class: HttpStatusClass::Success,
                    cache_state: CacheState::NotConnected,
                },
                Duration::from_millis(1),
            );
        }
        let series = metrics.snapshot();
        assert_eq!(series.len(), 6);
        assert!(
            !serde_json::to_string(&series)
                .unwrap()
                .contains("PRIVATE_SENTINEL")
        );
        assert!(!format!("{series:?}").contains("private_identifier"));
    }

    #[test]
    fn closed_dimensions_separate_routes_statuses_and_units() {
        let metrics = GroupedMetrics::default();
        for (route, status_class, elapsed) in [
            (
                HttpRoute::Home,
                HttpStatusClass::Success,
                Duration::from_millis(400),
            ),
            (
                HttpRoute::Status,
                HttpStatusClass::ServerError,
                Duration::from_millis(800),
            ),
        ] {
            metrics.duration(
                SeriesKey::HttpHeadersSeconds {
                    backend: DatabaseBackend::Sqlite,
                    route,
                    method: HttpMethod::Get,
                    status_class,
                    cache_state: CacheState::NotConnected,
                },
                elapsed,
            );
        }
        let values = metrics.snapshot();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].sample.sum, 0.4);
        assert_eq!(values[0].sample.buckets, Some([0, 0, 0, 0, 0, 1, 1, 1]));
        assert_eq!(values[1].sample.sum, 0.8);
        let unknown = "PRIVATE_SENTINEL/user/193";
        assert_eq!(HttpRoute::from_path(unknown), HttpRoute::Other);
        assert_eq!(HttpMethod::from_method(unknown), HttpMethod::Other);
        assert_eq!(SinkGroup::from_name(unknown), SinkGroup::Other);
        assert!(
            !serde_json::to_string(&values)
                .unwrap()
                .contains("PRIVATE_SENTINEL")
        );
        for (name, expected) in [
            ("local.probe", SinkGroup::Local),
            ("mail.delivery", SinkGroup::Mail),
            ("integrations.effects", SinkGroup::AdapterEffects),
        ] {
            assert_eq!(SinkGroup::from_name(name), expected);
        }
        assert_eq!(metrics.active_jobs(), 0);
        let job = metrics.active_job();
        assert_eq!(metrics.active_jobs(), 1);
        drop(job);
        assert_eq!(metrics.active_jobs(), 0);
    }
}
