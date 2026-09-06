//! WP-MVP-D02 处置与申诉的隔离测试。
use super::*;
use crate::enforcement_repo;
use chrono::Duration;
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
    /// 被处置用户
    subject: i64,
    /// 原处置执行人
    operator: i64,
    /// 独立复核人
    reviewer: i64,
}
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-enforce-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = Config::default().database;
        config.sqlite_path = dir.join("enforce.sqlite3").to_string_lossy().into();
        config.schema_prefix = "enforce_probe_".into();
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
        for name in ["subject", "operator", "reviewer"] {
            tx.connection().execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"),
                &[DbValue::Text(platform::transaction::new_operation_id()), DbValue::Text(platform::transaction::new_operation_id()), DbValue::Bool(false), DbValue::Text(name.into()), DbValue::Text(name.into()), DbValue::Text(format!("{name}@example.invalid")), DbValue::Text(format!("{name}@example.invalid")), DbValue::Bool(true), DbValue::Time(at()), DbValue::Time(at())]).await.unwrap();
            ids.push(tx.connection().last_insert_id().await.unwrap());
        }
        tx.commit().await.unwrap();
        Self {
            dir,
            pool,
            renderer,
            subject: ids[0],
            operator: ids[1],
            reviewer: ids[2],
        }
    }
    fn mute(&self, expires: Option<DateTime<Utc>>) -> Enforcement {
        Enforcement {
            case_id: 1,
            action: Action::Mute,
            target_kind: TargetKind::Account,
            target_id: self.subject,
            subject_id: self.subject,
            operator_id: self.operator,
            reason_code: "spam_repeat".into(),
            rule_version: 1,
            evidence_digest: "a".repeat(64),
            before_state: "normal".into(),
            after_state: "muted".into(),
            expires_at: expires,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn at() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-06T00:00:00.000000Z").unwrap()
}

/// MOD-005/006：作用面与期限必须自洽，记录字段不得缺失。
#[tokio::test]
async fn enforcement_records_require_matching_scope_and_expiry() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();

    // 账号级动作必须指向账号；对回复执行封禁属目标错配，拒绝。
    let mut wrong = f.mute(Some(at() + Duration::days(3)));
    wrong.target_kind = TargetKind::Post;
    assert!(record(&mut c, &f.renderer, &wrong, at()).await.is_err());

    // 内容级动作不得指向账号。
    let mut content = f.mute(Some(at() + Duration::days(3)));
    content.action = Action::Hide;
    content.expires_at = None;
    assert!(record(&mut c, &f.renderer, &content, at()).await.is_err());

    // 限言必须带期限；不带一律拒绝。
    assert!(
        record(&mut c, &f.renderer, &f.mute(None), at())
            .await
            .is_err()
    );
    // 期限必须在未来。
    assert!(
        record(&mut c, &f.renderer, &f.mute(Some(at())), at())
            .await
            .is_err()
    );
    // 永久封禁不得带期限。
    let mut ban = f.mute(Some(at() + Duration::days(3)));
    ban.action = Action::Ban;
    assert!(record(&mut c, &f.renderer, &ban, at()).await.is_err());
    ban.expires_at = None;
    record(&mut c, &f.renderer, &ban, at()).await.unwrap();

    // 缺原因码、规则版本非正一律拒绝。
    let mut bad = f.mute(Some(at() + Duration::days(3)));
    bad.reason_code = "   ".into();
    assert!(record(&mut c, &f.renderer, &bad, at()).await.is_err());
    let mut bad = f.mute(Some(at() + Duration::days(3)));
    bad.rule_version = 0;
    assert!(record(&mut c, &f.renderer, &bad, at()).await.is_err());

    // 十类动作的作用面判定。
    for action in [Action::Warn, Action::Mute, Action::Suspend, Action::Ban] {
        assert!(action.targets_account(), "{}", action.as_str());
    }
    for action in [
        Action::Ignore,
        Action::Obscure,
        Action::Hide,
        Action::SoftDelete,
        Action::Lock,
        Action::Move,
    ] {
        assert!(!action.targets_account(), "{}", action.as_str());
    }
    assert!(Action::parse("unknown").is_err());
    tx.commit().await.unwrap();
}

/// 临时处罚到期自然失效，永久处罚不失效，撤销后立即停止生效但记录保留。
#[tokio::test]
async fn temporary_actions_expire_naturally_and_revocation_keeps_the_record() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    let expires = at() + Duration::days(3);
    let temporary = record(&mut c, &f.renderer, &f.mute(Some(expires)), at())
        .await
        .unwrap();
    let mut permanent = f.mute(None);
    permanent.action = Action::Ban;
    let forever = record(&mut c, &f.renderer, &permanent, at()).await.unwrap();

    // 到期前两条都生效。
    let active = enforcement_repo::active_actions(&mut c, &f.renderer, f.subject, at())
        .await
        .unwrap();
    assert_eq!(active.len(), 2);

    // 到期后临时处罚自然失效，无需任何清理任务；永久处罚仍在。
    let active = enforcement_repo::active_actions(
        &mut c,
        &f.renderer,
        f.subject,
        expires + Duration::seconds(1),
    )
    .await
    .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, forever);

    // 撤销后立即停止生效，但记录仍在库里，可查到撤销事实。
    revoke(&mut c, &f.renderer, forever, f.reviewer, "误判", at())
        .await
        .unwrap();
    assert!(
        enforcement_repo::active_actions(&mut c, &f.renderer, f.subject, at())
            .await
            .unwrap()
            .iter()
            .all(|a| a.id != forever)
    );
    let record = enforcement_repo::action(&mut c, &f.renderer, forever)
        .await
        .unwrap()
        .unwrap();
    assert!(record.revoked, "撤销不删除原记录");
    // 重复撤销拒绝；空原因拒绝。
    assert!(
        revoke(&mut c, &f.renderer, forever, f.reviewer, "再撤", at())
            .await
            .is_err()
    );
    assert!(
        revoke(&mut c, &f.renderer, temporary, f.reviewer, "  ", at())
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
}

/// MOD-011：一次申诉、非原处理人复核、三种结论各自生效；MOD-007 用户可读说明。
#[tokio::test]
async fn appeals_are_single_use_and_reviewed_by_someone_other_than_the_operator() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    let expires = at() + Duration::days(7);
    let action_id = record(&mut c, &f.renderer, &f.mute(Some(expires)), at())
        .await
        .unwrap();

    // MOD-007：被处置用户能读到动作、理由、范围、期限与申诉入口，且不含内部信息。
    let notice = notice_for(&mut c, &f.renderer, action_id).await.unwrap();
    assert_eq!(notice.action, Action::Mute);
    assert_eq!(notice.reason_code, "spam_repeat");
    assert_eq!(notice.scope, TargetKind::Account);
    assert_eq!(notice.expires_at, Some(expires));
    assert!(notice.appeal_available);
    assert_eq!(notice.appeal_href, format!("/appeals/{action_id}"));

    // 只有被处置本人可以申诉。
    assert!(
        submit_appeal(&mut c, &f.renderer, action_id, f.reviewer, "不是我", at())
            .await
            .is_err()
    );
    let appeal_id = submit_appeal(&mut c, &f.renderer, action_id, f.subject, "误判", at())
        .await
        .unwrap();
    // MOD-011：只有一次正常申诉。
    assert!(
        submit_appeal(&mut c, &f.renderer, action_id, f.subject, "再申诉", at())
            .await
            .is_err()
    );
    // 申诉之后不再提供申诉入口。
    assert!(
        !notice_for(&mut c, &f.renderer, action_id)
            .await
            .unwrap()
            .appeal_available
    );

    // 职责分离：原处置人与申诉人都不能复核。
    for bad in [f.operator, f.subject] {
        assert!(
            review_appeal(
                &mut c,
                &f.renderer,
                appeal_id,
                bad,
                AppealOutcome::Upheld,
                "维持",
                None,
                at(),
            )
            .await
            .is_err()
        );
    }
    // 减轻必须真的更轻：不早于原期限一律拒绝。
    for candidate in [expires, expires + Duration::days(1), at()] {
        assert!(
            review_appeal(
                &mut c,
                &f.renderer,
                appeal_id,
                f.reviewer,
                AppealOutcome::Reduced,
                "减轻",
                Some(candidate),
                at(),
            )
            .await
            .is_err()
        );
    }
    let shorter = at() + Duration::days(2);
    review_appeal(
        &mut c,
        &f.renderer,
        appeal_id,
        f.reviewer,
        AppealOutcome::Reduced,
        "情节较轻",
        Some(shorter),
        at(),
    )
    .await
    .unwrap();
    assert_eq!(
        enforcement_repo::action(&mut c, &f.renderer, action_id)
            .await
            .unwrap()
            .unwrap()
            .expires_at,
        Some(shorter)
    );
    // 复核只能进行一次。
    assert!(
        review_appeal(
            &mut c,
            &f.renderer,
            appeal_id,
            f.reviewer,
            AppealOutcome::Upheld,
            "再判",
            None,
            at(),
        )
        .await
        .is_err()
    );

    // 撤销结论会同事务撤销原处罚。
    let second = record(&mut c, &f.renderer, &f.mute(Some(expires)), at())
        .await
        .unwrap();
    let appeal2 = submit_appeal(&mut c, &f.renderer, second, f.subject, "申诉", at())
        .await
        .unwrap();
    review_appeal(
        &mut c,
        &f.renderer,
        appeal2,
        f.reviewer,
        AppealOutcome::Overturned,
        "确属误判",
        None,
        at(),
    )
    .await
    .unwrap();
    assert!(
        enforcement_repo::action(&mut c, &f.renderer, second)
            .await
            .unwrap()
            .unwrap()
            .revoked
    );
    // 已撤销的处罚不能再申诉——没有可申诉的对象。
    assert!(
        submit_appeal(&mut c, &f.renderer, second, f.subject, "再来", at())
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
}
