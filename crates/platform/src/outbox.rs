//! Reliable event contracts shared by storage, audit and the worker. No provider code lives here.
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Failure,
    Corrected,
}

/// Only opaque references and a fixed outcome are admitted. Arbitrary text, mail bodies,
/// secrets and C3 fields cannot enter the event through this contract.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct SafePayload {
    references: BTreeMap<String, String>,
    outcome: Option<Outcome>,
}
impl<'de> Deserialize<'de> for SafePayload {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            references: BTreeMap<String, String>,
            outcome: Option<Outcome>,
        }
        let encoded = Encoded::deserialize(deserializer)?;
        let mut value = SafePayload::empty();
        for (key, reference) in encoded.references {
            value = value
                .with_reference(&key, &reference)
                .map_err(|_| serde::de::Error::custom("invalid safe event reference"))?;
        }
        value.outcome = encoded.outcome;
        Ok(value)
    }
}
impl SafePayload {
    pub fn empty() -> Self {
        Self::default()
    }
    pub fn with_reference(mut self, key: &str, value: &str) -> Result<Self, OutboxError> {
        if !matches!(
            key,
            "operation_id" | "audit_event_id" | "resource_id" | "version"
        ) || !is_reference(value)
        {
            return Err(OutboxError::InvalidInput);
        }
        self.references.insert(key.into(), value.into());
        Ok(self)
    }
    pub fn with_outcome(mut self, outcome: Outcome) -> Self {
        self.outcome = Some(outcome);
        self
    }
    pub fn reference(&self, key: &str) -> Option<&str> {
        self.references.get(key).map(String::as_str)
    }
    pub fn to_json(&self) -> Result<String, OutboxError> {
        serde_json::to_string(self).map_err(|_| OutboxError::InvalidInput)
    }
    pub fn from_json(json: &str) -> Result<Self, OutboxError> {
        let value: Self = serde_json::from_str(json).map_err(|_| OutboxError::CorruptData)?;
        for (key, reference) in &value.references {
            Self::empty()
                .with_reference(key, reference)
                .map_err(|_| OutboxError::CorruptData)?;
        }
        Ok(value)
    }
}
fn is_reference(value: &str) -> bool {
    crate::transaction::validate_operation_id(value)
        || (!value.starts_with('0')
            && value.parse::<i64>().is_ok_and(|v| v > 0)
            && value.bytes().all(|c| c.is_ascii_digit()))
}
pub fn is_machine_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEvent {
    pub event_id: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub event_type: String,
    pub version: i64,
    /// Encoding is defined by platform::db and architecture §6.2.
    pub occurred_at: DateTime<Utc>,
    pub payload: SafePayload,
    pub trace_id: String,
}
impl OutboxEvent {
    pub fn validate(&self) -> Result<(), OutboxError> {
        if !crate::transaction::validate_operation_id(&self.event_id)
            || !is_machine_name(&self.aggregate_type)
            || !is_reference(&self.aggregate_id)
            || !is_machine_name(&self.event_type)
            || self.version <= 0
            || !(self.trace_id.is_empty()
                || crate::transaction::validate_operation_id(&self.trace_id))
        {
            return Err(OutboxError::InvalidInput);
        }
        Ok(())
    }
}

/// INTEG-004 wire model only. No subscription creation or HTTP delivery is exposed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebhookEventV1 {
    pub schema_version: u32,
    pub event_id: String,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub aggregate_version: i64,
    pub occurred_at: String,
    pub data: SafePayload,
}
impl TryFrom<&OutboxEvent> for WebhookEventV1 {
    type Error = OutboxError;
    fn try_from(event: &OutboxEvent) -> Result<Self, Self::Error> {
        event.validate()?;
        Ok(Self {
            schema_version: 1,
            event_id: event.event_id.clone(),
            event_type: event.event_type.clone(),
            aggregate_type: event.aggregate_type.clone(),
            aggregate_id: event.aggregate_id.clone(),
            aggregate_version: event.version,
            occurred_at: crate::db::encode_time(event.occurred_at),
            data: event.payload.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasedEvent {
    pub event: OutboxEvent,
    pub owner: String,
    pub batch_id: String,
    pub lease_until: DateTime<Utc>,
    pub attempts: i64,
    pub pending_sinks: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct Claim {
    pub owner: String,
    pub batch_id: String,
    pub now: DateTime<Utc>,
    pub lease_until: DateTime<Utc>,
    pub limit: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCode {
    Retryable,
    Permanent,
    UnregisteredSink,
    ExternalDisabled,
    Cancelled,
}
impl FailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retryable => "retryable",
            Self::Permanent => "permanent",
            Self::UnregisteredSink => "unregistered_sink",
            Self::ExternalDisabled => "external_disabled",
            Self::Cancelled => "cancelled",
        }
    }
}
#[derive(Debug, Clone)]
pub struct Failure {
    pub code: FailureCode,
    pub now: DateTime<Utc>,
    pub available_at: DateTime<Utc>,
    pub dead_letter: bool,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum OutboxError {
    #[error("Outbox 参数非法")]
    InvalidInput,
    #[error("Outbox 存储不可用（底层信息已脱敏）")]
    Unavailable,
    #[error("Outbox 数据损坏")]
    CorruptData,
    #[error("Outbox 租约已过期或持有者已更换")]
    LeaseLost,
    #[error("Outbox 重放未授权或目标状态不允许")]
    ReplayDenied,
    #[error("Outbox 重放操作标识已绑定其它载荷")]
    ReplayConflict,
    #[error("Outbox 重放结果已过期；旧批准不得自动重用")]
    ReplayExpired,
    #[error("Outbox 重放提交结果未知；凭原 actor/scope/key 查询操作 {operation_id}")]
    CommitUnknown { operation_id: String },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkError {
    Retryable,
    Permanent,
}

pub trait Sink: Send + Sync {
    fn name(&self) -> &str;
    fn is_external(&self) -> bool {
        true
    }
    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>>;
}

/// Application authorization boundary; production implementations must verify current
/// actor, permission, scope and audit in the replay transaction. There is no HTTP entry.
#[derive(Debug, Clone)]
pub struct ReplayRequest {
    pub event_id: String,
    pub actor_id: String,
    pub reason_code: String,
    pub operation_id: String,
}

impl ReplayRequest {
    pub fn command_identity(&self) -> Result<crate::transaction::CommandIdentity, OutboxError> {
        let actor_id = self
            .actor_id
            .parse::<i64>()
            .map_err(|_| OutboxError::ReplayDenied)?;
        if actor_id <= 0
            || actor_id.to_string() != self.actor_id
            || !crate::transaction::validate_operation_id(&self.event_id)
            || !crate::transaction::validate_operation_id(&self.operation_id)
            || !is_machine_name(&self.reason_code)
        {
            return Err(OutboxError::ReplayDenied);
        }
        let payload = serde_json::to_vec(&(&self.event_id, &self.reason_code))
            .map_err(|_| OutboxError::InvalidInput)?;
        Ok(crate::transaction::CommandIdentity {
            scope: "outbox.replay".into(),
            actor_id,
            idempotency_key: self.operation_id.clone(),
            request_hash: crate::transaction::request_hash(&payload),
            operation_id: self.operation_id.clone(),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayResult {
    pub operation_id: String,
    pub replayed: bool,
}

/// Storage is injected by the application; platform-outbox does not depend on platform-db.
pub trait OutboxStore: Send + Sync {
    fn claim<'a>(&'a self, claim: Claim) -> BoxFuture<'a, Result<Vec<LeasedEvent>, OutboxError>>;
    fn renew<'a>(
        &'a self,
        lease: &'a LeasedEvent,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), OutboxError>>;
    fn delivery_done<'a>(
        &'a self,
        lease: &'a LeasedEvent,
        sink: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), OutboxError>>;
    fn complete<'a>(
        &'a self,
        lease: &'a LeasedEvent,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), OutboxError>>;
    fn fail<'a>(
        &'a self,
        lease: &'a LeasedEvent,
        failure: Failure,
    ) -> BoxFuture<'a, Result<(), OutboxError>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn payload_admits_references_and_refuses_sensitive_fields() {
        let payload = SafePayload::empty()
            .with_reference("resource_id", "42")
            .unwrap()
            .with_outcome(Outcome::Success);
        assert_eq!(
            SafePayload::from_json(&payload.to_json().unwrap()).unwrap(),
            payload
        );
        for bad in [
            r#"{"password":"secret"}"#,
            r#"{"references":{"token":"42"},"outcome":null}"#,
            r#"{"references":{"resource_id":"private data"},"outcome":null}"#,
            r#"{"references":{},"outcome":"c3"}"#,
        ] {
            assert_eq!(SafePayload::from_json(bad), Err(OutboxError::CorruptData));
        }
    }
    #[test]
    fn webhook_has_version_and_fixed_time_without_live_delivery() {
        let event = OutboxEvent {
            event_id: crate::transaction::new_operation_id(),
            aggregate_type: "local".into(),
            aggregate_id: "1".into(),
            event_type: "local.probe".into(),
            version: 1,
            occurred_at: crate::db::decode_time("2026-09-05T00:00:00Z").unwrap(),
            payload: SafePayload::empty(),
            trace_id: String::new(),
        };
        let wire = WebhookEventV1::try_from(&event).unwrap();
        assert_eq!(wire.schema_version, 1);
        assert_eq!(wire.occurred_at, "2026-09-05T00:00:00.000000Z");
        let json = serde_json::to_string(&wire).unwrap();
        assert_eq!(serde_json::from_str::<WebhookEventV1>(&json).unwrap(), wire);
    }
}
