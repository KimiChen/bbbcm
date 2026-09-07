//! 身份应用服务：所有业务写和事件使用当前事务；原始凭据只存在于内存。
use crate::{contract::*, repo};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use chrono::{DateTime, Duration, Utc};
use platform::{
    flags::Flag,
    identifiers::SchemaRenderer,
    transaction::{
        CommandIdentity, CommitOutcome, LockPlan, LockTarget, StoredResponse, new_operation_id,
        request_hash,
    },
};
use platform_db::{
    Pool,
    command::execute_command,
    transaction::{DbTransaction, DbValue, TransactionConnection, TransactionError},
};
use rand_core::{OsRng, RngCore};
use std::sync::Arc;

pub struct PasswordEngine {
    dummy_hash: String,
    minimum: usize,
    screen: Arc<dyn PasswordScreen>,
    #[cfg(test)]
    completed_verifications: std::sync::atomic::AtomicUsize,
}
impl PasswordEngine {
    pub fn new(minimum: usize, screen: Arc<dyn PasswordScreen>) -> Result<Self, IdentityError> {
        if !(8..=256).contains(&minimum) {
            return Err(IdentityError::Rejected);
        }
        let salt = SaltString::generate(&mut OsRng);
        let dummy_hash = Argon2::default()
            .hash_password(random_secret().expose().as_bytes(), &salt)
            .map_err(|_| IdentityError::Unavailable)?
            .to_string();
        Ok(Self {
            dummy_hash,
            minimum,
            screen,
            #[cfg(test)]
            completed_verifications: std::sync::atomic::AtomicUsize::new(0),
        })
    }
    pub fn hash(&self, password: &str) -> Result<String, IdentityError> {
        if !(self.minimum..=256).contains(&password.chars().count())
            || !self.screen.allowed(password)?
        {
            return Err(IdentityError::Rejected);
        }
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|v| v.to_string())
            .map_err(|_| IdentityError::Unavailable)
    }
    /// Always one Argon2id verification, including missing/locked/deleted accounts and oversized input.
    pub fn verify(&self, password: &str, stored: Option<&str>) -> bool {
        let valid_length = (8..=256).contains(&password.chars().count());
        let candidate = if valid_length {
            password
        } else {
            "invalid-fixed-cost-input"
        };
        let encoded = stored
            .and_then(|h| PasswordHash::new(h).ok())
            .filter(|h| h.algorithm.as_str() == "argon2id");
        let real = encoded.is_some();
        let parsed = encoded
            .unwrap_or_else(|| PasswordHash::new(&self.dummy_hash).expect("engine-created hash"));
        let valid = Argon2::default()
            .verify_password(candidate.as_bytes(), &parsed)
            .is_ok();
        #[cfg(test)]
        self.completed_verifications
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        real && valid_length && valid
    }
    #[cfg(test)]
    pub(crate) fn verification_count(&self) -> usize {
        self.completed_verifications
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}
pub fn random_secret() -> Secret {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    Secret(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn secret_hash(value: &str) -> String {
    request_hash(value.as_bytes())
}
pub fn normalized_now(now: DateTime<Utc>) -> DateTime<Utc> {
    platform::db::decode_time(&platform::db::encode_time(now)).expect("UTC roundtrip")
}
pub fn check_csrf(session: &SessionIdentity, submitted: &str) -> Result<(), IdentityError> {
    if !constant_equal(&secret_hash(submitted), &session.csrf_hash) {
        return Err(IdentityError::Csrf);
    }
    Ok(())
}
pub fn constant_equal(a: &str, b: &str) -> bool {
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= (x ^ y) as usize
    }
    diff == 0
}
pub(crate) fn identity_command(
    actor_id: i64,
    scope: &str,
    key: &str,
    canonical: &[u8],
) -> CommandIdentity {
    CommandIdentity {
        actor_id,
        scope: scope.into(),
        idempotency_key: key.into(),
        request_hash: request_hash(canonical),
        operation_id: new_operation_id(),
    }
}
pub(crate) fn response(resource_ref: Option<&str>) -> StoredResponse {
    let mut payload =
        platform::outbox::SafePayload::empty().with_outcome(platform::outbox::Outcome::Success);
    if let Some(reference) = resource_ref {
        payload = payload
            .with_reference("resource_id", reference)
            .expect("core uses opaque references");
    }
    StoredResponse {
        status_code: 200,
        body: payload.to_json().expect("safe payload serializes"),
    }
}
pub(crate) fn mutation(
    outcome: CommitOutcome<platform_db::command::CommandResult>,
) -> Result<MutationResult, IdentityError> {
    match outcome {
        CommitOutcome::Committed(r) => {
            if r.response.status_code != 200 {
                return Err(if r.response.status_code == 503 {
                    IdentityError::Unavailable
                } else {
                    IdentityError::Rejected
                });
            }
            let v = platform::outbox::SafePayload::from_json(&r.response.body)
                .map_err(|_| IdentityError::Unavailable)?;
            Ok(MutationResult {
                resource_ref: v.reference("resource_id").map(str::to_owned),
                replayed: r.replayed,
            })
        }
        CommitOutcome::Unknown { operation_id } => {
            Err(IdentityError::CommitUnknown { operation_id })
        }
        CommitOutcome::RolledBack => Err(IdentityError::Unavailable),
    }
}
pub(crate) fn locks(user_id: i64) -> LockPlan {
    LockPlan::new(vec![
        LockTarget::new("identity", "users", user_id).expect("registered"),
    ])
}
pub(crate) async fn assert_flags(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    required: &[Flag],
) -> Result<(), TransactionError> {
    let table = r.table(tx.backend(), "platform", "feature_flags")?;
    let rows = tx
        .fetch_all(
            &format!("SELECT flag_key, enabled, version FROM {table} ORDER BY flag_key"),
            &[],
        )
        .await?;
    let mut facts = Vec::new();
    for row in rows {
        facts.push((
            row.text("flag_key")?,
            row.boolean("enabled")?,
            row.i64("version")?,
        ));
    }
    let flags = platform::flags::FlagSnapshot::from_rows(facts)
        .map_err(|_| TransactionError::CorruptData)?;
    if flags.get(Flag::ReadOnlyMode).enabled || required.iter().any(|f| !flags.get(*f).enabled) {
        return Err(TransactionError::InvalidInput);
    }
    Ok(())
}
#[derive(Clone)]
pub struct IdentityService {
    pub(crate) pool: Pool,
    pub(crate) renderer: SchemaRenderer,
    pub(crate) policy: SharedPolicy,
    pub(crate) policy_reader: Arc<dyn platform_db::policy::PolicyReader>,
    pub(crate) passwords: Arc<PasswordEngine>,
    pub(crate) token_issuer: Option<Arc<TokenIssuer>>,
    pub(crate) risk_policy: Arc<dyn trust::risk::RiskPolicy>,
    pub(crate) challenge_provider: Option<Arc<crate::challenge::LocalChallengeProvider>>,
    registration_analytics_excluded: bool,
}
impl IdentityService {
    pub fn new(
        pool: Pool,
        renderer: SchemaRenderer,
        policy: SharedPolicy,
        policy_reader: Arc<dyn platform_db::policy::PolicyReader>,
        passwords: Arc<PasswordEngine>,
    ) -> Self {
        Self {
            pool,
            renderer,
            policy,
            policy_reader,
            passwords,
            token_issuer: None,
            risk_policy: Arc::new(trust::risk::UnconfiguredRiskPolicy),
            challenge_provider: None,
            registration_analytics_excluded: true,
        }
    }
    pub fn with_token_issuer(mut self, issuer: Arc<TokenIssuer>) -> Self {
        self.token_issuer = Some(issuer);
        self
    }
    pub fn with_challenge_provider(
        mut self,
        provider: Arc<crate::challenge::LocalChallengeProvider>,
    ) -> Self {
        self.challenge_provider = Some(provider);
        self
    }
    /// Application-derived test/robot/source fact, never a form field. Default excludes unconfigured local runs.
    pub fn with_registration_analytics_exclusion(mut self, excluded: bool) -> Self {
        self.registration_analytics_excluded = excluded;
        self
    }
    pub fn with_risk_policy(mut self, policy: Arc<dyn trust::risk::RiskPolicy>) -> Self {
        self.risk_policy = policy;
        self
    }
    pub async fn create_preauth(&self, now: DateTime<Utc>) -> Result<PreauthGrant, IdentityError> {
        let now = normalized_now(now);
        let expires = self.policy.preauth_expires_at(now)?;
        if expires <= now {
            return Err(IdentityError::Rejected);
        }
        let token = random_secret();
        let csrf = random_secret();
        let mut tx = DbTransaction::begin(&self.pool).await?;
        repo::create_principal(
            &mut tx.connection(),
            &self.renderer,
            &secret_hash(token.expose()),
            &secret_hash(csrf.expose()),
            now,
            expires,
        )
        .await?;
        tx.commit().await?;
        Ok(PreauthGrant { token, csrf })
    }
    pub(crate) async fn public_command(
        &self,
        context: &RequestContext,
        scope: &str,
        canonical: &[u8],
        now: DateTime<Utc>,
    ) -> Result<CommandIdentity, IdentityError> {
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let id = repo::principal(&mut tx.connection(), &self.renderer, context, now).await?;
        tx.rollback().await?;
        Ok(identity_command(
            id,
            scope,
            &context.idempotency_key,
            &bound_request_hash(context.preauth.expose(), canonical),
        ))
    }
    pub async fn authenticate(
        &self,
        raw: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<SessionIdentity>, IdentityError> {
        let now = normalized_now(now);
        let mut tx = DbTransaction::begin(&self.pool).await?;
        let identity =
            repo::authenticate(&mut tx.connection(), &self.renderer, &secret_hash(raw), now)
                .await?;
        tx.commit().await?;
        Ok(identity)
    }
    pub async fn logout(
        &self,
        session: &SessionIdentity,
        csrf: &str,
        key: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        check_csrf(session, csrf)?;
        let now = normalized_now(now);
        let command = identity_command(
            session.user_id,
            "identity.logout",
            key,
            session.session_ref.as_bytes(),
        );
        let service = self.clone();
        let session = session.clone();
        mutation(
            execute_command(
                &self.pool,
                &self.renderer,
                &command,
                &locks(session.user_id),
                now,
                move |tx| {
                    let service = service.clone();
                    let session = session.clone();
                    Box::pin(async move {
                        repo::require_session(tx, &service.renderer, &session, now, false).await?;
                        repo::revoke_one(
                            tx,
                            &service.renderer,
                            session.user_id,
                            &session.session_ref,
                        )
                        .await?;
                        Ok(response(None))
                    })
                },
            )
            .await?,
        )
    }
    pub async fn reauthenticate(
        &self,
        session: &SessionIdentity,
        csrf: &str,
        password: &str,
        now: DateTime<Utc>,
    ) -> Result<(), IdentityError> {
        check_csrf(session, csrf)?;
        let now = normalized_now(now);
        let mut tx = DbTransaction::begin(&self.pool).await?;
        let mut conn = tx.connection();
        conn.lock(
            &self.renderer,
            &LockTarget::new("identity", "users", session.user_id).expect("registered"),
        )
        .await?;
        repo::require_session(&mut conn, &self.renderer, session, now, false).await?;
        let hash = repo::password_hash(&mut conn, &self.renderer, session.user_id).await?;
        if !self.passwords.verify(password, hash.as_deref()) {
            return Err(IdentityError::Rejected);
        }
        repo::set_reauthenticated(&mut conn, &self.renderer, session.session_id, now).await?;
        tx.commit().await?;
        Ok(())
    }
}
/// Key custody is application supplied. No default/config loader enables this pending design.
/// Context has no raw bearer; retry reconstructs the same HMAC output using external key custody.
pub struct TokenIssuer {
    key: Secret,
    pub(crate) version: String,
}
impl TokenIssuer {
    pub fn new(key: Secret, version: String) -> Result<Self, IdentityError> {
        if key.expose().len() < 32
            || version.is_empty()
            || version.len() > 64
            || !version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(IdentityError::Rejected);
        }
        Ok(Self { key, version })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn derive(
        &self,
        id: &str,
        nonce: &str,
        user_id: i64,
        purpose: &str,
        email_key: &str,
        expires: DateTime<Utc>,
        credential_version: i64,
    ) -> Secret {
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(self.key.expose().as_bytes())
            .expect("HMAC accepts any key");
        mac.update(format!("bbbcm.identity.token.v1\n{}\n{id}\n{nonce}\n{user_id}\n{purpose}\n{email_key}\n{}\n{credential_version}",self.version,platform::db::encode_time(expires)).as_bytes());
        Secret(
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        )
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn queue_token(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    purpose: &str,
    target_email: &str,
    target_key: &str,
    now: DateTime<Utc>,
    expires: DateTime<Utc>,
    issuer: &TokenIssuer,
) -> Result<String, TransactionError> {
    if !matches!(purpose, "verify_email" | "reset_password" | "change_email") || expires <= now {
        return Err(TransactionError::InvalidInput);
    }
    let account = repo::account_by_id(tx, r, user_id)
        .await?
        .ok_or(TransactionError::InvalidInput)?;
    repo::invalidate_token_purpose(tx, r, user_id, purpose).await?;
    let id = new_operation_id();
    let nonce = random_secret();
    let secret = issuer.derive(
        &id,
        nonce.expose(),
        user_id,
        purpose,
        target_key,
        expires,
        account.credential_version,
    );
    let t = repo::table(tx, r, "one_time_tokens")?;
    tx.execute(&format!("INSERT INTO {t} (token_id, user_id, purpose, token_hash, target_email, target_email_key, created_at, expires_at, consumed, key_version, token_nonce, issued_credential_version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),&[repo::text(&id),user_id.into(),repo::text(purpose),repo::text(&secret_hash(secret.expose())),repo::text(target_email),repo::text(target_key),DbValue::Time(now),DbValue::Time(expires),DbValue::Bool(false),repo::text(&issuer.version),repo::text(nonce.expose()),account.credential_version.into()]).await?;
    queue_mail(tx, r, user_id, &id, target_email, target_key, purpose, now).await?;
    Ok(id)
}
pub(crate) async fn queue_notice(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    account: &Account,
    kind: &str,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    queue_mail(
        tx,
        r,
        account.id,
        "",
        &account.email,
        &account.email_key,
        kind,
        now,
    )
    .await?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
async fn queue_mail(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    token_id: &str,
    email: &str,
    email_key: &str,
    kind: &str,
    now: DateTime<Utc>,
) -> Result<String, TransactionError> {
    if !matches!(
        kind,
        "verify_email"
            | "reset_password"
            | "change_email"
            | "password_changed"
            | "email_changed"
            | "login_security"
    ) {
        return Err(TransactionError::InvalidInput);
    }
    let message_id = new_operation_id();
    let proposed_ref = new_operation_id();
    let recipients = repo::table(tx, r, "mail_recipients")?;
    let address_hash = secret_hash(email_key);
    tx.insert_if_absent(
        &format!("INSERT INTO {recipients} (address_hash, recipient_ref) VALUES (?, ?)"),
        &[repo::text(&address_hash), repo::text(&proposed_ref)],
    )
    .await?;
    let recipient_ref = tx
        .fetch_optional(
            &format!("SELECT recipient_ref FROM {recipients} WHERE address_hash = ?"),
            &[repo::text(&address_hash)],
        )
        .await?
        .ok_or(TransactionError::CorruptData)?
        .text("recipient_ref")?;
    let t = repo::table(tx, r, "mail_requests")?;
    tx.execute(&format!("INSERT INTO {t} (message_id, user_id, token_id, recipient_email, recipient_ref, kind, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)"),&[repo::text(&message_id),user_id.into(),repo::text(token_id),repo::text(email),repo::text(&recipient_ref),repo::text(kind),DbValue::Time(now)]).await?;
    let event = platform::outbox::OutboxEvent {
        event_id: new_operation_id(),
        aggregate_type: "identity".into(),
        aggregate_id: user_id.to_string(),
        event_type: "mail.requested".into(),
        version: 1,
        occurred_at: now,
        payload: platform::outbox::SafePayload::empty()
            .with_reference("resource_id", &message_id)
            .map_err(|_| TransactionError::InvalidInput)?,
        trace_id: String::new(),
    }
    .with_inherited_trace()
    .map_err(|_| TransactionError::InvalidInput)?;
    platform_db::outbox_repo::enqueue(tx, r, &event, &["mail.delivery"])
        .await
        .map_err(|_| TransactionError::CorruptData)?;
    Ok(message_id)
}
pub(crate) fn bound_request_hash(secret: &str, canonical: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(canonical);
    mac.finalize().into_bytes().to_vec()
}
pub(crate) async fn enqueue_analytics(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    kind: platform::analytics::EventKind,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    enqueue_analytics_identified(tx, r, user_id, kind, new_operation_id(), now).await
}
async fn enqueue_analytics_identified(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    kind: platform::analytics::EventKind,
    event_id: String,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    use platform::analytics::*;
    let t = repo::table(tx, r, "users")?;
    let row = tx
        .fetch_optional(
            &format!("SELECT analytics_pseudonym, analytics_excluded FROM {t} WHERE id = ?"),
            &[user_id.into()],
        )
        .await?
        .ok_or(TransactionError::CorruptData)?;
    let fact = AnalyticsFact::from_authority(
        event_id,
        kind,
        now,
        row.text("analytics_pseudonym")?,
        None,
        Dimensions {
            forum_id: None,
            tag_ids: Default::default(),
            channel_id: None,
            device: Device::Unknown,
        },
        TrustedExclusions::from_authority(false, row.boolean("analytics_excluded")?, false, false),
    )
    .map_err(|_| TransactionError::CorruptData)?;
    platform_db::analytics_repo::enqueue(tx, r, &fact, now, new_operation_id())
        .await
        .map_err(|_| TransactionError::CorruptData)?;
    Ok(())
}
impl IdentityService {
    pub async fn current_policies(&self) -> Result<platform::policy::PolicySet, IdentityError> {
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let result = self
            .policy_reader
            .current(&mut tx.connection(), &self.renderer)
            .await
            .map_err(|_| IdentityError::PolicyPending)?;
        tx.rollback().await?;
        Ok(result)
    }
    pub async fn policy_documents(
        &self,
    ) -> Result<Vec<platform::policy::PolicyVersion>, IdentityError> {
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let result = self
            .policy_reader
            .documents(&mut tx.connection(), &self.renderer)
            .await
            .map_err(|_| IdentityError::PolicyPending)?;
        tx.rollback().await?;
        Ok(result)
    }
    pub async fn register(
        &self,
        context: &RequestContext,
        input: RegistrationInput,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        let now = normalized_now(now);
        let username = self.policy.username(&input.username)?;
        let email = self.policy.email(&input.email)?;
        let issuer = self
            .token_issuer
            .clone()
            .ok_or(IdentityError::PolicyPending)?;
        let get = |name: &str| {
            input
                .policies
                .iter()
                .find(|(kind, _)| kind == name)
                .and_then(|(_, id)| id.parse::<i64>().ok())
                .ok_or(IdentityError::Rejected)
        };
        if input.policies.len() != 3 {
            return Err(IdentityError::Rejected);
        }
        let policies = platform::policy::PolicySet {
            terms: get("terms")?,
            privacy: get("privacy")?,
            community: get("community")?,
        };
        policies.validate().map_err(|_| IdentityError::Rejected)?;
        let canonical = serde_json::to_vec(&(
            username.key.clone(),
            username.display.clone(),
            email.key.clone(),
            email.display.clone(),
            input.password.expose(),
            input.invitation.expose(),
            &input.policies,
        ))
        .map_err(|_| IdentityError::Rejected)?;
        let challenge = self
            .prepare_challenge(
                context,
                crate::challenge::ChallengeAction::Register,
                &canonical,
                now,
            )
            .await?;
        let command = self
            .public_command(context, "identity.public.register", &canonical, now)
            .await?;
        if crate::challenge::hard_blocked(context, challenge.as_ref()) {
            self.passwords.verify("registration-fixed-cost", None);
            return Ok(MutationResult {
                resource_ref: None,
                replayed: false,
            });
        }
        let password_hash = self.passwords.hash(input.password.expose())?;
        let invite_hash = secret_hash(input.invitation.expose());
        let mut read = DbTransaction::begin_read(&self.pool).await?;
        let invite = repo::invite_by_hash(&mut read.connection(), &self.renderer, &invite_hash)
            .await?
            .ok_or(IdentityError::Rejected)?;
        read.rollback().await?;
        let lock_plan = LockPlan::new(vec![
            LockTarget::new("identity", "users", invite.creator_id).expect("registered"),
            LockTarget::new("identity", "invitations", invite.id).expect("registered"),
        ]);
        let service = self.clone();
        let context = context.clone();
        let public_id = new_operation_id();
        let pseudonym = new_operation_id();
        registration_outcome(execute_command(&self.pool,&self.renderer,&command,&lock_plan,now,move|tx|{let service=service.clone();let context=context.clone();let username=username.clone();let email=email.clone();let password_hash=password_hash.clone();let invite_hash=invite_hash.clone();let public_id=public_id.clone();let pseudonym=pseudonym.clone();let issuer=issuer.clone();let challenge=challenge.clone();Box::pin(async move{
            if context.hard_rate_limited { return Err(TransactionError::InvalidInput); }
            repo::principal(tx,&service.renderer,&context,now).await?;assert_flags(tx,&service.renderer,&[Flag::Registration,Flag::EmailVerification]).await?;repo::assert_unlocked(tx,&service.renderer,Flag::RegistrationLock).await?;repo::assert_unlocked(tx,&service.renderer,Flag::InvitationLock).await?;
            service.consume_challenge(tx,challenge.as_ref(),now).await?;
            service.policy_reader.require_current(tx,&service.renderer,&policies).await.map_err(|_|TransactionError::InvalidInput)?;
            let invitation=repo::invite_by_hash(tx,&service.renderer,&invite_hash).await?.ok_or(TransactionError::InvalidInput)?;
            if invitation.state!="available"||invitation.expires_at<=now{return Err(TransactionError::InvalidInput)}
            let risk=record_risk(tx,&service,None,trust::risk::RiskEventKind::Registration,context.risk_signals.clone(),now).await?;
            if !risk.decision().allowed(){return Ok(risk_response(&risk))}
            // Existing addresses receive the same public acceptance response. No invite
            // is consumed and no account or mail is created for that address.
            if repo::account_by_identifier(tx,&service.renderer,&email.key,true).await?.is_some(){return Ok(response(None))}
            let user_id=repo::create_account(tx,&service.renderer,&public_id,&pseudonym,&username,&email,&password_hash,service.registration_analytics_excluded,now).await?;
            repo::consume_invitation(tx,&service.renderer,invitation.id,user_id,now).await?;
            let consents=repo::table(tx,&service.renderer,"policy_consents")?;for (kind,id) in policies.references(){tx.execute(&format!("INSERT INTO {consents} (user_id, kind, version_ref, accepted_at) VALUES (?, ?, ?, ?)"),&[user_id.into(),repo::text(kind),repo::text(&id.to_string()),DbValue::Time(now)]).await?;}
            trust::core::register_initial(tx,&service.renderer,user_id,invitation.id,invitation.creator_id,now).await.map_err(|_|TransactionError::InvalidInput)?;
            queue_token(tx,&service.renderer,user_id,"verify_email",&email.display,&email.key,now,now+Duration::hours(24),&issuer).await?;
            enqueue_analytics_identified(tx,&service.renderer,user_id,platform::analytics::EventKind::Registered,risk.event_id().to_owned(),now).await?;
            enqueue_analytics(tx,&service.renderer,user_id,platform::analytics::EventKind::InvitationUsed,now).await?;
            Ok(response(None))
        })}).await)
    }
    pub async fn login(
        &self,
        context: &RequestContext,
        input: LoginInput,
        now: DateTime<Utc>,
    ) -> Result<LoginResult, IdentityError> {
        let now = normalized_now(now);
        let is_email = input.identifier.contains('@');
        let normalized = if is_email {
            self.policy.email(&input.identifier)
        } else {
            self.policy.username(&input.identifier)
        };
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let account = if let Ok(ref key) = normalized {
            repo::account_by_identifier(&mut tx.connection(), &self.renderer, &key.key, is_email)
                .await?
        } else {
            None
        };
        let hash = match &account {
            Some(a) => repo::password_hash(&mut tx.connection(), &self.renderer, a.id).await?,
            None => None,
        };
        tx.rollback().await?;
        let password_ok = self.passwords.verify(
            input.password.expose(),
            if context.hard_rate_limited {
                None
            } else {
                hash.as_deref()
            },
        );
        let account = account.filter(|a| a.status == "active" && password_ok);
        if input.device_summary.len() > 96 || input.device_summary.chars().any(char::is_control) {
            return Err(IdentityError::Rejected);
        }
        // Only an allowlisted coarse label is retained; raw user agents and IPs are never stored.
        let device = match input.device_summary.as_str() {
            "desktop" => "desktop",
            "mobile" => "mobile",
            "tablet" => "tablet",
            _ => "unknown",
        }
        .to_owned();
        let canonical = serde_json::to_vec(&(input.identifier, input.password.expose(), &device))
            .map_err(|_| IdentityError::Rejected)?;
        let challenge = self
            .prepare_challenge(
                context,
                crate::challenge::ChallengeAction::Login,
                &canonical,
                now,
            )
            .await?;
        let command = self
            .public_command(context, "identity.public.login", &canonical, now)
            .await?;
        if crate::challenge::hard_blocked(context, challenge.as_ref()) {
            return Err(IdentityError::Rejected);
        }
        let token = random_secret();
        let csrf = random_secret();
        let session_ref = new_operation_id();
        let service = self.clone();
        let context = context.clone();
        let token_hash = secret_hash(token.expose());
        let csrf_hash = secret_hash(csrf.expose());
        let grant_ref = session_ref.clone();
        let lock_plan = account.as_ref().map(|a| locks(a.id)).unwrap_or_default();
        let outcome = execute_command(
            &self.pool,
            &self.renderer,
            &command,
            &lock_plan,
            now,
            move |tx| {
                let service = service.clone();
                let context = context.clone();
                let account = account.clone();
                let token_hash = token_hash.clone();
                let csrf_hash = csrf_hash.clone();
                let session_ref = session_ref.clone();
                let device = device.clone();
                let challenge = challenge.clone();
                Box::pin(async move {
                    if context.hard_rate_limited {
                        return Err(TransactionError::InvalidInput);
                    }
                    repo::principal(tx, &service.renderer, &context, now).await?;
                    assert_flags(tx, &service.renderer, &[Flag::PublicRead]).await?;
                    service
                        .consume_challenge(tx, challenge.as_ref(), now)
                        .await?;
                    let risk = record_risk(
                        tx,
                        &service,
                        account.as_ref().map(|a| a.id),
                        trust::risk::RiskEventKind::Login,
                        context.risk_signals.clone(),
                        now,
                    )
                    .await?;
                    if !risk.decision().allowed() {
                        return Ok(risk_response(&risk));
                    }
                    let Some(account) = account else {
                        return Ok(rejected_response());
                    };
                    let current = repo::account_by_id(tx, &service.renderer, account.id)
                        .await?
                        .ok_or(TransactionError::InvalidInput)?;
                    if current.status != "active"
                        || current.credential_version != account.credential_version
                    {
                        return Ok(rejected_response());
                    }
                    repo::create_session(
                        tx,
                        &service.renderer,
                        &current,
                        &session_ref,
                        &token_hash,
                        &csrf_hash,
                        &device,
                        now,
                    )
                    .await?;
                    Ok(response(None))
                })
            },
        )
        .await?;
        let result = mutation(outcome)?;
        Ok(LoginResult {
            grant: if result.replayed {
                None
            } else {
                Some(SessionGrant {
                    token,
                    csrf,
                    session_ref: grant_ref,
                })
            },
            replayed: result.replayed,
        })
    }
    /// `03` §6 的独立管理登录。
    ///
    /// 与普通登录**共用同一条密码校验路径**：同一个 `PasswordEngine::verify`、
    /// 同一套四维限流与风险事件、同样的固定成本比较（账号不存在时也传 `None`
    /// 走一遍完整工作量，避免用响应时间区分账号是否存在）。区别只有三处：
    ///   1. 额外要求目标账号是 active 的 staff principal；
    ///   2. 建立的是管理会话（8 小时绝对 / 15 分钟空闲），不是普通用户会话；
    ///   3. 返回值带上是否已完成 TOTP 绑定，供调用方决定跳绑定流程还是首页。
    ///
    /// 不是 staff、账号不存在、密码错误三种情况返回**同一个** `Unauthenticated`：
    /// 区分它们等于把 `/manage/login` 变成一个「谁是管理员」的探测接口。
    pub async fn management_login(
        &self,
        context: &RequestContext,
        input: LoginInput,
        now: DateTime<Utc>,
    ) -> Result<ManagementLoginResult, IdentityError> {
        let now = normalized_now(now);
        let is_email = input.identifier.contains('@');
        let normalized = if is_email {
            self.policy.email(&input.identifier)
        } else {
            self.policy.username(&input.identifier)
        };
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let account = if let Ok(ref key) = normalized {
            repo::account_by_identifier(&mut tx.connection(), &self.renderer, &key.key, is_email)
                .await?
        } else {
            None
        };
        let hash = match &account {
            Some(a) => repo::password_hash(&mut tx.connection(), &self.renderer, a.id).await?,
            None => None,
        };
        tx.rollback().await?;
        // 与普通登录逐字相同的固定成本比较：硬限流时传 None，同样跑满一次校验工作量。
        let password_ok = self.passwords.verify(
            input.password.expose(),
            if context.hard_rate_limited {
                None
            } else {
                hash.as_deref()
            },
        );
        let account = account.filter(|a| a.status == "active" && password_ok);
        let canonical = serde_json::to_vec(&(&input.identifier, input.password.expose(), "manage"))
            .map_err(|_| IdentityError::Rejected)?;
        let challenge = self
            .prepare_challenge(
                context,
                crate::challenge::ChallengeAction::Login,
                &canonical,
                now,
            )
            .await?;
        let command = self
            .public_command(context, "identity.manage.login", &canonical, now)
            .await?;
        if crate::challenge::hard_blocked(context, challenge.as_ref()) {
            return Err(IdentityError::Rejected);
        }
        let token = random_secret();
        let csrf = random_secret();
        let service = self.clone();
        let context = context.clone();
        let raw_token = token.expose().to_owned();
        let raw_csrf = csrf.expose().to_owned();
        let lock_plan = account.as_ref().map(|a| locks(a.id)).unwrap_or_default();
        let outcome = execute_command(
            &self.pool,
            &self.renderer,
            &command,
            &lock_plan,
            now,
            move |tx| {
                let service = service.clone();
                let context = context.clone();
                let account = account.clone();
                let challenge = challenge.clone();
                let raw_token = raw_token.clone();
                let raw_csrf = raw_csrf.clone();
                Box::pin(async move {
                    if context.hard_rate_limited {
                        return Err(TransactionError::InvalidInput);
                    }
                    repo::principal(tx, &service.renderer, &context, now).await?;
                    assert_flags(tx, &service.renderer, &[Flag::PublicRead]).await?;
                    service
                        .consume_challenge(tx, challenge.as_ref(), now)
                        .await?;
                    let risk = record_risk(
                        tx,
                        &service,
                        account.as_ref().map(|a| a.id),
                        trust::risk::RiskEventKind::Login,
                        context.risk_signals.clone(),
                        now,
                    )
                    .await?;
                    if !risk.decision().allowed() {
                        return Ok(risk_response(&risk));
                    }
                    let Some(account) = account else {
                        return Ok(rejected_response());
                    };
                    let current = repo::account_by_id(tx, &service.renderer, account.id)
                        .await?
                        .ok_or(TransactionError::InvalidInput)?;
                    if current.status != "active"
                        || current.credential_version != account.credential_version
                    {
                        return Ok(rejected_response());
                    }
                    // 管理会话的额外前提：目标必须是 active 的 staff principal。
                    // `open_management_session` 内部再查一次，这里不预判也不缓存结果。
                    match crate::management::open_management_session(
                        tx,
                        &service.renderer,
                        current.id,
                        &raw_token,
                        &raw_csrf,
                        now,
                    )
                    .await
                    {
                        // 幂等结果体只接受**不透明引用**（正整数或 UUIDv7，见
                        // `platform::outbox::is_reference`）。此前这里直接塞
                        // `"ready"`／`"binding"` 两个词，`with_reference` 因此返回
                        // `Err`，紧随其后的 `.expect` 把它变成一次 panic——而
                        // `/manage/login` 是**未认证可达**的端点，等于一个拒绝服务面。
                        // 改为用受信整数编码访问级别：1 = 已绑定可直接进入，
                        // 2 = 需先完成 TOTP 绑定。
                        Ok(access) => Ok(response(Some(match access {
                            crate::management::ManagementAccess::Ready => "1",
                            crate::management::ManagementAccess::BindingRequired => "2",
                        }))),
                        // 不是 staff：与密码错误返回同一种结果，不泄露谁是管理员。
                        Err(_) => Ok(rejected_response()),
                    }
                })
            },
        )
        .await?;
        let result = mutation(outcome)?;
        // 与上面的编码一一对应。未知取值一律按未认证处理，不猜一个默认值——
        // 猜「已绑定」会让未完成 MFA 的会话直接进入后台。
        let bound = match result.resource_ref.as_deref() {
            Some("1") => true,
            Some("2") => false,
            _ => return Err(IdentityError::Unauthenticated),
        };
        Ok(ManagementLoginResult { token, csrf, bound })
    }

    pub async fn request_reset(
        &self,
        context: &RequestContext,
        identifier: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        let now = normalized_now(now);
        let issuer = self
            .token_issuer
            .clone()
            .ok_or(IdentityError::PolicyPending)?;
        let is_email = identifier.contains('@');
        let normalized = if is_email {
            self.policy.email(identifier)
        } else {
            self.policy.username(identifier)
        };
        // Recovery takes the same password work unit for all account outcomes.
        self.passwords.verify("recovery-fixed-cost", None);
        let challenge = self
            .prepare_challenge(
                context,
                crate::challenge::ChallengeAction::Recovery,
                identifier.as_bytes(),
                now,
            )
            .await?;
        let command = self
            .public_command(
                context,
                "identity.public.recovery",
                identifier.as_bytes(),
                now,
            )
            .await?;
        if crate::challenge::hard_blocked(context, challenge.as_ref()) {
            return Ok(MutationResult {
                resource_ref: None,
                replayed: false,
            });
        }
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let account = if let Ok(ref key) = normalized {
            repo::account_by_identifier(&mut tx.connection(), &self.renderer, &key.key, is_email)
                .await?
        } else {
            None
        };
        tx.rollback().await?;
        let lock_plan = account.as_ref().map(|a| locks(a.id)).unwrap_or_default();
        let service = self.clone();
        let context = context.clone();
        mutation(
            execute_command(
                &self.pool,
                &self.renderer,
                &command,
                &lock_plan,
                now,
                move |tx| {
                    let service = service.clone();
                    let context = context.clone();
                    let account = account.clone();
                    let issuer = issuer.clone();
                    let challenge = challenge.clone();
                    Box::pin(async move {
                        if context.hard_rate_limited {
                            return Err(TransactionError::InvalidInput);
                        }
                        repo::principal(tx, &service.renderer, &context, now).await?;
                        assert_flags(tx, &service.renderer, &[Flag::PublicRead]).await?;
                        service
                            .consume_challenge(tx, challenge.as_ref(), now)
                            .await?;
                        if let Some(account) = account {
                            let current = repo::account_by_id(tx, &service.renderer, account.id)
                                .await?
                                .ok_or(TransactionError::InvalidInput)?;
                            if current.status == "active" {
                                queue_token(
                                    tx,
                                    &service.renderer,
                                    current.id,
                                    "reset_password",
                                    &current.email,
                                    &current.email_key,
                                    now,
                                    now + Duration::hours(1),
                                    &issuer,
                                )
                                .await?;
                            }
                        }
                        Ok(response(None))
                    })
                },
            )
            .await?,
        )
    }
    pub async fn consume_verification(
        &self,
        context: &RequestContext,
        raw: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        self.consume_public_token(context, raw, None, None, now)
            .await
    }
    pub async fn reset_password(
        &self,
        context: &RequestContext,
        raw: &str,
        password: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        let hash = self.passwords.hash(password)?;
        self.consume_public_token(context, raw, Some(hash), Some(password), now)
            .await
    }
    async fn consume_public_token(
        &self,
        context: &RequestContext,
        raw: &str,
        new_hash: Option<String>,
        password: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        let now = normalized_now(now);
        let purpose = if new_hash.is_some() {
            "reset_password"
        } else {
            "verify_email"
        };
        let hash = secret_hash(raw);
        let mut read = DbTransaction::begin_read(&self.pool).await?;
        let token = repo::token_owner(&mut read.connection(), &self.renderer, &hash, purpose)
            .await?
            .ok_or(IdentityError::Rejected)?;
        read.rollback().await?;
        let canonical =
            serde_json::to_vec(&(raw, password)).map_err(|_| IdentityError::Rejected)?;
        let command = self
            .public_command(
                context,
                if new_hash.is_some() {
                    "identity.public.reset"
                } else {
                    "identity.public.verify"
                },
                &canonical,
                now,
            )
            .await?;
        let service = self.clone();
        let context = context.clone();
        mutation(
            execute_command(
                &self.pool,
                &self.renderer,
                &command,
                &locks(token.user_id),
                now,
                move |tx| {
                    let service = service.clone();
                    let context = context.clone();
                    let hash = hash.clone();
                    let new_hash = new_hash.clone();
                    Box::pin(async move {
                        repo::principal(tx, &service.renderer, &context, now).await?;
                        assert_flags(
                            tx,
                            &service.renderer,
                            &[if new_hash.is_some() {
                                Flag::PublicRead
                            } else {
                                Flag::EmailVerification
                            }],
                        )
                        .await?;
                        let token = repo::token_by_hash(tx, &service.renderer, &hash, purpose, now)
                            .await?
                            .ok_or(TransactionError::InvalidInput)?;
                        let account = repo::account_by_id(tx, &service.renderer, token.user_id)
                            .await?
                            .ok_or(TransactionError::InvalidInput)?;
                        if account.status != "active"
                            || account.credential_version != token.issued_credential_version
                            || account.email_key != token.target_email_key
                        {
                            return Err(TransactionError::InvalidInput);
                        }
                        repo::consume_token(tx, &service.renderer, &token.token_id, now).await?;
                        if let Some(new_hash) = new_hash {
                            repo::update_password(
                                tx,
                                &service.renderer,
                                account.id,
                                &new_hash,
                                now,
                            )
                            .await?;
                            repo::revoke_sessions(tx, &service.renderer, account.id, None).await?;
                            repo::invalidate_tokens(tx, &service.renderer, account.id).await?;
                            queue_notice(tx, &service.renderer, &account, "password_changed", now)
                                .await?;
                        } else if !account.email_verified {
                            repo::mark_verified(tx, &service.renderer, account.id).await?;
                            trust::core::mark_email_verified(
                                tx,
                                &service.renderer,
                                account.id,
                                now,
                            )
                            .await
                            .map_err(|_| TransactionError::InvalidInput)?;
                            enqueue_analytics(
                                tx,
                                &service.renderer,
                                account.id,
                                platform::analytics::EventKind::EmailVerified,
                                now,
                            )
                            .await?;
                        }
                        Ok(response(None))
                    })
                },
            )
            .await?,
        )
    }
}
impl IdentityService {
    pub async fn invitations(
        &self,
        session: &SessionIdentity,
        now: DateTime<Utc>,
    ) -> Result<InvitationPage, IdentityError> {
        let now = normalized_now(now);
        let window = self.policy.invitation_window(now)?;
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        repo::require_session(&mut tx.connection(), &self.renderer, session, now, false).await?;
        let result = repo::invitation_page(
            &mut tx.connection(),
            &self.renderer,
            session.user_id,
            &window,
            now,
        )
        .await?;
        tx.rollback().await?;
        Ok(result)
    }
    pub async fn generate_invitation(
        &self,
        session: &SessionIdentity,
        csrf: &str,
        key: &str,
        now: DateTime<Utc>,
    ) -> Result<InvitationGrant, IdentityError> {
        self.generate_invitation_with_signals(
            session,
            csrf,
            key,
            None,
            trust::risk::RiskSignals::default(),
            now,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn generate_invitation_with_signals(
        &self,
        session: &SessionIdentity,
        csrf: &str,
        key: &str,
        days: Option<u8>,
        signals: trust::risk::RiskSignals,
        now: DateTime<Utc>,
    ) -> Result<InvitationGrant, IdentityError> {
        check_csrf(session, csrf)?;
        let now = normalized_now(now);
        let days = days.unwrap_or(crate::policy::DEFAULT_INVITATION_DAYS);
        if !(1..=crate::policy::MAX_INVITATION_DAYS).contains(&days) {
            return Err(IdentityError::Rejected);
        }
        let mut window = self.policy.invitation_window(now)?;
        window.expires_at = now
            .checked_add_signed(Duration::days(i64::from(days)))
            .ok_or(IdentityError::Rejected)?;
        if window.start > now
            || window.end <= now
            || window.expires_at <= now
            || window.daily_limit <= 0
            || window.unused_limit <= 0
        {
            return Err(IdentityError::Rejected);
        }
        let command = identity_command(
            session.user_id,
            "identity.invitation.generate",
            key,
            &[days],
        );
        let service = self.clone();
        let session = session.clone();
        let code = random_secret();
        let code_hash = secret_hash(code.expose());
        let reference = new_operation_id();
        let grant_ref = reference.clone();
        let result = mutation(
            execute_command(
                &self.pool,
                &self.renderer,
                &command,
                &locks(session.user_id),
                now,
                move |tx| {
                    let service = service.clone();
                    let session = session.clone();
                    let code_hash = code_hash.clone();
                    let reference = reference.clone();
                    let window = window.clone();
                    let signals = signals.clone();
                    Box::pin(async move {
                        assert_flags(tx, &service.renderer, &[Flag::PublicRead]).await?;
                        repo::assert_unlocked(tx, &service.renderer, Flag::InvitationLock).await?;
                        let account =
                            repo::require_session(tx, &service.renderer, &session, now, false)
                                .await?;
                        if !account.email_verified {
                            return Err(TransactionError::InvalidInput);
                        }
                        if !matches!(
                            trust::core::authorize_invitation(
                                tx,
                                &service.renderer,
                                session.user_id
                            )
                            .await
                            .map_err(|_| TransactionError::InvalidInput)?,
                            trust::contract::InvitationAuthorization::Allowed { .. }
                        ) {
                            return Err(TransactionError::InvalidInput);
                        }
                        let page = repo::invitation_page(
                            tx,
                            &service.renderer,
                            session.user_id,
                            &window,
                            now,
                        )
                        .await?;
                        if page.generated_today >= window.daily_limit
                            || page.unused >= window.unused_limit
                        {
                            return Err(TransactionError::InvalidInput);
                        }
                        let risk = record_risk(
                            tx,
                            &service,
                            Some(session.user_id),
                            trust::risk::RiskEventKind::Invitation,
                            signals,
                            now,
                        )
                        .await?;
                        if !risk.decision().allowed() {
                            return Ok(risk_response(&risk));
                        }
                        repo::create_invitation(
                            tx,
                            &service.renderer,
                            session.user_id,
                            &reference,
                            &code_hash,
                            now,
                            window.expires_at,
                        )
                        .await?;
                        Ok(response(Some(&reference)))
                    })
                },
            )
            .await?,
        )?;
        if result.replayed {
            return Err(IdentityError::Rejected);
        }
        Ok(InvitationGrant {
            code,
            invitation_ref: grant_ref,
        })
    }
    pub async fn revoke_invitation(
        &self,
        session: &SessionIdentity,
        csrf: &str,
        key: &str,
        reference: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        check_csrf(session, csrf)?;
        let now = normalized_now(now);
        let command = identity_command(
            session.user_id,
            "identity.invitation.revoke",
            key,
            reference.as_bytes(),
        );
        let service = self.clone();
        let session = session.clone();
        let reference = reference.to_owned();
        mutation(execute_command(&self.pool,&self.renderer,&command,&locks(session.user_id),now,move|tx|{let service=service.clone();let session=session.clone();let reference=reference.clone();Box::pin(async move{
            assert_flags(tx,&service.renderer,&[Flag::PublicRead]).await?;repo::require_session(tx,&service.renderer,&session,now,false).await?;let t=repo::table(tx,&service.renderer,"invitations")?;
            if tx.execute(&format!("UPDATE {t} SET state = ? WHERE invite_ref = ? AND creator_id = ? AND state = ?"),&[repo::text("revoked"),repo::text(&reference),session.user_id.into(),repo::text("available")]).await?!=1{return Err(TransactionError::InvalidInput)}Ok(response(None))
        })}).await?)
    }
}

fn rejected_response() -> StoredResponse {
    StoredResponse {
        status_code: 400,
        body: platform::outbox::SafePayload::empty()
            .with_outcome(platform::outbox::Outcome::Failure)
            .to_json()
            .expect("safe payload"),
    }
}
async fn record_risk(
    tx: &mut TransactionConnection<'_>,
    service: &IdentityService,
    actor: Option<i64>,
    kind: trust::risk::RiskEventKind,
    signals: trust::risk::RiskSignals,
    now: DateTime<Utc>,
) -> Result<trust::risk::RiskEvent, TransactionError> {
    let event = trust::core::record_identity_event(
        tx,
        &service.renderer,
        &new_operation_id(),
        actor,
        kind,
        signals,
        service.risk_policy.as_ref(),
        now,
    )
    .await
    .map_err(|_| TransactionError::InvalidInput)?;
    Ok(event)
}
impl IdentityService {
    pub async fn public_operation(
        &self,
        context: &RequestContext,
        scope: &str,
        key: &str,
        operation_id: &str,
        now: DateTime<Utc>,
    ) -> Result<platform::transaction::OperationStatus, IdentityError> {
        if !matches!(
            scope,
            "identity.public.register"
                | "identity.public.login"
                | "identity.public.recovery"
                | "identity.public.resend_verification"
                | "identity.public.verify"
                | "identity.public.reset"
                | "identity.public.email_confirm"
        ) {
            return Err(IdentityError::Rejected);
        }
        let now = normalized_now(now);
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let actor_id = repo::principal(&mut tx.connection(), &self.renderer, context, now).await?;
        let query = platform::transaction::OperationQuery {
            actor_id,
            scope: scope.into(),
            idempotency_key: key.into(),
            operation_id: operation_id.into(),
        };
        let status = platform_db::idempotency_repo::query_result(
            &mut tx.connection(),
            &self.renderer,
            &query,
            now,
        )
        .await?;
        tx.rollback().await?;
        Ok(status)
    }
    pub async fn session_operation(
        &self,
        session: &SessionIdentity,
        csrf: &str,
        scope: &str,
        key: &str,
        operation_id: &str,
        now: DateTime<Utc>,
    ) -> Result<platform::transaction::OperationStatus, IdentityError> {
        if !matches!(
            scope,
            "identity.logout"
                | "identity.invitation.generate"
                | "identity.invitation.revoke"
                | "identity.profile.bio"
                | "identity.profile.username"
                | "identity.profile.email_request"
                | "identity.profile.password"
                | "identity.sessions.revoke"
                | "identity.sessions.revoke_others"
        ) {
            return Err(IdentityError::Rejected);
        }
        check_csrf(session, csrf)?;
        let now = normalized_now(now);
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        repo::require_session(&mut tx.connection(), &self.renderer, session, now, false).await?;
        let query = platform::transaction::OperationQuery {
            actor_id: session.user_id,
            scope: scope.into(),
            idempotency_key: key.into(),
            operation_id: operation_id.into(),
        };
        let status = platform_db::idempotency_repo::query_result(
            &mut tx.connection(),
            &self.renderer,
            &query,
            now,
        )
        .await?;
        tx.rollback().await?;
        Ok(status)
    }
}

fn registration_outcome(
    outcome: Result<CommitOutcome<platform_db::command::CommandResult>, TransactionError>,
) -> Result<MutationResult, IdentityError> {
    match outcome {
        // The whole command rolled back. Concurrent address/name unique races use
        // the same public acceptance envelope; no account or invite side effect survives.
        Err(TransactionError::Database(platform::transaction::DbErrorClass::UniqueConflict)) => {
            Ok(MutationResult {
                resource_ref: None,
                replayed: false,
            })
        }
        Ok(value) => mutation(value),
        Err(error) => Err(error.into()),
    }
}
impl IdentityService {
    /// A revoked bearer can prove ownership of an already submitted operation only.
    /// This path returns no authenticated identity and cannot authorize a business command.
    pub async fn session_operation_by_token(
        &self,
        raw: &str,
        csrf: &str,
        scope: &str,
        key: &str,
        operation_id: &str,
        now: DateTime<Utc>,
    ) -> Result<platform::transaction::OperationStatus, IdentityError> {
        if !matches!(
            scope,
            "identity.logout"
                | "identity.invitation.generate"
                | "identity.invitation.revoke"
                | "identity.profile.bio"
                | "identity.profile.username"
                | "identity.profile.email_request"
                | "identity.profile.password"
                | "identity.sessions.revoke"
                | "identity.sessions.revoke_others"
        ) {
            return Err(IdentityError::Rejected);
        }
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let t = repo::table(&tx.connection(), &self.renderer, "sessions")?;
        let row = tx
            .connection()
            .fetch_optional(
                &format!(
                    "SELECT user_id, csrf_hash, absolute_expires_at FROM {t} WHERE token_hash = ?"
                ),
                &[repo::text(&secret_hash(raw))],
            )
            .await?
            .ok_or(IdentityError::Unauthenticated)?;
        if !constant_equal(&row.text("csrf_hash")?, &secret_hash(csrf))
            || row.time("absolute_expires_at")? <= now
        {
            return Err(IdentityError::Csrf);
        }
        let query = platform::transaction::OperationQuery {
            actor_id: row.i64("user_id")?,
            scope: scope.into(),
            idempotency_key: key.into(),
            operation_id: operation_id.into(),
        };
        let result = platform_db::idempotency_repo::query_result(
            &mut tx.connection(),
            &self.renderer,
            &query,
            normalized_now(now),
        )
        .await?;
        tx.rollback().await?;
        Ok(result)
    }
}

fn risk_response(event: &trust::risk::RiskEvent) -> StoredResponse {
    let mut response = rejected_response();
    if matches!(
        event.decision().action(),
        trust::risk::RiskAction::Unavailable | trust::risk::RiskAction::Challenge
    ) {
        response.status_code = 503;
    }
    response
}

#[path = "content_access.rs"]
mod content_access;
pub use content_access::*;
