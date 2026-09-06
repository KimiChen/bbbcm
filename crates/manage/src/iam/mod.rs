//! 角色、权限、ABAC 范围与审批的机器可读 registry（`DEC-058`／`CR-011`／`CR-013`）。
//!
//! 唯一事实源是 `crates/manage/iam/registry.toml`：构建期由 `build.rs` 生成本模块下方的
//! `PERMISSIONS` 与 `ROLES` 常量，同一份文件也用于生成数据库种子。运行时再校验
//! 「代码常量 = registry = 数据库」三者一致，任一不符即拒绝启动。
//!
//! 因此**不要**在别处手写第二份角色或权限清单——`CR-013` 要求的正是这一点。

/// 一项权限。`resource` 用于「权限与资源类型匹配」校验（`03` §4 第 7 步），
/// `high_risk` 命中时额外要求一条已通过的双人审批（`03` §7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permission {
    pub key: &'static str,
    pub resource: &'static str,
    pub high_risk: bool,
}

/// 一个角色。`scopes` 限定该角色的分配可以使用哪些 ABAC 范围类型（`03` §5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Role {
    pub key: &'static str,
    pub name: &'static str,
    pub scopes: &'static [&'static str],
    pub permissions: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/registry.rs"));

pub mod authorize;
pub mod commands;
pub mod contract;
pub mod repo;

/// 按键查权限。未登记的键返回 `None`——闭合 registry 意味着「查不到就是没有」，
/// 绝不因为字符串看起来合理就放行。
pub fn permission(key: &str) -> Option<&'static Permission> {
    PERMISSIONS.iter().find(|p| p.key == key)
}

/// 按键查角色。
pub fn role(key: &str) -> Option<&'static Role> {
    ROLES.iter().find(|r| r.key == key)
}

/// 某角色是否持有某权限。
///
/// 逐项比较而不是任何形式的通配：`CR-011` 明确禁止用 `all permissions` 代替精确集合，
/// registry 的构建期校验也已经拒绝了 `*` 与 `xxx.*` 写法。
pub fn role_grants(role_key: &str, permission_key: &str) -> bool {
    role(role_key).is_some_and(|r| r.permissions.contains(&permission_key))
}

/// 某角色的分配是否允许使用某种范围类型。
pub fn role_allows_scope(role_key: &str, scope: &str) -> bool {
    role(role_key).is_some_and(|r| r.scopes.contains(&scope))
}

/// 生成数据库种子所需的 (角色, 权限) 对，按稳定顺序排列。
///
/// 迁移与启动校验都用它，保证「种子」和「代码」来自同一处。
pub fn seed_pairs() -> Vec<(&'static str, &'static str)> {
    let mut pairs: Vec<_> = ROLES
        .iter()
        .flat_map(|role| role.permissions.iter().map(move |p| (role.key, *p)))
        .collect();
    pairs.sort_unstable();
    pairs
}

#[cfg(test)]
mod tests;
