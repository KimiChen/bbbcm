//! 管理动作的统一授权门。
//!
//! 内容领域与治理领域各有自己的授权合同，但**判定只有这一份实现**：
//! 两条入口共用同一条路径，避免其中一条日后被改松而没人察觉。
//!
//! 路径固定为：
//!   1. 独立管理会话（`03` §6，不接受普通用户会话）；
//!   2. 管理侧 CSRF 校验（与普通会话的 CSRF 互不通用）；
//!   3. TOTP 已绑定——未绑定的会话只能访问绑定流程（`AUTH-014`）；
//!   4. `03` §4 的十步授权求值，事实在**调用方事务内**重新读取（`03` §5）。
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::TransactionConnection;

/// 授权失败的统一结果。
///
/// 刻意不区分「会话无效」「未绑定 MFA」「无权限」：区分只会给攻击者一个探测接口。
/// 具体原因由服务端日志另行记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Denied;

/// 校验一次管理动作，成功时返回操作者账号 ID。
#[allow(clippy::too_many_arguments)]
pub async fn authorize(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    raw_session: &str,
    raw_csrf: &str,
    permission_key: &str,
    resource_type: &str,
    resource_id: i64,
    forum_id: Option<i64>,
    now: DateTime<Utc>,
) -> Result<i64, Denied> {
    let identity =
        identity::management::authorize_management_session(tx, renderer, raw_session, now)
            .await
            .map_err(|_| Denied)?;
    identity.check_csrf(raw_csrf).map_err(|_| Denied)?;
    if identity.access != identity::management::ManagementAccess::Ready {
        return Err(Denied);
    }
    // authz 版本取账号当前值：管理会话刚校验过，而权限变更会同时递增账号版本，
    // 使旧会话在下一次请求时失效。
    let version = manage::iam::repo::account_authz(tx, renderer, identity.user_id)
        .await
        .map_err(|_| Denied)?
        .map(|(_, version)| version)
        .unwrap_or_default();
    let facts = manage::iam::repo::collect_facts(
        tx,
        renderer,
        identity.user_id,
        true,
        true,
        version,
        true,
        permission_key,
        resource_type,
        resource_id,
        now,
    )
    .await
    .map_err(|_| Denied)?;
    if manage::iam::authorize::authorize(
        &manage::iam::authorize::AuthorizationRequest {
            actor_id: identity.user_id,
            permission_key,
            resource_type,
            resource_id,
            // ABAC 版块范围按目标当前所属版块判定（`03` §5）。
            forum_id,
        },
        &facts,
        now,
    ) != manage::iam::authorize::Decision::Allowed
    {
        return Err(Denied);
    }
    Ok(identity.user_id)
}
