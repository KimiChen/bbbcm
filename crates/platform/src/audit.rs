//! 追加审计共享合同；只容纳类型化元数据与摘要，不容纳秘密原文。
use crate::transaction::{new_operation_id, validate_operation_id};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

pub type AuditFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AuditError>> + Send + 'a>>;
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuditError {
    #[error("审计输入非法")]
    Invalid,
    #[error("访问未获授权")]
    Denied,
    #[error("访问审计未确定提交，数据不可返回")]
    Unavailable,
    #[error("独立权限域审计复制处于关闭状态")]
    ReplicaDisabled,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    SyntheticWrite,
    SensitiveRead,
    OutboxReplay,
    Correction,
    /// CAT-014：管理员把主题移动到另一个版块（DEC-056）。
    ContentMoved,
    /// AUTH-014：首个站点所有者的一次性 CLI 提权。成功与被拒都记录。
    OwnerBootstrapped,
    /// `03` §4：授予或撤销管理角色。授权变更必须可复核，成功与被拒都记录。
    RoleAssigned,
    RoleRevoked,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditReason {
    SyntheticTest,
    SupportCase,
    Recovery,
    Correction,
    /// 日常内容治理动作，例如把发错版块的主题移回正确版块。
    Moderation,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditResult {
    Succeeded,
    Denied,
    Failed,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Synthetic,
    Audit,
    Outbox,
    /// 社区主题。目标 ID 使用主题的数据库 ID，与权威永久链接 `/t/{id}` 一致。
    Topic,
    /// staff principal。目标 ID 为对应账号的数据库 ID。
    StaffPrincipal,
    /// IAM 角色分配。目标 ID 为 `iam.role_assignments` 的行 ID。
    RoleAssignment,
}

/// 类型化目标，当前仅引擎和合成测试；未来领域在本注册中显式扩展。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditTarget {
    pub kind: TargetKind,
    pub id: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditDetails {
    pub approval_ids: Vec<String>,
    /// 仅接受SHA-256摘要或redacted标志，不能存任意说明/原文。
    pub before: Option<AuditSummary>,
    pub after: Option<AuditSummary>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditSummary {
    digest: Option<String>,
}
#[derive(Clone, Copy)]
pub enum Classification {
    C0,
    C1,
    C2,
    C3,
    Secret,
}
impl AuditSummary {
    pub fn from_value(class: Classification, value: &[u8]) -> Self {
        use sha2::{Digest, Sha256};
        match class {
            Classification::C3 | Classification::Secret => Self { digest: None },
            _ => Self {
                digest: Some(format!("{:x}", Sha256::digest(value))),
            },
        }
    }
    fn valid(&self) -> bool {
        self.digest.as_ref().is_none_or(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEvent {
    pub event_id: String,
    pub actor_id: i64,
    pub action: AuditAction,
    pub target: AuditTarget,
    pub reason: AuditReason,
    pub ticket_id: Option<u64>,
    /// 02 §6.2 encode_time: UTC、固定6位微秒、27字节。
    pub occurred_at: String,
    pub result: AuditResult,
    pub operation_id: String,
    pub trace_id: Option<String>,
    pub version: i64,
    pub details: AuditDetails,
    pub correction_of: Option<String>,
}
impl AuditEvent {
    pub fn synthetic(actor_id: i64, operation_id: String) -> Self {
        Self {
            event_id: new_operation_id(),
            actor_id,
            action: AuditAction::SyntheticWrite,
            target: AuditTarget {
                kind: TargetKind::Synthetic,
                id: "1".into(),
            },
            reason: AuditReason::SyntheticTest,
            ticket_id: None,
            occurred_at: crate::db::encode_time(chrono::Utc::now()),
            result: AuditResult::Succeeded,
            operation_id,
            trace_id: None,
            version: 1,
            details: AuditDetails::default(),
            correction_of: None,
        }
    }
    pub fn validate(&self) -> Result<(), AuditError> {
        let time = crate::db::decode_time(&self.occurred_at).map_err(|_| AuditError::Invalid)?;
        let valid_ref = |s: &str| {
            validate_operation_id(s)
                || s.parse::<u64>()
                    .is_ok_and(|id| id > 0 && id.to_string() == s)
        };
        if !validate_operation_id(&self.event_id)
            || !validate_operation_id(&self.operation_id)
            || self.actor_id <= 0
            || self.version < 1
            || !valid_ref(&self.target.id)
            || self.occurred_at != crate::db::encode_time(time)
            || self
                .trace_id
                .as_deref()
                .is_some_and(|s| !validate_operation_id(s))
            || self
                .correction_of
                .as_deref()
                .is_some_and(|s| !validate_operation_id(s))
            || self.details.approval_ids.len() > 100
            || self
                .details
                .approval_ids
                .iter()
                .any(|s| !validate_operation_id(s))
            || self.details.before.as_ref().is_some_and(|s| !s.valid())
            || self.details.after.as_ref().is_some_and(|s| !s.valid())
        {
            return Err(AuditError::Invalid);
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct SensitiveAccessEvent(pub AuditEvent);
impl SensitiveAccessEvent {
    pub fn validate(&self) -> Result<(), AuditError> {
        self.0.validate()?;
        if !matches!(self.0.action, AuditAction::SensitiveRead)
            || !matches!(self.0.result, AuditResult::Succeeded)
        {
            return Err(AuditError::Invalid);
        }
        Ok(())
    }
}

/// 会话由数据库适配器持有短事务；授权与投影读取必须在该事务内完成。
/// 仅受信应用装配实现本合同，不能将请求里的权限布尔值当作授权器。
pub trait SensitiveReadSession<T>: Send {
    fn authorize(&mut self, event: &SensitiveAccessEvent) -> AuditFuture<'_, ()>;
    fn stage(&mut self) -> AuditFuture<'_, ()>;
    fn append(&mut self, event: &SensitiveAccessEvent) -> AuditFuture<'_, ()>;
    fn commit(&mut self) -> AuditFuture<'_, T>;
    fn rollback(&mut self) -> AuditFuture<'_, ()>;
}
/// 复制仅接受脱敏事件；真实供应商未批准，不配置默认provider。
pub trait AuditReplica: Send + Sync {
    fn append<'a>(&'a self, event: &'a AuditEvent) -> AuditFuture<'a, ()>;
}

/// 专用审计复制投影；不提供任意审计查询或历史修改。
pub trait AuditExport: Send + Sync {
    fn load<'a>(&'a self, event_id: &'a str) -> AuditFuture<'a, AuditEvent>;
}
