use crate::{contract::*, core::*, repo};
use chrono::{DateTime, Duration, Utc};
use platform::{
    config::Config,
    identifiers::SchemaRenderer,
    outbox::BoxFuture,
    policy::{PolicyError, PolicySet, PolicyVersion},
    transaction::new_operation_id,
};
use platform_db::{
    Pool,
    transaction::{DbTransaction, DbValue},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
struct SyntheticPolicy;
impl IdentityPolicy for SyntheticPolicy {
    fn username(&self, v: &str) -> Result<NormalizedIdentity, IdentityError> {
        if !(3..=24).contains(&v.len())
            || !v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || ["admin", "moderator"].contains(&v.to_ascii_lowercase().as_str())
        {
            return Err(IdentityError::Rejected);
        }
        Ok(NormalizedIdentity {
            display: v.into(),
            key: v.to_ascii_lowercase(),
        })
    }
    fn email(&self, v: &str) -> Result<NormalizedIdentity, IdentityError> {
        let v = v.trim().to_ascii_lowercase();
        if !v.contains('@') {
            return Err(IdentityError::Rejected);
        }
        Ok(NormalizedIdentity {
            display: v.clone(),
            key: v,
        })
    }
    fn bio(&self, v: &str) -> Result<String, IdentityError> {
        if v.chars().count() > 100 {
            return Err(IdentityError::Rejected);
        }
        Ok(v.into())
    }
    fn preauth_expires_at(&self, n: DateTime<Utc>) -> Result<DateTime<Utc>, IdentityError> {
        Ok(n + Duration::hours(2))
    }
    fn email_change_expires_at(&self, n: DateTime<Utc>) -> Result<DateTime<Utc>, IdentityError> {
        Ok(n + Duration::hours(2))
    }
    fn invitation_window(&self, n: DateTime<Utc>) -> Result<InvitationWindow, IdentityError> {
        let start = n.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
        Ok(InvitationWindow {
            start,
            end: start + Duration::days(1),
            expires_at: n + Duration::days(2),
            daily_limit: 5,
            unused_limit: 5,
        })
    }
}
struct SyntheticReader;
impl platform_db::policy::PolicyReader for SyntheticReader {
    fn current<'a, 'c>(
        &'a self,
        _: &'a mut platform_db::transaction::TransactionConnection<'c>,
        _: &'a SchemaRenderer,
    ) -> BoxFuture<'a, Result<PolicySet, PolicyError>> {
        Box::pin(async {
            Ok(PolicySet {
                terms: 1,
                privacy: 2,
                community: 3,
            })
        })
    }
    fn documents<'a, 'c>(
        &'a self,
        _: &'a mut platform_db::transaction::TransactionConnection<'c>,
        _: &'a SchemaRenderer,
    ) -> BoxFuture<'a, Result<Vec<PolicyVersion>, PolicyError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}
struct SyntheticRisk;
impl trust::risk::RiskPolicy for SyntheticRisk {
    fn assess(
        &self,
        _: &trust::risk::TrustedRiskContext,
    ) -> Result<trust::risk::RiskDecision, trust::contract::TrustError> {
        trust::risk::RiskDecision::new(
            1,
            trust::risk::RiskAction::Allow,
            trust::risk::RiskReason::ApprovedPolicy,
        )
    }
}
pub(crate) struct Fixture {
    pub(crate) dir: PathBuf,
    pub(crate) pool: Pool,
    pub(crate) r: SchemaRenderer,
    pub(crate) service: IdentityService,
    pub(crate) issuer: Arc<TokenIssuer>,
    invite: Secret,
}
impl Fixture {
    pub(crate) async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("bbbcm-identity-{}", new_operation_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let mut cfg = Config::default().database;
        cfg.sqlite_path = dir.join("identity.sqlite3").to_string_lossy().into();
        cfg.schema_prefix = "idtest_".into();
        platform_db::migrations::migrate(
            &cfg,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&cfg).await.unwrap();
        let r = SchemaRenderer::new(&cfg.schema_prefix).unwrap();
        let engine = Arc::new(PasswordEngine::new(8, Arc::new(DisabledPasswordScreen)).unwrap());
        let issuer = Arc::new(TokenIssuer::new(random_secret(), "synthetic_v1".into()).unwrap());
        let service = IdentityService::new(
            pool.clone(),
            r.clone(),
            Arc::new(SyntheticPolicy),
            Arc::new(SyntheticReader),
            engine,
        )
        .with_token_issuer(issuer.clone())
        .with_risk_policy(Arc::new(SyntheticRisk));
        let mut tx = DbTransaction::begin(&pool).await.unwrap();
        let users = repo::table(&tx.connection(), &r, "users").unwrap();
        tx.connection().execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),&[new_operation_id().into(),new_operation_id().into(),DbValue::Bool(true),"ordinary_inviter".into(),"ordinary_inviter".into(),"inviter@example.invalid".into(),"inviter@example.invalid".into(),"active".into(),DbValue::Bool(true),"".into(),DbValue::Time(at()),DbValue::Time(at()),1.into(),1.into()]).await.unwrap();
        let inviter = tx.connection().last_insert_id().await.unwrap();
        let invite = random_secret();
        repo::create_invitation(
            &mut tx.connection(),
            &r,
            inviter,
            &new_operation_id(),
            &secret_hash(invite.expose()),
            at(),
            at() + Duration::days(2),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        Self {
            dir,
            pool,
            r,
            service,
            issuer,
            invite,
        }
    }
    pub(crate) async fn enable(&self) {
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        let t = self
            .r
            .table(tx.connection().backend(), "platform", "feature_flags")
            .unwrap();
        for key in ["public_read", "registration", "email_verification"] {
            tx.connection()
                .execute(
                    &format!(
                        "UPDATE {t} SET enabled = ?, version = version + 1 WHERE flag_key = ?"
                    ),
                    &[DbValue::Bool(true), key.into()],
                )
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
    }
    pub(crate) async fn context(&self) -> RequestContext {
        let grant = self.service.create_preauth(at()).await.unwrap();
        RequestContext::new(grant.token, grant.csrf, new_operation_id())
    }
    pub(crate) fn registration(&self) -> RegistrationInput {
        RegistrationInput {
            username: "ordinary_member".into(),
            email: "member@example.invalid".into(),
            password: "Synthetic password 123".to_owned().into(),
            invitation: self.invite.clone(),
            policies: vec![
                ("terms".into(), "1".into()),
                ("privacy".into(), "2".into()),
                ("community".into(), "3".into()),
            ],
        }
    }
    pub(crate) async fn count(&self, schema: &str, table: &str) -> i64 {
        let mut tx = DbTransaction::begin_read(&self.pool).await.unwrap();
        let t = self
            .r
            .table(tx.connection().backend(), schema, table)
            .unwrap();
        let n = tx
            .connection()
            .fetch_optional(&format!("SELECT COUNT(*) AS total FROM {t}"), &[])
            .await
            .unwrap()
            .unwrap()
            .i64("total")
            .unwrap();
        tx.rollback().await.unwrap();
        n
    }
    pub(crate) async fn token(&self, purpose: &str) -> (String, String) {
        let mut tx = DbTransaction::begin_read(&self.pool).await.unwrap();
        let t = repo::table(&tx.connection(), &self.r, "one_time_tokens").unwrap();
        let row=tx.connection().fetch_optional(&format!("SELECT token_id, token_nonce, user_id, target_email_key, expires_at, issued_credential_version FROM {t} WHERE purpose = ? ORDER BY created_at DESC, token_id DESC LIMIT 1"),&[purpose.into()]).await.unwrap().unwrap();
        let id = row.text("token_id").unwrap();
        let raw = self.issuer.derive(
            &id,
            &row.text("token_nonce").unwrap(),
            row.i64("user_id").unwrap(),
            purpose,
            &row.text("target_email_key").unwrap(),
            row.time("expires_at").unwrap(),
            row.i64("issued_credential_version").unwrap(),
        );
        tx.rollback().await.unwrap();
        (id, raw.expose().into())
    }
    pub(crate) async fn login(&self) -> (SessionGrant, SessionIdentity) {
        let c = self.context().await;
        let result = self
            .service
            .login(
                &c,
                LoginInput {
                    identifier: "ordinary_member".into(),
                    password: "Synthetic password 123".to_owned().into(),
                    device_summary: "raw user-agent should not persist".into(),
                },
                at(),
            )
            .await
            .unwrap();
        let grant = result.grant.unwrap();
        let identity = self
            .service
            .authenticate(grant.token.expose(), at())
            .await
            .unwrap()
            .unwrap();
        (grant, identity)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
pub(crate) fn at() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-05T00:00:00.000000Z").unwrap()
}
#[tokio::test]
async fn registration_atomic_replay_rollback_single_use_expiry_and_default_closed() {
    let f = Fixture::new().await;
    let c = f.context().await;
    assert!(
        f.service
            .register(&c, f.registration(), at())
            .await
            .is_err()
    );
    assert_eq!(f.count("identity", "users").await, 1);
    f.enable().await;
    let mut invalid = f.registration();
    invalid.policies[0].1 = "9".into();
    assert!(f.service.register(&c, invalid, at()).await.is_err());
    assert_eq!(f.count("identity", "users").await, 1);
    assert_eq!(f.count("trust", "invite_uses").await, 0);
    let first = f
        .service
        .register(&c, f.registration(), at())
        .await
        .unwrap();
    assert!(!first.replayed);
    let replay = f
        .service
        .register(&c, f.registration(), at())
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(first.resource_ref, replay.resource_ref);
    assert_eq!(f.count("identity", "users").await, 2);
    assert_eq!(f.count("identity", "policy_consents").await, 3);
    assert_eq!(f.count("trust", "invite_uses").await, 1);
    assert_eq!(f.count("analytics", "event_facts").await, 2);
    let mut duplicate = f.registration();
    duplicate.username = "another_member".into();
    duplicate.email = "other@example.invalid".into();
    assert!(
        f.service
            .register(&f.context().await, duplicate, at())
            .await
            .is_err()
    );
    assert_eq!(f.count("identity", "users").await, 2);
    // A later failure after account insertion rolls back claims, credentials, invite and events.
    let tx_failure = Fixture::new().await;
    tx_failure.enable().await;
    let mut tx = DbTransaction::begin(&tx_failure.pool).await.unwrap();
    let t = repo::table(&tx.connection(), &tx_failure.r, "username_claims").unwrap();
    tx.connection().execute(&format!("INSERT INTO {t} (username_key, user_id, kind, reserved_until, version) VALUES (?, ?, ?, ?, ?)"),&["ordinary_member".into(),1.into(),"current".into(),DbValue::Time(at()),1.into()]).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        tx_failure
            .service
            .register(&tx_failure.context().await, tx_failure.registration(), at())
            .await
            .is_err()
    );
    assert_eq!(tx_failure.count("identity", "users").await, 1);
    assert_eq!(tx_failure.count("identity", "credentials").await, 0);
    let expired = Fixture::new().await;
    expired.enable().await;
    let ctx = expired
        .service
        .create_preauth(at() + Duration::days(3))
        .await
        .unwrap();
    let context = RequestContext::new(ctx.token, ctx.csrf, new_operation_id());
    assert!(
        expired
            .service
            .register(&context, expired.registration(), at() + Duration::days(3))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn token_single_use_reset_revokes_sessions_and_anonymous_errors_match() {
    let f = Fixture::new().await;
    f.enable().await;
    f.service
        .register(&f.context().await, f.registration(), at())
        .await
        .unwrap();
    let (_, raw) = f.token("verify_email").await;
    let c = f.context().await;
    assert!(f.service.consume_verification(&c, &raw, at()).await.is_ok());
    assert!(
        f.service
            .consume_verification(&c, &raw, at())
            .await
            .unwrap()
            .replayed
    );
    assert!(
        f.service
            .consume_verification(&f.context().await, &raw, at())
            .await
            .is_err()
    );
    assert_eq!(f.count("analytics", "event_facts").await, 3);
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let facts =
        f.r.table(tx.connection().backend(), "analytics", "event_facts")
            .unwrap();
    let rows = tx
        .connection()
        .fetch_all(&format!("SELECT fact_json FROM {facts}"), &[])
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert!(
            platform::analytics::AnalyticsFact::from_storage_json(&row.text("fact_json").unwrap())
                .unwrap()
                .excluded()
        );
    }
    tx.rollback().await.unwrap();

    let (grant, session) = f.login().await;
    assert_eq!(
        f.service
            .logout(&session, "bad-csrf", &new_operation_id(), at())
            .await
            .unwrap_err(),
        IdentityError::Csrf
    );
    assert!(
        f.service
            .authenticate(grant.token.expose(), at() + Duration::days(14))
            .await
            .unwrap()
            .is_none()
    );
    for (id, pwd) in [
        ("missing", "incorrect-pass"),
        ("ordinary_member", "incorrect-pass"),
        ("ordinary_member", "tiny"),
    ] {
        let e = f
            .service
            .login(
                &f.context().await,
                LoginInput {
                    identifier: id.into(),
                    password: pwd.to_owned().into(),
                    device_summary: "unknown".into(),
                },
                at(),
            )
            .await
            .unwrap_err();
        assert_eq!(e, IdentityError::Rejected);
    }
    let recovery = f.context().await;
    f.service
        .request_reset(&recovery, "ordinary_member", at())
        .await
        .unwrap();
    assert!(
        f.service
            .request_reset(&recovery, "ordinary_member", at())
            .await
            .unwrap()
            .replayed
    );
    let (_, raw) = f.token("reset_password").await;
    let c = f.context().await;
    f.service
        .reset_password(&c, &raw, "New synthetic password", at())
        .await
        .unwrap();
    assert!(
        f.service
            .reset_password(&c, &raw, "New synthetic password", at())
            .await
            .unwrap()
            .replayed
    );
    assert!(
        f.service
            .reset_password(&c, &raw, "Different synthetic password", at())
            .await
            .is_err()
    );
    assert!(
        f.service
            .authenticate(grant.token.expose(), at())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.service
            .reset_password(&f.context().await, &raw, "New synthetic password", at())
            .await
            .is_err()
    );
    let unknown = f
        .service
        .request_reset(&f.context().await, "unknown_member", at())
        .await
        .unwrap();
    assert_eq!(unknown.resource_ref, None);
}
#[tokio::test]
async fn immutable_mail_source_local_output_suppression_and_restart_replay() {
    use platform::{
        config::Environment,
        integrations::local::LocalRoot,
        mail::local::LocalMailProvider,
        mail::{FeedbackKind, FeedbackStore, MailMessageSource, MailSink, TrustedFeedback},
        outbox::Sink,
    };
    let f = Fixture::new().await;
    f.enable().await;
    f.service
        .register(&f.context().await, f.registration(), at())
        .await
        .unwrap();
    let source = Arc::new(
        crate::mail_source::IdentityMailSource::new_local(
            f.pool.clone(),
            f.r.clone(),
            vec![f.issuer.clone()],
            "noreply@example.invalid".into(),
            "http://127.0.0.1:58833".into(),
        )
        .unwrap(),
    );
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let requests = repo::table(&tx.connection(), &f.r, "mail_requests").unwrap();
    let id = tx
        .connection()
        .fetch_optional(
            &format!("SELECT message_id FROM {requests} WHERE kind = ?"),
            &["verify_email".into()],
        )
        .await
        .unwrap()
        .unwrap()
        .text("message_id")
        .unwrap();
    let outbox =
        f.r.table(tx.connection().backend(), "platform", "outbox_events")
            .unwrap();
    let row=tx.connection().fetch_optional(&format!("SELECT event_id, aggregate_id, occurred_at, payload, trace_id FROM {outbox} WHERE event_type = ?"),&["mail.requested".into()]).await.unwrap().unwrap();
    let event = platform::outbox::OutboxEvent {
        event_id: row.text("event_id").unwrap(),
        aggregate_type: "identity".into(),
        aggregate_id: row.text("aggregate_id").unwrap(),
        event_type: "mail.requested".into(),
        version: 1,
        occurred_at: row.time("occurred_at").unwrap(),
        payload: platform::outbox::SafePayload::from_json(&row.text("payload").unwrap()).unwrap(),
        trace_id: row.text("trace_id").unwrap(),
    };
    tx.rollback().await.unwrap();
    let mail = source.load(&id).await.unwrap();
    let (_, raw) = f.token("verify_email").await;
    assert!(mail.text.contains(&raw));
    assert!(!format!("{mail:?} {event:?}").contains(&raw));
    let local = LocalRoot::new(&f.dir.join("spool"), Environment::Test).unwrap();
    let store = Arc::new(platform_db::mail_repo::DbMailStore::new(
        f.pool.clone(),
        f.r.clone(),
    ));
    let sink = MailSink {
        source: source.clone(),
        store: store.clone(),
        provider: Arc::new(LocalMailProvider {
            root: local.clone(),
        }),
    };
    sink.deliver(&event).await.unwrap();
    assert!(
        local
            .read("mail", &event.event_id, "eml", 1_000_000)
            .await
            .unwrap()
            .windows(raw.len())
            .any(|w| w == raw.as_bytes())
    );
    f.service
        .consume_verification(&f.context().await, &raw, at())
        .await
        .unwrap();
    let restarted = MailSink {
        source: Arc::new(
            crate::mail_source::IdentityMailSource::new_local(
                f.pool.clone(),
                f.r.clone(),
                vec![f.issuer.clone()],
                "noreply@example.invalid".into(),
                "http://127.0.0.1:58833".into(),
            )
            .unwrap(),
        ),
        store: store.clone(),
        provider: Arc::new(LocalMailProvider { root: local }),
    };
    restarted.deliver(&event).await.unwrap();
    assert_eq!(source.load(&id).await.unwrap().text, mail.text);
    store
        .apply(&TrustedFeedback {
            provider: "local_test".into(),
            feedback_id: new_operation_id(),
            message_id: id,
            kind: FeedbackKind::Complaint,
            occurred_at: at(),
            received_at: at(),
            simulated: true,
        })
        .await
        .unwrap();
    f.service
        .request_reset(&f.context().await, "ordinary_member", at())
        .await
        .unwrap();
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let second = tx
        .connection()
        .fetch_optional(
            &format!("SELECT message_id FROM {requests} WHERE kind = ?"),
            &["reset_password".into()],
        )
        .await
        .unwrap()
        .unwrap()
        .text("message_id")
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        source.load(&second).await.unwrap().recipient_ref,
        mail.recipient_ref
    );
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let rows=tx.connection().fetch_all(&format!("SELECT event_id, aggregate_id, occurred_at, payload, trace_id FROM {outbox} WHERE event_type = ?"),&["mail.requested".into()]).await.unwrap();
    let row = rows
        .into_iter()
        .find(|row| {
            platform::outbox::SafePayload::from_json(&row.text("payload").unwrap())
                .unwrap()
                .reference("resource_id")
                == Some(second.as_str())
        })
        .unwrap();
    let suppressed_event = platform::outbox::OutboxEvent {
        event_id: row.text("event_id").unwrap(),
        aggregate_type: "identity".into(),
        aggregate_id: row.text("aggregate_id").unwrap(),
        event_type: "mail.requested".into(),
        version: 1,
        occurred_at: row.time("occurred_at").unwrap(),
        payload: platform::outbox::SafePayload::from_json(&row.text("payload").unwrap()).unwrap(),
        trace_id: row.text("trace_id").unwrap(),
    };
    tx.rollback().await.unwrap();
    assert_eq!(
        restarted.deliver(&suppressed_event).await.unwrap_err(),
        platform::outbox::SinkError::Permanent
    );
    let spool = LocalRoot::new(&f.dir.join("spool"), Environment::Test).unwrap();
    assert!(
        spool
            .read("mail", &suppressed_event.event_id, "eml", 1_000_000)
            .await
            .is_err()
    );
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let dispatches =
        f.r.table(tx.connection().backend(), "platform", "mail_dispatches")
            .unwrap();
    let state = tx
        .connection()
        .fetch_optional(
            &format!("SELECT state FROM {dispatches} WHERE event_id = ?"),
            &[suppressed_event.event_id.clone().into()],
        )
        .await
        .unwrap()
        .unwrap()
        .text("state")
        .unwrap();
    assert_eq!(state, "suppressed");
    tx.rollback().await.unwrap();
    let rebuilt = MailSink {
        source,
        store: Arc::new(platform_db::mail_repo::DbMailStore::new(
            f.pool.clone(),
            f.r.clone(),
        )),
        provider: Arc::new(LocalMailProvider { root: spool }),
    };
    assert_eq!(
        rebuilt.deliver(&suppressed_event).await.unwrap_err(),
        platform::outbox::SinkError::Permanent
    );
}
#[test]
fn passwords_have_fixed_argon2_path_and_secret_debug_redaction() {
    let e = PasswordEngine::new(8, Arc::new(DisabledPasswordScreen)).unwrap();
    let hash = e.hash("synthetic password").unwrap();
    assert!(hash.starts_with("$argon2id$"));
    assert!(e.verify("synthetic password", Some(&hash)));
    assert!(!e.verify("wrong password", Some(&hash)));
    assert!(!e.verify("synthetic password", None));
    assert!(!e.verify("synthetic password", Some("broken hash")));
    assert_eq!(e.verification_count(), 4);
    assert!(e.hash("短密码").is_err());
    assert!(!format!("{:?}", Secret("PRIVATE_SENTINEL".into())).contains("PRIVATE_SENTINEL"));
}
#[tokio::test]
async fn scheduler_failure_retry_and_recreated_worker_resume_persistent_identity_mail() {
    use platform::{
        config::Environment,
        integrations::{AdapterError, local::LocalRoot},
        mail::local::LocalMailProvider,
        mail::{DeliveryReceipt, MailMessage, MailProvider, MailSink},
    };
    use platform_outbox::{ExternalEffects, Scheduler, Settings, SinkRegistry};
    struct SyntheticEffects;
    impl ExternalEffects for SyntheticEffects {
        fn allowed(&self) -> bool {
            true
        }
    }
    struct FailingLocal;
    impl MailProvider for FailingLocal {
        fn simulated(&self) -> bool {
            true
        }
        fn submit<'a>(
            &'a self,
            _: &'a str,
            _: &'a MailMessage,
        ) -> BoxFuture<'a, Result<DeliveryReceipt, AdapterError>> {
            Box::pin(async { Err(AdapterError::Unavailable) })
        }
    }
    let f = Fixture::new().await;
    f.enable().await;
    f.service
        .register(&f.context().await, f.registration(), at())
        .await
        .unwrap();
    let build = |provider: Arc<dyn MailProvider>| {
        let source = Arc::new(
            crate::mail_source::IdentityMailSource::new_local(
                f.pool.clone(),
                f.r.clone(),
                vec![f.issuer.clone()],
                "noreply@example.invalid".into(),
                "http://127.0.0.1:58833".into(),
            )
            .unwrap(),
        );
        let mut registry = SinkRegistry::default();
        registry
            .register(Arc::new(MailSink {
                source,
                store: Arc::new(platform_db::mail_repo::DbMailStore::new(
                    f.pool.clone(),
                    f.r.clone(),
                )),
                provider,
            }))
            .unwrap();
        registry
            .register(Arc::new(platform::analytics::AnalyticsSink::new(Arc::new(
                platform_db::analytics_repo::DbAnalyticsStore::new(f.pool.clone(), f.r.clone()),
            ))))
            .unwrap();
        Scheduler::new(
            Arc::new(platform_db::outbox_repo::DbOutboxStore::new(
                f.pool.clone(),
                f.r.clone(),
            )),
            registry,
            Arc::new(SyntheticEffects),
            Settings {
                retry_base: std::time::Duration::from_millis(1),
                retry_max: std::time::Duration::from_millis(1),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let failed = build(Arc::new(FailingLocal));
    let cancellation = tokio_util::sync::CancellationToken::new();
    assert_eq!(failed.tick(&cancellation).await.unwrap(), 3);
    drop(failed);
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let outbox =
        f.r.table(tx.connection().backend(), "platform", "outbox_events")
            .unwrap();
    let row = tx
        .connection()
        .fetch_optional(
            &format!("SELECT event_id, state, attempts FROM {outbox} WHERE event_type = ?"),
            &["mail.requested".into()],
        )
        .await
        .unwrap()
        .unwrap();
    let event_id = row.text("event_id").unwrap();
    assert_eq!(row.text("state").unwrap(), "ready");
    assert_eq!(row.i64("attempts").unwrap(), 1);
    tx.rollback().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let spool = LocalRoot::new(&f.dir.join("scheduler-spool"), Environment::Test).unwrap();
    let restarted = build(Arc::new(LocalMailProvider {
        root: spool.clone(),
    }));
    assert_eq!(restarted.tick(&cancellation).await.unwrap(), 1);
    drop(restarted);
    let bytes = spool
        .read("mail", &event_id, "eml", 1_000_000)
        .await
        .unwrap();
    assert!(
        String::from_utf8(bytes.clone())
            .unwrap()
            .contains("X-BBBcm-Delivery: local-test-only")
    );
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let row = tx
        .connection()
        .fetch_optional(
            &format!("SELECT state, attempts FROM {outbox} WHERE event_id = ?"),
            &[event_id.clone().into()],
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.text("state").unwrap(), "delivered");
    assert_eq!(row.i64("attempts").unwrap(), 2);
    tx.rollback().await.unwrap();
    let restarted_again = build(Arc::new(LocalMailProvider {
        root: spool.clone(),
    }));
    assert_eq!(restarted_again.tick(&cancellation).await.unwrap(), 0);
    assert_eq!(
        spool
            .read("mail", &event_id, "eml", 1_000_000)
            .await
            .unwrap(),
        bytes
    );
}
#[tokio::test]
async fn invitation_authority_quota_revocation_privacy_and_operation_binding() {
    let f = Fixture::new().await;
    f.enable().await;
    let registration_context = f.context().await;
    f.service
        .register(&registration_context, f.registration(), at())
        .await
        .unwrap();
    let (grant, session) = f.login().await;
    assert!(
        f.service
            .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), at())
            .await
            .is_err()
    );
    let (_, verification) = f.token("verify_email").await;
    f.service
        .consume_verification(&f.context().await, &verification, at())
        .await
        .unwrap();
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let snapshot = trust::core::load_identity_snapshot(&mut tx.connection(), &f.r, session.user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.level(), trust::contract::TrustLevel::T0);
    assert!(snapshot.invitation_allowed());
    tx.rollback().await.unwrap();
    let first = f
        .service
        .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), at())
        .await
        .unwrap();
    let revoke_key = new_operation_id();
    f.service
        .revoke_invitation(
            &session,
            grant.csrf.expose(),
            &revoke_key,
            &first.invitation_ref,
            at(),
        )
        .await
        .unwrap();
    assert!(
        f.service
            .revoke_invitation(
                &session,
                grant.csrf.expose(),
                &revoke_key,
                &first.invitation_ref,
                at()
            )
            .await
            .unwrap()
            .replayed
    );
    for _ in 0..4 {
        f.service
            .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), at())
            .await
            .unwrap();
    }
    assert!(
        f.service
            .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), at())
            .await
            .is_err()
    );
    let page = f.service.invitations(&session, at()).await.unwrap();
    assert_eq!(page.generated_today, 5);
    assert_eq!(page.unused, 4);
    assert!(!format!("{page:?}").contains(first.code.expose()));
    assert!(!format!("{page:?}").contains("member@example.invalid"));
    let mut rejected = f.registration();
    rejected.username = "new_member".into();
    rejected.email = "new@example.invalid".into();
    rejected.invitation = first.code;
    assert!(
        f.service
            .register(&f.context().await, rejected, at())
            .await
            .is_err()
    );
    let next_day = at() + Duration::days(1);
    f.service
        .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), next_day)
        .await
        .unwrap();
    let next_page = f.service.invitations(&session, next_day).await.unwrap();
    assert_eq!(next_page.generated_today, 1);
    assert_eq!(next_page.unused, 5);
    assert!(
        f.service
            .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), next_day)
            .await
            .is_err()
    );
    let logout_key = new_operation_id();
    f.service
        .logout(&session, grant.csrf.expose(), &logout_key, at())
        .await
        .unwrap();
    assert!(
        f.service
            .authenticate(grant.token.expose(), at())
            .await
            .unwrap()
            .is_none()
    );
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let t =
        f.r.table(tx.connection().backend(), "platform", "idempotency_records")
            .unwrap();
    let id=tx.connection().fetch_optional(&format!("SELECT operation_id FROM {t} WHERE scope = ? AND actor_id = ? AND idempotency_key = ?"),&["identity.logout".into(),session.user_id.into(),logout_key.clone().into()]).await.unwrap().unwrap().text("operation_id").unwrap();
    tx.rollback().await.unwrap();
    assert!(matches!(
        f.service
            .session_operation_by_token(
                grant.token.expose(),
                grant.csrf.expose(),
                "identity.logout",
                &logout_key,
                &id,
                at()
            )
            .await
            .unwrap(),
        platform::transaction::OperationStatus::Completed(_)
    ));
    assert!(matches!(
        f.service
            .session_operation_by_token(
                grant.token.expose(),
                grant.csrf.expose(),
                "identity.logout",
                "other-key",
                &id,
                at()
            )
            .await
            .unwrap(),
        platform::transaction::OperationStatus::NotObserved
    ));
    assert!(
        f.service
            .session_operation_by_token(
                grant.token.expose(),
                "wrong-csrf",
                "identity.logout",
                &logout_key,
                &id,
                at()
            )
            .await
            .is_err()
    );
    assert!(
        f.service
            .session_operation_by_token(
                grant.token.expose(),
                grant.csrf.expose(),
                "identity.public.register",
                &logout_key,
                &id,
                at()
            )
            .await
            .is_err()
    );
}
#[tokio::test]
async fn risk_unavailable_is_persisted_and_successful_registration_links_private_analytics_actor() {
    let f = Fixture::new().await;
    f.enable().await;
    let unavailable = f
        .service
        .clone()
        .with_risk_policy(Arc::new(trust::risk::UnconfiguredRiskPolicy));
    assert_eq!(
        unavailable
            .register(&f.context().await, f.registration(), at())
            .await
            .unwrap_err(),
        IdentityError::Unavailable
    );
    assert_eq!(f.count("identity", "users").await, 1);
    assert_eq!(f.count("identity", "mail_requests").await, 0);
    assert_eq!(f.count("trust", "risk_events").await, 1);
    let before = unavailable.passwords.verification_count();
    assert_eq!(
        unavailable
            .login(
                &f.context().await,
                LoginInput {
                    identifier: "missing_member".into(),
                    password: "incorrect password".to_owned().into(),
                    device_summary: "unknown".into()
                },
                at()
            )
            .await
            .unwrap_err(),
        IdentityError::Unavailable
    );
    assert_eq!(unavailable.passwords.verification_count(), before + 1);
    assert_eq!(f.count("trust", "risk_events").await, 2);
    f.service
        .register(&f.context().await, f.registration(), at())
        .await
        .unwrap();
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let facts =
        f.r.table(tx.connection().backend(), "analytics", "event_facts")
            .unwrap();
    let risks =
        f.r.table(tx.connection().backend(), "trust", "risk_events")
            .unwrap();
    let row=tx.connection().fetch_optional(&format!("SELECT f.fact_json, r.event_kind, r.outcome FROM {facts} f JOIN {risks} r ON r.event_id = f.event_id WHERE f.event_kind = ?"),&["registered".into()]).await.unwrap().unwrap();
    assert_eq!(row.text("event_kind").unwrap(), "registration");
    assert_eq!(row.text("outcome").unwrap(), "allow");
    let fact =
        platform::analytics::AnalyticsFact::from_storage_json(&row.text("fact_json").unwrap())
            .unwrap();
    let users = repo::table(&tx.connection(), &f.r, "users").unwrap();
    let owner = tx
        .connection()
        .fetch_optional(
            &format!("SELECT username FROM {users} WHERE analytics_pseudonym = ?"),
            &[fact.actor_pseudonym().into()],
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(owner.text("username").unwrap(), "ordinary_member");
    tx.rollback().await.unwrap();
}
