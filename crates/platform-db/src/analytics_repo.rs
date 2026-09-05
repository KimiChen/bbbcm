//! Repositories borrow the caller transaction. The injected service owns commit/rollback.
//! Lock order: projection watermark first, then receipt, then sorted aggregate keys.
use chrono::{DateTime, Utc};
use platform::analytics::{
    Aggregate, AnalyticsError, AnalyticsFact, AnalyticsStore, DICTIONARY_VERSION, EVENT_TYPE,
    PROJECTION, ProcessOutcome, SINK_NAME, StatisticsWindow, Watermark, WindowSummary,
};
use platform::db::{Backend, lock_sql};
use platform::identifiers::SchemaRenderer;
use platform::outbox::{BoxFuture, OutboxEvent};

use crate::Pool;
use crate::transaction::{DbRow, DbTransaction, DbValue, TransactionConnection};

fn unavailable<T>(_: T) -> AnalyticsError {
    AnalyticsError::Unavailable
}
fn corrupt<T>(_: T) -> AnalyticsError {
    AnalyticsError::CorruptData
}
fn text(value: &str) -> DbValue {
    DbValue::Text(value.into())
}
fn table(
    renderer: &SchemaRenderer,
    backend: Backend,
    name: &str,
) -> Result<String, AnalyticsError> {
    renderer
        .table(backend, "analytics", name)
        .map_err(unavailable)
}

/// Call only from a domain core with authority-derived facts, in the business transaction.
/// Source facts and the Outbox delivery plan roll back together; this never commits.
pub async fn enqueue(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    fact: &AnalyticsFact,
    received_at: DateTime<Utc>,
    trace_id: String,
) -> Result<OutboxEvent, AnalyticsError> {
    fact.validate()?;
    if received_at < fact.occurred_at() {
        return Err(AnalyticsError::InvalidInput);
    }
    let event = fact
        .outbox_event(trace_id)?
        .with_inherited_trace()
        .map_err(|_| AnalyticsError::InvalidInput)?;
    crate::outbox_repo::enqueue(tx, renderer, &event, &[SINK_NAME])
        .await
        .map_err(unavailable)?;
    let facts = table(renderer, tx.backend(), "event_facts")?;
    tx.execute(&format!("INSERT INTO {facts} (event_id, sink, dictionary_version, event_kind, occurred_at, received_at, fact_json) VALUES (?, ?, ?, ?, ?, ?, ?)"),
        &[text(fact.event_id()), text(SINK_NAME), DbValue::I64(DICTIONARY_VERSION), text(fact.kind().as_str()),
            DbValue::Time(fact.occurred_at()), DbValue::Time(received_at), text(&fact.to_json()?)])
        .await.map_err(unavailable)?;
    Ok(event)
}

struct StoredFact {
    fact: AnalyticsFact,
    received_at: DateTime<Utc>,
    trace_id: String,
}
fn decode(row: &DbRow) -> Result<StoredFact, AnalyticsError> {
    let fact = AnalyticsFact::from_storage_json(&row.text("fact_json").map_err(corrupt)?)?;
    let received_at = row.time("received_at").map_err(corrupt)?;
    let trace_id = row.text("source_trace_id").map_err(corrupt)?;
    let payload =
        platform::outbox::SafePayload::from_json(&row.text("source_payload").map_err(corrupt)?)
            .map_err(corrupt)?;
    if row.text("event_id").map_err(corrupt)? != fact.event_id()
        || row.i64("dictionary_version").map_err(corrupt)? != DICTIONARY_VERSION
        || row.text("event_kind").map_err(corrupt)? != fact.kind().as_str()
        || row.time("occurred_at").map_err(corrupt)? != fact.occurred_at()
        || received_at < fact.occurred_at()
        || row.text("source_type").map_err(corrupt)? != EVENT_TYPE
        || row.i64("source_version").map_err(corrupt)? != DICTIONARY_VERSION
        || row.time("source_time").map_err(corrupt)? != fact.occurred_at()
        || row.text("aggregate_type").map_err(corrupt)? != "analytics"
        || row.text("aggregate_id").map_err(corrupt)? != fact.event_id()
        || (!trace_id.is_empty() && !platform::transaction::validate_operation_id(&trace_id))
        || payload != platform::outbox::SafePayload::empty()
    {
        return Err(AnalyticsError::SourceMismatch);
    }
    Ok(StoredFact {
        fact,
        received_at,
        trace_id,
    })
}
enum SourceSelection {
    All,
    Event,
    ProjectedWindow,
}

fn source_select(
    renderer: &SchemaRenderer,
    backend: Backend,
    selection: SourceSelection,
) -> Result<String, AnalyticsError> {
    let facts = table(renderer, backend, "event_facts")?;
    let events = renderer
        .table(backend, "platform", "outbox_events")
        .map_err(unavailable)?;
    let targets = renderer
        .table(backend, "platform", "sink_deliveries")
        .map_err(unavailable)?;
    // Keep each query complete so the shared SQL gate can inspect every clause.
    Ok(match selection {
        SourceSelection::All => format!(
            "SELECT f.event_id, f.dictionary_version, f.event_kind, f.occurred_at, f.received_at, f.fact_json, e.event_type AS source_type, e.version AS source_version, e.occurred_at AS source_time, e.aggregate_type, e.aggregate_id, e.trace_id AS source_trace_id, e.payload AS source_payload FROM {facts} f JOIN {events} e ON e.event_id = f.event_id JOIN {targets} d ON d.event_id = f.event_id AND d.sink = ?"
        ),
        SourceSelection::Event => format!(
            "SELECT f.event_id, f.dictionary_version, f.event_kind, f.occurred_at, f.received_at, f.fact_json, e.event_type AS source_type, e.version AS source_version, e.occurred_at AS source_time, e.aggregate_type, e.aggregate_id, e.trace_id AS source_trace_id, e.payload AS source_payload FROM {facts} f JOIN {events} e ON e.event_id = f.event_id JOIN {targets} d ON d.event_id = f.event_id AND d.sink = ? WHERE f.event_id = ?"
        ),
        SourceSelection::ProjectedWindow => {
            let receipts = table(renderer, backend, "projection_receipts")?;
            format!(
                "SELECT f.event_id, f.dictionary_version, f.event_kind, f.occurred_at, f.received_at, f.fact_json, e.event_type AS source_type, e.version AS source_version, e.occurred_at AS source_time, e.aggregate_type, e.aggregate_id, e.trace_id AS source_trace_id, e.payload AS source_payload FROM {facts} f JOIN {events} e ON e.event_id = f.event_id JOIN {targets} d ON d.event_id = f.event_id AND d.sink = ? JOIN {receipts} r ON r.event_id = f.event_id AND r.projection = ? WHERE f.occurred_at >= ? AND f.occurred_at < ?"
            )
        }
    })
}

async fn lock_watermark(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
) -> Result<Watermark, AnalyticsError> {
    let watermarks = table(renderer, tx.backend(), "watermarks")?;
    let sql = lock_sql(
        tx.backend(),
        &format!(
            "SELECT processed_count, excluded_count, max_occurred_at, max_received_at, updated_at, maximum_delay_ms, version FROM {watermarks} WHERE projection = ?"
        ),
        false,
    );
    let row = tx
        .fetch_optional(&sql, &[text(PROJECTION)])
        .await
        .map_err(unavailable)?
        .ok_or(AnalyticsError::CorruptData)?;
    let processed = row.i64("processed_count").map_err(corrupt)?;
    let excluded = row.i64("excluded_count").map_err(corrupt)?;
    let version = row.i64("version").map_err(corrupt)?;
    let delay = row.i64("maximum_delay_ms").map_err(corrupt)?;
    if processed < 0 || excluded < 0 || excluded > processed || version <= 0 || delay < 0 {
        return Err(AnalyticsError::CorruptData);
    }
    Ok(Watermark {
        processed,
        excluded,
        version,
        max_occurred_at: (processed > 0)
            .then(|| row.time("max_occurred_at").map_err(corrupt))
            .transpose()?,
        max_received_at: (processed > 0)
            .then(|| row.time("max_received_at").map_err(corrupt))
            .transpose()?,
        updated_at: (processed > 0)
            .then(|| row.time("updated_at").map_err(corrupt))
            .transpose()?,
        maximum_delay_ms: (processed > 0).then_some(delay),
    })
}
fn plus(value: i64, increment: i64) -> Result<i64, AnalyticsError> {
    value
        .checked_add(increment)
        .ok_or(AnalyticsError::CorruptData)
}
async fn write_watermark(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    watermark: &Watermark,
) -> Result<(), AnalyticsError> {
    let table = table(renderer, tx.backend(), "watermarks")?;
    tx.execute_versioned(&format!("UPDATE {table} SET processed_count = ?, excluded_count = ?, max_occurred_at = ?, max_received_at = ?, updated_at = ?, maximum_delay_ms = ?, version = ? WHERE projection = ? AND version = ?"),
        &[DbValue::I64(watermark.processed), DbValue::I64(watermark.excluded),
            DbValue::Time(watermark.max_occurred_at.unwrap_or(DateTime::UNIX_EPOCH)),
            DbValue::Time(watermark.max_received_at.unwrap_or(DateTime::UNIX_EPOCH)),
            DbValue::Time(watermark.updated_at.unwrap_or(DateTime::UNIX_EPOCH)),
            DbValue::I64(watermark.maximum_delay_ms.unwrap_or(0)), DbValue::I64(plus(watermark.version, 1)?), text(PROJECTION), DbValue::I64(watermark.version)])
        .await.map_err(unavailable)
}
async fn project(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    stored: &StoredFact,
    at: DateTime<Utc>,
    watermark: &mut Watermark,
) -> Result<ProcessOutcome, AnalyticsError> {
    if at < stored.received_at {
        return Err(AnalyticsError::InvalidInput);
    }
    let receipts = table(renderer, tx.backend(), "projection_receipts")?;
    if !tx
        .insert_if_absent(
            &format!(
                "INSERT INTO {receipts} (projection, event_id, processed_at) VALUES (?, ?, ?)"
            ),
            &[
                text(PROJECTION),
                text(stored.fact.event_id()),
                DbValue::Time(at),
            ],
        )
        .await
        .map_err(unavailable)?
    {
        return Ok(ProcessOutcome::Duplicate);
    }
    let fact = &stored.fact;
    if !fact.excluded() {
        let aggregates = table(renderer, tx.backend(), "aggregates")?;
        let mut buckets = fact.dimensions().buckets();
        buckets.sort();
        for (dimension, value) in buckets {
            let key = [
                text(PROJECTION),
                DbValue::Time(fact.utc_day()),
                text(fact.kind().as_str()),
                text(dimension),
                text(&value),
            ];
            if let Some(row) = tx
                .fetch_optional(
                    &format!("SELECT event_count, version FROM {aggregates} WHERE projection = ? AND utc_day = ? AND event_kind = ? AND dimension = ? AND dimension_value = ?"),
                    &key,
                )
                .await
                .map_err(unavailable)?
            {
                let version = row.i64("version").map_err(corrupt)?;
                let count = row.i64("event_count").map_err(corrupt)?;
                if version <= 0 || count <= 0 {
                    return Err(AnalyticsError::CorruptData);
                }
                let values = [
                    DbValue::I64(plus(count, 1)?),
                    DbValue::I64(plus(version, 1)?),
                    key[0].clone(),
                    key[1].clone(),
                    key[2].clone(),
                    key[3].clone(),
                    key[4].clone(),
                    DbValue::I64(version),
                ];
                tx.execute_versioned(&format!("UPDATE {aggregates} SET event_count = ?, version = ? WHERE projection = ? AND utc_day = ? AND event_kind = ? AND dimension = ? AND dimension_value = ? AND version = ?"), &values).await.map_err(unavailable)?;
            } else {
                tx.execute(&format!("INSERT INTO {aggregates} (projection, utc_day, event_kind, dimension, dimension_value, event_count, version) VALUES (?, ?, ?, ?, ?, 1, 1)"), &key).await.map_err(unavailable)?;
            }
        }
    }
    watermark.processed = plus(watermark.processed, 1)?;
    watermark.excluded = plus(watermark.excluded, i64::from(fact.excluded()))?;
    watermark.max_occurred_at = Some(
        watermark
            .max_occurred_at
            .map_or(fact.occurred_at(), |t| t.max(fact.occurred_at())),
    );
    watermark.max_received_at = Some(
        watermark
            .max_received_at
            .map_or(stored.received_at, |t| t.max(stored.received_at)),
    );
    watermark.updated_at = Some(at);
    let delay = (at - fact.occurred_at()).num_milliseconds();
    watermark.maximum_delay_ms = Some(watermark.maximum_delay_ms.unwrap_or(0).max(delay));
    Ok(if fact.excluded() {
        ProcessOutcome::Excluded
    } else {
        ProcessOutcome::Applied
    })
}

/// Private processing function: source rows are selected on this transaction's connection,
/// so uncommitted events in another transaction cannot become analytic facts.
async fn process_committed(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    event: &OutboxEvent,
    at: DateTime<Utc>,
) -> Result<ProcessOutcome, AnalyticsError> {
    event.validate().map_err(|_| AnalyticsError::InvalidInput)?;
    let mut watermark = lock_watermark(tx, renderer).await?;
    let sql = source_select(renderer, tx.backend(), SourceSelection::Event)?;
    let row = tx
        .fetch_optional(&sql, &[text(SINK_NAME), text(&event.event_id)])
        .await
        .map_err(unavailable)?
        .ok_or(AnalyticsError::SourceMismatch)?;
    let stored = decode(&row)?;
    // The event envelope from the Worker must match the trusted persisted source.
    let expected = stored.fact.outbox_event(stored.trace_id.clone())?;
    if &expected != event {
        return Err(AnalyticsError::SourceMismatch);
    }
    let result = project(tx, renderer, &stored, at, &mut watermark).await?;
    if result != ProcessOutcome::Duplicate {
        write_watermark(tx, renderer, &watermark).await?;
    }
    Ok(result)
}

/// Rebuild only this derived projection. No source facts, Outbox deliveries or privacy
/// records are deleted. The root lock serializes consumers with the one source snapshot.
pub async fn rebuild(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    at: DateTime<Utc>,
) -> Result<Watermark, AnalyticsError> {
    let mut watermark = lock_watermark(tx, renderer).await?;
    let rows = tx
        .fetch_all(
            &source_select(renderer, tx.backend(), SourceSelection::All)?,
            &[text(SINK_NAME)],
        )
        .await
        .map_err(unavailable)?;
    let facts = rows.iter().map(decode).collect::<Result<Vec<_>, _>>()?;
    for name in ["projection_receipts", "aggregates"] {
        let table = table(renderer, tx.backend(), name)?;
        tx.execute(
            &format!("DELETE FROM {table} WHERE projection = ?"),
            &[text(PROJECTION)],
        )
        .await
        .map_err(unavailable)?;
    }
    watermark.processed = 0;
    watermark.excluded = 0;
    watermark.max_occurred_at = None;
    watermark.max_received_at = None;
    watermark.updated_at = None;
    watermark.maximum_delay_ms = None;
    for stored in facts {
        project(tx, renderer, &stored, at, &mut watermark).await?;
    }
    write_watermark(tx, renderer, &watermark).await?;
    watermark.version = plus(watermark.version, 1)?;
    Ok(watermark)
}

pub async fn snapshot(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
) -> Result<(Watermark, Vec<Aggregate>), AnalyticsError> {
    let watermark = lock_watermark(tx, renderer).await?;
    let table = table(renderer, tx.backend(), "aggregates")?;
    let rows = tx.fetch_all(&format!("SELECT utc_day, event_kind, dimension, dimension_value, event_count FROM {table} WHERE projection = ? ORDER BY utc_day, event_kind, dimension, dimension_value"), &[text(PROJECTION)]).await.map_err(unavailable)?;
    let aggregates = rows
        .iter()
        .map(|row| {
            Ok(Aggregate {
                utc_day: row.time("utc_day").map_err(corrupt)?,
                event_kind: row.text("event_kind").map_err(corrupt)?,
                dimension: row.text("dimension").map_err(corrupt)?,
                dimension_value: row.text("dimension_value").map_err(corrupt)?,
                count: row.i64("event_count").map_err(corrupt)?,
            })
        })
        .collect::<Result<_, AnalyticsError>>()?;
    Ok((watermark, aggregates))
}
/// Internal, worker-authorized query. Only already-projected committed facts contribute.
pub async fn window_summary(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    window: StatisticsWindow,
) -> Result<WindowSummary, AnalyticsError> {
    window.validate()?;
    lock_watermark(tx, renderer).await?;
    let sql = source_select(renderer, tx.backend(), SourceSelection::ProjectedWindow)?;
    let rows = tx
        .fetch_all(
            &sql,
            &[
                text(SINK_NAME),
                text(PROJECTION),
                DbValue::Time(window.from),
                DbValue::Time(window.until),
            ],
        )
        .await
        .map_err(unavailable)?;
    let stored = rows.iter().map(decode).collect::<Result<Vec<_>, _>>()?;
    platform::analytics::summarize(stored.iter().map(|row| &row.fact), window)
}

#[derive(Clone)]
pub struct DbAnalyticsStore {
    pool: Pool,
    renderer: SchemaRenderer,
}
impl DbAnalyticsStore {
    pub fn new(pool: Pool, renderer: SchemaRenderer) -> Self {
        Self { pool, renderer }
    }
    pub async fn rebuild(&self, at: DateTime<Utc>) -> Result<Watermark, AnalyticsError> {
        let mut tx = DbTransaction::begin(&self.pool)
            .await
            .map_err(unavailable)?;
        let watermark = rebuild(&mut tx.connection(), &self.renderer, at).await?;
        tx.commit().await.map_err(unavailable)?;
        Ok(watermark)
    }
    pub async fn snapshot(&self) -> Result<(Watermark, Vec<Aggregate>), AnalyticsError> {
        let mut tx = DbTransaction::begin(&self.pool)
            .await
            .map_err(unavailable)?;
        let result = snapshot(&mut tx.connection(), &self.renderer).await?;
        tx.commit().await.map_err(unavailable)?;
        Ok(result)
    }
    pub async fn window_summary(
        &self,
        window: StatisticsWindow,
    ) -> Result<WindowSummary, AnalyticsError> {
        let mut tx = DbTransaction::begin(&self.pool)
            .await
            .map_err(unavailable)?;
        let result = window_summary(&mut tx.connection(), &self.renderer, window).await?;
        tx.commit().await.map_err(unavailable)?;
        Ok(result)
    }
}
impl AnalyticsStore for DbAnalyticsStore {
    fn process<'a>(
        &'a self,
        event: &'a OutboxEvent,
        at: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<ProcessOutcome, AnalyticsError>> {
        Box::pin(async move {
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(unavailable)?;
            let result = process_committed(&mut tx.connection(), &self.renderer, event, at).await?;
            tx.commit().await.map_err(unavailable)?;
            Ok(result)
        })
    }
}

#[cfg(test)]
mod tests;
