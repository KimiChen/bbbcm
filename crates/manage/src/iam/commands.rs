//! IAM 受控命令：目录一致性校验、角色分配与撤销、高危审批。
//!
//! 这些是授权状态的**唯一**写入路径（`03` §4：IAM catalogue 对运行角色只读，
//! 授权状态变更通过受控命令和专用权限执行）。谁能调用这些命令，由调用方按
//! 十步授权判定；本模块负责保证「写进去的东西自洽」。
use crate::iam::{contract::IamError, repo, role, role_allows_scope, seed_pairs};
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::TransactionConnection;

/// 校验代码常量、`registry.toml` 与数据库目录三者一致（`CR-013`／`DEC-058`）。
///
/// 代码常量本身就是构建期从 `registry.toml` 生成的，所以「代码 = registry」由编译保证；
/// 这里补上「数据库 = 代码」这条边，三角就闭合了。
///
/// 不一致时返回 `RegistryMismatch`，调用方应当**拒绝启动**而不是继续跑：
/// 一个与代码不同步的权限目录，比没有权限目录更危险——它会让人以为权限已经收紧了。
pub async fn verify_registry(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
) -> Result<(), IamError> {
    let stored = repo::catalogue_pairs(tx, r).await?;
    let expected: Vec<(String, String)> = seed_pairs()
        .into_iter()
        .map(|(role, permission)| (role.to_owned(), permission.to_owned()))
        .collect();
    if stored == expected {
        Ok(())
    } else {
        Err(IamError::RegistryMismatch)
    }
}

/// 一次角色分配的输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment<'a> {
    pub user_id: i64,
    pub role_key: &'a str,
    pub granted_by: i64,
    pub scope_type: &'a str,
    /// `forums` 范围覆盖的版块；`global` 必须为空。
    pub forum_ids: &'a [i64],
    pub starts_at: DateTime<Utc>,
    /// 到期时间；`None` 表示不设到期。
    pub expires_at: Option<DateTime<Utc>>,
}

/// 分配一个角色。
///
/// 校验点：
///   * 角色必须在闭合 registry 中；
///   * 范围类型必须是该角色被允许的类型——registry 声明版主只能绑版块，
///     给版主一个 global 分配会在这里被拒，而不是等到求值时才失效；
///   * `forums` 范围必须给出至少一个版块，`global` 不得携带版块；
///   * 到期时间必须晚于开始时间。
///
/// 成功后递增目标账号的 `authz_version`，使其既有管理会话立即失效（`03` §4 第 5 步）。
pub async fn assign_role(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    input: &Assignment<'_>,
    now: DateTime<Utc>,
) -> Result<i64, IamError> {
    if input.user_id <= 0 || input.granted_by <= 0 {
        return Err(IamError::InvalidInput);
    }
    if role(input.role_key).is_none() || !role_allows_scope(input.role_key, input.scope_type) {
        return Err(IamError::InvalidInput);
    }
    match input.scope_type {
        "global" if !input.forum_ids.is_empty() => return Err(IamError::InvalidInput),
        "forums" if input.forum_ids.is_empty() => return Err(IamError::InvalidInput),
        "global" | "forums" => {}
        _ => return Err(IamError::InvalidInput),
    }
    if input.forum_ids.iter().any(|id| *id <= 0) {
        return Err(IamError::InvalidInput);
    }
    if input.expires_at.is_some_and(|at| at <= input.starts_at) {
        return Err(IamError::InvalidInput);
    }
    let id = repo::insert_assignment(
        tx,
        r,
        input.user_id,
        input.role_key,
        input.granted_by,
        input.starts_at,
        input.expires_at,
        now,
    )
    .await?;
    repo::insert_scope(tx, r, id, input.scope_type, input.forum_ids).await?;
    repo::bump_authz_version(tx, r, input.user_id).await?;
    Ok(id)
}

/// 撤销一条分配，并递增目标账号的 `authz_version`。
///
/// 撤销只写时间戳不删行：授权变更历史必须可复核，否则事后无法回答
/// 「他在做那件事的时候到底有没有权限」。
pub async fn revoke_role(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    assignment_id: i64,
    user_id: i64,
    now: DateTime<Utc>,
) -> Result<(), IamError> {
    if assignment_id <= 0 || user_id <= 0 {
        return Err(IamError::InvalidInput);
    }
    repo::revoke_assignment(tx, r, assignment_id, now).await?;
    repo::bump_authz_version(tx, r, user_id).await
}

/// 申请一次高危操作审批（`03` §7）。
///
/// 申请人与将要执行操作的人可以是同一人（自己申请、别人批准），
/// 但**批准人**必须是第三方，这条在 `approve` 与查询里双重强制。
#[allow(clippy::too_many_arguments)]
pub async fn request_approval(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    permission_key: &str,
    requester_id: i64,
    operator_id: i64,
    target_kind: &str,
    target_id: i64,
    valid_for: chrono::Duration,
    now: DateTime<Utc>,
) -> Result<i64, IamError> {
    if requester_id <= 0
        || operator_id <= 0
        || target_id < 0
        || valid_for <= chrono::Duration::zero()
    {
        return Err(IamError::InvalidInput);
    }
    // 只有 registry 里标记为高危的权限才需要（也才允许）走审批，
    // 避免给普通权限造出一条看似更"正式"的旁路。
    let permission = crate::iam::permission(permission_key).ok_or(IamError::InvalidInput)?;
    if !permission.high_risk {
        return Err(IamError::InvalidInput);
    }
    if permission.resource != target_kind {
        return Err(IamError::InvalidInput);
    }
    let expires = now
        .checked_add_signed(valid_for)
        .ok_or(IamError::InvalidInput)?;
    repo::insert_approval(
        tx,
        r,
        permission_key,
        requester_id,
        operator_id,
        target_kind,
        target_id,
        expires,
        now,
    )
    .await
}

/// 批准一次申请。
///
/// `03` §7：同一人不得自提自批。这里拒绝的是「批准人 == 操作者」，
/// 因为真正危险的是「执行人给自己开绿灯」；申请人可以是第三方代提。
pub async fn approve(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    approval_id: i64,
    approver_id: i64,
    now: DateTime<Utc>,
) -> Result<(), IamError> {
    let record = repo::approval(tx, r, approval_id)
        .await?
        .ok_or(IamError::InvalidInput)?;
    if record.state != "pending" || record.expires_at <= now {
        return Err(IamError::Conflict);
    }
    if approver_id <= 0 || approver_id == record.operator_id {
        return Err(IamError::Denied);
    }
    repo::set_approval_state(tx, r, &record, "approved", approver_id, now).await
}

/// 撤销一次已通过的批准。
///
/// `03` §7：撤销批准不能继续执行旧操作。撤销后 `approval_satisfied` 立即返回假，
/// 因此下一次授权求值就会拒绝——不需要去追杀已经发出的请求。
pub async fn revoke_approval(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    approval_id: i64,
    revoked_by: i64,
    now: DateTime<Utc>,
) -> Result<(), IamError> {
    let record = repo::approval(tx, r, approval_id)
        .await?
        .ok_or(IamError::InvalidInput)?;
    if record.state == "revoked" {
        return Err(IamError::Conflict);
    }
    if revoked_by <= 0 {
        return Err(IamError::InvalidInput);
    }
    repo::set_approval_state(tx, r, &record, "revoked", revoked_by, now).await
}

#[cfg(test)]
mod tests;
