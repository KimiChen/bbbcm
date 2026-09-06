use super::*;
use crate::{
    contract::TrustLevel,
    risk::{RiskAction, RiskDecision, RiskReason, SignalDigest, UnconfiguredRiskPolicy},
};
use platform::config::{Config, DatabaseConfig};
use platform_db::{
    Pool,
    transaction::{DbTransaction, DbValue},
};
use std::path::{Path, PathBuf};

struct Fixture {
    dir: PathBuf,
    pool: Pool,
    renderer: SchemaRenderer,
    config: DatabaseConfig,
    inviter: i64,
    invitee: i64,
    invitation: i64,
}
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-trust-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = Config::default().database;
        config.sqlite_path = dir.join("trust.sqlite3").to_string_lossy().into();
        config.schema_prefix = "trust_probe_".into();
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
        for name in ["ordinary-inviter", "ordinary-invitee"] {
            tx.connection().execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"),
                &[DbValue::Text(platform::transaction::new_operation_id()), DbValue::Text(platform::transaction::new_operation_id()), DbValue::Bool(true), DbValue::Text(name.into()), DbValue::Text(name.into()), DbValue::Text(format!("{name}@example.invalid")), DbValue::Text(format!("{name}@example.invalid")), DbValue::Bool(false), DbValue::Time(at()), DbValue::Time(at())]).await.unwrap();
            ids.push(tx.connection().last_insert_id().await.unwrap());
        }
        let invitations = renderer
            .table(tx.connection().backend(), "identity", "invitations")
            .unwrap();
        tx.connection().execute(&format!("INSERT INTO {invitations} (invite_ref, creator_id, code_hash, source, state, created_at, expires_at, consumed_at, consumer_id) VALUES (?, ?, ?, 'user', 'used', ?, ?, ?, ?)"),
            &[DbValue::Text(platform::transaction::new_operation_id()), DbValue::I64(ids[0]), DbValue::Text("synthetic-nonredeemable-hash".into()), DbValue::Time(at()), DbValue::Time(at() + chrono::Duration::days(1)), DbValue::Time(at()), DbValue::I64(ids[1])]).await.unwrap();
        let invitation = tx.connection().last_insert_id().await.unwrap();
        tx.commit().await.unwrap();
        Self {
            dir,
            pool,
            renderer,
            config,
            inviter: ids[0],
            invitee: ids[1],
            invitation,
        }
    }
    async fn initial(&self, tx: &mut TransactionConnection<'_>) -> TrustSnapshot {
        register_initial(
            tx,
            &self.renderer,
            self.invitee,
            self.invitation,
            self.inviter,
            at(),
        )
        .await
        .unwrap()
    }
    async fn count(&self, tx: &mut TransactionConnection<'_>, name: &str) -> i64 {
        let table = self.renderer.table(tx.backend(), "trust", name).unwrap();
        tx.fetch_optional(&format!("SELECT COUNT(*) AS count FROM {table}"), &[])
            .await
            .unwrap()
            .unwrap()
            .i64("count")
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn at() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-05T00:00:00.000000Z").unwrap()
}

#[tokio::test]
async fn initial_facts_and_invite_use_share_commit_and_rollback_without_any_grant() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let snapshot = f.initial(&mut tx.connection()).await;
    assert_eq!(snapshot.level(), TrustLevel::Pending);
    assert!(!snapshot.email_verified() && !snapshot.invitation_allowed());
    assert_eq!(snapshot.policy_version(), 0);
    assert_eq!(f.count(&mut tx.connection(), "invite_uses").await, 1);
    tx.rollback().await.unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    assert!(
        load_identity_snapshot(&mut tx.connection(), &f.renderer, f.invitee)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(f.count(&mut tx.connection(), "invite_uses").await, 0);
    f.initial(&mut tx.connection()).await;
    tx.commit().await.unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    assert_eq!(
        authorize_invitation(&mut tx.connection(), &f.renderer, f.invitee)
            .await
            .unwrap(),
        InvitationAuthorization::EmailUnverified
    );
    let verified = mark_email_verified(
        &mut tx.connection(),
        &f.renderer,
        f.invitee,
        at() + chrono::Duration::minutes(1),
    )
    .await
    .unwrap();
    assert!(verified.email_verified());
    // DEC-054：验证在同一事务内授予 T0。
    // DEC-051 的邀请资格语义不变，仍可从 `invitation_allowed` 读到，两者互不替代。
    assert_eq!(verified.level(), TrustLevel::T0);
    assert_eq!(verified.reason_code(), "email_verified_tier_t0");
    assert!(verified.invitation_allowed());
    assert_eq!(verified.policy_version(), INVITATION_POLICY_VERSION);
    tx.commit().await.unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    assert_eq!(
        authorize_invitation(&mut tx.connection(), &f.renderer, f.invitee)
            .await
            .unwrap(),
        InvitationAuthorization::Allowed {
            policy_version: INVITATION_POLICY_VERSION
        }
    );
    let repeated = mark_email_verified(
        &mut tx.connection(),
        &f.renderer,
        f.invitee,
        at() + chrono::Duration::minutes(2),
    )
    .await
    .unwrap();
    assert_eq!(repeated.version(), verified.version());
    assert_eq!(f.count(&mut tx.connection(), "invite_uses").await, 1);
    tx.commit().await.unwrap();
    f.pool.close().await;
}

#[tokio::test]
async fn risk_decisions_are_versioned_deduplicated_and_persist_across_restart() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    f.initial(&mut tx.connection()).await;
    let event_id = platform::transaction::new_operation_id();
    let digest = "a".repeat(64);
    let signals = RiskSignals {
        ip: Some(SignalDigest::from_keyed_hash(digest.clone()).unwrap()),
        ..Default::default()
    };
    let event = record_identity_event(
        &mut tx.connection(),
        &f.renderer,
        &event_id,
        Some(f.invitee),
        RiskEventKind::Registration,
        signals.clone(),
        &UnconfiguredRiskPolicy,
        at(),
    )
    .await
    .unwrap();
    assert_eq!(event.decision().rule_version(), 0);
    assert_eq!(event.decision().action(), RiskAction::Unavailable);
    assert!(!format!("{event:?} {signals:?}").contains(&digest));
    record_identity_event(
        &mut tx.connection(),
        &f.renderer,
        &event_id,
        Some(f.invitee),
        RiskEventKind::Registration,
        signals.clone(),
        &UnconfiguredRiskPolicy,
        at(),
    )
    .await
    .unwrap();
    assert_eq!(f.count(&mut tx.connection(), "risk_events").await, 1);
    tx.commit().await.unwrap();
    f.pool.close().await;
    let pool = Pool::connect(&f.config).await.unwrap();
    let mut tx = DbTransaction::begin(&pool).await.unwrap();
    record_identity_event(
        &mut tx.connection(),
        &f.renderer,
        &event_id,
        Some(f.invitee),
        RiskEventKind::Registration,
        signals,
        &UnconfiguredRiskPolicy,
        at(),
    )
    .await
    .unwrap();
    assert_eq!(f.count(&mut tx.connection(), "risk_events").await, 1);
    assert!(matches!(
        record_identity_event(
            &mut tx.connection(),
            &f.renderer,
            &event_id,
            Some(f.invitee),
            RiskEventKind::Login,
            RiskSignals::default(),
            &UnconfiguredRiskPolicy,
            at()
        )
        .await,
        Err(TrustError::Conflict)
    ));
    tx.rollback().await.unwrap();
    let mut tx = DbTransaction::begin(&pool).await.unwrap();
    record_identity_event(
        &mut tx.connection(),
        &f.renderer,
        &platform::transaction::new_operation_id(),
        None,
        RiskEventKind::Login,
        RiskSignals::default(),
        &UnconfiguredRiskPolicy,
        at(),
    )
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    let mut tx = DbTransaction::begin_read(&pool).await.unwrap();
    assert_eq!(f.count(&mut tx.connection(), "risk_events").await, 1);
    tx.rollback().await.unwrap();
    pool.close().await;
}

#[test]
fn policy_absence_cannot_be_misrepresented_as_allow_and_signals_reject_raw_metadata() {
    assert!(RiskDecision::new(0, RiskAction::Allow, RiskReason::ApprovedPolicy).is_err());
    assert!(RiskDecision::new(2, RiskAction::Allow, RiskReason::PolicyNotConfigured).is_err());
    for value in [
        "192.0.2.1",
        "ordinary@example.invalid",
        "raw-device",
        "ABCDE",
    ] {
        assert!(SignalDigest::from_keyed_hash(value.into()).is_err());
    }
    assert!(RiskDecision::new(7, RiskAction::Challenge, RiskReason::ExplicitChallenge).is_ok());
}

#[tokio::test]
async fn approved_invitation_qualification_keeps_every_existing_level_and_repairs_unqualified_snapshot()
 {
    use crate::risk::{ApprovedIdentityRiskPolicy, RiskPolicy};
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    f.initial(&mut tx.connection()).await;
    tx.commit().await.unwrap();
    for level in [TrustLevel::Pending, TrustLevel::T0, TrustLevel::T1] {
        let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
        let table = f
            .renderer
            .table(tx.connection().backend(), "trust", "identity_snapshots")
            .unwrap();
        // Preexisting level is a synthetic fact for preservation, never a service grant.
        tx.connection().execute(&format!("UPDATE {table} SET level = ?, email_verified = ?, invitation_allowed = ?, policy_version = ?, reason_code = ?, version = version + 1 WHERE user_id = ?"), &[level.as_str().into(), DbValue::Bool(true), DbValue::Bool(false), (if level == TrustLevel::Pending { 0_i64 } else { 1 }).into(), "awaiting_trust_policy".into(), f.invitee.into()]).await.unwrap();
        let snapshot = mark_email_verified(&mut tx.connection(), &f.renderer, f.invitee, at())
            .await
            .unwrap();
        // DEC-054：既有的 T0／T1 等级原样保留（验证不会降级），
        // 而「已验证但仍为 pending」的历史行会在这里收敛到 T0，不再永久停留。
        assert_eq!(
            snapshot.level(),
            if level == TrustLevel::Pending {
                TrustLevel::T0
            } else {
                level
            }
        );
        assert!(snapshot.email_verified() && snapshot.invitation_allowed());
        assert_eq!(snapshot.policy_version(), 1);
        let context = TrustedRiskContext {
            actor_id: Some(f.invitee),
            snapshot: Some(snapshot),
            kind: RiskEventKind::Invitation,
            signals: Default::default(),
        };
        let decision = ApprovedIdentityRiskPolicy.assess(&context).unwrap();
        assert_eq!(decision.action(), RiskAction::Allow);
        assert_eq!(decision.rule_version(), 1);
        tx.commit().await.unwrap();
    }
    let absent = TrustedRiskContext {
        actor_id: None,
        snapshot: None,
        kind: RiskEventKind::Invitation,
        signals: Default::default(),
    };
    assert_eq!(
        ApprovedIdentityRiskPolicy.assess(&absent).unwrap().reason(),
        RiskReason::AccountUnavailable
    );
    for kind in [RiskEventKind::Login, RiskEventKind::Registration] {
        let context = TrustedRiskContext {
            kind,
            ..absent.clone()
        };
        assert!(
            ApprovedIdentityRiskPolicy
                .assess(&context)
                .unwrap()
                .allowed()
        );
    }
    f.pool.close().await;
}

/// DEC-054 的 T1 规则边界测试。
///
/// 规则是确定性的，因此逐个边界都只依据落库事实断言：满 2 个自然日（UTC）、
/// 至少 5 条计入的有效贡献、无生效处罚。四类边界各覆盖一次：
///   1. 贡献差 1 条时，账号再老也不升级；
///   2. 贡献够了但差 1 分钟未满 2 个自然日时不升级；
///   3. 年龄与贡献都够，但存在生效处罚时不升级；
///   4. 三者同时满足才升级，且原因码可复核。
/// 最后验证重复评估幂等且永不降级。
///
/// 每次评估放在独立事务里，与生产路径一致——生产中 `trust.identity_snapshots`
/// 是最近取得的锁，重取同一目标不违反锁序；而在同一测试事务里连续调用会因为
/// 先前取过 `trust.invite_uses`（排序更靠后）而触发锁序回退。
#[tokio::test]
async fn deterministic_t1_rule_applies_only_at_every_approved_boundary() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    f.initial(&mut tx.connection()).await;
    let verified = mark_email_verified(&mut tx.connection(), &f.renderer, f.invitee, at())
        .await
        .unwrap();
    assert_eq!(verified.level(), TrustLevel::T0);
    let opened = verified.created_at();
    // 先写入 4 条贡献，比批准门槛少 1 条。
    for post in 1..T1_CONTRIBUTIONS {
        record_contribution(
            &mut tx.connection(),
            &f.renderer,
            f.invitee,
            ContributionKind::Reply,
            post,
            2,
            at(),
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();

    let evaluate = async |penalty: bool, now: DateTime<Utc>| {
        let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
        let snapshot = evaluate_tier(
            &mut tx.connection(),
            &f.renderer,
            f.invitee,
            TierInputs {
                active_penalty: penalty,
            },
            now,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        snapshot
    };

    // 边界一：贡献不足时，账号再老（这里给到 30 天）也不升级。
    assert_eq!(
        evaluate(false, opened + chrono::Duration::days(30))
            .await
            .level(),
        TrustLevel::T0
    );

    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    record_contribution(
        &mut tx.connection(),
        &f.renderer,
        f.invitee,
        ContributionKind::Topic,
        T1_CONTRIBUTIONS,
        10,
        at(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let boundary = opened.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc()
        + chrono::Duration::days(T1_ACCOUNT_DAYS);
    // 边界二：贡献已够，但距离第 2 个自然日结束还差 1 分钟，仍为 T0。
    assert_eq!(
        evaluate(false, boundary - chrono::Duration::minutes(1))
            .await
            .level(),
        TrustLevel::T0
    );
    // 边界三：年龄与贡献都满足，但存在生效处罚，依然不升级。
    assert_eq!(evaluate(true, boundary).await.level(), TrustLevel::T0);

    let promoted = evaluate(false, boundary).await;
    assert_eq!(promoted.level(), TrustLevel::T1);
    assert_eq!(promoted.reason_code(), "tier_rule_v1_days_contributions");

    // 幂等与不降级：升级后再次评估（哪怕此时带上处罚）既不改写等级也不递增版本。
    let again = evaluate(true, boundary + chrono::Duration::days(1)).await;
    assert_eq!(again.level(), TrustLevel::T1);
    assert_eq!(again.version(), promoted.version());
    f.pool.close().await;
}

/// 贡献事实的可重算性测试。
///
/// 覆盖三点：同一 post_id 重复写入不会重复计数（命令重放安全）；撤销只把该条移出总数
/// 而保留事实行；重复撤销是空操作，非法 post_id 与负长度被拒绝。
#[tokio::test]
async fn contribution_facts_are_idempotent_and_withdrawal_keeps_the_row() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    f.initial(&mut tx.connection()).await;
    mark_email_verified(&mut tx.connection(), &f.renderer, f.invitee, at())
        .await
        .unwrap();
    for _ in 0..3 {
        record_contribution(
            &mut tx.connection(),
            &f.renderer,
            f.invitee,
            ContributionKind::Topic,
            42,
            10,
            at(),
        )
        .await
        .unwrap();
    }
    assert_eq!(f.count(&mut tx.connection(), "contribution_facts").await, 1);
    assert_eq!(
        crate::repo::count_contributions(&mut tx.connection(), &f.renderer, f.invitee)
            .await
            .unwrap(),
        1
    );
    withdraw_contribution(&mut tx.connection(), &f.renderer, 42, at())
        .await
        .unwrap();
    assert_eq!(f.count(&mut tx.connection(), "contribution_facts").await, 1);
    assert_eq!(
        crate::repo::count_contributions(&mut tx.connection(), &f.renderer, f.invitee)
            .await
            .unwrap(),
        0
    );
    // 重复撤销无副作用；post_id 非正数、字符数为负一律拒绝。
    withdraw_contribution(&mut tx.connection(), &f.renderer, 42, at())
        .await
        .unwrap();
    assert!(
        withdraw_contribution(&mut tx.connection(), &f.renderer, 0, at())
            .await
            .is_err()
    );
    assert!(
        record_contribution(
            &mut tx.connection(),
            &f.renderer,
            f.invitee,
            ContributionKind::Reply,
            7,
            -1,
            at(),
        )
        .await
        .is_err()
    );
    tx.commit().await.unwrap();
    f.pool.close().await;
}
