//! 幂等 Repository：只借当前事务；读取结果不授予 actor 身份或管理权限。

use crate::transaction::{DbRow, DbValue, TransactionConnection, TransactionError};
use chrono::{DateTime, Duration, Utc};
use platform::db::lock_sql;
use platform::identifiers::SchemaRenderer;
pub use platform::transaction::request_hash;
use platform::transaction::{CommandIdentity, OperationQuery, OperationStatus, StoredResponse};

const SELECT_KEY: &str = "SELECT id, request_hash, operation_id, state, response_status, response_json, expires_at, version FROM {table} WHERE scope = ? AND actor_id = ? AND idempotency_key = ?";
const INSERT: &str = "INSERT INTO {table} (scope, actor_id, idempotency_key, request_hash, operation_id, state, response_status, response_json, created_at, completed_at, expires_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
const COMPLETE: &str = "UPDATE {table} SET state = ?, response_status = ?, response_json = ?, completed_at = ?, version = version + 1 WHERE scope = ? AND actor_id = ? AND idempotency_key = ? AND operation_id = ? AND request_hash = ? AND state = ? AND version = ?";
const SELECT_OPERATION: &str = "SELECT id, request_hash, operation_id, state, response_status, response_json, expires_at, version FROM {table} WHERE scope = ? AND actor_id = ? AND idempotency_key = ? AND operation_id = ?";

#[derive(Debug, PartialEq, Eq)]
pub enum Reservation {
    New,
    Replay {
        operation_id: String,
        response: StoredResponse,
    },
}

pub async fn reserve(
    conn: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    command: &CommandIdentity,
    now: DateTime<Utc>,
) -> Result<Reservation, TransactionError> {
    if !command.is_valid() {
        return Err(TransactionError::InvalidInput);
    }
    conn.start_idempotency()?;
    let table = renderer.table(conn.backend(), "platform", "idempotency_records")?;
    let inserted = conn
        .insert_if_absent(
            &INSERT.replace("{table}", &table),
            &[
                command.scope.clone().into(),
                command.actor_id.into(),
                command.idempotency_key.clone().into(),
                command.request_hash.clone().into(),
                command.operation_id.clone().into(),
                "pending".into(),
                0_i64.into(),
                "".into(),
                DbValue::Time(now),
                DbValue::Time(now),
                DbValue::Time(now + Duration::days(30)),
                1_i64.into(),
            ],
        )
        .await?;
    let sql = lock_sql(
        conn.backend(),
        &SELECT_KEY.replace("{table}", &table),
        false,
    );
    let row = conn
        .fetch_optional(&sql, &key(command))
        .await?
        .ok_or(TransactionError::IdempotencyConflict)?;
    // 记录锁也进入总序，阻止业务锁之后再次预留另一个幂等命令。
    let target =
        platform::transaction::LockTarget::new("platform", "idempotency_records", row.i64("id")?)
            .ok_or(TransactionError::CorruptData)?;
    conn.lock(renderer, &target).await?;
    if row.text("request_hash")? != command.request_hash {
        return Err(TransactionError::IdempotencyConflict);
    }
    if inserted {
        return Ok(Reservation::New);
    }
    match decode_status(&row, now)? {
        OperationStatus::Completed(response) => Ok(Reservation::Replay {
            operation_id: row.text("operation_id")?,
            response,
        }),
        OperationStatus::ResponseExpired => Err(TransactionError::ResponseExpired),
        _ => Err(TransactionError::CorruptData),
    }
}

fn key(command: &CommandIdentity) -> [DbValue; 3] {
    [
        command.scope.clone().into(),
        command.actor_id.into(),
        command.idempotency_key.clone().into(),
    ]
}

pub async fn complete(
    conn: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    command: &CommandIdentity,
    response: &StoredResponse,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    if !command.is_valid() || !valid_response(response) {
        return Err(TransactionError::InvalidInput);
    }
    conn.enter_append_phase();
    let table = renderer.table(conn.backend(), "platform", "idempotency_records")?;
    conn.execute_versioned(
        &COMPLETE.replace("{table}", &table),
        &[
            "completed".into(),
            i64::from(response.status_code).into(),
            response.body.clone().into(),
            DbValue::Time(now),
            command.scope.clone().into(),
            command.actor_id.into(),
            command.idempotency_key.clone().into(),
            command.operation_id.clone().into(),
            command.request_hash.clone().into(),
            "pending".into(),
            1_i64.into(),
        ],
    )
    .await
}

pub async fn query_result(
    conn: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    query: &OperationQuery,
    now: DateTime<Utc>,
) -> Result<OperationStatus, TransactionError> {
    if !query.is_valid() {
        return Err(TransactionError::InvalidInput);
    }
    let table = renderer.table(conn.backend(), "platform", "idempotency_records")?;
    let row = conn
        .fetch_optional(
            &SELECT_OPERATION.replace("{table}", &table),
            &[
                query.scope.clone().into(),
                query.actor_id.into(),
                query.idempotency_key.clone().into(),
                query.operation_id.clone().into(),
            ],
        )
        .await?;
    match row {
        Some(row) => decode_status(&row, now),
        None => Ok(OperationStatus::NotObserved),
    }
}

fn decode_status(row: &DbRow, now: DateTime<Utc>) -> Result<OperationStatus, TransactionError> {
    if row.i64("version")? <= 0
        || !platform::transaction::validate_operation_id(&row.text("operation_id")?)
    {
        return Err(TransactionError::CorruptData);
    }
    if row.time("expires_at")? <= now {
        return Ok(OperationStatus::ResponseExpired);
    }
    match row.text("state")?.as_str() {
        "pending" => Ok(OperationStatus::Pending),
        "completed" => {
            let response = StoredResponse {
                status_code: u16::try_from(row.i64("response_status")?)
                    .map_err(|_| TransactionError::CorruptData)?,
                body: row.text("response_json")?,
            };
            if !valid_response(&response) {
                return Err(TransactionError::CorruptData);
            }
            Ok(OperationStatus::Completed(response))
        }
        _ => Err(TransactionError::CorruptData),
    }
}

fn valid_response(response: &StoredResponse) -> bool {
    (200..600).contains(&response.status_code)
        && response.body.len() <= 16_384
        && platform::outbox::SafePayload::from_json(&response.body).is_ok()
}
