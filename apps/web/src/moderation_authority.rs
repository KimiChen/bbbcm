//! 治理命令的管理授权实现。
//!
//! 与 `content_authority` 共用同一条路径：独立管理会话（`03` §6）→ 管理侧 CSRF →
//! TOTP 已绑定（`AUTH-014`）→ 十步授权求值（`03` §4）。两处不各写一遍判定逻辑，
//! 而是共用 `management_gate`，避免两条治理入口的安全强度出现差异。
use chrono::{DateTime, Utc};
use moderation::authority::{
    FutureResult, ManagementActor, ManagementAuthority, ManagementCapability,
    ManagementCredentials, ManagementTarget,
};
use moderation::contract::ModerationError;
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::TransactionConnection;

/// 治理能力到 registry 权限键的映射。
///
/// 映射写在应用装配层：moderation 只声明「我需要哪项能力」，
/// 具体对应哪个权限键、哪些角色持有它，是 `DEC-058` registry 的事。
fn permission_key(capability: ManagementCapability) -> (&'static str, &'static str) {
    match capability {
        ManagementCapability::WriteCase => ("moderation.cases.write", "case"),
        ManagementCapability::EnforceContent => ("moderation.enforcement.content", "content"),
        ManagementCapability::EnforceAccount => ("moderation.enforcement.account", "account"),
        ManagementCapability::ReviewAppeal => ("moderation.appeals.review", "appeal"),
    }
}

pub struct ApplicationModerationAuthority {
    renderer: SchemaRenderer,
    /// 是否已接入 `DEC-058` 的权限 registry。`false` 时所有治理命令整体失败关闭。
    assembled: bool,
}
impl ApplicationModerationAuthority {
    pub fn approved(renderer: SchemaRenderer) -> Self {
        Self {
            renderer,
            assembled: true,
        }
    }
    /// 未装配的实现，仅测试使用：保证「失败关闭」这条路径持续被覆盖。
    #[cfg(test)]
    pub fn unassembled(renderer: SchemaRenderer) -> Self {
        Self {
            renderer,
            assembled: false,
        }
    }
}

impl ManagementAuthority for ApplicationModerationAuthority {
    fn authorize<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        credentials: &'a ManagementCredentials,
        capability: ManagementCapability,
        target: ManagementTarget,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ManagementActor> {
        Box::pin(async move {
            if !self.assembled {
                return Err(ModerationError::Denied);
            }
            let (key, resource) = permission_key(capability);
            crate::management_gate::authorize(
                tx,
                &self.renderer,
                credentials.session(),
                credentials.csrf(),
                key,
                resource,
                target.resource_id,
                target.forum_id,
                now,
            )
            .await
            .map(|user_id| ManagementActor { user_id })
            .map_err(|_| ModerationError::Denied)
        })
    }
}
