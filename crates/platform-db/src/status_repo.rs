//! Read-only status adapter. All queries borrow a read transaction; no management writes.
use crate::{
    Pool,
    transaction::{DbTransaction, DbValue, TransactionConnection},
};
use chrono::{DateTime, Utc};
use platform::{
    config::DatabaseConfig,
    db::Backend,
    identifiers::SchemaRenderer,
    observability::{Measurement, Metric, SinkGroup, metrics},
    outbox::BoxFuture,
    status::*,
};
use tokio::sync::Mutex;

pub struct DbStatusSource {
    config: DatabaseConfig,
    pool: Mutex<Option<Pool>>,
}
impl DbStatusSource {
    pub fn new(config: DatabaseConfig) -> Self {
        Self {
            config,
            pool: Mutex::new(None),
        }
    }
    pub fn with_pool(config: DatabaseConfig, pool: Pool) -> Self {
        Self {
            config,
            pool: Mutex::new(Some(pool)),
        }
    }
    async fn snapshot(&self, now: DateTime<Utc>) -> Result<StatusSnapshot, StatusError> {
        let pool = {
            let mut stored = self.pool.lock().await;
            if stored.is_none() {
                *stored = Some(
                    Pool::connect_existing(&self.config)
                        .await
                        .map_err(|_| StatusError::Unavailable)?,
                );
            }
            stored.as_ref().ok_or(StatusError::Unavailable)?.clone()
        };
        let renderer = SchemaRenderer::new(&self.config.schema_prefix)
            .map_err(|_| StatusError::InvalidData)?;
        let mut tx = DbTransaction::begin_read(&pool)
            .await
            .map_err(|_| StatusError::Unavailable)?;
        let mut snapshot = read(&mut tx.connection(), &renderer, now).await?;
        tx.rollback().await.map_err(|_| StatusError::Unavailable)?;
        let (size, idle) = match &pool {
            Pool::Postgres(p, _) => (p.size(), p.num_idle()),
            Pool::Sqlite(p, _) => (p.size(), p.num_idle()),
        };
        snapshot.observations.pool_size = size;
        snapshot.observations.pool_idle = idle;
        Ok(snapshot)
    }
}
impl StatusSource for DbStatusSource {
    fn read(&self, now: DateTime<Utc>) -> BoxFuture<'_, Result<StatusSnapshot, StatusError>> {
        Box::pin(async move {
            let result =
                tokio::time::timeout(std::time::Duration::from_secs(2), self.snapshot(now))
                    .await
                    .unwrap_or(Err(StatusError::Unavailable));
            if result.is_err() {
                metrics().increment(Metric::DatabaseError);
            }
            result
        })
    }
}

pub async fn read(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    now: DateTime<Utc>,
) -> Result<StatusSnapshot, StatusError> {
    let unavailable = |_| StatusError::Unavailable;
    let invalid = |_| StatusError::InvalidData;
    let table = |schema, name| {
        r.table(tx.backend(), schema, name)
            .map_err(|_| StatusError::InvalidData)
    };
    let announcements = table("operations", "status_announcements")?;
    let flags = table("platform", "feature_flags")?;
    let outbox = table("platform", "outbox_events")?;
    let deliveries = table("platform", "sink_deliveries")?;
    let rows = tx.fetch_all(&format!("SELECT kind, title, body, starts_at, ends_at FROM {announcements} WHERE published = ? AND published_at <= ? AND ends_at > ? ORDER BY starts_at, announcement_id LIMIT ?"), &[DbValue::Bool(true), DbValue::Time(now), DbValue::Time(now), 50i64.into()]).await.map_err(unavailable)?;
    let notices = rows
        .into_iter()
        .map(|row| {
            let kind = match row.text("kind").map_err(invalid)?.as_str() {
                "maintenance" => AnnouncementKind::Maintenance,
                "incident" => AnnouncementKind::Incident,
                _ => return Err(StatusError::InvalidData),
            };
            let title = row.text("title").map_err(invalid)?;
            let body = row.text("body").map_err(invalid)?;
            let starts_at = row.time("starts_at").map_err(invalid)?;
            let ends_at = row.time("ends_at").map_err(invalid)?;
            if title.trim().is_empty()
                || title.chars().count() > 120
                || body.chars().count() > 4000
                || ends_at <= starts_at
            {
                return Err(StatusError::InvalidData);
            }
            Ok(Announcement {
                kind,
                title,
                body,
                starts_at,
                ends_at,
            })
        })
        .collect::<Result<Vec<_>, StatusError>>()?;
    let flag_rows = tx
        .fetch_all(
            &format!("SELECT flag_key, enabled, version FROM {flags} ORDER BY flag_key"),
            &[],
        )
        .await
        .map_err(unavailable)?;
    let raw_flags = flag_rows
        .into_iter()
        .map(|row| {
            Ok((
                row.text("flag_key").map_err(invalid)?,
                row.boolean("enabled").map_err(invalid)?,
                row.i64("version").map_err(invalid)?,
            ))
        })
        .collect::<Result<Vec<_>, StatusError>>()?;
    let flags = platform::flags::FlagSnapshot::from_rows(raw_flags)
        .map_err(|_| StatusError::InvalidData)?;
    let public_read_enabled = flags.get(platform::flags::Flag::PublicRead).enabled;
    let rows = tx
        .fetch_all(
            &format!("SELECT state, error_code, COUNT(*) AS total FROM {outbox} GROUP BY state, error_code"),
            &[],
        )
        .await
        .map_err(unavailable)?;
    let mut pending = 0;
    let mut dlq = 0;
    let mut leased = 0;
    let mut events = OutboxStateCounts::default();
    for row in rows {
        let count = u64::try_from(row.i64("total").map_err(invalid)?)
            .map_err(|_| StatusError::InvalidData)?;
        let state = row.text("state").map_err(invalid)?;
        events.add(&state, &row.text("error_code").map_err(invalid)?, count)?;
        match state.as_str() {
            "ready" => pending += count,
            "leased" => {
                leased += count;
                pending += count;
            }
            "dlq" => dlq += count,
            "delivered" => {}
            _ => return Err(StatusError::InvalidData),
        }
    }
    let delivery_rows = tx.fetch_all(&format!("SELECT d.sink, d.state AS delivery_state, e.state AS event_state, e.error_code, COUNT(*) AS total FROM {deliveries} d JOIN {outbox} e ON e.event_id = d.event_id GROUP BY d.sink, d.state, e.state, e.error_code"), &[]).await.map_err(unavailable)?;
    let mut by_sink = std::collections::BTreeMap::<SinkGroup, OutboxStateCounts>::new();
    for row in delivery_rows {
        let name = row.text("sink").map_err(invalid)?;
        let sink = SinkGroup::from_name(&name);
        let state = match row.text("delivery_state").map_err(invalid)?.as_str() {
            "delivered" => "delivered".to_owned(),
            "pending" => row.text("event_state").map_err(invalid)?,
            _ => return Err(StatusError::InvalidData),
        };
        let count = u64::try_from(row.i64("total").map_err(invalid)?)
            .map_err(|_| StatusError::InvalidData)?;
        by_sink.entry(sink).or_default().add(
            &state,
            &row.text("error_code").map_err(invalid)?,
            count,
        )?;
    }
    let oldest = tx.fetch_optional(&format!("SELECT occurred_at FROM {outbox} WHERE state IN ('ready','leased') ORDER BY occurred_at LIMIT 1"), &[]).await.map_err(unavailable)?;
    // Occurred-at age is explicitly not claimed to be exact commit-to-sink latency.
    let age = match oldest {
        Some(row) => (now - row.time("occurred_at").map_err(invalid)?)
            .num_milliseconds()
            .max(0) as u64,
        None => 0,
    };
    let unsupported = if tx.backend() == Backend::Sqlite {
        Measurement::Unsupported
    } else {
        Measurement::NotConnected
    };
    Ok(StatusSnapshot {
        public_read_enabled,
        announcements: notices,
        observations: DatabaseObservations {
            pool_size: 0,
            pool_idle: 0,
            outbox_pending: pending,
            outbox_dlq: dlq,
            outbox_leased: leased,
            outbox_oldest_pending_ms: Measurement::Collected(age),
            outbox_events: events,
            outbox_deliveries: by_sink
                .into_iter()
                .map(|(sink, counts)| DeliveryObservation { sink, counts })
                .collect(),
            // The existing event schema does not contain a database commit timestamp.
            outbox_commit_to_sink_ms: Measurement::NotConnected,
            lock_waits: unsupported.clone(),
            long_transactions: unsupported.clone(),
            deadlocks: unsupported.clone(),
            autovacuum: unsupported.clone(),
            dead_tuples: unsupported.clone(),
            wal_bytes: unsupported.clone(),
            relation_bytes: unsupported,
        },
    })
}

#[cfg(test)]
mod tests;
