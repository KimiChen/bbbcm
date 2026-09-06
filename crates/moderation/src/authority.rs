//! 治理命令的管理授权入口。
//!
//! 与内容领域的做法对称：moderation 只声明「我需要哪一项管理能力、作用在哪个目标上」，
//! **谁能行使**由应用装配层按 `DEC-058` 的权限 registry 判定。moderation 不持有角色枚举，
//! 也不接受任何来自 HTTP 的角色声明。
//!
//! 未装配时必须整体失败关闭，绝不允许「找不到权限规则就放行」。
use crate::contract::ModerationError;
use chrono::{DateTime, Utc};
use platform_db::transaction::TransactionConnection;
use std::{future::Future, pin::Pin};

pub type FutureResult<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ModerationError>> + Send + 'a>>;

/// 治理侧需要的管理能力。
///
/// 与 registry 的权限键一一对应，但映射关系写在应用装配层——领域只说需求，不认权限键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagementCapability {
    /// 分派、改优先级、加内部备注、推进案件状态（`MOD-003`／`MOD-004`）。
    WriteCase,
    /// 内容级处置：遮蔽、隐藏、软删除、锁帖（`MOD-005`）。
    EnforceContent,
    /// 账号级处置：警告、限言、临时封禁、永久封禁（`MOD-005`）。registry 中标记为高危。
    EnforceAccount,
    /// 复核申诉（`MOD-011`）。职责分离由领域另行强制，不靠权限区分。
    ReviewAppeal,
}

/// 管理动作的目标。
///
/// `forum_id` 必须是目标内容**当前**所属的版块（`03` §5）；账号级目标为 `None`。
/// 由调用方解析后传入——moderation 不反向读取内容领域的仓储。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagementTarget {
    pub resource_id: i64,
    pub forum_id: Option<i64>,
}

/// 已通过授权的管理操作者，只携带审计与职责分离所需的最小事实。
#[derive(Clone, PartialEq, Eq)]
pub struct ManagementActor {
    pub user_id: i64,
}
impl std::fmt::Debug for ManagementActor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagementActor").finish_non_exhaustive()
    }
}

/// 管理会话凭据。与普通用户会话是两套独立体系（`03` §6）。
#[derive(Clone)]
pub struct ManagementCredentials {
    session: String,
    csrf: String,
}
impl ManagementCredentials {
    pub fn new(session: impl Into<String>, csrf: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            csrf: csrf.into(),
        }
    }
    pub fn session(&self) -> &str {
        &self.session
    }
    pub fn csrf(&self) -> &str {
        &self.csrf
    }
}
impl std::fmt::Debug for ManagementCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagementCredentials([REDACTED])")
    }
}

/// 由应用装配层实现的管理授权合同。
pub trait ManagementAuthority: Send + Sync {
    /// 校验本次请求是否被授权行使某项管理能力，并返回可用于审计的操作者事实。
    ///
    /// 实现必须在调用方的**同一事务**内完成校验（`03` §5：范围变更、到期与撤销
    /// 必须在最终写入事务内重新读取），不接受任何事务外快照。
    fn authorize<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        credentials: &'a ManagementCredentials,
        capability: ManagementCapability,
        target: ManagementTarget,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ManagementActor>;
}

/// 未装配的授权实现：任何管理能力一律拒绝。
///
/// 它是「失败关闭」的具体形态——装配不完整的部署不会退化成放行，而是整体拒绝治理动作。
pub struct UnconfiguredAuthority;
impl ManagementAuthority for UnconfiguredAuthority {
    fn authorize<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _credentials: &'a ManagementCredentials,
        _capability: ManagementCapability,
        _target: ManagementTarget,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ManagementActor> {
        Box::pin(async { Err(ModerationError::Denied) })
    }
}
