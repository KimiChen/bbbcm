//! A04 共享事务合同；不依赖数据库驱动，也不提供 HTTP 鉴权旁路。

use crate::db::Backend;
use crate::identifiers::TABLES;
use sha2::{Digest, Sha256};

/// 服务先按领域合同规范化载荷再哈希；对象字段顺序等属于领域规范化合同。
pub fn request_hash(canonical_request: &[u8]) -> String {
    format!("{:x}", Sha256::digest(canonical_request))
}

/// 事件和操作 ID 都由应用在提交前生成，数据库不生成 UUID。
pub fn new_operation_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

pub fn validate_operation_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbErrorClass {
    UniqueConflict,
    Serialization,
    Deadlock,
    Busy,
    BusySnapshot,
    LockTimeout,
    Connection,
    Other,
}

/// 仅接受驱动的错误码；驱动错误正文可能包含值或凭证，不能进入日志。
pub fn classify_database_code(backend: Backend, code: &str) -> DbErrorClass {
    use DbErrorClass::*;
    match (backend, code) {
        (Backend::Postgres, "23505") | (Backend::Sqlite, "2067" | "1555") => UniqueConflict,
        (Backend::Postgres, "40001") => Serialization,
        (Backend::Postgres, "40P01") => Deadlock,
        (Backend::Postgres, "55P03") => LockTimeout,
        (Backend::Sqlite, "5") => Busy,
        (Backend::Sqlite, "517") => BusySnapshot,
        (Backend::Postgres, code) if code.starts_with("08") => Connection,
        _ => Other,
    }
}

/// 初次执行之外最多重试三次；必须已确认回滚且具有幂等键。
#[derive(Debug, Default)]
pub struct RetryBudget {
    retries: u8,
}
impl RetryBudget {
    pub fn take(
        &mut self,
        backend: Backend,
        error: DbErrorClass,
        has_idempotency_key: bool,
        rollback_confirmed: bool,
    ) -> bool {
        let eligible = matches!(
            (backend, error),
            (
                Backend::Postgres,
                DbErrorClass::Serialization | DbErrorClass::Deadlock
            ) | (
                Backend::Sqlite,
                DbErrorClass::Busy | DbErrorClass::BusySnapshot
            )
        );
        if eligible && has_idempotency_key && rollback_confirmed && self.retries < 3 {
            self.retries += 1;
            true
        } else {
            false
        }
    }

    pub fn retries(&self) -> u8 {
        self.retries
    }
}

/// 服务层必须先完成授权；本合同携带身份绑定，不验证真实会话或授予权限。
#[derive(Clone, PartialEq, Eq)]
pub struct CommandIdentity {
    pub scope: String,
    pub actor_id: i64,
    pub idempotency_key: String,
    pub request_hash: String,
    pub operation_id: String,
}

/// 领域写命令的完整公共元数据；新建资源使用 None 表示尚无实体版本。
/// 授权版本必须在最终事务重读验证，不能相信请求里的角色/能力声明。
pub struct WriteCommand<P> {
    pub identity: CommandIdentity,
    pub request_id: String,
    pub expected_entity_version: Option<i64>,
    pub expected_actor_authz_version: i64,
    pub payload: P,
}

/// 供后续管理服务扩展，不在本轮生成真实分配、审批或 HTTP 路由。
pub struct ManageCommandContext {
    pub active_assignment_id: i64,
    pub reason_code: String,
    pub ticket_id: String,
    pub reauthentication_id: String,
    pub approval_id: String,
}
impl std::fmt::Debug for CommandIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandIdentity")
            .field("scope", &self.scope)
            .field("operation_id", &self.operation_id)
            .finish_non_exhaustive()
    }
}
impl CommandIdentity {
    pub fn is_valid(&self) -> bool {
        self.actor_id > 0
            && valid_scope(&self.scope)
            && valid_key(&self.idempotency_key)
            && self.request_hash.len() == 64
            && self
                .request_hash
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            && validate_operation_id(&self.operation_id)
    }
}

fn valid_scope(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._:-".contains(&c))
}
fn valid_key(value: &str) -> bool {
    !value.is_empty() && value.len() <= 200 && value.bytes().all(|c| c.is_ascii_graphic())
}

/// 操作 ID 本身不构成访问凭据；原 actor、scope、key 必须全部匹配。
#[derive(Clone)]
pub struct OperationQuery {
    pub actor_id: i64,
    pub scope: String,
    pub idempotency_key: String,
    pub operation_id: String,
}
impl OperationQuery {
    pub fn is_valid(&self) -> bool {
        self.actor_id > 0
            && valid_scope(&self.scope)
            && valid_key(&self.idempotency_key)
            && validate_operation_id(&self.operation_id)
    }
}
impl From<&CommandIdentity> for OperationQuery {
    fn from(command: &CommandIdentity) -> Self {
        Self {
            actor_id: command.actor_id,
            scope: command.scope.clone(),
            idempotency_key: command.idempotency_key.clone(),
            operation_id: command.operation_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResponse {
    pub status_code: u16,
    /// 仅保存调用者裁剪后的最小 JSON 摘要，不能保存 Secret/C3 或响应头。
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationStatus {
    Completed(StoredResponse),
    /// 只代表当前查询未观察到记录；不能据此断定未知提交已回滚或自动重放。
    NotObserved,
    Pending,
    /// 响应期限已到；业务唯一事实不会因此被删除或重新授予资格。
    ResponseExpired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome<T> {
    Committed(T),
    RolledBack,
    Unknown { operation_id: String },
}

/// 排序字段完全由受信注册确定，不接受调用者自选领域排序号。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LockTarget {
    category: u8,
    table_order: usize,
    id: i64,
    schema: &'static str,
    table: &'static str,
}
impl LockTarget {
    pub fn id(&self) -> i64 {
        self.id
    }
    pub fn schema(&self) -> &'static str {
        self.schema
    }
    pub fn table(&self) -> &'static str {
        self.table
    }
    pub fn new(schema: &str, table: &str, id: i64) -> Option<Self> {
        let (table_order, entry) = TABLES.iter().enumerate().find(|(_, item)| {
            item.schema == schema && item.table == table && item.identity_column == Some("id")
        })?;
        if id <= 0 {
            return None;
        }
        let category = match schema {
            "platform" if table == "idempotency_records" => 0,
            "iam" => 1,
            "identity" => 2,
            "trust" => 3,
            "community" => 4,
            "media" => 5,
            "interaction" => 6,
            "moderation" => 7,
            "support" => 8,
            "privacy" => 9,
            "operations" => 10,
            "campaign" => 11,
            "commerce" => 12,
            "analytics" => 13,
            _ => return None,
        };
        Some(Self {
            category,
            table_order,
            id,
            schema: entry.schema,
            table: entry.table,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct LockPlan(Vec<LockTarget>);
impl LockPlan {
    pub fn new(mut targets: Vec<LockTarget>) -> Self {
        targets.sort();
        targets.dedup();
        Self(targets)
    }
    pub fn targets(&self) -> &[LockTarget] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_is_bounded_backend_specific_and_requires_safe_rollback() {
        let mut budget = RetryBudget::default();
        assert!(!budget.take(Backend::Postgres, DbErrorClass::Deadlock, false, true));
        assert!(!budget.take(Backend::Postgres, DbErrorClass::Deadlock, true, false));
        assert!(!budget.take(Backend::Sqlite, DbErrorClass::Serialization, true, true));
        assert!(!budget.take(Backend::Sqlite, DbErrorClass::UniqueConflict, true, true));
        for _ in 0..3 {
            assert!(budget.take(Backend::Postgres, DbErrorClass::Deadlock, true, true));
        }
        assert!(!budget.take(Backend::Postgres, DbErrorClass::Deadlock, true, true));
        assert_eq!(budget.retries(), 3);
        assert_eq!(
            classify_database_code(Backend::Sqlite, "517"),
            DbErrorClass::BusySnapshot
        );
        assert_eq!(
            classify_database_code(Backend::Sqlite, "1555"),
            DbErrorClass::UniqueConflict
        );
        assert_eq!(
            classify_database_code(Backend::Postgres, "23505"),
            DbErrorClass::UniqueConflict
        );
        assert_eq!(
            classify_database_code(Backend::Postgres, "55P03"),
            DbErrorClass::LockTimeout
        );
    }

    #[test]
    fn lock_plan_uses_registry_order_and_numeric_ids() {
        let forum = |id| LockTarget::new("community", "forums", id).unwrap();
        let rule = LockTarget::new("moderation", "rule_versions", 1).unwrap();
        let plan = LockPlan::new(vec![rule.clone(), forum(10), forum(2), forum(2)]);
        assert_eq!(plan.targets(), &[forum(2), forum(10), rule]);
        assert!(LockTarget::new("community", "unregistered", 1).is_none());
        assert!(LockTarget::new("community", "forums", 0).is_none());
    }

    #[test]
    fn operation_ids_are_canonical_uuid_v7() {
        let a = new_operation_id();
        assert!(validate_operation_id(&a));
        assert_ne!(a, new_operation_id());
        assert!(!validate_operation_id(
            "00000000-0000-4000-8000-000000000001"
        ));
    }
}
