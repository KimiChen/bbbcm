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
    assert_eq!(verified.level(), TrustLevel::Pending);
    assert_eq!(verified.reason_code(), "email_verified_invitation_eligible");
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
        assert_eq!(snapshot.level(), level);
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
