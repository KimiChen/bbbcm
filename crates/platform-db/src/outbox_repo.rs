//! Transaction-borrowing Outbox repository and application-injected storage adapter.
use crate::Pool;
use crate::transaction::{DbRow, DbTransaction, DbValue, TransactionConnection, TransactionError};
use chrono::{DateTime, Utc};
use platform::db::lock_sql;
use platform::identifiers::SchemaRenderer;
use platform::outbox::{
    BoxFuture, Claim, Failure, FailureCode, LeasedEvent, OutboxError, OutboxEvent, OutboxStore,
    ReplayRequest, ReplayResult, SafePayload,
};

fn unavailable<T>(_: T) -> OutboxError {
    OutboxError::Unavailable
}
fn corrupt<T>(_: T) -> OutboxError {
    OutboxError::CorruptData
}
fn table(
    renderer: &SchemaRenderer,
    backend: platform::db::Backend,
    name: &str,
) -> Result<String, OutboxError> {
    renderer
        .table(backend, "platform", name)
        .map_err(unavailable)
}
fn epoch() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}
fn text(value: &str) -> DbValue {
    DbValue::Text(value.into())
}

/// Appending a delivery plan is part of the caller's existing business transaction.
/// A later failure must roll back this event, its target rows and the business together.
pub async fn enqueue(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    event: &OutboxEvent,
    sinks: &[&str],
) -> Result<(), OutboxError> {
    let event = event.clone().with_inherited_trace()?;
    if sinks.is_empty()
        || sinks.len() > 32
        || sinks.iter().any(|s| !platform::outbox::is_machine_name(s))
        || sinks
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != sinks.len()
    {
        return Err(OutboxError::InvalidInput);
    }
    tx.enter_append_phase();
    let events = table(renderer, tx.backend(), "outbox_events")?;
    let deliveries = table(renderer, tx.backend(), "sink_deliveries")?;
    tx.execute(&format!("INSERT INTO {events} (event_id, aggregate_type, aggregate_id, event_type, version, occurred_at, payload, trace_id, state, attempts, available_at, lease_owner, lease_batch_id, lease_until, error_code, replay_count) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'ready', 0, ?, '', '', ?, '', 0)"), &[text(&event.event_id), text(&event.aggregate_type), text(&event.aggregate_id), text(&event.event_type), DbValue::I64(event.version), DbValue::Time(event.occurred_at), text(&event.payload.to_json()?), text(&event.trace_id), DbValue::Time(event.occurred_at), DbValue::Time(epoch())]).await.map_err(unavailable)?;
    for sink in sinks {
        tx.execute(&format!("INSERT INTO {deliveries} (sink, event_id, state, delivered_at) VALUES (?, ?, 'pending', ?)"), &[text(sink), text(&event.event_id), DbValue::Time(epoch())]).await.map_err(unavailable)?;
    }
    Ok(())
}

pub async fn claim(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    request: &Claim,
) -> Result<Vec<LeasedEvent>, OutboxError> {
    if request.owner.is_empty()
        || request.owner.len() > 192
        || !platform::transaction::validate_operation_id(&request.batch_id)
        || request.limit == 0
        || request.limit > 100
        || request.lease_until <= request.now
    {
        return Err(OutboxError::InvalidInput);
    }
    let events = table(renderer, tx.backend(), "outbox_events")?;
    let deliveries = table(renderer, tx.backend(), "sink_deliveries")?;
    // PostgreSQL locks just the selected batch; SQLite BEGIN IMMEDIATE protects all three steps.
    let select = lock_sql(
        tx.backend(),
        &format!(
            "SELECT event_id FROM {events} WHERE (state = 'ready' AND available_at <= ?) OR (state = 'leased' AND lease_until <= ?) ORDER BY available_at, event_id LIMIT ?"
        ),
        true,
    );
    let rows = tx
        .fetch_all(
            &select,
            &[
                DbValue::Time(request.now),
                DbValue::Time(request.now),
                DbValue::I64(i64::from(request.limit)),
            ],
        )
        .await
        .map_err(unavailable)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = rows
        .iter()
        .map(|r| r.text("event_id").map_err(corrupt))
        .collect::<Result<_, _>>()?;
    let update = platform::db::outbox_claim_update_sql(renderer, tx.backend(), ids.len())
        .map_err(unavailable)?;
    let values: Vec<DbValue> = [
        text(&request.owner),
        text(&request.batch_id),
        DbValue::Time(request.lease_until),
    ]
    .into_iter()
    .chain(ids.iter().map(|id| text(id)))
    .chain([DbValue::Time(request.now), DbValue::Time(request.now)])
    .collect();
    let affected = tx.execute(&update, &values).await.map_err(unavailable)?;
    if affected != ids.len() as u64 {
        return Err(OutboxError::LeaseLost);
    }
    let rows = tx.fetch_all(&format!("SELECT event_id, aggregate_type, aggregate_id, event_type, version, occurred_at, payload, trace_id, attempts, lease_owner, lease_batch_id, lease_until FROM {events} WHERE lease_batch_id = ? ORDER BY available_at, event_id"), &[text(&request.batch_id)]).await.map_err(unavailable)?;
    if rows.len() != ids.len() {
        return Err(OutboxError::CorruptData);
    }
    let mut result = vec![None; rows.len()];
    for (index, row) in rows.into_iter().enumerate() {
        let event = read_event(&row)?;
        let pending = tx.fetch_all(&format!("SELECT sink FROM {deliveries} WHERE event_id = ? AND state = 'pending' ORDER BY sink"), &[text(&event.event_id)]).await.map_err(unavailable)?;
        result[index] = Some(LeasedEvent {
            event,
            owner: row.text("lease_owner").map_err(corrupt)?,
            batch_id: row.text("lease_batch_id").map_err(corrupt)?,
            lease_until: row.time("lease_until").map_err(corrupt)?,
            attempts: row.i64("attempts").map_err(corrupt)?,
            pending_sinks: pending
                .iter()
                .map(|r| r.text("sink").map_err(corrupt))
                .collect::<Result<_, _>>()?,
        });
    }
    Ok(result.into_iter().flatten().collect())
}
fn read_event(row: &DbRow) -> Result<OutboxEvent, OutboxError> {
    let event = OutboxEvent {
        event_id: row.text("event_id").map_err(corrupt)?,
        aggregate_type: row.text("aggregate_type").map_err(corrupt)?,
        aggregate_id: row.text("aggregate_id").map_err(corrupt)?,
        event_type: row.text("event_type").map_err(corrupt)?,
        version: row.i64("version").map_err(corrupt)?,
        occurred_at: row.time("occurred_at").map_err(corrupt)?,
        payload: SafePayload::from_json(&row.text("payload").map_err(corrupt)?)?,
        trace_id: row.text("trace_id").map_err(corrupt)?,
    };
    event.validate().map_err(corrupt)?;
    Ok(event)
}
fn lease_values(lease: &LeasedEvent, now: DateTime<Utc>) -> Vec<DbValue> {
    vec![
        text(&lease.event.event_id),
        text(&lease.owner),
        text(&lease.batch_id),
        DbValue::Time(now),
    ]
}
async fn ensure_held(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    lease: &LeasedEvent,
    now: DateTime<Utc>,
) -> Result<(), OutboxError> {
    let events = table(renderer, tx.backend(), "outbox_events")?;
    let sql = lock_sql(
        tx.backend(),
        &format!(
            "SELECT event_id FROM {events} WHERE event_id = ? AND lease_owner = ? AND lease_batch_id = ? AND state = 'leased' AND lease_until > ?"
        ),
        false,
    );
    if tx
        .fetch_optional(&sql, &lease_values(lease, now))
        .await
        .map_err(unavailable)?
        .is_none()
    {
        return Err(OutboxError::LeaseLost);
    }
    Ok(())
}
pub async fn renew(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    lease: &LeasedEvent,
    now: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<(), OutboxError> {
    if until <= now {
        return Err(OutboxError::InvalidInput);
    }
    let events = table(renderer, tx.backend(), "outbox_events")?;
    let values: Vec<DbValue> = [DbValue::Time(until)]
        .into_iter()
        .chain(lease_values(lease, now))
        .collect();
    let count = tx.execute(&format!("UPDATE {events} SET lease_until = ? WHERE event_id = ? AND lease_owner = ? AND lease_batch_id = ? AND state = 'leased' AND lease_until > ?"), &values).await.map_err(unavailable)?;
    if count != 1 {
        return Err(OutboxError::LeaseLost);
    }
    Ok(())
}
pub async fn delivery_done(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    lease: &LeasedEvent,
    sink: &str,
    now: DateTime<Utc>,
) -> Result<(), OutboxError> {
    ensure_held(tx, renderer, lease, now).await?;
    let deliveries = table(renderer, tx.backend(), "sink_deliveries")?;
    let count = tx.execute(&format!("UPDATE {deliveries} SET state = 'delivered', delivered_at = ? WHERE sink = ? AND event_id = ? AND state = 'pending'"), &[DbValue::Time(now), text(sink), text(&lease.event.event_id)]).await.map_err(unavailable)?;
    if count == 0 {
        let row = tx
            .fetch_optional(
                &format!("SELECT state FROM {deliveries} WHERE sink = ? AND event_id = ?"),
                &[text(sink), text(&lease.event.event_id)],
            )
            .await
            .map_err(unavailable)?
            .ok_or(OutboxError::InvalidInput)?;
        if row.text("state").map_err(corrupt)? != "delivered" {
            return Err(OutboxError::CorruptData);
        }
    }
    Ok(())
}
pub async fn complete(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    lease: &LeasedEvent,
    now: DateTime<Utc>,
) -> Result<(), OutboxError> {
    ensure_held(tx, renderer, lease, now).await?;
    let events = table(renderer, tx.backend(), "outbox_events")?;
    let deliveries = table(renderer, tx.backend(), "sink_deliveries")?;
    let remaining = tx
        .fetch_optional(
            &format!(
                "SELECT sink FROM {deliveries} WHERE event_id = ? AND state = 'pending' LIMIT 1"
            ),
            &[text(&lease.event.event_id)],
        )
        .await
        .map_err(unavailable)?;
    if remaining.is_some() {
        return Err(OutboxError::InvalidInput);
    }
    let values: Vec<DbValue> = [DbValue::Time(epoch())]
        .into_iter()
        .chain(lease_values(lease, now))
        .collect();
    if tx.execute(&format!("UPDATE {events} SET state = 'delivered', lease_owner = '', lease_batch_id = '', lease_until = ?, error_code = '' WHERE event_id = ? AND lease_owner = ? AND lease_batch_id = ? AND state = 'leased' AND lease_until > ?"), &values).await.map_err(unavailable)? != 1 { return Err(OutboxError::LeaseLost); }
    Ok(())
}
pub async fn fail(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    lease: &LeasedEvent,
    failure: &Failure,
) -> Result<(), OutboxError> {
    if failure.available_at < failure.now {
        return Err(OutboxError::InvalidInput);
    }
    let events = table(renderer, tx.backend(), "outbox_events")?;
    let paused = matches!(
        failure.code,
        FailureCode::ExternalDisabled | FailureCode::Cancelled
    );
    if paused && failure.dead_letter {
        return Err(OutboxError::InvalidInput);
    }
    let values: Vec<DbValue> = [
        text(if failure.dead_letter { "dlq" } else { "ready" }),
        DbValue::Time(failure.available_at),
        DbValue::Time(epoch()),
        text(failure.code.as_str()),
        DbValue::I64(i64::from(paused)),
    ]
    .into_iter()
    .chain(lease_values(lease, failure.now))
    .collect();
    if tx.execute(&format!("UPDATE {events} SET state = ?, available_at = ?, lease_owner = '', lease_batch_id = '', lease_until = ?, error_code = ?, attempts = attempts - ? WHERE event_id = ? AND lease_owner = ? AND lease_batch_id = ? AND state = 'leased' AND lease_until > ?"), &values).await.map_err(unavailable)? != 1 { return Err(OutboxError::LeaseLost); }
    Ok(())
}

/// Production implementations must authorize current actor/scope and append replay audit
/// in this borrowed transaction. No permissive implementation ships in runtime code.
pub trait ReplayPolicy: Send + Sync {
    fn authorize<'a, 'c>(
        &'a self,
        tx: &'a mut TransactionConnection<'c>,
        request: &'a ReplayRequest,
    ) -> BoxFuture<'a, Result<(), OutboxError>>;
    fn append_audit<'a, 'c>(
        &'a self,
        tx: &'a mut TransactionConnection<'c>,
        request: &'a ReplayRequest,
    ) -> BoxFuture<'a, Result<(), OutboxError>>;
}
fn replay_error(error: TransactionError) -> OutboxError {
    match error {
        TransactionError::IdempotencyConflict => OutboxError::ReplayConflict,
        TransactionError::ResponseExpired => OutboxError::ReplayExpired,
        _ => OutboxError::Unavailable,
    }
}

#[derive(Clone)]
pub struct DbOutboxStore {
    pool: Pool,
    renderer: SchemaRenderer,
}
impl DbOutboxStore {
    pub fn new(pool: Pool, renderer: SchemaRenderer) -> Self {
        Self { pool, renderer }
    }
    pub async fn replay(
        &self,
        request: &ReplayRequest,
        policy: &dyn ReplayPolicy,
        now: DateTime<Utc>,
    ) -> Result<ReplayResult, OutboxError> {
        let identity = request.command_identity()?;
        let mut tx = DbTransaction::begin(&self.pool)
            .await
            .map_err(unavailable)?;
        let result = async {
            let mut conn = tx.connection();
            // The idempotency record is always first in the platform lock order.
            let reservation = crate::idempotency_repo::reserve(&mut conn, &self.renderer, &identity, now).await.map_err(replay_error)?;
            policy.authorize(&mut conn, request).await?;
            if let crate::idempotency_repo::Reservation::Replay { operation_id, .. } = reservation {
                return Ok(ReplayResult { operation_id, replayed: true });
            }
            let events = table(&self.renderer, conn.backend(), "outbox_events")?;
            let sql = lock_sql(conn.backend(), &format!("SELECT state FROM {events} WHERE event_id = ?"), false);
            let row = conn.fetch_optional(&sql, &[text(&request.event_id)]).await.map_err(unavailable)?.ok_or(OutboxError::ReplayDenied)?;
            if row.text("state").map_err(corrupt)? != "dlq" { return Err(OutboxError::ReplayDenied); }
            if conn.execute(&format!("UPDATE {events} SET state = 'ready', attempts = 0, available_at = ?, error_code = '', replay_count = replay_count + 1 WHERE event_id = ? AND state = 'dlq'"), &[DbValue::Time(now), text(&request.event_id)]).await.map_err(unavailable)? != 1 { return Err(OutboxError::ReplayDenied); }
            policy.append_audit(&mut conn, request).await?;
            let response = platform::transaction::StoredResponse { status_code: 200, body: SafePayload::empty().with_reference("operation_id", &identity.operation_id)?.to_json()? };
            crate::idempotency_repo::complete(&mut conn, &self.renderer, &identity, &response, now).await.map_err(replay_error)?;
            Ok(ReplayResult { operation_id: identity.operation_id.clone(), replayed: false })
        }.await;
        match result {
            Ok(result) if result.replayed => {
                tx.rollback().await.map_err(unavailable)?;
                Ok(result)
            }
            Ok(result) => match tx.commit().await {
                Ok(()) => Ok(result),
                Err(TransactionError::CommitUnknown) => Err(OutboxError::CommitUnknown {
                    operation_id: identity.operation_id,
                }),
                Err(error) => Err(replay_error(error)),
            },
            Err(error) => {
                tx.rollback().await.map_err(unavailable)?;
                Err(error)
            }
        }
    }
}
macro_rules! stored {
    ($this:ident, $body:expr) => {{
        let mut tx = DbTransaction::begin(&$this.pool)
            .await
            .map_err(unavailable)?;
        let result = {
            let mut connection = tx.connection();
            $body(&mut connection).await
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
impl OutboxStore for DbOutboxStore {
    fn claim<'a>(&'a self, request: Claim) -> BoxFuture<'a, Result<Vec<LeasedEvent>, OutboxError>> {
        Box::pin(async move { stored!(self, |conn| claim(conn, &self.renderer, &request)) })
    }
    fn renew<'a>(
        &'a self,
        lease: &'a LeasedEvent,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), OutboxError>> {
        Box::pin(
            async move { stored!(self, |conn| renew(conn, &self.renderer, lease, now, until)) },
        )
    }
    fn delivery_done<'a>(
        &'a self,
        lease: &'a LeasedEvent,
        sink: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), OutboxError>> {
        Box::pin(async move {
            stored!(self, |conn| delivery_done(
                conn,
                &self.renderer,
                lease,
                sink,
                now
            ))
        })
    }
    fn complete<'a>(
        &'a self,
        lease: &'a LeasedEvent,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), OutboxError>> {
        Box::pin(async move { stored!(self, |conn| complete(conn, &self.renderer, lease, now)) })
    }
    fn fail<'a>(
        &'a self,
        lease: &'a LeasedEvent,
        failure: Failure,
    ) -> BoxFuture<'a, Result<(), OutboxError>> {
        Box::pin(async move { stored!(self, |conn| fail(conn, &self.renderer, lease, &failure)) })
    }
}

#[cfg(test)]
mod tests;
