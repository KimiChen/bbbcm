//! IAM 受控命令的隔离测试：真实 SQLite 事务，覆盖从分配到求值的完整链路。
use super::*;
use crate::iam::authorize::{AuthorizationRequest, Decision, authorize};
use platform::config::Config;
use platform_db::{
    Pool,
    transaction::{DbTransaction, DbValue},
};
use std::path::{Path, PathBuf};

struct Fixture {
    dir: PathBuf,
    pool: Pool,
    renderer: SchemaRenderer,
    /// 被授予角色的操作者
    operator: i64,
    /// 授予者／审批人
    other: i64,
}
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-iam-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = Config::default().database;
        config.sqlite_path = dir.join("iam.sqlite3").to_string_lossy().into();
        config.schema_prefix = "iam_probe_".into();
        platform_db::migrations::migrate(
            &config,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&config).await.unwrap();
        let renderer = SchemaRenderer::new(&config.schema_prefix).unwrap();
        let mut tx = DbTransaction::begin(&pool).await.unwrap();
        let users = renderer
            .table(tx.connection().backend(), "identity", "users")
            .unwrap();
        let mut ids = Vec::new();
        for name in ["operator", "approver"] {
            tx.connection().execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"),
                &[DbValue::Text(platform::transaction::new_operation_id()), DbValue::Text(platform::transaction::new_operation_id()), DbValue::Bool(false), DbValue::Text(name.into()), DbValue::Text(name.into()), DbValue::Text(format!("{name}@example.invalid")), DbValue::Text(format!("{name}@example.invalid")), DbValue::Bool(true), DbValue::Time(at()), DbValue::Time(at())]).await.unwrap();
            ids.push(tx.connection().last_insert_id().await.unwrap());
        }
        tx.commit().await.unwrap();
        Self {
            dir,
            pool,
            renderer,
            operator: ids[0],
            other: ids[1],
        }
    }
    /// 以「会话有效、MFA 已满足」为前提组装事实并求值，模拟一次真实请求。
    #[allow(clippy::too_many_arguments)]
    async fn decide(
        &self,
        tx: &mut TransactionConnection<'_>,
        permission: &str,
        resource: &str,
        target_id: i64,
        forum_id: Option<i64>,
        session_version: i64,
        now: DateTime<Utc>,
    ) -> Decision {
        let facts = repo::collect_facts(
            tx,
            &self.renderer,
            self.operator,
            true,
            true,
            session_version,
            true,
            permission,
            resource,
            target_id,
            now,
        )
        .await
        .unwrap();
        authorize(
            &AuthorizationRequest {
                actor_id: self.operator,
                permission_key: permission,
                resource_type: resource,
                resource_id: target_id,
                forum_id,
            },
            &facts,
            now,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn at() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-07T00:00:00.000000Z").unwrap()
}

/// CR-013：数据库目录必须与 registry 完全一致；任何漂移都让启动校验失败。
#[tokio::test]
async fn registry_verification_detects_any_catalogue_drift() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    // 迁移 0017 的种子由 registry 生成，因此开箱即一致。
    verify_registry(&mut c, &f.renderer).await.unwrap();

    // 有人手工往目录里加了一条权限：立即被发现。
    let t = f
        .renderer
        .table(c.backend(), "iam", "role_permissions")
        .unwrap();
    c.execute(
        &format!("INSERT INTO {t} (role_key, permission_key, version) VALUES ('auditor', 'iam.keys.rotate', 1)"),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        verify_registry(&mut c, &f.renderer).await.unwrap_err(),
        IamError::RegistryMismatch
    );

    // 删掉一条同样被发现——多一条和少一条都是漂移。
    c.execute(&format!("DELETE FROM {t} WHERE role_key = 'auditor'"), &[])
        .await
        .unwrap();
    assert_eq!(
        verify_registry(&mut c, &f.renderer).await.unwrap_err(),
        IamError::RegistryMismatch
    );
    tx.rollback().await.unwrap();
}

/// 分配 → 放行 → 撤销 → 立即拒绝，以及 authz 版本对旧会话的影响。
#[tokio::test]
async fn assigning_and_revoking_a_role_takes_effect_immediately() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();

    // 未分配任何角色时一律拒绝。
    assert_eq!(
        f.decide(
            &mut c,
            "community.content.move",
            "content",
            42,
            Some(9),
            1,
            at()
        )
        .await,
        Decision::Denied
    );

    let assignment = assign_role(
        &mut c,
        &f.renderer,
        &Assignment {
            user_id: f.operator,
            role_key: "moderator",
            granted_by: f.other,
            scope_type: "forums",
            forum_ids: &[9],
            starts_at: at() - chrono::Duration::hours(1),
            expires_at: None,
        },
        at(),
    )
    .await
    .unwrap();

    // 分配递增了 authz 版本，因此仍持旧版本的会话必须失效。
    let (_, version) = repo::account_authz(&mut c, &f.renderer, f.operator)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(version, 2, "分配必须递增 authz 版本");
    assert_eq!(
        f.decide(
            &mut c,
            "community.content.move",
            "content",
            42,
            Some(9),
            1,
            at()
        )
        .await,
        Decision::Denied,
        "旧会话版本必须立即失效"
    );
    // 用当前版本重新建立的会话可以放行。
    assert_eq!(
        f.decide(
            &mut c,
            "community.content.move",
            "content",
            42,
            Some(9),
            version,
            at()
        )
        .await,
        Decision::Allowed
    );
    // 范围外的版块仍然拒绝。
    assert_eq!(
        f.decide(
            &mut c,
            "community.content.move",
            "content",
            42,
            Some(10),
            version,
            at()
        )
        .await,
        Decision::Denied
    );
    // 版主没有账号级处置权限。
    assert_eq!(
        f.decide(
            &mut c,
            "moderation.enforcement.account",
            "account",
            f.other,
            None,
            version,
            at()
        )
        .await,
        Decision::Denied
    );

    // 撤销后立即失效。
    revoke_role(&mut c, &f.renderer, assignment, f.operator, at())
        .await
        .unwrap();
    let (_, version) = repo::account_authz(&mut c, &f.renderer, f.operator)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        f.decide(
            &mut c,
            "community.content.move",
            "content",
            42,
            Some(9),
            version,
            at()
        )
        .await,
        Decision::Denied
    );
    tx.rollback().await.unwrap();
}

/// 分配命令自身的校验：范围类型、版块集合与到期时间必须自洽。
#[tokio::test]
async fn assignment_rejects_scopes_the_role_is_not_allowed_to_use() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    let base = Assignment {
        user_id: f.operator,
        role_key: "moderator",
        granted_by: f.other,
        scope_type: "forums",
        forum_ids: &[9],
        starts_at: at(),
        expires_at: None,
    };
    assign_role(&mut c, &f.renderer, &base, at()).await.unwrap();

    // registry 声明版主只能绑版块：给版主一个 global 分配直接拒绝，
    // 而不是写进去之后靠求值时忽略。
    assert!(
        assign_role(
            &mut c,
            &f.renderer,
            &Assignment {
                scope_type: "global",
                forum_ids: &[],
                ..base.clone()
            },
            at(),
        )
        .await
        .is_err()
    );
    // global 不得携带版块；forums 必须给出版块。
    assert!(
        assign_role(
            &mut c,
            &f.renderer,
            &Assignment {
                role_key: "administrator",
                scope_type: "global",
                forum_ids: &[9],
                ..base.clone()
            },
            at(),
        )
        .await
        .is_err()
    );
    assert!(
        assign_role(
            &mut c,
            &f.renderer,
            &Assignment {
                forum_ids: &[],
                ..base.clone()
            },
            at(),
        )
        .await
        .is_err()
    );
    // 未登记的角色、非法版块 ID、到期早于开始一律拒绝。
    for bad in [
        Assignment {
            role_key: "root",
            ..base.clone()
        },
        Assignment {
            forum_ids: &[0],
            ..base.clone()
        },
        Assignment {
            expires_at: Some(at()),
            ..base.clone()
        },
    ] {
        assert!(assign_role(&mut c, &f.renderer, &bad, at()).await.is_err());
    }
    tx.rollback().await.unwrap();
}

/// 03 §7：高危操作需要第三方批准，禁自提自批，撤销批准后立即失效。
#[tokio::test]
async fn high_risk_approval_forbids_self_approval_and_revocation_takes_effect() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    assign_role(
        &mut c,
        &f.renderer,
        &Assignment {
            user_id: f.operator,
            role_key: "administrator",
            granted_by: f.other,
            scope_type: "global",
            forum_ids: &[],
            starts_at: at() - chrono::Duration::hours(1),
            expires_at: None,
        },
        at(),
    )
    .await
    .unwrap();
    let (_, version) = repo::account_authz(&mut c, &f.renderer, f.operator)
        .await
        .unwrap()
        .unwrap();

    // 管理员持有账号级处置权限，但它是高危：缺审批一律拒绝。
    assert_eq!(
        f.decide(
            &mut c,
            "moderation.enforcement.account",
            "account",
            f.other,
            None,
            version,
            at()
        )
        .await,
        Decision::Denied
    );

    let approval = request_approval(
        &mut c,
        &f.renderer,
        "moderation.enforcement.account",
        f.operator,
        f.operator,
        "account",
        f.other,
        chrono::Duration::hours(1),
        at(),
    )
    .await
    .unwrap();

    // 自提自批被拒：执行人不能给自己开绿灯。
    assert_eq!(
        approve(&mut c, &f.renderer, approval, f.operator, at())
            .await
            .unwrap_err(),
        IamError::Denied
    );
    // 待批准状态下仍然拒绝。
    assert_eq!(
        f.decide(
            &mut c,
            "moderation.enforcement.account",
            "account",
            f.other,
            None,
            version,
            at()
        )
        .await,
        Decision::Denied
    );

    approve(&mut c, &f.renderer, approval, f.other, at())
        .await
        .unwrap();
    assert_eq!(
        f.decide(
            &mut c,
            "moderation.enforcement.account",
            "account",
            f.other,
            None,
            version,
            at()
        )
        .await,
        Decision::Allowed
    );
    // 审批只对申请时指定的目标有效，换个目标不生效。
    assert_eq!(
        f.decide(
            &mut c,
            "moderation.enforcement.account",
            "account",
            f.operator,
            None,
            version,
            at()
        )
        .await,
        Decision::Denied
    );
    // 过期后失效。
    assert_eq!(
        f.decide(
            &mut c,
            "moderation.enforcement.account",
            "account",
            f.other,
            None,
            version,
            at() + chrono::Duration::hours(2)
        )
        .await,
        Decision::Denied
    );

    // 撤销批准后立即失效——不需要去追杀已经发出的请求。
    revoke_approval(&mut c, &f.renderer, approval, f.other, at())
        .await
        .unwrap();
    assert_eq!(
        f.decide(
            &mut c,
            "moderation.enforcement.account",
            "account",
            f.other,
            None,
            version,
            at()
        )
        .await,
        Decision::Denied
    );

    // 非高危权限不接受审批申请，避免给普通权限造出一条看似更正式的旁路。
    assert!(
        request_approval(
            &mut c,
            &f.renderer,
            "community.content.move",
            f.operator,
            f.operator,
            "content",
            1,
            chrono::Duration::hours(1),
            at(),
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
}
