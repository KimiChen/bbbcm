//! `03` §4 的授权求值：每次请求都重新读库校验，不接受任何缓存的判定结果。
//!
//! 十步顺序与 `03` §4 一一对应，任何一步失败都返回同一个 `Denied`——不透露失败在哪一步，
//! 否则枚举错误信息就能反推出「这个账号有没有某个角色」「这个版块归谁管」。
//!
//! 客户端提供的角色名、权限布尔值、目标关系、对象状态与审批结果一律不可信，
//! 本模块只接受目标标识与请求的权限键，其余全部从数据库重新读取。
use crate::iam::{Permission, permission, role_allows_scope, role_grants};
use chrono::{DateTime, Utc};

/// 授权请求。
///
/// 刻意只带「操作者、要什么权限、对哪个资源」，不带任何角色或权限声明：
/// 角色从数据库读，权限从 registry 查。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationRequest<'a> {
    pub actor_id: i64,
    pub permission_key: &'a str,
    /// 目标资源类型，必须与 registry 中该权限声明的 resource 一致（`03` §4 第 7 步）。
    pub resource_type: &'a str,
    /// 目标资源 ID。版块范围按目标内容**当前**的 `forum_id` 验证，由调用方解析后传入。
    pub resource_id: i64,
    /// 目标内容当前所属版块；账号级或环境级资源为 `None`。
    pub forum_id: Option<i64>,
}

/// 从数据库读到的一条有效角色分配及其范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignmentFacts {
    pub role_key: String,
    pub scope_type: String,
    /// `forums` 范围覆盖的版块 ID 集合；`global` 为空。
    pub forum_ids: Vec<i64>,
    pub starts_at: DateTime<Utc>,
    /// 到期时间；`None` 表示不设到期。
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked: bool,
}

/// 求值所需的、必须由调用方在同一事务内重新读取的事实。
///
/// 把这些做成显式输入而不是让本模块自己查库，是为了让求值逻辑保持纯函数：
/// 可以被穷举测试，也不会因为「在哪个事务里读的」产生歧义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationFacts {
    /// 账号生命周期状态是否为 active（第 1 步）。
    pub account_active: bool,
    /// staff principal 是否 active（第 2 步）。
    pub principal_active: bool,
    /// 管理会话是否有效（未撤销、未过期、未空闲超时）（第 3 步）。
    pub session_valid: bool,
    /// 会话建立时记录的 authz 版本。
    pub session_authz_version: i64,
    /// 账号当前的 authz 版本（第 5 步）。
    pub current_authz_version: i64,
    /// 角色分配及其范围（第 4、6、8 步）。
    pub assignments: Vec<AssignmentFacts>,
    /// MFA 已绑定且（对高危操作）重新认证仍在有效期内（第 9 步）。
    pub mfa_satisfied: bool,
    /// 是否存在一条对本次操作有效、且审批人不同于操作者的已通过审批（第 10 步）。
    pub approval_satisfied: bool,
}

/// 授权结论。拒绝原因刻意不区分，只在服务端日志里另行记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    Denied,
}

/// 按 `03` §4 的十步顺序求值。
///
/// 顺序本身有意义：先确认「这个人还在、还是 staff、会话还有效」，再看角色与范围，
/// 最后才检查 MFA 与审批。这样任何一步的否定都能立即短路，也让审计日志里的
/// 拒绝点是稳定的。
pub fn authorize(
    request: &AuthorizationRequest<'_>,
    facts: &AuthorizationFacts,
    now: DateTime<Utc>,
) -> Decision {
    // 第 6 步的一部分：权限必须在闭合 registry 中。查不到就是没有。
    let Some(permission) = permission(request.permission_key) else {
        return Decision::Denied;
    };
    // 第 1–3 步：账号、staff principal、管理会话。
    if !facts.account_active || !facts.principal_active || !facts.session_valid {
        return Decision::Denied;
    }
    // 第 5 步：authz 版本必须与会话一致。权限变更后递增版本，旧会话立即失效。
    if facts.session_authz_version != facts.current_authz_version {
        return Decision::Denied;
    }
    // 第 7 步：权限与资源类型匹配。
    if permission.resource != request.resource_type {
        return Decision::Denied;
    }
    // 第 4、6、8 步：逐条分配校验有效期、撤销、registry 一致性与 ABAC 范围。
    let granted = facts
        .assignments
        .iter()
        .any(|assignment| assignment_permits(assignment, request, permission, now));
    if !granted {
        return Decision::Denied;
    }
    // 第 9 步：MFA 与适用的重新认证。
    if !facts.mfa_satisfied {
        return Decision::Denied;
    }
    // 第 10 步：高危操作需要一条已通过、且审批人不是操作者本人的审批。
    if permission.high_risk && !facts.approval_satisfied {
        return Decision::Denied;
    }
    Decision::Allowed
}

/// 单条分配是否覆盖本次请求。
fn assignment_permits(
    assignment: &AssignmentFacts,
    request: &AuthorizationRequest<'_>,
    permission: &Permission,
    now: DateTime<Utc>,
) -> bool {
    // 第 4 步：开始、到期、撤销。到期与撤销靠时间比较自然生效，不依赖清理任务。
    if assignment.revoked
        || now < assignment.starts_at
        || assignment.expires_at.is_some_and(|at| at <= now)
    {
        return false;
    }
    // 第 6 步：角色与权限都必须在闭合 registry 中，且该角色确实持有这项权限。
    if !role_grants(&assignment.role_key, permission.key) {
        return false;
    }
    // 第 6 步续：分配使用的范围类型必须是该角色被允许的类型。
    // 这条防的是「给版主分配一个 global 范围」——registry 已声明版主只能绑版块。
    if !role_allows_scope(&assignment.role_key, &assignment.scope_type) {
        return false;
    }
    // 第 8 步：ABAC 范围必须覆盖目标资源及其当前关系。
    match assignment.scope_type.as_str() {
        "global" => true,
        "forums" => request
            .forum_id
            .is_some_and(|forum| assignment.forum_ids.contains(&forum)),
        // 未实现的范围类型一律不覆盖，而不是当成 global。
        _ => false,
    }
}

#[cfg(test)]
mod tests;
