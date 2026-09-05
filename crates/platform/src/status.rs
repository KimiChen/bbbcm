//! Public C0 status projection. Detailed observations are a separate, internal contract.
use crate::observability::Measurement;
use crate::outbox::BoxFuture;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyState {
    Available,
    Disabled,
    Unavailable,
    NotConnected,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Health {
    pub live: bool,
    pub ready: bool,
    pub degraded: bool,
}
impl Health {
    pub fn from_dependencies(database: DependencyState, redis: DependencyState) -> Self {
        Self {
            live: true,
            ready: database == DependencyState::Available,
            degraded: matches!(redis, DependencyState::Unavailable),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    pub kind: AnnouncementKind,
    pub title: String,
    pub body: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnouncementKind {
    Maintenance,
    Incident,
}
impl AnnouncementKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Maintenance => "计划维护",
            Self::Incident => "事故公告",
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct DatabaseObservations {
    pub pool_size: u32,
    pub pool_idle: usize,
    pub outbox_pending: u64,
    pub outbox_dlq: u64,
    pub outbox_leased: u64,
    pub outbox_oldest_pending_ms: Measurement<u64>,
    pub outbox_events: OutboxStateCounts,
    pub outbox_deliveries: Vec<DeliveryObservation>,
    pub outbox_commit_to_sink_ms: Measurement<u64>,
    pub lock_waits: Measurement<u64>,
    pub long_transactions: Measurement<u64>,
    pub deadlocks: Measurement<u64>,
    pub autovacuum: Measurement<u64>,
    pub dead_tuples: Measurement<u64>,
    pub wal_bytes: Measurement<u64>,
    pub relation_bytes: Measurement<u64>,
}
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct OutboxStateCounts {
    pub ready: u64,
    pub leased: u64,
    pub retry: u64,
    pub paused: u64,
    pub dlq: u64,
    pub completed: u64,
}
impl OutboxStateCounts {
    /// Current persisted event states mapped to mutually exclusive public-safe buckets.
    pub fn add(&mut self, state: &str, error: &str, count: u64) -> Result<(), StatusError> {
        match (state, error) {
            ("ready", "external_disabled" | "unregistered_sink") => self.paused += count,
            ("ready", "retryable" | "cancelled") => self.retry += count,
            ("ready", "") => self.ready += count,
            ("leased", _) => self.leased += count,
            ("dlq", _) => self.dlq += count,
            ("delivered", _) => self.completed += count,
            _ => return Err(StatusError::InvalidData),
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct DeliveryObservation {
    /// Closed vocabulary; never the raw registry name read from the database.
    pub sink: crate::observability::SinkGroup,
    pub counts: OutboxStateCounts,
}
#[derive(Debug, Clone)]
pub struct StatusSnapshot {
    pub public_read_enabled: bool,
    pub announcements: Vec<Announcement>,
    pub observations: DatabaseObservations,
}
#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum StatusError {
    #[error("状态信息暂不可用")]
    Unavailable,
    #[error("状态数据未符合发布要求")]
    InvalidData,
}
pub trait StatusSource: Send + Sync {
    fn read(&self, now: DateTime<Utc>) -> BoxFuture<'_, Result<StatusSnapshot, StatusError>>;
}
pub struct UnavailableStatus;
impl StatusSource for UnavailableStatus {
    fn read(&self, _: DateTime<Utc>) -> BoxFuture<'_, Result<StatusSnapshot, StatusError>> {
        Box::pin(async { Err(StatusError::Unavailable) })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redis_failure_degrades_without_blocking_readiness() {
        let h = Health::from_dependencies(DependencyState::Available, DependencyState::Unavailable);
        assert!(h.live && h.ready && h.degraded);
        let h = Health::from_dependencies(DependencyState::Unavailable, DependencyState::Disabled);
        assert!(h.live && !h.ready && !h.degraded);
    }
}
