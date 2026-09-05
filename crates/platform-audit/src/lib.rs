//! 审计策略与受控敏感读取；存储通过platform共享合同注入，无横向依赖。
pub use platform::audit::*;

/// 数据只在授权、访问审计、明确提交全部成功后离开该函数。
pub async fn read_sensitive<T: Send>(
    session: &mut impl SensitiveReadSession<T>,
    event: &SensitiveAccessEvent,
) -> Result<T, AuditError> {
    let pending = async {
        event.validate()?;
        session.authorize(event).await?;
        session.stage().await?;
        session.append(event).await?;
        session.commit().await
    }
    .await;
    if pending.is_err() {
        let _ = session.rollback().await;
    }
    pending
}

pub struct DisabledReplica;
impl AuditReplica for DisabledReplica {
    fn append<'a>(&'a self, _: &'a AuditEvent) -> AuditFuture<'a, ()> {
        Box::pin(async { Err(AuditError::ReplicaDisabled) })
    }
}
/// 本机测试适配器：按event_id去重；不代表独立生产权限域。
#[derive(Default)]
pub struct LocalReplica(std::sync::Mutex<std::collections::BTreeMap<String, AuditEvent>>);
impl LocalReplica {
    pub fn len(&self) -> usize {
        self.0.lock().expect("local replica mutex").len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
impl AuditReplica for LocalReplica {
    fn append<'a>(&'a self, event: &'a AuditEvent) -> AuditFuture<'a, ()> {
        Box::pin(async move {
            event.validate()?;
            self.0
                .lock()
                .map_err(|_| AuditError::Unavailable)?
                .entry(event.event_id.clone())
                .or_insert_with(|| event.clone());
            Ok(())
        })
    }
}

/// Outbox只携带审计ID，副本内容由受信最小投影加载。
pub struct ReplicaSink {
    export: std::sync::Arc<dyn AuditExport>,
    replica: std::sync::Arc<dyn AuditReplica>,
}
impl ReplicaSink {
    pub fn new(
        export: std::sync::Arc<dyn AuditExport>,
        replica: std::sync::Arc<dyn AuditReplica>,
    ) -> Self {
        Self { export, replica }
    }
}
impl platform::outbox::Sink for ReplicaSink {
    fn name(&self) -> &str {
        "audit.replica"
    }
    fn deliver<'a>(
        &'a self,
        event: &'a platform::outbox::OutboxEvent,
    ) -> platform::outbox::BoxFuture<'a, Result<(), platform::outbox::SinkError>> {
        Box::pin(async move {
            use platform::outbox::SinkError;
            if event.event_type != "audit.appended" || event.version != 1 {
                return Err(SinkError::Permanent);
            }
            let id = event
                .payload
                .reference("audit_event_id")
                .ok_or(SinkError::Permanent)?;
            let record = self
                .export
                .load(id)
                .await
                .map_err(|_| SinkError::Retryable)?;
            self.replica
                .append(&record)
                .await
                .map_err(|error| match error {
                    AuditError::ReplicaDisabled => SinkError::Disabled,
                    _ => SinkError::Retryable,
                })
        })
    }
}
