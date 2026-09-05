//! Versioned, server-produced analytics facts. No public ingestion or provider is exposed.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::outbox::{BoxFuture, OutboxEvent, SafePayload, Sink, SinkError};

pub const DICTIONARY_VERSION: i64 = 1;
pub const PROJECTION: &str = "events.v1";
pub const SINK_NAME: &str = "analytics.project";
pub const EVENT_TYPE: &str = "analytics.recorded";
pub const STATISTICS_TIMEZONE: &str = "UTC";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Registered,
    EmailVerified,
    InvitationUsed,
    Read,
    TopicPublished,
    ReplyPublished,
    ValidContribution,
}
impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::EmailVerified => "email_verified",
            Self::InvitationUsed => "invitation_used",
            Self::Read => "read",
            Self::TopicPublished => "topic_published",
            Self::ReplyPublished => "reply_published",
            Self::ValidContribution => "valid_contribution",
        }
    }
    pub fn producer(self) -> &'static str {
        match self {
            Self::Registered | Self::EmailVerified | Self::InvitationUsed => "identity",
            Self::Read => "interaction",
            Self::TopicPublished | Self::ReplyPublished => "content",
            Self::ValidContribution => "trust",
        }
    }
}
pub const EVENT_DICTIONARY: &[EventKind] = &[
    EventKind::Registered,
    EventKind::EmailVerified,
    EventKind::InvitationUsed,
    EventKind::Read,
    EventKind::TopicPublished,
    EventKind::ReplyPublished,
    EventKind::ValidContribution,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    Desktop,
    Mobile,
    Tablet,
    Unknown,
}
impl Device {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Mobile => "mobile",
            Self::Tablet => "tablet",
            Self::Unknown => "unknown",
        }
    }
}

/// Opaque, server-controlled catalogue IDs only; no referrer URLs, raw user agents or tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dimensions {
    pub forum_id: Option<i64>,
    pub tag_ids: BTreeSet<i64>,
    pub channel_id: Option<i64>,
    pub device: Device,
}
impl Dimensions {
    fn validate(&self) -> Result<(), AnalyticsError> {
        if self.forum_id.is_some_and(|id| id <= 0)
            || self.channel_id.is_some_and(|id| id <= 0)
            || self.tag_ids.iter().any(|id| *id <= 0)
        {
            return Err(AnalyticsError::InvalidInput);
        }
        Ok(())
    }
    pub fn buckets(&self) -> Vec<(&'static str, String)> {
        let mut result = vec![
            ("all", "all".into()),
            ("device", self.device.as_str().into()),
        ];
        if let Some(id) = self.forum_id {
            result.push(("forum", id.to_string()));
        }
        if let Some(id) = self.channel_id {
            result.push(("channel", id.to_string()));
        }
        result.extend(self.tag_ids.iter().map(|id| ("tag", id.to_string())));
        result
    }
}

/// Construct only after current identity/content authorization has been read in the
/// producer transaction. It intentionally cannot be deserialized from HTTP input.
#[derive(Clone, Copy)]
pub struct TrustedExclusions {
    administrator: bool,
    test_account: bool,
    robot: bool,
    campaign_participation: bool,
}
impl TrustedExclusions {
    pub fn from_authority(
        administrator: bool,
        test_account: bool,
        robot: bool,
        campaign_participation: bool,
    ) -> Self {
        Self {
            administrator,
            test_account,
            robot,
            campaign_participation,
        }
    }
    fn mask(self) -> i64 {
        i64::from(self.administrator)
            | (i64::from(self.test_account) << 1)
            | (i64::from(self.robot) << 2)
            | (i64::from(self.campaign_participation) << 3)
    }
}

/// Pseudonym is an analytics-only UUIDv7, never a session token or an account identifier.
/// The future domain producer owns its mapping, validity decisions and source idempotency.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct AnalyticsFact {
    dictionary_version: i64,
    event_id: String,
    kind: EventKind,
    occurred_at: DateTime<Utc>,
    actor_pseudonym: String,
    resource_id: Option<i64>,
    dimensions: Dimensions,
    exclusion_mask: i64,
}
impl std::fmt::Debug for AnalyticsFact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalyticsFact")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl AnalyticsFact {
    pub fn from_authority(
        event_id: String,
        kind: EventKind,
        occurred_at: DateTime<Utc>,
        actor_pseudonym: String,
        resource_id: Option<i64>,
        dimensions: Dimensions,
        exclusions: TrustedExclusions,
    ) -> Result<Self, AnalyticsError> {
        let occurred_at = crate::db::decode_time(&crate::db::encode_time(occurred_at))
            .map_err(|_| AnalyticsError::InvalidInput)?;
        let fact = Self {
            dictionary_version: DICTIONARY_VERSION,
            event_id,
            kind,
            occurred_at,
            actor_pseudonym,
            resource_id,
            dimensions,
            exclusion_mask: exclusions.mask(),
        };
        fact.validate()?;
        Ok(fact)
    }
    pub fn validate(&self) -> Result<(), AnalyticsError> {
        if self.dictionary_version != DICTIONARY_VERSION
            || !crate::transaction::validate_operation_id(&self.event_id)
            || !crate::transaction::validate_operation_id(&self.actor_pseudonym)
            || self.resource_id.is_some_and(|id| id <= 0)
            || !(0..=15).contains(&self.exclusion_mask)
            || self.occurred_at < DateTime::UNIX_EPOCH
            || !self
                .occurred_at
                .timestamp_subsec_nanos()
                .is_multiple_of(1000)
        {
            return Err(AnalyticsError::InvalidInput);
        }
        self.dimensions.validate()
    }
    pub fn event_id(&self) -> &str {
        &self.event_id
    }
    pub fn kind(&self) -> EventKind {
        self.kind
    }
    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.occurred_at
    }
    pub fn actor_pseudonym(&self) -> &str {
        &self.actor_pseudonym
    }
    pub fn resource_id(&self) -> Option<i64> {
        self.resource_id
    }
    pub fn dimensions(&self) -> &Dimensions {
        &self.dimensions
    }
    pub fn excluded(&self) -> bool {
        self.exclusion_mask != 0
    }
    pub fn utc_day(&self) -> DateTime<Utc> {
        self.occurred_at
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("valid midnight")
            .and_utc()
    }
    pub fn to_json(&self) -> Result<String, AnalyticsError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|_| AnalyticsError::InvalidInput)
    }
    /// Storage-only decoding does not grant producer authority. Consumers only call this
    /// after loading the source row joined to committed Outbox and its delivery target.
    pub fn from_storage_json(json: &str) -> Result<Self, AnalyticsError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Stored {
            dictionary_version: i64,
            event_id: String,
            kind: EventKind,
            occurred_at: DateTime<Utc>,
            actor_pseudonym: String,
            resource_id: Option<i64>,
            dimensions: Dimensions,
            exclusion_mask: i64,
        }
        let stored: Stored = serde_json::from_str(json).map_err(|_| AnalyticsError::CorruptData)?;
        let fact = Self {
            dictionary_version: stored.dictionary_version,
            event_id: stored.event_id,
            kind: stored.kind,
            occurred_at: stored.occurred_at,
            actor_pseudonym: stored.actor_pseudonym,
            resource_id: stored.resource_id,
            dimensions: stored.dimensions,
            exclusion_mask: stored.exclusion_mask,
        };
        fact.validate().map_err(|_| AnalyticsError::CorruptData)?;
        Ok(fact)
    }
    pub fn outbox_event(&self, trace_id: String) -> Result<OutboxEvent, AnalyticsError> {
        self.validate()?;
        let event = OutboxEvent {
            event_id: self.event_id.clone(),
            aggregate_type: "analytics".into(),
            aggregate_id: self.event_id.clone(),
            event_type: EVENT_TYPE.into(),
            version: DICTIONARY_VERSION,
            occurred_at: self.occurred_at,
            payload: SafePayload::empty(),
            trace_id,
        };
        event.validate().map_err(|_| AnalyticsError::InvalidInput)?;
        Ok(event)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregate {
    pub utc_day: DateTime<Utc>,
    pub event_kind: String,
    pub dimension: String,
    pub dimension_value: String,
    pub count: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watermark {
    pub processed: i64,
    pub excluded: i64,
    /// These maxima describe observations, never a cursor used to discard late events.
    pub max_occurred_at: Option<DateTime<Utc>>,
    pub max_received_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub maximum_delay_ms: Option<i64>,
    pub version: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutcome {
    Applied,
    Excluded,
    Duplicate,
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnalyticsError {
    #[error("分析处理已关闭")]
    Disabled,
    #[error("分析输入非法")]
    InvalidInput,
    #[error("分析存储不可用（底层信息已脱敏）")]
    Unavailable,
    #[error("分析事实与已提交事件不匹配")]
    SourceMismatch,
    #[error("分析数据损坏")]
    CorruptData,
}
pub trait AnalyticsStore: Send + Sync {
    fn process<'a>(
        &'a self,
        event: &'a OutboxEvent,
        at: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<ProcessOutcome, AnalyticsError>>;
}
pub struct DisabledAnalytics;
impl AnalyticsStore for DisabledAnalytics {
    fn process<'a>(
        &'a self,
        _: &'a OutboxEvent,
        _: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<ProcessOutcome, AnalyticsError>> {
        Box::pin(async { Err(AnalyticsError::Disabled) })
    }
}
pub struct AnalyticsSink {
    store: Arc<dyn AnalyticsStore>,
}
impl AnalyticsSink {
    pub fn new(store: Arc<dyn AnalyticsStore>) -> Self {
        Self { store }
    }
}
impl Sink for AnalyticsSink {
    fn name(&self) -> &str {
        SINK_NAME
    }
    fn is_external(&self) -> bool {
        false
    }
    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            self.store
                .process(event, Utc::now())
                .await
                .map(|_| ())
                .map_err(|error| match error {
                    AnalyticsError::Disabled => SinkError::Disabled,
                    AnalyticsError::Unavailable => SinkError::Retryable,
                    _ => SinkError::Permanent,
                })
        })
    }
}

/// A UTC half-open query interval. It does not guess a WAU/MAU or contributor definition.
#[derive(Debug, Clone, Copy)]
pub struct StatisticsWindow {
    pub from: DateTime<Utc>,
    pub until: DateTime<Utc>,
}
impl StatisticsWindow {
    pub fn natural_day(at: DateTime<Utc>) -> Result<Self, AnalyticsError> {
        let date = at.date_naive();
        Self::dates(date, date.succ_opt().ok_or(AnalyticsError::InvalidInput)?)
    }
    pub fn natural_month(at: DateTime<Utc>) -> Result<Self, AnalyticsError> {
        let from = NaiveDate::from_ymd_opt(at.year(), at.month(), 1)
            .ok_or(AnalyticsError::InvalidInput)?;
        let (year, month) = if at.month() == 12 {
            (at.year() + 1, 1)
        } else {
            (at.year(), at.month() + 1)
        };
        let until = NaiveDate::from_ymd_opt(year, month, 1).ok_or(AnalyticsError::InvalidInput)?;
        Self::dates(from, until)
    }
    fn dates(from: NaiveDate, until: NaiveDate) -> Result<Self, AnalyticsError> {
        let window = Self {
            from: from
                .and_hms_opt(0, 0, 0)
                .ok_or(AnalyticsError::InvalidInput)?
                .and_utc(),
            until: until
                .and_hms_opt(0, 0, 0)
                .ok_or(AnalyticsError::InvalidInput)?
                .and_utc(),
        };
        window.validate()?;
        Ok(window)
    }
    pub fn validate(self) -> Result<(), AnalyticsError> {
        if self.from < DateTime::UNIX_EPOCH
            || self.from >= self.until
            || !self.from.timestamp_subsec_nanos().is_multiple_of(1000)
            || !self.until.timestamp_subsec_nanos().is_multiple_of(1000)
        {
            return Err(AnalyticsError::InvalidInput);
        }
        Ok(())
    }
    pub fn contains(self, time: DateTime<Utc>) -> bool {
        self.from <= time && time < self.until
    }
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WindowSummary {
    pub event_counts: BTreeMap<EventKind, usize>,
    pub unique_actors: usize,
    pub unique_actors_by_kind: BTreeMap<EventKind, usize>,
}
pub fn summarize<'a>(
    facts: impl IntoIterator<Item = &'a AnalyticsFact>,
    window: StatisticsWindow,
) -> Result<WindowSummary, AnalyticsError> {
    window.validate()?;
    let mut summary = WindowSummary::default();
    let mut actors = BTreeSet::new();
    let mut actors_by_kind: BTreeMap<EventKind, BTreeSet<&str>> = BTreeMap::new();
    let mut events = BTreeSet::new();
    for fact in facts {
        fact.validate()?;
        if fact.excluded()
            || !window.contains(fact.occurred_at())
            || !events.insert(fact.event_id())
        {
            continue;
        }
        *summary.event_counts.entry(fact.kind()).or_default() += 1;
        actors.insert(fact.actor_pseudonym());
        actors_by_kind
            .entry(fact.kind())
            .or_default()
            .insert(fact.actor_pseudonym());
    }
    summary.unique_actors = actors.len();
    summary.unique_actors_by_kind = actors_by_kind
        .into_iter()
        .map(|(kind, ids)| (kind, ids.len()))
        .collect();
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dictionary_and_storage_boundary_have_no_arbitrary_text() {
        assert_eq!(EVENT_DICTIONARY.len(), 7);
        assert!(AnalyticsFact::from_storage_json(r#"{"password":"secret"}"#).is_err());
        assert_eq!(STATISTICS_TIMEZONE, "UTC");
        let window = StatisticsWindow {
            from: DateTime::UNIX_EPOCH,
            until: DateTime::UNIX_EPOCH,
        };
        assert_eq!(window.validate(), Err(AnalyticsError::InvalidInput));
    }
}
