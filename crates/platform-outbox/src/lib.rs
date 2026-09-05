//! Transactional Outbox：事件模型与 sink 契约。
//! 事件只随业务事务写入主库；lease 调度、重试与 DLQ 引擎随后续工作包接入。

/// 已提交业务事件的投递载荷；payload 为最小脱敏 JSON 文本。
#[derive(Debug, Clone)]
pub struct OutboxEvent {
    /// UUIDv7 文本。
    pub event_id: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub event_type: String,
    pub version: i64,
    /// RFC3339 UTC。
    pub occurred_at: String,
    pub payload: String,
    pub trace_id: Option<String>,
}

/// sink 幂等键：`(sink, event_id)` 唯一，重复投递不得产生重复外部副作用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkKey {
    pub sink: String,
    pub event_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    /// 网络/依赖临时故障：指数退避后重试。
    #[error("可重试失败: {0}")]
    Retryable(String),
    /// 载荷或对端永久拒绝：重试无意义，进入 DLQ。
    #[error("不可重试失败: {0}")]
    Permanent(String),
}
