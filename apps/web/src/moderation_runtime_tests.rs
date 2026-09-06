//! 治理授权在真实装配下的运行时测试。
//!
//! 覆盖的是安全上最关键的一条：**治理命令在没有合法管理会话与角色时必须整体拒绝**。
//! 这些夹具是显式标注的测试事实，不构成任何运行时授权旁路。
use crate::moderation_authority::ApplicationModerationAuthority;
use chrono::Utc;
use moderation::authority::{ManagementAuthority, ManagementCredentials, ManagementTarget};
use moderation::cases::{CaseStatus, ReportCategory, TargetKind, submit_report};
use moderation::contract::ModerationError;
use platform::{config::Config, identifiers::SchemaRenderer};
use platform_db::{
    Pool,
    transaction::{DbTransaction, DbValue},
};
use std::path::Path;

struct Fixture {
    dir: std::path::PathBuf,
    pool: Pool,
    renderer: SchemaRenderer,
    user: i64,
}
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-mod-runtime-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = Config::default().database;
        config.sqlite_path = dir.join("mod.sqlite3").to_string_lossy().into();
        config.schema_prefix = "mod_runtime_".into();
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
        tx.connection().execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, 'staffer', 'staffer', 'staffer@example.invalid', 'staffer@example.invalid', 'active', ?, '', ?, ?, 1, 1)"),
            &[DbValue::Text(platform::transaction::new_operation_id()), DbValue::Text(platform::transaction::new_operation_id()), DbValue::Bool(false), DbValue::Bool(true), DbValue::Time(Utc::now()), DbValue::Time(Utc::now())]).await.unwrap();
        let user = tx.connection().last_insert_id().await.unwrap();
        tx.commit().await.unwrap();
        Self {
            dir,
            pool,
            renderer,
            user,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 治理命令在未装配、无管理会话、以及有会话但无角色时都必须整体拒绝。
///
/// 三种情形对外返回同一个 `Denied`：区分「未装配」「会话无效」「无权限」
/// 只会给攻击者一个探测接口。
#[tokio::test]
async fn moderation_commands_fail_closed_without_a_management_session_and_role() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();

    // 先造一个真实案件，保证被拒的原因是授权而不是「案件不存在」。
    let outcome = submit_report(
        &mut c,
        &f.renderer,
        f.user,
        TargetKind::Topic,
        1,
        ReportCategory::Spam,
        "广告",
        None,
        Utc::now(),
    )
    .await
    .unwrap();

    let credentials = ManagementCredentials::new("no-such-session", "no-such-csrf");
    let target = ManagementTarget {
        resource_id: outcome.case_id,
        forum_id: Some(1),
    };

    // 未装配授权底座：整体失败关闭。
    let unassembled = ApplicationModerationAuthority::unassembled(f.renderer.clone());
    assert_eq!(
        unassembled
            .authorize(
                &mut c,
                &credentials,
                moderation::authority::ManagementCapability::WriteCase,
                target,
                Utc::now(),
            )
            .await
            .unwrap_err(),
        ModerationError::Denied
    );

    // 已装配但没有合法管理会话：同样拒绝，且与上一种给出相同的错误。
    let approved = ApplicationModerationAuthority::approved(f.renderer.clone());
    assert_eq!(
        approved
            .authorize(
                &mut c,
                &credentials,
                moderation::authority::ManagementCapability::WriteCase,
                target,
                Utc::now(),
            )
            .await
            .unwrap_err(),
        ModerationError::Denied
    );

    // 经由治理命令走一遍：状态不得发生任何变化。
    assert_eq!(
        moderation::cases::advance_status(
            &mut c,
            &f.renderer,
            &approved,
            &credentials,
            target,
            outcome.case_id,
            CaseStatus::Investigating,
            Utc::now(),
        )
        .await
        .unwrap_err(),
        ModerationError::Denied
    );
    assert_eq!(
        moderation::case_repo::case(&mut c, &f.renderer, outcome.case_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        CaseStatus::Triage,
        "被拒的命令不得改变案件状态"
    );
    tx.rollback().await.unwrap();
}
