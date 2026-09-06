//! IAM 仓储：目录校验、角色分配、ABAC 范围与审批。
//!
//! 借用调用方事务、绝不提交，也不做授权判定——判定在 `iam::authorize`，
//! 本模块只负责把 `03` §4 需要的事实如实读出来、把受控命令的结果如实写下去。
use crate::iam::{
    authorize::{AssignmentFacts, AuthorizationFacts},
    contract::IamError,
};
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::{DbValue, TransactionConnection};

/// 「未发生」的哨兵：未撤销、无到期、未决策都用 Unix 纪元表示。
fn never() -> DbValue {
    DbValue::Time(DateTime::UNIX_EPOCH)
}
fn is_never(value: DateTime<Utc>) -> bool {
    value <= DateTime::UNIX_EPOCH
}
fn text(value: &str) -> DbValue {
    DbValue::Text(value.into())
}
fn iam(tx: &TransactionConnection<'_>, r: &SchemaRenderer, name: &str) -> Result<String, IamError> {
    r.table(tx.backend(), "iam", name)
        .map_err(|_| IamError::Unavailable)
}

/// 读出数据库里的 (角色, 权限) 目录对，按稳定顺序排列。
///
/// 供启动时的三方一致性校验使用：代码常量、`registry.toml` 与这里读到的必须完全相同。
pub async fn catalogue_pairs(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
) -> Result<Vec<(String, String)>, IamError> {
    let t = iam(tx, r, "role_permissions")?;
    let rows = tx
        .fetch_all(
            &format!("SELECT role_key, permission_key FROM {t} ORDER BY role_key, permission_key"),
            &[],
        )
        .await?;
    rows.iter()
        .map(|row| Ok((row.text("role_key")?, row.text("permission_key")?)))
        .collect()
}

/// 分配一个角色，返回分配 ID。范围类型是否被该角色允许由调用方按 registry 校验。
#[allow(clippy::too_many_arguments)]
pub async fn insert_assignment(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    role_key: &str,
    granted_by: i64,
    starts_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<i64, IamError> {
    let t = iam(tx, r, "role_assignments")?;
    tx.execute(&format!("INSERT INTO {t} (user_id, role_key, granted_by, starts_at, expires_at, revoked_at, created_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"),
        &[user_id.into(), text(role_key), granted_by.into(), DbValue::Time(starts_at),
          expires_at.map(DbValue::Time).unwrap_or_else(never), never(), DbValue::Time(now), DbValue::I64(1)]).await?;
    tx.last_insert_id().await.map_err(Into::into)
}

/// 为一条分配写入 ABAC 范围。`global` 不带资源 ID；`forums` 每个版块一行。
///
/// 数字 ID 必须同时携带资源类型（`03` §5），因此 `resource_type` 是必填列而不是可选。
pub async fn insert_scope(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    assignment_id: i64,
    scope_type: &str,
    forum_ids: &[i64],
) -> Result<(), IamError> {
    let t = iam(tx, r, "assignment_scopes")?;
    if scope_type == "global" {
        tx.execute(&format!("INSERT INTO {t} (assignment_id, scope_type, resource_type, resource_id, version) VALUES (?, ?, ?, 0, 1)"),
            &[assignment_id.into(), text(scope_type), text("global")]).await?;
        return Ok(());
    }
    for forum in forum_ids {
        tx.execute(&format!("INSERT INTO {t} (assignment_id, scope_type, resource_type, resource_id, version) VALUES (?, ?, ?, ?, 1)"),
            &[assignment_id.into(), text(scope_type), text("forum"), (*forum).into()]).await?;
    }
    Ok(())
}

/// 撤销一条分配。记录撤销时间，不删除行——授权变更历史必须可复核。
pub async fn revoke_assignment(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    assignment_id: i64,
    now: DateTime<Utc>,
) -> Result<(), IamError> {
    let t = iam(tx, r, "role_assignments")?;
    tx.execute(
        &format!(
            "UPDATE {t} SET revoked_at = ?, version = version + 1 WHERE id = ? AND revoked_at = ?"
        ),
        &[DbValue::Time(now), assignment_id.into(), never()],
    )
    .await?;
    Ok(())
}

/// 递增账号的 authz 版本。
///
/// 任何角色或范围变更后都必须调用：`03` §4 第 5 步要求会话记录的版本与账号当前版本
/// 一致，递增后该账号的所有既有管理会话立即失效，不需要逐个撤销会话。
pub async fn bump_authz_version(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<(), IamError> {
    let t = r
        .table(tx.backend(), "identity", "users")
        .map_err(|_| IamError::Unavailable)?;
    tx.execute(
        &format!(
            "UPDATE {t} SET authz_version = authz_version + 1, version = version + 1 WHERE id = ?"
        ),
        &[user_id.into()],
    )
    .await?;
    Ok(())
}

/// 读取账号当前的 authz 版本与生命周期状态。
pub async fn account_authz(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<Option<(bool, i64)>, IamError> {
    let t = r
        .table(tx.backend(), "identity", "users")
        .map_err(|_| IamError::Unavailable)?;
    tx.fetch_optional(
        &format!("SELECT status, authz_version FROM {t} WHERE id = ?"),
        &[user_id.into()],
    )
    .await?
    .map(|row| Ok((row.text("status")? == "active", row.i64("authz_version")?)))
    .transpose()
}

/// 读取某账号的全部角色分配及其 ABAC 范围。
///
/// 到期与撤销**不**在 SQL 里过滤：求值函数需要看到完整事实才能做出稳定判定，
/// 而且过滤条件散落在 SQL 里会让「什么时候算有效」出现第二处定义。
pub async fn assignments(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<Vec<AssignmentFacts>, IamError> {
    let a = iam(tx, r, "role_assignments")?;
    let s = iam(tx, r, "assignment_scopes")?;
    let rows = tx.fetch_all(&format!("SELECT id, role_key, starts_at, expires_at, revoked_at FROM {a} WHERE user_id = ? ORDER BY id"), &[user_id.into()]).await?;
    // 一次取出该账号所有分配的范围行，再在内存里分组：
    // 逐条分配再查一次范围会变成 N+1 查询，而授权求值在每个管理请求上都要跑。
    let scope_rows = tx.fetch_all(&format!("SELECT sc.assignment_id AS assignment_id, sc.scope_type AS scope_type, sc.resource_id AS resource_id FROM {s} sc JOIN {a} ra ON ra.id = sc.assignment_id WHERE ra.user_id = ? ORDER BY sc.id"), &[user_id.into()]).await?;
    let scopes = scope_rows
        .iter()
        .map(|row| {
            Ok((
                row.i64("assignment_id")?,
                row.text("scope_type")?,
                row.i64("resource_id")?,
            ))
        })
        .collect::<Result<Vec<_>, IamError>>()?;
    rows.iter()
        .map(|row| {
            let id = row.i64("id")?;
            // 一条分配的范围类型是同一种；取第一行的类型，版块 ID 汇总成集合。
            let scope_type = scopes
                .iter()
                .find(|(assignment, _, _)| *assignment == id)
                .map(|(_, kind, _)| kind.clone())
                .unwrap_or_default();
            let forum_ids = scopes
                .iter()
                .filter(|(assignment, _, _)| *assignment == id)
                .map(|(_, _, resource)| *resource)
                .collect();
            let expires = row.time("expires_at")?;
            Ok(AssignmentFacts {
                role_key: row.text("role_key")?,
                scope_type,
                forum_ids,
                starts_at: row.time("starts_at")?,
                expires_at: (!is_never(expires)).then_some(expires),
                revoked: !is_never(row.time("revoked_at")?),
            })
        })
        .collect()
}

/// 申请一次高危操作审批。返回审批 ID。
#[allow(clippy::too_many_arguments)]
pub async fn insert_approval(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    permission_key: &str,
    requester_id: i64,
    operator_id: i64,
    target_kind: &str,
    target_id: i64,
    expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<i64, IamError> {
    let t = iam(tx, r, "approvals")?;
    tx.execute(&format!("INSERT INTO {t} (permission_key, requester_id, operator_id, approver_id, target_kind, target_id, state, created_at, decided_at, expires_at, version) VALUES (?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?)"),
        &[text(permission_key), requester_id.into(), operator_id.into(), text(target_kind), target_id.into(),
          text("pending"), DbValue::Time(now), never(), DbValue::Time(expires_at), DbValue::I64(1)]).await?;
    tx.last_insert_id().await.map_err(Into::into)
}

/// 一条审批的当前事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    pub id: i64,
    pub requester_id: i64,
    pub operator_id: i64,
    pub approver_id: i64,
    pub state: String,
    pub expires_at: DateTime<Utc>,
    pub version: i64,
}

pub async fn approval(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
) -> Result<Option<ApprovalRecord>, IamError> {
    let t = iam(tx, r, "approvals")?;
    tx.fetch_optional(&format!("SELECT id, requester_id, operator_id, approver_id, state, expires_at, version FROM {t} WHERE id = ?"), &[id.into()]).await?
        .map(|row| Ok(ApprovalRecord {
            id: row.i64("id")?,
            requester_id: row.i64("requester_id")?,
            operator_id: row.i64("operator_id")?,
            approver_id: row.i64("approver_id")?,
            state: row.text("state")?,
            expires_at: row.time("expires_at")?,
            version: row.i64("version")?,
        }))
        .transpose()
}

pub async fn set_approval_state(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    record: &ApprovalRecord,
    state: &str,
    approver_id: i64,
    now: DateTime<Utc>,
) -> Result<(), IamError> {
    let t = iam(tx, r, "approvals")?;
    tx.execute_versioned(&format!("UPDATE {t} SET state = ?, approver_id = ?, decided_at = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[text(state), approver_id.into(), DbValue::Time(now), record.id.into(), record.version.into()]).await?;
    Ok(())
}

/// 是否存在一条对本次操作有效、且审批人不是操作者本人的已通过审批（`03` §7）。
///
/// 「审批人不是操作者」在 SQL 里直接排除，而不是读出来再判断：这条约束太关键，
/// 放在查询里可以保证任何调用路径都绕不过去。
pub async fn approval_satisfied(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    permission_key: &str,
    operator_id: i64,
    target_kind: &str,
    target_id: i64,
    now: DateTime<Utc>,
) -> Result<bool, IamError> {
    let t = iam(tx, r, "approvals")?;
    let rows = tx.fetch_all(&format!("SELECT id FROM {t} WHERE permission_key = ? AND operator_id = ? AND target_kind = ? AND target_id = ? AND state = ? AND approver_id <> operator_id AND expires_at > ? ORDER BY id"),
        &[text(permission_key), operator_id.into(), text(target_kind), target_id.into(), text("approved"), DbValue::Time(now)]).await?;
    Ok(!rows.is_empty())
}

/// 组装十步授权所需的全部事实。
///
/// `03` §5 要求范围变更、到期与撤销必须在最终写入事务内重新读取，因此本函数
/// 必须在真正执行动作的那个事务里调用，不能提前读好带过来。
#[allow(clippy::too_many_arguments)]
pub async fn collect_facts(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    principal_active: bool,
    session_valid: bool,
    session_authz_version: i64,
    mfa_satisfied: bool,
    permission_key: &str,
    target_kind: &str,
    target_id: i64,
    now: DateTime<Utc>,
) -> Result<AuthorizationFacts, IamError> {
    let (account_active, current_authz_version) =
        account_authz(tx, r, user_id).await?.unwrap_or((false, 0));
    Ok(AuthorizationFacts {
        account_active,
        principal_active,
        session_valid,
        session_authz_version,
        current_authz_version,
        assignments: assignments(tx, r, user_id).await?,
        mfa_satisfied,
        approval_satisfied: approval_satisfied(
            tx,
            r,
            permission_key,
            user_id,
            target_kind,
            target_id,
            now,
        )
        .await?,
    })
}
