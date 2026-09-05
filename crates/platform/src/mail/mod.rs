//! Mail delivery and feedback contracts. Addresses and bodies never enter Outbox,
//! diagnostics or Debug; sources are injected by the owning application service.
use crate::integrations::{
    AdapterError, DependencyKind, DiagnosticCode, ErrorTracker, SafeDiagnostic,
};
use crate::outbox::{BoxFuture, OutboxEvent, Sink, SinkError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub mod local;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailClass {
    Security,
    Transactional,
    Marketing,
}
impl MailClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::Transactional => "transactional",
            Self::Marketing => "marketing",
        }
    }
    pub fn parse(value: &str) -> Result<Self, AdapterError> {
        match value {
            "security" => Ok(Self::Security),
            "transactional" => Ok(Self::Transactional),
            "marketing" => Ok(Self::Marketing),
            _ => Err(AdapterError::InvalidInput),
        }
    }
}

#[derive(Clone)]
pub struct MailMessage {
    pub message_id: String,
    pub recipient_ref: String,
    pub class: MailClass,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub text: String,
    pub unsubscribe_url: Option<String>,
    pub queued_at: DateTime<Utc>,
}
impl std::fmt::Debug for MailMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailMessage")
            .field("class", &self.class)
            .field("content", &"[REDACTED]")
            .finish()
    }
}
fn mailbox(value: &str) -> bool {
    value.len() <= 254
        && value.is_ascii()
        && !value.bytes().any(|b| {
            b.is_ascii_whitespace()
                || b.is_ascii_control()
                || matches!(b, b'<' | b'>' | b'"' | b'(' | b')' | b',' | b';')
        })
        && value.split('@').count() == 2
        && value.split('@').all(|part| !part.is_empty())
}
impl MailMessage {
    pub fn validate(&self) -> Result<(), AdapterError> {
        if !crate::transaction::validate_operation_id(&self.message_id)
            || !crate::transaction::validate_operation_id(&self.recipient_ref)
            || !mailbox(&self.from)
            || !mailbox(&self.to)
            || self.subject.is_empty()
            || self.subject.len() > 512
            || self.subject.chars().any(char::is_control)
            || self.text.is_empty()
            || self.text.len() > 262_144
            || self.text.contains('\0')
            || !self.queued_at.timestamp_subsec_nanos().is_multiple_of(1000)
        {
            return Err(AdapterError::InvalidInput);
        }
        match (self.class, &self.unsubscribe_url) {
            (MailClass::Marketing, Some(value)) => {
                let url = url::Url::parse(value).map_err(|_| AdapterError::InvalidInput)?;
                if url.scheme() != "https"
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.host_str().is_none()
                    || url.fragment().is_some()
                    || value.contains(['\r', '\n', '<', '>'])
                    || value.len() > 2048
                {
                    return Err(AdapterError::InvalidInput);
                }
            }
            (MailClass::Marketing, None) | (_, Some(_)) => return Err(AdapterError::InvalidInput),
            (_, None) => {}
        }
        Ok(())
    }
}
pub trait MailMessageSource: Send + Sync {
    fn load<'a>(&'a self, message_id: &'a str) -> BoxFuture<'a, Result<MailMessage, AdapterError>>;
}
pub struct UnwiredMailSource;
impl MailMessageSource for UnwiredMailSource {
    fn load<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<MailMessage, AdapterError>> {
        Box::pin(async { Err(AdapterError::Disabled) })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    Queued,
    LocalRecorded,
    Accepted,
    Suppressed,
    Failed,
}
impl DeliveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::LocalRecorded => "local_recorded",
            Self::Accepted => "accepted",
            Self::Suppressed => "suppressed",
            Self::Failed => "failed",
        }
    }
    pub fn parse(value: &str) -> Result<Self, AdapterError> {
        match value {
            "queued" => Ok(Self::Queued),
            "local_recorded" => Ok(Self::LocalRecorded),
            "accepted" => Ok(Self::Accepted),
            "suppressed" => Ok(Self::Suppressed),
            "failed" => Ok(Self::Failed),
            _ => Err(AdapterError::InvalidInput),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryReceipt {
    pub state: DeliveryState,
    pub simulated: bool,
}
pub trait MailProvider: Send + Sync {
    fn enabled(&self) -> bool {
        true
    }
    fn simulated(&self) -> bool {
        false
    }
    /// Provider must implement durable idempotency for event_id; accepted is not delivered.
    fn submit<'a>(
        &'a self,
        event_id: &'a str,
        message: &'a MailMessage,
    ) -> BoxFuture<'a, Result<DeliveryReceipt, AdapterError>>;
}
pub struct DisabledMailProvider;
impl MailProvider for DisabledMailProvider {
    fn enabled(&self) -> bool {
        false
    }
    fn submit<'a>(
        &'a self,
        _: &'a str,
        _: &'a MailMessage,
    ) -> BoxFuture<'a, Result<DeliveryReceipt, AdapterError>> {
        Box::pin(async { Err(AdapterError::Disabled) })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchDecision {
    Submit,
    AlreadyRecorded,
    Suppressed,
}
pub trait MailStore: Send + Sync {
    fn prepare<'a>(
        &'a self,
        event: &'a OutboxEvent,
        message: &'a MailMessage,
        simulated: bool,
    ) -> BoxFuture<'a, Result<DispatchDecision, AdapterError>>;
    fn record<'a>(
        &'a self,
        event_id: &'a str,
        receipt: DeliveryReceipt,
        observed_at: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), AdapterError>>;
}

/// No public HTTP parser implements this trust boundary. Real providers must verify
/// authenticity and bind feedback to their sent message before yielding these records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    Delivered,
    Delayed,
    SoftBounce,
    HardBounce,
    Complaint,
    Unsubscribe,
}
impl FeedbackKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Delayed => "delayed",
            Self::SoftBounce => "soft_bounce",
            Self::HardBounce => "hard_bounce",
            Self::Complaint => "complaint",
            Self::Unsubscribe => "unsubscribe",
        }
    }
}
#[derive(Clone)]
pub struct TrustedFeedback {
    pub provider: String,
    pub feedback_id: String,
    pub message_id: String,
    pub kind: FeedbackKind,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub simulated: bool,
}
impl std::fmt::Debug for TrustedFeedback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustedFeedback")
            .field("kind", &self.kind)
            .field("simulated", &self.simulated)
            .finish()
    }
}
impl TrustedFeedback {
    pub fn validate(&self) -> Result<(), AdapterError> {
        if !crate::outbox::is_machine_name(&self.provider)
            || !crate::transaction::validate_operation_id(&self.feedback_id)
            || !crate::transaction::validate_operation_id(&self.message_id)
            || self.received_at < self.occurred_at
            || !self
                .occurred_at
                .timestamp_subsec_nanos()
                .is_multiple_of(1000)
            || !self
                .received_at
                .timestamp_subsec_nanos()
                .is_multiple_of(1000)
        {
            return Err(AdapterError::InvalidInput);
        }
        Ok(())
    }
    /// Reception time may differ on retry; identity binds stable provider semantics.
    pub fn fingerprint(&self) -> String {
        crate::integrations::local::digest(
            format!(
                "{}\n{}\n{}\n{}\n{}\n{}",
                self.provider,
                self.feedback_id,
                self.message_id,
                self.kind.as_str(),
                crate::db::encode_time(self.occurred_at),
                self.simulated
            )
            .as_bytes(),
        )
    }
}
pub trait FeedbackSource: Send + Sync {
    /// Read without acknowledging. Failed persistence must leave the feedback pending.
    fn next<'a>(&'a self) -> BoxFuture<'a, Result<Option<TrustedFeedback>, AdapterError>>;
    fn acknowledge<'a>(
        &'a self,
        feedback: &'a TrustedFeedback,
    ) -> BoxFuture<'a, Result<(), AdapterError>>;
}
pub trait FeedbackStore: Send + Sync {
    fn apply<'a>(
        &'a self,
        feedback: &'a TrustedFeedback,
    ) -> BoxFuture<'a, Result<FeedbackOutcome, AdapterError>>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackOutcome {
    pub duplicate: bool,
    pub suppressed: bool,
    pub alert_id: Option<String>,
}
pub async fn consume_feedback(
    source: &dyn FeedbackSource,
    store: &dyn FeedbackStore,
) -> Result<Option<FeedbackOutcome>, AdapterError> {
    match source.next().await? {
        Some(feedback) => {
            let result = store.apply(&feedback).await?;
            source.acknowledge(&feedback).await?;
            Ok(Some(result))
        }
        None => Ok(None),
    }
}

/// Production thresholds intentionally have no defaults. All rates use a supplied
/// closed observation window and explicit minimum sample size, not process uptime.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MailAlertConfig {
    pub min_samples: Option<u64>,
    pub delivery_delay_ms: Option<u64>,
    pub bounce_rate: Option<f64>,
    pub complaint_rate: Option<f64>,
}
impl MailAlertConfig {
    pub fn validate(&self) -> Result<(), AdapterError> {
        if self.min_samples == Some(0)
            || self.delivery_delay_ms == Some(0)
            || [self.bounce_rate, self.complaint_rate]
                .into_iter()
                .flatten()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
            || ((self.bounce_rate.is_some() || self.complaint_rate.is_some())
                && self.min_samples.is_none())
        {
            return Err(AdapterError::InvalidInput);
        }
        Ok(())
    }
    pub fn evaluate(
        &self,
        sample: &MailMetricSample,
    ) -> Result<Vec<MailMetricAlert>, AdapterError> {
        self.validate()?;
        let mut alerts = Vec::new();
        if self
            .delivery_delay_ms
            .zip(sample.max_delay_ms)
            .is_some_and(|(limit, measured)| measured > limit)
        {
            alerts.push(MailMetricAlert::Delay);
        }
        if self.min_samples.is_some_and(|min| sample.submitted >= min) && sample.submitted > 0 {
            if self
                .bounce_rate
                .is_some_and(|limit| sample.bounced as f64 / sample.submitted as f64 > limit)
            {
                alerts.push(MailMetricAlert::BounceRate);
            }
            if self
                .complaint_rate
                .is_some_and(|limit| sample.complained as f64 / sample.submitted as f64 > limit)
            {
                alerts.push(MailMetricAlert::ComplaintRate);
            }
        }
        Ok(alerts)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MailMetricAlert {
    Delay,
    BounceRate,
    ComplaintRate,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MailMetricSample {
    pub simulated: bool,
    pub submitted: u64,
    pub delivered: u64,
    pub delayed: u64,
    pub bounced: u64,
    pub complained: u64,
    pub max_delay_ms: Option<u64>,
}

pub struct MailSink {
    pub source: Arc<dyn MailMessageSource>,
    pub store: Arc<dyn MailStore>,
    pub provider: Arc<dyn MailProvider>,
}
impl Sink for MailSink {
    fn name(&self) -> &str {
        "mail.delivery"
    }
    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            event.validate().map_err(|_| SinkError::Permanent)?;
            if event.event_type != "mail.requested" || event.version != 1 {
                return Err(SinkError::Permanent);
            }
            // An unselected provider must not bind a pending event to a real/local
            // mode. The authoritative Outbox remains queued for later selection.
            if !self.provider.enabled() {
                return Err(SinkError::Disabled);
            }
            let reference = event
                .payload
                .reference("resource_id")
                .ok_or(SinkError::Permanent)?;
            let message = self.source.load(reference).await?;
            message.validate()?;
            if message.message_id != reference {
                return Err(SinkError::Permanent);
            }
            match self
                .store
                .prepare(event, &message, self.provider.simulated())
                .await?
            {
                DispatchDecision::AlreadyRecorded => return Ok(()),
                DispatchDecision::Suppressed => return Err(SinkError::Permanent),
                DispatchDecision::Submit => {}
            }
            let receipt = match self.provider.submit(&event.event_id, &message).await {
                Ok(receipt) => receipt,
                Err(error) => {
                    if error != AdapterError::Disabled {
                        crate::observability::metrics()
                            .increment(crate::observability::Metric::MailError);
                        self.store
                            .record(
                                &event.event_id,
                                DeliveryReceipt {
                                    state: DeliveryState::Failed,
                                    simulated: self.provider.simulated(),
                                },
                                Utc::now(),
                            )
                            .await?;
                    }
                    return Err(error.into());
                }
            };
            if receipt.simulated != self.provider.simulated()
                || !matches!(
                    (receipt.state, receipt.simulated),
                    (DeliveryState::LocalRecorded, true) | (DeliveryState::Accepted, false)
                )
            {
                return Err(SinkError::Permanent);
            }
            self.store
                .record(&event.event_id, receipt, Utc::now())
                .await?;
            Ok(())
        })
    }
}

pub trait MailAlertSource: Send + Sync {
    fn load<'a>(&'a self, alert_id: &'a str)
    -> BoxFuture<'a, Result<DiagnosticCode, AdapterError>>;
}
pub struct MailAlertSink {
    pub source: Arc<dyn MailAlertSource>,
    pub tracker: Arc<dyn ErrorTracker>,
}
impl Sink for MailAlertSink {
    fn name(&self) -> &str {
        "mail.alert"
    }
    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            if event.event_type != "mail.alert" || event.version != 1 {
                return Err(SinkError::Permanent);
            }
            let id = event
                .payload
                .reference("resource_id")
                .ok_or(SinkError::Permanent)?;
            let code = self.source.load(id).await?;
            self.tracker
                .record(
                    &event.event_id,
                    &SafeDiagnostic {
                        dependency: DependencyKind::Mail,
                        code,
                    },
                )
                .await?;
            Ok(())
        })
    }
}
