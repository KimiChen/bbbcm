//! 追加存储没有修改/删除接口；高危写由应用将本调用置于业务事务中。
use crate::transaction::{DbTransaction, DbValue, TransactionConnection, TransactionError};
use platform::audit::*;
use platform::identifiers::SchemaRenderer;

fn enum_text<T: serde::Serialize>(value: T) -> Result<String, TransactionError> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or(TransactionError::InvalidInput)
}
async fn append(
    conn: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    table: &str,
    event: &AuditEvent,
) -> Result<(), TransactionError> {
    event
        .validate()
        .map_err(|_| TransactionError::InvalidInput)?;
    let table = renderer.table(conn.backend(), "audit", table)?;
    let occurred = platform::db::decode_time(&event.occurred_at)
        .map_err(|_| TransactionError::InvalidInput)?;
    conn.enter_append_phase();
    conn.execute(&format!("INSERT INTO {table} (event_id, actor_id, action, target_type, target_id, reason_code, ticket_id, occurred_at, result, operation_id, trace_id, version, details, correction_of) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"), &[
        event.event_id.clone().into(), event.actor_id.into(), enum_text(event.action)?.into(),
        enum_text(event.target.kind)?.into(), event.target.id.clone().into(), enum_text(event.reason)?.into(),
        event.ticket_id.map(|v| v.to_string()).unwrap_or_default().into(), DbValue::Time(occurred),
        enum_text(event.result)?.into(), event.operation_id.clone().into(), event.trace_id.clone().unwrap_or_default().into(),
        event.version.into(), serde_json::to_string(&event.details).map_err(|_| TransactionError::InvalidInput)?.into(),
        event.correction_of.clone().unwrap_or_default().into(),
    ]).await?;
    Ok(())
}
pub async fn append_audit(
    conn: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    event: &AuditEvent,
) -> Result<(), TransactionError> {
    append(conn, renderer, "audit_events", event).await?;
    let copy = platform::outbox::OutboxEvent {
        event_id: platform::transaction::new_operation_id(),
        aggregate_type: "audit".into(),
        aggregate_id: event.event_id.clone(),
        event_type: "audit.appended".into(),
        version: 1,
        occurred_at: platform::db::decode_time(&event.occurred_at)
            .map_err(|_| TransactionError::InvalidInput)?,
        payload: platform::outbox::SafePayload::empty()
            .with_reference("audit_event_id", &event.event_id)
            .map_err(|_| TransactionError::InvalidInput)?,
        trace_id: event.trace_id.clone().unwrap_or_default(),
    };
    crate::outbox_repo::enqueue(conn, renderer, &copy, &["audit.replica"])
        .await
        .map_err(|_| TransactionError::Database(platform::transaction::DbErrorClass::Other))
}
pub async fn append_access(
    conn: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    event: &SensitiveAccessEvent,
) -> Result<(), TransactionError> {
    event
        .validate()
        .map_err(|_| TransactionError::InvalidInput)?;
    append(conn, renderer, "sensitive_access_events", &event.0).await
}

/// 应用注入的权限服务；必须用当前短事务重新验证actor、目标和用途。
pub trait ReadPolicy<T>: Send {
    fn authorize<'a>(
        &'a mut self,
        conn: TransactionConnection<'a>,
        event: &'a SensitiveAccessEvent,
    ) -> AuditFuture<'a, ()>;
    /// 只返回允许的C0/C1/C2投影；C3应只给专用验证结果。
    fn read<'a>(&'a mut self, conn: TransactionConnection<'a>) -> AuditFuture<'a, T>;
}
pub struct DbSensitiveSession<P, T> {
    staged: Option<T>,
    tx: Option<DbTransaction>,
    renderer: SchemaRenderer,
    policy: P,
    authorized_event: Option<String>,
    authorized: bool,
    read: bool,
    audited: bool,
}
impl<P, T> DbSensitiveSession<P, T> {
    pub fn new(tx: DbTransaction, renderer: SchemaRenderer, policy: P) -> Self {
        Self {
            staged: None,
            tx: Some(tx),
            renderer,
            policy,
            authorized_event: None,
            authorized: false,
            read: false,
            audited: false,
        }
    }
}
impl<T: Send, P: ReadPolicy<T>> SensitiveReadSession<T> for DbSensitiveSession<P, T> {
    fn authorize(&mut self, event: &SensitiveAccessEvent) -> AuditFuture<'_, ()> {
        let event = event.clone();
        Box::pin(async move {
            event.validate()?;
            if self.authorized || self.read {
                return Err(AuditError::Denied);
            }
            let tx = self.tx.as_mut().ok_or(AuditError::Unavailable)?;
            self.policy.authorize(tx.connection(), &event).await?;
            self.authorized_event =
                Some(serde_json::to_string(&event.0).map_err(|_| AuditError::Invalid)?);
            self.authorized = true;
            Ok(())
        })
    }
    fn stage(&mut self) -> AuditFuture<'_, ()> {
        Box::pin(async move {
            if !self.authorized || self.read {
                return Err(AuditError::Denied);
            }
            self.read = true;
            self.staged = Some(
                self.policy
                    .read(
                        self.tx
                            .as_mut()
                            .ok_or(AuditError::Unavailable)?
                            .connection(),
                    )
                    .await?,
            );
            Ok(())
        })
    }
    fn append(&mut self, event: &SensitiveAccessEvent) -> AuditFuture<'_, ()> {
        let event = event.clone();
        Box::pin(async move {
            if !self.authorized || !self.read || self.staged.is_none() || self.audited {
                return Err(AuditError::Denied);
            }
            if self.authorized_event.as_deref()
                != Some(
                    serde_json::to_string(&event.0)
                        .map_err(|_| AuditError::Invalid)?
                        .as_str(),
                )
            {
                return Err(AuditError::Denied);
            }
            append_access(
                &mut self
                    .tx
                    .as_mut()
                    .ok_or(AuditError::Unavailable)?
                    .connection(),
                &self.renderer,
                &event,
            )
            .await
            .map_err(|_| AuditError::Unavailable)?;
            self.audited = true;
            Ok(())
        })
    }
    fn commit(&mut self) -> AuditFuture<'_, T> {
        Box::pin(async move {
            if !self.authorized || !self.read || self.staged.is_none() || !self.audited {
                return Err(AuditError::Denied);
            }
            self.tx
                .take()
                .ok_or(AuditError::Unavailable)?
                .commit()
                .await
                .map_err(|_| AuditError::Unavailable)?;
            self.staged.take().ok_or(AuditError::Unavailable)
        })
    }
    fn rollback(&mut self) -> AuditFuture<'_, ()> {
        Box::pin(async move {
            self.staged.take();
            if let Some(tx) = self.tx.take() {
                tx.rollback().await.map_err(|_| AuditError::Unavailable)?;
            }
            Ok(())
        })
    }
}

pub struct DbAuditExport {
    pool: crate::Pool,
    renderer: SchemaRenderer,
}
impl DbAuditExport {
    pub fn new(pool: crate::Pool, renderer: SchemaRenderer) -> Self {
        Self { pool, renderer }
    }
}
impl AuditExport for DbAuditExport {
    fn load<'a>(&'a self, event_id: &'a str) -> AuditFuture<'a, AuditEvent> {
        Box::pin(async move {
            if !platform::transaction::validate_operation_id(event_id) {
                return Err(AuditError::Invalid);
            }
            let mut tx = DbTransaction::begin(&self.pool)
                .await
                .map_err(|_| AuditError::Unavailable)?;
            let result = async {
                let table=self.renderer.table(self.pool.backend(), "audit", "audit_events").map_err(|_| TransactionError::InvalidInput)?;
                let row=tx.connection().fetch_optional(&format!("SELECT event_id, actor_id, action, target_type, target_id, reason_code, ticket_id, occurred_at, result, operation_id, trace_id, version, details, correction_of FROM {table} WHERE event_id = ?"), &[event_id.into()]).await?.ok_or(TransactionError::CorruptData)?;
                let text_enum = |column| -> Result<serde_json::Value, TransactionError> { Ok(serde_json::Value::String(row.text(column)?)) };
                let nonempty = |column| -> Result<Option<String>, TransactionError> { Ok(Some(row.text(column)?).filter(|v| !v.is_empty())) };
                Ok::<_,TransactionError>(AuditEvent {
                    event_id: row.text("event_id")?, actor_id: row.i64("actor_id")?,
                    action: serde_json::from_value(text_enum("action")?).map_err(|_| TransactionError::CorruptData)?,
                    target: AuditTarget { kind: serde_json::from_value(text_enum("target_type")?).map_err(|_| TransactionError::CorruptData)?, id: row.text("target_id")? },
                    reason: serde_json::from_value(text_enum("reason_code")?).map_err(|_| TransactionError::CorruptData)?,
                    ticket_id: nonempty("ticket_id")?.map(|v| v.parse()).transpose().map_err(|_| TransactionError::CorruptData)?,
                    occurred_at: platform::db::encode_time(row.time("occurred_at")?),
                    result: serde_json::from_value(text_enum("result")?).map_err(|_| TransactionError::CorruptData)?,
                    operation_id: row.text("operation_id")?, trace_id: nonempty("trace_id")?, version: row.i64("version")?,
                    details: serde_json::from_str(&row.text("details")?).map_err(|_| TransactionError::CorruptData)?, correction_of: nonempty("correction_of")?,
                })
            }.await;
            tx.rollback().await.map_err(|_| AuditError::Unavailable)?;
            let event = result.map_err(|_| AuditError::Unavailable)?;
            event.validate()?;
            Ok(event)
        })
    }
}
