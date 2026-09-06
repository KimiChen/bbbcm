//! Test-only ordinary identities and deliberately synthetic product policy.
//! These fixtures are isolated files, never a runtime identity/bootstrap bypass.
use crate::{
    contract::*,
    core::{IdentityService, PasswordEngine, TokenIssuer, queue_token, random_secret, secret_hash},
    repo,
};
use chrono::{DateTime, Duration, Utc};
use platform::{
    config::Config,
    db::Backend,
    identifiers::SchemaRenderer,
    outbox::BoxFuture,
    policy::{PolicyError, PolicySet, PolicyVersion},
    transaction::new_operation_id,
};
use platform_db::{
    Pool,
    policy::PolicyReader,
    transaction::{DbTransaction, DbValue, TransactionConnection},
};
use std::{path::PathBuf, sync::Arc};

struct SyntheticPolicy;
impl IdentityPolicy for SyntheticPolicy {
    fn username(&self, input: &str) -> Result<NormalizedIdentity, IdentityError> {
        let key = input.to_ascii_lowercase();
        if key.len() < 3
            || key.len() > 32
            || !key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            || ["admin", "moderator", "strawberry_staff"].contains(&key.as_str())
        {
            return Err(IdentityError::Rejected);
        }
        Ok(NormalizedIdentity {
            display: input.into(),
            key,
        })
    }
    fn email(&self, input: &str) -> Result<NormalizedIdentity, IdentityError> {
        if !input.ends_with("@example.invalid") || !input.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(IdentityError::Rejected);
        }
        Ok(NormalizedIdentity {
            display: input.into(),
            key: input.to_ascii_lowercase(),
        })
    }
    fn bio(&self, input: &str) -> Result<String, IdentityError> {
        if input.chars().count() > 120 {
            return Err(IdentityError::Rejected);
        }
        Ok(input.into())
    }
    fn preauth_expires_at(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>, IdentityError> {
        Ok(now + Duration::hours(1))
    }
    fn email_change_expires_at(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>, IdentityError> {
        Ok(now + Duration::hours(24))
    }
    fn invitation_window(&self, _: DateTime<Utc>) -> Result<InvitationWindow, IdentityError> {
        Err(IdentityError::PolicyPending)
    }
}
struct UnusedPolicies;
impl PolicyReader for UnusedPolicies {
    fn current<'a, 'c>(
        &'a self,
        _: &'a mut TransactionConnection<'c>,
        _: &'a SchemaRenderer,
    ) -> BoxFuture<'a, Result<PolicySet, PolicyError>> {
        Box::pin(async { Err(PolicyError::NotReady) })
    }
    fn documents<'a, 'c>(
        &'a self,
        _: &'a mut TransactionConnection<'c>,
        _: &'a SchemaRenderer,
    ) -> BoxFuture<'a, Result<Vec<PolicyVersion>, PolicyError>> {
        Box::pin(async { Err(PolicyError::NotReady) })
    }
}

struct Fixture {
    service: IdentityService,
    dir: PathBuf,
    now: DateTime<Utc>,
    a: i64,
    b: i64,
}
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("bbbcm-settings-{}", new_operation_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default().database;
        cfg.sqlite_path = dir.join("settings.sqlite3").to_str().unwrap().into();
        cfg.schema_prefix = "settings_test_".into();
        platform_db::migrations::migrate(
            &cfg,
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&cfg).await.unwrap();
        let renderer = SchemaRenderer::new(&cfg.schema_prefix).unwrap();
        let passwords = Arc::new(PasswordEngine::new(8, Arc::new(DisabledPasswordScreen)).unwrap());
        let service = IdentityService::new(
            pool,
            renderer,
            Arc::new(SyntheticPolicy),
            Arc::new(UnusedPolicies),
            passwords.clone(),
        )
        .with_token_issuer(Arc::new(
            TokenIssuer::new(
                Secret::from("synthetic_settings_key_never_a_runtime_secret".to_owned()),
                "synthetic_test".into(),
            )
            .unwrap(),
        ));
        let now: DateTime<Utc> = "2026-09-05T12:00:00Z".parse().unwrap();
        let hash = passwords.hash("synthetic initial password").unwrap();
        let mut tx = DbTransaction::begin(&service.pool).await.unwrap();
        let mut ids = Vec::new();
        for name in ["orchard_a", "orchard_b"] {
            let id = repo::create_account(
                &mut tx.connection(),
                &service.renderer,
                &new_operation_id(),
                &new_operation_id(),
                &SyntheticPolicy.username(name).unwrap(),
                &SyntheticPolicy
                    .email(&format!("{name}@example.invalid"))
                    .unwrap(),
                &hash,
                true,
                now - Duration::days(31),
            )
            .await
            .unwrap();
            let trust = service
                .renderer
                .table(Backend::Sqlite, "trust", "identity_snapshots")
                .unwrap();
            tx.connection().execute(&format!("INSERT INTO {trust} (user_id, level, email_verified, invitation_allowed, policy_version, reason_code, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"),
                &[id.into(), "pending".into(), DbValue::Bool(false), DbValue::Bool(false), 0_i64.into(), "awaiting_email_verification".into(), DbValue::Time(now - Duration::days(31)), DbValue::Time(now - Duration::days(31)), 1_i64.into()]).await.unwrap();
            ids.push(id);
        }
        tx.commit().await.unwrap();
        Self {
            service,
            dir,
            now,
            a: ids[0],
            b: ids[1],
        }
    }
    async fn enable(&self) {
        let flags = self.table("platform", "feature_flags");
        let mut tx = DbTransaction::begin(&self.service.pool).await.unwrap();
        for flag in ["public_read", "email_verification"] {
            tx.connection()
                .execute(
                    &format!(
                        "UPDATE {flags} SET enabled = ?, version = version + 1 WHERE flag_key = ?"
                    ),
                    &[DbValue::Bool(true), flag.into()],
                )
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
    }
    fn table(&self, schema: &str, name: &str) -> String {
        self.service
            .renderer
            .table(Backend::Sqlite, schema, name)
            .unwrap()
    }
    async fn account(&self, id: i64) -> Account {
        let mut tx = DbTransaction::begin_read(&self.service.pool).await.unwrap();
        let account = repo::account_by_id(&mut tx.connection(), &self.service.renderer, id)
            .await
            .unwrap()
            .unwrap();
        tx.rollback().await.unwrap();
        account
    }
    async fn session(&self, user_id: i64, now: DateTime<Utc>) -> (SessionIdentity, Secret, Secret) {
        let raw = random_secret();
        let csrf = random_secret();
        let account = self.account(user_id).await;
        let mut tx = DbTransaction::begin(&self.service.pool).await.unwrap();
        repo::create_session(
            &mut tx.connection(),
            &self.service.renderer,
            &account,
            &new_operation_id(),
            &secret_hash(raw.expose()),
            &secret_hash(csrf.expose()),
            "Synthetic browser / desktop",
            now,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let identity = self
            .service
            .authenticate(raw.expose(), now)
            .await
            .unwrap()
            .unwrap();
        (identity, csrf, raw)
    }
    async fn context(&self, now: DateTime<Utc>) -> RequestContext {
        let grant = self.service.create_preauth(now).await.unwrap();
        RequestContext::new(grant.token, grant.csrf, new_operation_id())
    }
    async fn count(&self, schema: &str, name: &str) -> i64 {
        let mut tx = DbTransaction::begin_read(&self.service.pool).await.unwrap();
        let count = tx
            .connection()
            .fetch_optional(
                &format!("SELECT COUNT(*) AS n FROM {}", self.table(schema, name)),
                &[],
            )
            .await
            .unwrap()
            .unwrap()
            .i64("n")
            .unwrap();
        tx.rollback().await.unwrap();
        count
    }
    async fn token(&self, user_id: i64, purpose: &str) -> Secret {
        let mut tx = DbTransaction::begin_read(&self.service.pool).await.unwrap();
        let row = tx.connection().fetch_optional(&format!("SELECT token_id, token_nonce, target_email_key, expires_at, issued_credential_version FROM {} WHERE user_id = ? AND purpose = ? AND consumed = ?", self.table("identity", "one_time_tokens")), &[user_id.into(), purpose.into(), DbValue::Bool(false)]).await.unwrap().unwrap();
        let raw = self.service.token_issuer.as_ref().unwrap().derive(
            &row.text("token_id").unwrap(),
            &row.text("token_nonce").unwrap(),
            user_id,
            purpose,
            &row.text("target_email_key").unwrap(),
            row.time("expires_at").unwrap(),
            row.i64("issued_credential_version").unwrap(),
        );
        tx.rollback().await.unwrap();
        raw
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn profile_csrf_owner_versions_replay_and_default_closed() {
    let f = Fixture::new().await;
    let (session, csrf, _) = f.session(f.a, f.now).await;
    assert!(
        f.service
            .update_bio(
                &session,
                csrf.expose(),
                "disabled",
                1,
                "Synthetic bio",
                f.now
            )
            .await
            .is_err()
    );
    assert_eq!(f.account(f.a).await.version, 1);
    f.enable().await;
    assert!(matches!(
        f.service
            .update_bio(&session, "wrong", "csrf", 1, "Synthetic bio", f.now)
            .await,
        Err(IdentityError::Csrf)
    ));
    let first = f
        .service
        .update_bio(&session, csrf.expose(), "bio", 1, "Synthetic bio", f.now)
        .await
        .unwrap();
    assert!(!first.replayed);
    assert!(
        f.service
            .update_bio(&session, csrf.expose(), "bio", 1, "Synthetic bio", f.now)
            .await
            .unwrap()
            .replayed
    );
    assert!(
        f.service
            .update_bio(&session, csrf.expose(), "bio", 1, "other content", f.now)
            .await
            .is_err()
    );
    assert!(
        f.service
            .update_bio(&session, csrf.expose(), "stale", 1, "stale write", f.now)
            .await
            .is_err()
    );
    assert_eq!(f.account(f.a).await.version, 2);
    assert_eq!(f.account(f.b).await.bio, "");
    let private = f.service.profile(&session, f.now).await.unwrap();
    assert_eq!(private.public.bio, "Synthetic bio");
    assert_eq!(private.public.trust_level, Some("pending"));
    assert!(!private.public.content_data_available);
    assert!(!private.public.role_data_available);
    assert_eq!(format!("{private:?}"), "OwnProfile([REDACTED])");
    let public = f
        .service
        .public_profile(&private.public.public_ref, f.now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(public.username, "orchard_a");
    assert!(!format!("{public:?}").contains("example.invalid"));
}

#[tokio::test]
async fn username_cooldown_unique_history_and_policy_impersonation() {
    let f = Fixture::new().await;
    f.enable().await;
    let (a, csrf_a, _) = f.session(f.a, f.now).await;
    let (b, csrf_b, _) = f.session(f.b, f.now).await;
    assert!(
        f.service
            .change_username(&a, csrf_a.expose(), "reserved", 1, "Admin", f.now)
            .await
            .is_err()
    );
    assert!(
        f.service
            .change_username(&a, csrf_a.expose(), "collision", 1, "Orchard_B", f.now)
            .await
            .is_err()
    );
    assert_eq!(f.count("identity", "username_history").await, 0);
    f.service
        .change_username(&a, csrf_a.expose(), "rename", 1, "Orchard_New", f.now)
        .await
        .unwrap();
    assert!(
        f.service
            .change_username(&a, csrf_a.expose(), "cooldown", 2, "orchard_later", f.now)
            .await
            .is_err()
    );
    assert!(
        f.service
            .change_username(&b, csrf_b.expose(), "history", 1, "ORCHARD_A", f.now)
            .await
            .is_err()
    );
    assert_eq!(f.account(f.a).await.username, "Orchard_New");
    assert_eq!(f.account(f.b).await.username, "orchard_b");
    assert_eq!(f.count("identity", "username_history").await, 1);
    let just_before = f.now + Duration::days(90) - Duration::milliseconds(1);
    let (b, csrf_b, _) = f.session(f.b, just_before).await;
    assert!(
        f.service
            .change_username(
                &b,
                csrf_b.expose(),
                "still_reserved",
                1,
                "orchard_a",
                just_before
            )
            .await
            .is_err()
    );
    f.service
        .change_username(
            &b,
            csrf_b.expose(),
            "expired_reservation",
            1,
            "orchard_a",
            f.now + Duration::days(90),
        )
        .await
        .unwrap();
    assert_eq!(f.account(f.b).await.username, "orchard_a");
    assert_eq!(f.count("identity", "username_history").await, 2);
    let (a, csrf_a, _) = f.session(f.a, f.now + Duration::days(30)).await;
    f.service
        .change_username(
            &a,
            csrf_a.expose(),
            "cooldown_boundary",
            2,
            "orchard_later",
            f.now + Duration::days(30),
        )
        .await
        .unwrap();
    assert_eq!(f.count("identity", "username_history").await, 3);
}

#[tokio::test]
async fn device_owner_reauthentication_revoke_and_expiry() {
    let f = Fixture::new().await;
    f.enable().await;
    let (a, csrf_a, raw_a) = f.session(f.a, f.now).await;
    let (a_other, _, raw_other) = f.session(f.a, f.now).await;
    let (b, _, raw_b) = f.session(f.b, f.now).await;
    let devices = f.service.sessions(&a, f.now).await.unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices.iter().filter(|d| d.current).count(), 1);
    assert!(devices.iter().all(|d| d.session_ref != b.session_ref()));
    assert!(
        f.service
            .revoke_session(&a, csrf_a.expose(), "wrong_owner", b.session_ref(), f.now)
            .await
            .is_err()
    );
    assert!(
        f.service
            .revoke_session(
                &a,
                csrf_a.expose(),
                "reauth_expired",
                a_other.session_ref(),
                f.now + Duration::seconds(301)
            )
            .await
            .is_err()
    );
    f.service
        .revoke_session(&a, csrf_a.expose(), "single", a_other.session_ref(), f.now)
        .await
        .unwrap();
    assert!(
        f.service
            .authenticate(raw_other.expose(), f.now)
            .await
            .unwrap()
            .is_none()
    );
    let (_, _, raw_third) = f.session(f.a, f.now).await;
    f.service
        .revoke_other_sessions(&a, csrf_a.expose(), "others", f.now)
        .await
        .unwrap();
    assert!(
        f.service
            .authenticate(raw_a.expose(), f.now)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        f.service
            .authenticate(raw_b.expose(), f.now)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        f.service
            .authenticate(raw_third.expose(), f.now)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.service
            .sessions(&a, f.now + Duration::days(14))
            .await
            .is_err()
    );
    assert_eq!(format!("{:?}", devices[0]), "DeviceSummary([REDACTED])");
}

#[tokio::test]
async fn email_change_is_confirmed_once_and_revokes_old_credentials_and_tokens() {
    let f = Fixture::new().await;
    f.enable().await;
    let (a, csrf, raw_a) = f.session(f.a, f.now).await;
    let (_, _, raw_other) = f.session(f.a, f.now).await;
    f.service
        .request_email_change(
            &a,
            csrf.expose(),
            "new_email",
            1,
            "changed@example.invalid",
            f.now,
        )
        .await
        .unwrap();
    assert_eq!(f.account(f.a).await.email, "orchard_a@example.invalid");
    assert!(
        f.service
            .authenticate(raw_a.expose(), f.now)
            .await
            .unwrap()
            .is_some()
    );
    let token = f.token(f.a, "change_email").await;
    let context = f.context(f.now).await;
    let confirmed = f
        .service
        .confirm_email_change(&context, token.expose(), f.now)
        .await
        .unwrap();
    assert!(!confirmed.replayed);
    assert!(
        f.service
            .confirm_email_change(&context, token.expose(), f.now)
            .await
            .unwrap()
            .replayed
    );
    let fresh_context = f.context(f.now).await;
    assert!(
        f.service
            .confirm_email_change(&fresh_context, token.expose(), f.now)
            .await
            .is_err()
    );
    let account = f.account(f.a).await;
    assert_eq!(account.email, "changed@example.invalid");
    assert!(account.email_verified);
    assert_eq!(account.credential_version, 2);
    assert!(
        f.service
            .authenticate(raw_a.expose(), f.now)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.service
            .authenticate(raw_other.expose(), f.now)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(f.count("identity", "mail_requests").await, 3);
    assert_eq!(f.count("platform", "outbox_events").await, 4);
    let mut tx = DbTransaction::begin_read(&f.service.pool).await.unwrap();
    let snapshot =
        trust::core::load_identity_snapshot(&mut tx.connection(), &f.service.renderer, f.a)
            .await
            .unwrap()
            .unwrap();
    assert!(snapshot.email_verified());
    assert!(snapshot.invitation_allowed());
    assert_eq!(snapshot.level(), trust::contract::TrustLevel::T0);
    for row in tx
        .connection()
        .fetch_all(
            &format!(
                "SELECT payload FROM {}",
                f.table("platform", "outbox_events")
            ),
            &[],
        )
        .await
        .unwrap()
    {
        let payload = row.text("payload").unwrap();
        assert!(!payload.contains(token.expose()));
        assert!(!payload.contains("example.invalid"));
    }
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn email_duplicate_request_has_uniform_response_and_token_expiry_or_collision_rolls_back() {
    let f = Fixture::new().await;
    f.enable().await;
    let (a, csrf_a, _) = f.session(f.a, f.now).await;
    let existing = f
        .service
        .request_email_change(
            &a,
            csrf_a.expose(),
            "existing",
            1,
            "orchard_b@example.invalid",
            f.now,
        )
        .await
        .unwrap();
    assert!(existing.resource_ref.is_none());
    assert_eq!(f.account(f.a).await.version, 2);
    assert_eq!(f.count("identity", "mail_requests").await, 0);
    f.service
        .request_email_change(
            &a,
            csrf_a.expose(),
            "expiry",
            2,
            "change_later@example.invalid",
            f.now,
        )
        .await
        .unwrap();
    let expired = f.token(f.a, "change_email").await;
    let tomorrow = f.now + Duration::hours(24);
    assert!(
        f.service
            .confirm_email_change(&f.context(tomorrow).await, expired.expose(), tomorrow)
            .await
            .is_err()
    );
    assert_eq!(f.account(f.a).await.email, "orchard_a@example.invalid");
    f.service
        .request_email_change(
            &a,
            csrf_a.expose(),
            "newer",
            3,
            "contended@example.invalid",
            f.now,
        )
        .await
        .unwrap();
    let token_a = f.token(f.a, "change_email").await;
    let (b, csrf_b, _) = f.session(f.b, f.now).await;
    f.service
        .request_email_change(
            &b,
            csrf_b.expose(),
            "contend",
            1,
            "contended@example.invalid",
            f.now,
        )
        .await
        .unwrap();
    let token_b = f.token(f.b, "change_email").await;
    f.service
        .confirm_email_change(&f.context(f.now).await, token_b.expose(), f.now)
        .await
        .unwrap();
    assert!(
        f.service
            .confirm_email_change(&f.context(f.now).await, token_a.expose(), f.now)
            .await
            .is_err()
    );
    assert_eq!(f.account(f.a).await.email, "orchard_a@example.invalid");
    assert_eq!(f.account(f.a).await.credential_version, 1);
    assert_eq!(
        f.token(f.a, "change_email").await.expose(),
        token_a.expose()
    );
}

#[tokio::test]
async fn password_change_invalidates_sessions_recovery_and_stores_no_cleartext_digest() {
    let f = Fixture::new().await;
    f.enable().await;
    let (a, csrf, raw_a) = f.session(f.a, f.now).await;
    let (_, _, raw_other) = f.session(f.a, f.now).await;
    let mut tx = DbTransaction::begin(&f.service.pool).await.unwrap();
    queue_token(
        &mut tx.connection(),
        &f.service.renderer,
        f.a,
        "reset_password",
        "orchard_a@example.invalid",
        "orchard_a@example.invalid",
        f.now,
        f.now + Duration::hours(1),
        f.service.token_issuer.as_ref().unwrap(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let reset = f.token(f.a, "reset_password").await;
    let password = "synthetic replacement password";
    f.service
        .change_password(&a, csrf.expose(), "change_password", 1, password, f.now)
        .await
        .unwrap();
    assert!(
        f.service
            .authenticate(raw_a.expose(), f.now)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.service
            .authenticate(raw_other.expose(), f.now)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(f.account(f.a).await.credential_version, 2);
    assert!(
        f.service
            .reset_password(
                &f.context(f.now).await,
                reset.expose(),
                "synthetic attacker replacement",
                f.now
            )
            .await
            .is_err()
    );
    let mut tx = DbTransaction::begin_read(&f.service.pool).await.unwrap();
    let stored = repo::password_hash(&mut tx.connection(), &f.service.renderer, f.a)
        .await
        .unwrap()
        .unwrap();
    assert!(f.service.passwords.verify(password, Some(&stored)));
    assert!(
        !f.service
            .passwords
            .verify("synthetic initial password", Some(&stored))
    );
    let canonical = serde_json::to_vec(&(1, password)).unwrap();
    let unbound_hash = platform::transaction::request_hash(&canonical);
    let idempotency = f.table("platform", "idempotency_records");
    for row in tx
        .connection()
        .fetch_all(&format!("SELECT request_hash FROM {idempotency}"), &[])
        .await
        .unwrap()
    {
        assert_ne!(row.text("request_hash").unwrap(), unbound_hash);
    }
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn mail_outbox_failure_rolls_back_email_request_password_and_revocations() {
    let f = Fixture::new().await;
    f.enable().await;
    let (a, csrf, raw) = f.session(f.a, f.now).await;
    let Pool::Sqlite(pool, _) = &f.service.pool else {
        unreachable!()
    };
    sqlx::query(&format!("CREATE TRIGGER synthetic_mail_failure BEFORE INSERT ON {} BEGIN SELECT RAISE(ABORT, 'synthetic test failure'); END", f.table("identity", "mail_requests"))).execute(pool).await.unwrap();
    assert!(
        f.service
            .request_email_change(
                &a,
                csrf.expose(),
                "retryable",
                1,
                "changed@example.invalid",
                f.now
            )
            .await
            .is_err()
    );
    assert_eq!(f.account(f.a).await.version, 1);
    assert!(
        f.service
            .change_password(
                &a,
                csrf.expose(),
                "failed_password",
                1,
                "synthetic rollback password",
                f.now
            )
            .await
            .is_err()
    );
    assert_eq!(f.account(f.a).await.credential_version, 1);
    assert_eq!(f.account(f.a).await.version, 1);
    assert!(
        f.service
            .authenticate(raw.expose(), f.now)
            .await
            .unwrap()
            .is_some()
    );
    let mut read = DbTransaction::begin_read(&f.service.pool).await.unwrap();
    let hash = repo::password_hash(&mut read.connection(), &f.service.renderer, f.a)
        .await
        .unwrap()
        .unwrap();
    assert!(
        f.service
            .passwords
            .verify("synthetic initial password", Some(&hash))
    );
    read.rollback().await.unwrap();
    assert_eq!(f.count("identity", "one_time_tokens").await, 0);
    assert_eq!(f.count("platform", "outbox_events").await, 0);
    assert_eq!(f.count("platform", "idempotency_records").await, 0);
    sqlx::query("DROP TRIGGER synthetic_mail_failure")
        .execute(pool)
        .await
        .unwrap();
    f.service
        .request_email_change(
            &a,
            csrf.expose(),
            "retryable",
            1,
            "changed@example.invalid",
            f.now,
        )
        .await
        .unwrap();
    assert_eq!(f.account(f.a).await.version, 2);
}
