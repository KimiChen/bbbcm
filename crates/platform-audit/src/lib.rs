//! 管理操作审计日志：追加式记录模型。
//! 写入必须与业务命令在同一主库事务提交；存储接入与查询接口随后续工作包接入。
//! 审计历史对运行角色只读，管理员不可删除。

/// 一条管理操作记录；不记录敏感原文（C3 不得进入审计）。
#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub actor_id: i64,
    pub action: String,
    pub target: String,
    pub reason: Option<String>,
    pub ticket: Option<String>,
    /// RFC3339 UTC。
    pub occurred_at: String,
    pub result: String,
}
