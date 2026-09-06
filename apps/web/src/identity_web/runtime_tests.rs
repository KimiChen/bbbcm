//! Real application factories and worker scheduler; fixtures remain isolated ordinary accounts.
use super::*;
use chrono::{Duration, Utc};
use identity::contract::{
    LoginInput, RegistrationInput, RequestContext, SessionGrant, SessionIdentity,
};
use platform::{
    config::{Config, Environment},
    identifiers::SchemaRenderer,
    integrations::AdapterMode,
    transaction::new_operation_id,
};
use platform_db::{
    Pool,
    transaction::{DbTransaction, DbValue},
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

pub(crate) struct Fixture {
    pub(crate) dir: PathBuf,
    pub(crate) config: Config,
    pub(crate) pool: Pool,
    pub(crate) renderer: SchemaRenderer,
    pub(crate) service: IdentityService,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
impl Fixture {
    pub(crate) async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("bbbcm-runtime-{}", new_operation_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let mut config = Config::default();
        config.server.environment = Environment::Test;
        config.database.sqlite_path = dir.join("identity.sqlite3").to_string_lossy().into();
        config.database.schema_prefix = "runtime_probe_".into();
        config.adapters.mode = AdapterMode::LocalTest;
        config.adapters.local_root = Some(dir.join("spool").to_string_lossy().into());
        config.identity.active_token_version = Some("synthetic_v1".into());
        config.identity.token_keys =
            Some(format!("{{\"synthetic_v1\":\"{}\"}}", "a".repeat(64)).into());
        config.identity.risk_key = Some("b".repeat(64).into());
        config.identity.mail_from = Some("noreply@example.invalid".into());
        config.identity.mail_base_url = Some("http://127.0.0.1:18099".into());
        config.validate().unwrap();
        platform_db::migrations::migrate(
            &config.database,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect_existing(&config.database).await.unwrap();
        let renderer = SchemaRenderer::new(&config.database.schema_prefix).unwrap();
        let service = service(&config, pool.clone(), renderer.clone()).unwrap();
        let f = Self {
            dir,
            config,
            pool,
            renderer,
            service,
        };
        f.seed().await;
        f
    }
    pub(crate) async fn seed(&self) {
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        let mut c = tx.connection();
        let policy = self
            .renderer
            .table(c.backend(), "moderation", "policy_versions")
            .unwrap();
        for kind in ["terms", "privacy", "community"] {
            c.execute(&format!("INSERT INTO {policy} (kind, version, body, status, created_at) VALUES (?, ?, ?, ?, ?)"), &[kind.into(), 1.into(), "仅供本机测试：合成政策，无正式法律效力。".into(), "synthetic".into(), DbValue::Time(Utc::now())]).await.unwrap();
        }
        let users = self
            .renderer
            .table(c.backend(), "identity", "users")
            .unwrap();
        c.execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"), &[new_operation_id().into(), new_operation_id().into(), DbValue::Bool(true), "ordinary_inviter".into(), "ordinary_inviter".into(), "inviter@example.invalid".into(), "inviter@example.invalid".into(), "active".into(), DbValue::Bool(true), "".into(), DbValue::Time(Utc::now()), DbValue::Time(Utc::now()), 1.into(), 1.into()]).await.unwrap();
        let user_id = c.last_insert_id().await.unwrap();
        let invites = self
            .renderer
            .table(c.backend(), "identity", "invitations")
            .unwrap();
        c.execute(&format!("INSERT INTO {invites} (invite_ref, creator_id, code_hash, source, state, created_at, expires_at, consumed_at, consumer_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"), &[new_operation_id().into(), user_id.into(), format!("{:x}", Sha256::digest(b"synthetic-invitation-only")).into(), "user".into(), "available".into(), DbValue::Time(Utc::now()), DbValue::Time(Utc::now()+Duration::days(3)), DbValue::Time(Utc::now()), 0.into()]).await.unwrap();
        tx.commit().await.unwrap();
    }
    pub(crate) async fn enable(&self) {
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        let mut c = tx.connection();
        let t = self
            .renderer
            .table(c.backend(), "platform", "feature_flags")
            .unwrap();
        for key in ["public_read", "registration", "email_verification"] {
            c.execute(
                &format!("UPDATE {t} SET enabled = ?, version = version + 1 WHERE flag_key = ?"),
                &[DbValue::Bool(true), key.into()],
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
    }
    pub(crate) async fn context(&self) -> RequestContext {
        let grant = self.service.create_preauth(Utc::now()).await.unwrap();
        RequestContext::new(grant.token, grant.csrf, new_operation_id())
    }
    pub(crate) async fn registration(&self) -> RegistrationInput {
        let policies = self.service.current_policies().await.unwrap();
        RegistrationInput {
            username: "Orchard_member".into(),
            email: "Member@EXAMPLE.INVALID".into(),
            password: "Synthetic orchard password 734!".to_string().into(),
            invitation: "synthetic-invitation-only".to_string().into(),
            policies: vec![
                ("terms".into(), policies.terms.to_string()),
                ("privacy".into(), policies.privacy.to_string()),
                ("community".into(), policies.community.to_string()),
            ],
        }
    }
    pub(crate) async fn login(&self, password: &str) -> (SessionGrant, SessionIdentity) {
        let result = self
            .service
            .login(
                &self.context().await,
                LoginInput {
                    identifier: "orchard_MEMBER".into(),
                    password: password.to_string().into(),
                    device_summary: "浏览器".into(),
                },
                Utc::now(),
            )
            .await
            .unwrap();
        let grant = result.grant.unwrap();
        let identity = self
            .service
            .authenticate(grant.token.expose(), Utc::now())
            .await
            .unwrap()
            .unwrap();
        (grant, identity)
    }
    pub(crate) async fn message(&self, kind: &str) -> (String, String) {
        let mut tx = DbTransaction::begin_read(&self.pool).await.unwrap();
        let mut c = tx.connection();
        let t = self
            .renderer
            .table(c.backend(), "identity", "mail_requests")
            .unwrap();
        let row = c
            .fetch_optional(
                &format!("SELECT message_id FROM {t} WHERE kind = ? ORDER BY created_at DESC"),
                &[kind.into()],
            )
            .await
            .unwrap()
            .unwrap();
        let id = row.text("message_id").unwrap();
        tx.rollback().await.unwrap();
        let source = crate::identity_services::mail_source(
            &self.config,
            self.pool.clone(),
            self.renderer.clone(),
        )
        .unwrap();
        let message = source.load(&id).await.unwrap();
        (id, message.text)
    }
    pub(crate) async fn drain(&self, config: &Config) {
        let runtime = platform::cache::RedisRuntime::from_config(config).unwrap();
        let scheduler =
            crate::worker_services::scheduler(config, self.pool.clone(), &runtime).unwrap();
        for _ in 0..16 {
            if scheduler.tick(&CancellationToken::new()).await.unwrap() == 0 {
                return;
            }
        }
        panic!("bounded synthetic outbox failed to drain");
    }
    fn mails(&self) -> Vec<String> {
        std::fs::read_dir(self.dir.join("spool/mail"))
            .into_iter()
            .flatten()
            .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
            .collect()
    }
}
pub(crate) fn token(text: &str) -> String {
    text.lines()
        .find(|line| line.len() == 64 && line.bytes().all(|c| c.is_ascii_hexdigit()))
        .expect("local message has one bearer")
        .to_owned()
}

#[tokio::test]
async fn actual_runtime_registration_rotation_worker_recovery_and_email_change() {
    let mut f = Fixture::new().await;
    assert!(
        f.service
            .register(&f.context().await, f.registration().await, Utc::now())
            .await
            .is_err()
    );
    f.enable().await;
    let input = f.registration().await;
    let context = f.context().await;
    f.service
        .register(&context, input.clone(), Utc::now())
        .await
        .unwrap();
    assert!(
        f.service
            .register(&context, input, Utc::now())
            .await
            .unwrap()
            .replayed
    );
    let (verification_id, verification_text) = f.message("verify_email").await;
    let verification = token(&verification_text);
    // Rotation never silently replaces the old version. A missing key causes a durable retry.
    let mut missing_old = f.config.clone();
    missing_old.identity.active_token_version = Some("synthetic_v2".into());
    missing_old.identity.token_keys =
        Some(format!("{{\"synthetic_v2\":\"{}\"}}", "c".repeat(64)).into());
    let missing_source =
        crate::identity_services::mail_source(&missing_old, f.pool.clone(), f.renderer.clone())
            .unwrap();
    assert!(missing_source.load(&verification_id).await.is_err());
    f.drain(&missing_old).await;
    assert!(f.mails().is_empty());
    f.config.identity.active_token_version = Some("synthetic_v2".into());
    f.config.identity.token_keys = Some(
        format!(
            "{{\"synthetic_v1\":\"{}\",\"synthetic_v2\":\"{}\"}}",
            "a".repeat(64),
            "c".repeat(64)
        )
        .into(),
    );
    f.service = service(&f.config, f.pool.clone(), f.renderer.clone()).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    f.drain(&f.config).await;
    assert_eq!(f.mails().len(), 1);
    assert!(f.mails()[0].contains("X-BBBcm-Delivery: local-test-only"));
    assert!(f.mails()[0].contains(&verification));
    f.service
        .consume_verification(&f.context().await, &verification, Utc::now())
        .await
        .unwrap();
    assert!(
        f.service
            .consume_verification(&f.context().await, &verification, Utc::now())
            .await
            .is_err()
    );
    let (old_grant, _) = f.login("Synthetic orchard password 734!").await;
    f.service
        .request_reset(&f.context().await, "member@example.invalid", Utc::now())
        .await
        .unwrap();
    let reset = token(&f.message("reset_password").await.1);
    f.drain(&f.config).await;
    f.service
        .reset_password(
            &f.context().await,
            &reset,
            "Synthetic reset password 987!",
            Utc::now(),
        )
        .await
        .unwrap();
    assert!(
        f.service
            .authenticate(old_grant.token.expose(), Utc::now())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.service
            .reset_password(
                &f.context().await,
                &reset,
                "Synthetic reset password 987!",
                Utc::now()
            )
            .await
            .is_err()
    );
    let (grant, session) = f.login("Synthetic reset password 987!").await;
    let profile = f.service.profile(&session, Utc::now()).await.unwrap();
    f.service
        .request_email_change(
            &session,
            grant.csrf.expose(),
            &new_operation_id(),
            profile.version,
            "NewMember@example.invalid",
            Utc::now(),
        )
        .await
        .unwrap();
    let change = token(&f.message("change_email").await.1);
    f.service
        .confirm_email_change(&f.context().await, &change, Utc::now())
        .await
        .unwrap();
    assert!(
        f.service
            .authenticate(grant.token.expose(), Utc::now())
            .await
            .unwrap()
            .is_none()
    );
    f.drain(&f.config).await;
    let mails = f.mails();
    assert!(mails.iter().any(|m| m.contains("密码已变更")));
    assert!(mails.iter().any(|m| m.contains("邮箱已变更")));
    let count = mails.len();
    f.drain(&f.config).await;
    assert_eq!(
        f.mails().len(),
        count,
        "a reconstructed worker does not duplicate outputs"
    );
    let replay =
        crate::identity_services::mail_source(&f.config, f.pool.clone(), f.renderer.clone())
            .unwrap()
            .load(&verification_id)
            .await
            .unwrap();
    assert_eq!(
        replay.text, verification_text,
        "consumption and credential changes preserve old retry bytes"
    );
}

#[tokio::test]
async fn actual_runtime_password_corpus_failure_never_falls_back() {
    let f = Fixture::new().await;
    let mut config = f.config.clone();
    config.identity.password_corpus_path =
        Some(f.dir.join("missing.json").to_string_lossy().into());
    config.identity.password_corpus_sha256 = Some("d".repeat(64));
    assert!(service(&config, f.pool.clone(), f.renderer.clone()).is_err());
    f.enable().await;
    let mut registration = f.registration().await;
    registration.password = "Password123".to_string().into();
    assert!(
        f.service
            .register(&f.context().await, registration, Utc::now())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn actual_web_connect_requires_complete_identity_assembly_even_with_public_read() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let f = Fixture::new().await;
    f.enable().await;
    let mut incomplete = f.config.clone();
    incomplete.identity = Default::default();
    assert!(service(&incomplete, f.pool.clone(), f.renderer.clone()).is_err());
    let assets = Assets::load_for(Environment::Test, &f.dir.join("missing-assets"), false).unwrap();
    let mut state = AppState::new(incomplete, assets.clone());
    state.runtime = Some(platform::cache::RedisRuntime::from_config(&state.config).unwrap());
    connect(&mut state).await.unwrap();
    assert!(state.identity.service.is_none() && state.identity.flags.is_some());
    let response = crate::router::router(Arc::new(state))
        .oneshot(
            Request::builder()
                .uri("/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let mut missing_risk = f.config.clone();
    missing_risk.identity.risk_key = None;
    assert!(missing_risk.validate().is_err());
    let mut ready = AppState::new(f.config.clone(), assets);
    ready.runtime = Some(platform::cache::RedisRuntime::from_config(&ready.config).unwrap());
    connect(&mut ready).await.unwrap();
    assert!(ready.identity.service.is_some() && ready.identity.risk_signals.is_some());
    let response = crate::router::router(Arc::new(ready))
        .oneshot(
            Request::builder()
                .uri("/register")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(html.contains("草莓论坛") && html.contains("邀请码"));
    assert!(!html.contains("https://sb.sb"));
}
