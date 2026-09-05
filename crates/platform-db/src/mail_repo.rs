//! Mail side-effect storage. Repository functions only borrow the caller transaction.
//! Lock order: suppression recipient -> dispatch message -> feedback identity -> alerts
//! -> Outbox append. Message content/addresses are never persisted in these projections.
use crate::{
    Pool,
    transaction::{DbRow, DbTransaction, DbValue, TransactionConnection},
};
use chrono::{DateTime, Utc};
use platform::{
    db::lock_sql,
    identifiers::SchemaRenderer,
    integrations::{AdapterError, DiagnosticCode},
    mail::*,
    outbox::{BoxFuture, OutboxEvent, SafePayload},
};

fn unavailable<T>(_: T) -> AdapterError {
    AdapterError::Unavailable
}
fn table(
    tx: &TransactionConnection<'_>,
    r: &SchemaRenderer,
    name: &str,
) -> Result<String, AdapterError> {
    r.table(tx.backend(), "platform", name).map_err(unavailable)
}
fn text(value: &str) -> DbValue {
    DbValue::Text(value.into())
}
fn value(row: &DbRow, name: &str) -> Result<String, AdapterError> {
    row.text(name).map_err(unavailable)
}
fn number(row: &DbRow, name: &str) -> Result<i64, AdapterError> {
    row.i64(name).map_err(unavailable)
}
fn boolean(row: &DbRow, name: &str) -> Result<bool, AdapterError> {
    row.boolean(name).map_err(unavailable)
}
fn hash(message: &MailMessage) -> Result<String, AdapterError> {
    let encoded = serde_json::to_vec(&(
        &message.message_id,
        &message.recipient_ref,
        message.class.as_str(),
        &message.from,
        &message.to,
        &message.subject,
        &message.text,
        &message.unsubscribe_url,
        platform::db::encode_time(message.queued_at),
    ))
    .map_err(unavailable)?;
    Ok(platform::integrations::local::digest(&encoded))
}
async fn suppression(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    recipient: &str,
) -> Result<DbRow, AdapterError> {
    let name = table(tx, r, "mail_suppressions")?;
    tx.insert_if_absent(&format!("INSERT INTO {name} (recipient_ref, unsubscribe_marketing, complained, bounced, hard_bounces, version) VALUES (?, ?, ?, ?, 0, 1)"), &[text(recipient), DbValue::Bool(false), DbValue::Bool(false), DbValue::Bool(false)]).await.map_err(unavailable)?;
    tx.fetch_optional(&lock_sql(tx.backend(), &format!("SELECT unsubscribe_marketing, complained, bounced, hard_bounces, version FROM {name} WHERE recipient_ref = ?"), false), &[text(recipient)]).await.map_err(unavailable)?.ok_or(AdapterError::Unavailable)
}
fn is_suppressed(row: &DbRow, class: MailClass) -> Result<bool, AdapterError> {
    Ok(boolean(row, "bounced")?
        || boolean(row, "complained")?
        || (class == MailClass::Marketing && boolean(row, "unsubscribe_marketing")?))
}

pub async fn prepare(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    event: &OutboxEvent,
    message: &MailMessage,
    simulated: bool,
) -> Result<DispatchDecision, AdapterError> {
    message.validate()?;
    event.validate().map_err(|_| AdapterError::InvalidInput)?;
    if event.event_type != "mail.requested"
        || event.version != 1
        || event.payload.reference("resource_id") != Some(message.message_id.as_str())
    {
        return Err(AdapterError::InvalidInput);
    }
    let suppressed = is_suppressed(
        &suppression(tx, r, &message.recipient_ref).await?,
        message.class,
    )?;
    let name = table(tx, r, "mail_dispatches")?;
    let request_hash = hash(message)?;
    tx.insert_if_absent(&format!("INSERT INTO {name} (message_id, event_id, recipient_ref, class, state, request_hash, simulated, queued_at, observed_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1)"), &[text(&message.message_id), text(&event.event_id), text(&message.recipient_ref), text(message.class.as_str()), text(if suppressed { "suppressed" } else { "queued" }), text(&request_hash), DbValue::Bool(simulated), DbValue::Time(message.queued_at), DbValue::Time(message.queued_at)]).await.map_err(unavailable)?;
    let row = tx.fetch_optional(&lock_sql(tx.backend(), &format!("SELECT event_id, request_hash, simulated, state, version FROM {name} WHERE message_id = ?"), false), &[text(&message.message_id)]).await.map_err(unavailable)?.ok_or(AdapterError::Conflict)?;
    if value(&row, "event_id")? != event.event_id
        || value(&row, "request_hash")? != request_hash
        || boolean(&row, "simulated")? != simulated
    {
        return Err(AdapterError::Conflict);
    }
    let state = DeliveryState::parse(&value(&row, "state")?)?;
    // Acknowledgement loss may replay an already submitted delivery after new
    // feedback. Deduplication still succeeds without calling the provider again.
    if matches!(
        state,
        DeliveryState::Accepted | DeliveryState::LocalRecorded
    ) {
        return Ok(DispatchDecision::AlreadyRecorded);
    }
    if suppressed {
        // Preserve genuine prior submission observations, but persist the stop
        // reason for pending/failed deliveries instead of leaving a retryable state.
        if matches!(state, DeliveryState::Queued | DeliveryState::Failed) {
            tx.execute_versioned(&format!("UPDATE {name} SET state = 'suppressed', observed_at = ?, version = version + 1 WHERE message_id = ? AND version = ?"), &[DbValue::Time(Utc::now()), text(&message.message_id), DbValue::I64(number(&row, "version")?)]).await.map_err(unavailable)?;
        }
        return Ok(DispatchDecision::Suppressed);
    }
    if state == DeliveryState::Suppressed {
        return Ok(DispatchDecision::Suppressed);
    }
    Ok(DispatchDecision::Submit)
}

pub async fn record(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    event_id: &str,
    receipt: DeliveryReceipt,
    observed_at: DateTime<Utc>,
) -> Result<(), AdapterError> {
    if !platform::transaction::validate_operation_id(event_id)
        || !matches!(
            (receipt.state, receipt.simulated),
            (DeliveryState::LocalRecorded, true)
                | (DeliveryState::Accepted, false)
                | (DeliveryState::Failed, _)
        )
    {
        return Err(AdapterError::InvalidInput);
    }
    let name = table(tx, r, "mail_dispatches")?;
    let row = tx
        .fetch_optional(
            &lock_sql(
                tx.backend(),
                &format!(
                    "SELECT state, simulated, queued_at, version FROM {name} WHERE event_id = ?"
                ),
                false,
            ),
            &[text(event_id)],
        )
        .await
        .map_err(unavailable)?
        .ok_or(AdapterError::NotFound)?;
    if boolean(&row, "simulated")? != receipt.simulated
        || observed_at < row.time("queued_at").map_err(unavailable)?
    {
        return Err(AdapterError::InvalidInput);
    }
    let state = DeliveryState::parse(&value(&row, "state")?)?;
    if state == receipt.state {
        return Ok(());
    }
    if matches!(
        state,
        DeliveryState::Accepted | DeliveryState::LocalRecorded | DeliveryState::Suppressed
    ) {
        return Err(AdapterError::Conflict);
    }
    tx.execute_versioned(&format!("UPDATE {name} SET state = ?, observed_at = ?, version = version + 1 WHERE event_id = ? AND version = ?"), &[text(receipt.state.as_str()), DbValue::Time(observed_at), text(event_id), DbValue::I64(number(&row, "version")?)]).await.map_err(unavailable)
}

pub async fn apply_feedback(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    feedback: &TrustedFeedback,
) -> Result<FeedbackOutcome, AdapterError> {
    feedback.validate()?;
    let dispatches = table(tx, r, "mail_dispatches")?;
    // Read immutable binding first; suppression locks precede dispatch locks globally.
    let binding = tx
        .fetch_optional(
            &format!("SELECT recipient_ref FROM {dispatches} WHERE message_id = ?"),
            &[text(&feedback.message_id)],
        )
        .await
        .map_err(unavailable)?
        .ok_or(AdapterError::NotFound)?;
    let recipient = value(&binding, "recipient_ref")?;
    let previous = suppression(tx, r, &recipient).await?;
    let binding = tx.fetch_optional(&lock_sql(tx.backend(), &format!("SELECT recipient_ref, class, simulated, queued_at FROM {dispatches} WHERE message_id = ?"), false), &[text(&feedback.message_id)]).await.map_err(unavailable)?.ok_or(AdapterError::NotFound)?;
    if value(&binding, "recipient_ref")? != recipient
        || boolean(&binding, "simulated")? != feedback.simulated
        || feedback.occurred_at < binding.time("queued_at").map_err(unavailable)?
    {
        return Err(AdapterError::InvalidInput);
    }
    let class = MailClass::parse(&value(&binding, "class")?)?;
    // Marketing unsubscribe is only valid for a message that was actually marketing.
    if feedback.kind == FeedbackKind::Unsubscribe && class != MailClass::Marketing {
        return Err(AdapterError::InvalidInput);
    }
    let records = table(tx, r, "mail_feedback")?;
    let alerts = table(tx, r, "mail_alerts")?;
    let inserted = tx.insert_if_absent(&format!("INSERT INTO {records} (provider, feedback_id, message_id, kind, fingerprint, occurred_at, received_at, simulated) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"), &[text(&feedback.provider), text(&feedback.feedback_id), text(&feedback.message_id), text(feedback.kind.as_str()), text(&feedback.fingerprint()), DbValue::Time(feedback.occurred_at), DbValue::Time(feedback.received_at), DbValue::Bool(feedback.simulated)]).await.map_err(unavailable)?;
    if !inserted {
        let existing = tx
            .fetch_optional(
                &format!(
                    "SELECT fingerprint FROM {records} WHERE provider = ? AND feedback_id = ?"
                ),
                &[text(&feedback.provider), text(&feedback.feedback_id)],
            )
            .await
            .map_err(unavailable)?
            .ok_or(AdapterError::Conflict)?;
        if value(&existing, "fingerprint")? != feedback.fingerprint() {
            return Err(AdapterError::Conflict);
        }
        let alert = tx
            .fetch_optional(
                &format!("SELECT alert_id FROM {alerts} WHERE provider = ? AND feedback_id = ?"),
                &[text(&feedback.provider), text(&feedback.feedback_id)],
            )
            .await
            .map_err(unavailable)?
            .map(|row| value(&row, "alert_id"))
            .transpose()?;
        return Ok(FeedbackOutcome {
            duplicate: true,
            suppressed: is_suppressed(&previous, class)?,
            alert_id: alert,
        });
    }
    let unsubscribe =
        boolean(&previous, "unsubscribe_marketing")? || feedback.kind == FeedbackKind::Unsubscribe;
    let complained = boolean(&previous, "complained")? || feedback.kind == FeedbackKind::Complaint;
    let bounced = boolean(&previous, "bounced")?
        || matches!(
            feedback.kind,
            FeedbackKind::SoftBounce | FeedbackKind::HardBounce
        );
    let hard_bounces = number(&previous, "hard_bounces")?
        .checked_add(i64::from(feedback.kind == FeedbackKind::HardBounce))
        .ok_or(AdapterError::Unavailable)?;
    let suppressions = table(tx, r, "mail_suppressions")?;
    tx.execute_versioned(&format!("UPDATE {suppressions} SET unsubscribe_marketing = ?, complained = ?, bounced = ?, hard_bounces = ?, version = version + 1 WHERE recipient_ref = ? AND version = ?"), &[DbValue::Bool(unsubscribe), DbValue::Bool(complained), DbValue::Bool(bounced), DbValue::I64(hard_bounces), text(&recipient), DbValue::I64(number(&previous, "version")?)]).await.map_err(unavailable)?;
    let alert_id = if matches!(
        feedback.kind,
        FeedbackKind::HardBounce | FeedbackKind::Complaint
    ) {
        let id = platform::transaction::new_operation_id();
        tx.execute(&format!("INSERT INTO {alerts} (alert_id, provider, feedback_id, reason, created_at) VALUES (?, ?, ?, ?, ?)"), &[text(&id), text(&feedback.provider), text(&feedback.feedback_id), text(feedback.kind.as_str()), DbValue::Time(feedback.received_at)]).await.map_err(unavailable)?;
        let event = OutboxEvent {
            event_id: platform::transaction::new_operation_id(),
            aggregate_type: "mail_alert".into(),
            aggregate_id: id.clone(),
            event_type: "mail.alert".into(),
            version: 1,
            occurred_at: feedback.received_at,
            payload: SafePayload::empty()
                .with_reference("resource_id", &id)
                .map_err(unavailable)?,
            trace_id: String::new(),
        };
        crate::outbox_repo::enqueue(tx, r, &event, &["mail.alert"])
            .await
            .map_err(unavailable)?;
        Some(id)
    } else {
        None
    };
    Ok(FeedbackOutcome {
        duplicate: false,
        suppressed: bounced || complained || (unsubscribe && class == MailClass::Marketing),
        alert_id,
    })
}

/// Fixed-window cohorts; feedback numerator counts each submitted message once,
/// independently of how many provider feedback IDs it produced. Missing delay is None.
pub async fn metrics(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    simulated: bool,
) -> Result<MailMetricSample, AdapterError> {
    if start >= end
        || !start.timestamp_subsec_nanos().is_multiple_of(1000)
        || !end.timestamp_subsec_nanos().is_multiple_of(1000)
    {
        return Err(AdapterError::InvalidInput);
    }
    let dispatches = table(tx, r, "mail_dispatches")?;
    let feedback = table(tx, r, "mail_feedback")?;
    let rows = tx.fetch_all(&format!("SELECT message_id, queued_at, observed_at FROM {dispatches} WHERE simulated = ? AND queued_at >= ? AND queued_at < ? AND state IN ('local_recorded', 'accepted')"), &[DbValue::Bool(simulated), DbValue::Time(start), DbValue::Time(end)]).await.map_err(unavailable)?;
    let mut sample = MailMetricSample {
        simulated,
        submitted: rows.len() as u64,
        ..Default::default()
    };
    for row in rows {
        let queued = row.time("queued_at").map_err(unavailable)?;
        let observations = tx.fetch_all(&format!("SELECT kind, occurred_at FROM {feedback} WHERE message_id = ? AND simulated = ? AND occurred_at < ?"), &[text(&value(&row, "message_id")?), DbValue::Bool(simulated), DbValue::Time(end)]).await.map_err(unavailable)?;
        let mut delivered = false;
        let mut delayed = false;
        let mut bounced = false;
        let mut complained = false;
        for observation in observations {
            match value(&observation, "kind")?.as_str() {
                "delivered" => {
                    delivered = true;
                    let delay = observation
                        .time("occurred_at")
                        .map_err(unavailable)?
                        .signed_duration_since(queued)
                        .num_milliseconds();
                    let delay = u64::try_from(delay).map_err(unavailable)?;
                    sample.max_delay_ms = Some(
                        sample
                            .max_delay_ms
                            .map_or(delay, |previous| previous.max(delay)),
                    );
                }
                "delayed" => delayed = true,
                "soft_bounce" | "hard_bounce" => bounced = true,
                "complaint" => complained = true,
                "unsubscribe" => {}
                _ => return Err(AdapterError::Unavailable),
            }
        }
        sample.delivered += u64::from(delivered);
        sample.delayed += u64::from(delayed);
        sample.bounced += u64::from(bounced);
        sample.complained += u64::from(complained);
    }
    Ok(sample)
}

#[derive(Clone)]
pub struct DbMailStore {
    pool: Pool,
    renderer: SchemaRenderer,
}
impl DbMailStore {
    pub fn new(pool: Pool, renderer: SchemaRenderer) -> Self {
        Self { pool, renderer }
    }
}
impl DbMailStore {
    /// Explicit UTC cohort window; only aggregate counts escape this read path.
    pub async fn metric_snapshot(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        simulated: bool,
    ) -> Result<MailMetricSample, AdapterError> {
        let mut tx = DbTransaction::begin(&self.pool)
            .await
            .map_err(unavailable)?;
        let result = metrics(&mut tx.connection(), &self.renderer, start, end, simulated).await;
        tx.rollback().await.map_err(unavailable)?;
        result
    }
}
macro_rules! stored {
    ($this:ident, $body:expr) => {{
        let mut tx = DbTransaction::begin(&$this.pool)
            .await
            .map_err(unavailable)?;
        let result = {
            let mut conn = tx.connection();
            $body(&mut conn).await
        };
        match result {
            Ok(value) => {
                tx.commit().await.map_err(unavailable)?;
                Ok(value)
            }
            Err(error) => {
                tx.rollback().await.map_err(unavailable)?;
                Err(error)
            }
        }
    }};
}
impl MailStore for DbMailStore {
    fn prepare<'a>(
        &'a self,
        event: &'a OutboxEvent,
        message: &'a MailMessage,
        simulated: bool,
    ) -> BoxFuture<'a, Result<DispatchDecision, AdapterError>> {
        Box::pin(async move {
            stored!(self, |conn| prepare(
                conn,
                &self.renderer,
                event,
                message,
                simulated
            ))
        })
    }
    fn record<'a>(
        &'a self,
        event_id: &'a str,
        receipt: DeliveryReceipt,
        observed_at: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), AdapterError>> {
        Box::pin(async move {
            stored!(self, |conn| record(
                conn,
                &self.renderer,
                event_id,
                receipt,
                observed_at
            ))
        })
    }
}
impl FeedbackStore for DbMailStore {
    fn apply<'a>(
        &'a self,
        feedback: &'a TrustedFeedback,
    ) -> BoxFuture<'a, Result<FeedbackOutcome, AdapterError>> {
        Box::pin(
            async move { stored!(self, |conn| apply_feedback(conn, &self.renderer, feedback)) },
        )
    }
}
impl MailAlertSource for DbMailStore {
    fn load<'a>(
        &'a self,
        alert_id: &'a str,
    ) -> BoxFuture<'a, Result<DiagnosticCode, AdapterError>> {
        Box::pin(async move {
            if !platform::transaction::validate_operation_id(alert_id) {
                return Err(AdapterError::InvalidInput);
            }
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(unavailable)?;
            let name = table(&tx.connection(), &self.renderer, "mail_alerts")?;
            let row = tx
                .connection()
                .fetch_optional(
                    &format!("SELECT reason FROM {name} WHERE alert_id = ?"),
                    &[text(alert_id)],
                )
                .await
                .map_err(unavailable)?
                .ok_or(AdapterError::NotFound)?;
            tx.rollback().await.map_err(unavailable)?;
            match value(&row, "reason")?.as_str() {
                "hard_bounce" => Ok(DiagnosticCode::HardBounce),
                "complaint" => Ok(DiagnosticCode::Complaint),
                _ => Err(AdapterError::Unavailable),
            }
        })
    }
}

#[cfg(test)]
mod tests;
