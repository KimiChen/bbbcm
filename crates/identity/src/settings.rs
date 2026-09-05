//! 本人资料和普通账号设置；版本、owner和会话均在写事务内重读。
use crate::contract::*;
use crate::core::{
    IdentityService, assert_flags, bound_request_hash, check_csrf, enqueue_analytics,
    identity_command, locks, mutation, normalized_now, queue_notice, queue_token, response,
    secret_hash,
};
use crate::{repo, settings_repo};
use chrono::{DateTime, Duration, Utc};
use platform::flags::Flag;
use platform_db::{
    command::execute_command,
    transaction::{DbTransaction, TransactionError},
};

/// 公开资料仅包含本轮已接入的数据；邮箱、设备、历史用户名从未进入此投影。
#[derive(Clone)]
pub struct PublicProfile {
    pub public_ref: String,
    pub username: String,
    pub bio: String,
    pub joined_at: DateTime<Utc>,
    pub avatar_preset: &'static str,
    pub content_data_available: bool,
    pub trust_level: Option<&'static str>,
    pub role_data_available: bool,
}
#[derive(Clone)]
pub struct OwnProfile {
    pub public: PublicProfile,
    pub email: String,
    pub email_verified: bool,
    pub version: i64,
    pub username_change_after: DateTime<Utc>,
}
#[derive(Clone)]
pub struct DeviceSummary {
    pub session_ref: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub current: bool,
}
macro_rules! redacted_debug {
    ($($kind:ty),+) => {$(impl std::fmt::Debug for $kind {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(concat!(stringify!($kind), "([REDACTED])"))
        }
    })+};
}
redacted_debug!(PublicProfile, OwnProfile, DeviceSummary);

fn expect_version(account: &Account, expected: i64) -> Result<(), TransactionError> {
    if account.version != expected {
        return Err(TransactionError::VersionConflict);
    }
    Ok(())
}

impl IdentityService {
    pub async fn public_profile(
        &self,
        public_ref: &str,
        _now: DateTime<Utc>,
    ) -> Result<Option<PublicProfile>, IdentityError> {
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let result =
            match settings_repo::public_profile(&mut tx.connection(), &self.renderer, public_ref)
                .await?
            {
                Some((user_id, mut public)) => {
                    public.trust_level = trust::core::load_identity_snapshot(
                        &mut tx.connection(),
                        &self.renderer,
                        user_id,
                    )
                    .await
                    .map_err(|_| IdentityError::Unavailable)?
                    .map(|snapshot| snapshot.level().as_str());
                    Some(public)
                }
                None => None,
            };
        tx.rollback().await?;
        Ok(result)
    }

    pub async fn profile(
        &self,
        identity: &SessionIdentity,
        now: DateTime<Utc>,
    ) -> Result<OwnProfile, IdentityError> {
        let now = normalized_now(now);
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let account =
            repo::require_session(&mut tx.connection(), &self.renderer, identity, now, false)
                .await?;
        let public = PublicProfile {
            public_ref: account.public_id,
            username: account.username,
            bio: account.bio,
            joined_at: account.created_at,
            avatar_preset: "default",
            content_data_available: false,
            trust_level: trust::core::load_identity_snapshot(
                &mut tx.connection(),
                &self.renderer,
                account.id,
            )
            .await
            .map_err(|_| IdentityError::Unavailable)?
            .map(|snapshot| snapshot.level().as_str()),
            role_data_available: false,
        };
        let profile = OwnProfile {
            public,
            email: account.email,
            email_verified: account.email_verified,
            version: account.version,
            username_change_after: account.username_changed_at
                + Duration::days(USERNAME_COOLDOWN_DAYS),
        };
        tx.rollback().await?;
        Ok(profile)
    }

    pub async fn update_bio(
        &self,
        identity: &SessionIdentity,
        csrf: &str,
        key: &str,
        expected_version: i64,
        bio: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        check_csrf(identity, csrf)?;
        let bio = self.policy.bio(bio)?;
        let now = normalized_now(now);
        let canonical =
            serde_json::to_vec(&(expected_version, &bio)).map_err(|_| IdentityError::Rejected)?;
        let command = identity_command(identity.user_id, "identity.profile.bio", key, &canonical);
        let service = self.clone();
        let identity = identity.clone();
        mutation(
            execute_command(
                &self.pool,
                &self.renderer,
                &command,
                &locks(identity.user_id),
                now,
                move |tx| {
                    let service = service.clone();
                    let identity = identity.clone();
                    let bio = bio.clone();
                    Box::pin(async move {
                        assert_flags(tx, &service.renderer, &[Flag::PublicRead]).await?;
                        let account =
                            repo::require_session(tx, &service.renderer, &identity, now, false)
                                .await?;
                        expect_version(&account, expected_version)?;
                        settings_repo::update_bio(tx, &service.renderer, &account, &bio).await?;
                        Ok(response(Some(&account.public_id)))
                    })
                },
            )
            .await?,
        )
    }

    pub async fn change_username(
        &self,
        identity: &SessionIdentity,
        csrf: &str,
        key: &str,
        expected_version: i64,
        username: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        check_csrf(identity, csrf)?;
        // Normalization, legality and impersonation checks belong to the approved injected policy.
        let username = self.policy.username(username)?;
        let now = normalized_now(now);
        let canonical = serde_json::to_vec(&(expected_version, &username.display, &username.key))
            .map_err(|_| IdentityError::Rejected)?;
        let command = identity_command(
            identity.user_id,
            "identity.profile.username",
            key,
            &canonical,
        );
        let service = self.clone();
        let identity = identity.clone();
        mutation(
            execute_command(
                &self.pool,
                &self.renderer,
                &command,
                &locks(identity.user_id),
                now,
                move |tx| {
                    let service = service.clone();
                    let identity = identity.clone();
                    let username = username.clone();
                    Box::pin(async move {
                        assert_flags(tx, &service.renderer, &[Flag::PublicRead]).await?;
                        let account =
                            repo::require_session(tx, &service.renderer, &identity, now, true)
                                .await?;
                        expect_version(&account, expected_version)?;
                        if now
                            < account.username_changed_at + Duration::days(USERNAME_COOLDOWN_DAYS)
                        {
                            return Err(TransactionError::InvalidInput);
                        }
                        settings_repo::change_username(
                            tx,
                            &service.renderer,
                            &account,
                            &username,
                            now,
                        )
                        .await?;
                        Ok(response(Some(&account.public_id)))
                    })
                },
            )
            .await?,
        )
    }

    pub async fn request_email_change(
        &self,
        identity: &SessionIdentity,
        csrf: &str,
        key: &str,
        expected_version: i64,
        email: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        check_csrf(identity, csrf)?;
        let email = self.policy.email(email)?;
        let now = normalized_now(now);
        let expires = self.policy.email_change_expires_at(now)?;
        if expires <= now {
            return Err(IdentityError::Rejected);
        }
        let issuer = self
            .token_issuer
            .clone()
            .ok_or(IdentityError::PolicyPending)?;
        let canonical = serde_json::to_vec(&(expected_version, &email.display, &email.key))
            .map_err(|_| IdentityError::Rejected)?;
        let command = identity_command(
            identity.user_id,
            "identity.profile.email_request",
            key,
            &canonical,
        );
        let service = self.clone();
        let identity = identity.clone();
        mutation(
            execute_command(
                &self.pool,
                &self.renderer,
                &command,
                &locks(identity.user_id),
                now,
                move |tx| {
                    let service = service.clone();
                    let identity = identity.clone();
                    let email = email.clone();
                    let issuer = issuer.clone();
                    Box::pin(async move {
                        assert_flags(
                            tx,
                            &service.renderer,
                            &[Flag::PublicRead, Flag::EmailVerification],
                        )
                        .await?;
                        let account =
                            repo::require_session(tx, &service.renderer, &identity, now, true)
                                .await?;
                        expect_version(&account, expected_version)?;
                        if email.key == account.email_key {
                            return Err(TransactionError::InvalidInput);
                        }
                        settings_repo::touch_version(tx, &service.renderer, &account).await?;
                        settings_repo::invalidate_email_requests(tx, &service.renderer, account.id)
                            .await?;
                        // Existing addresses follow the same visible version change and response.
                        // The address is only reserved by the uniqueness check when confirmation commits.
                        if repo::account_by_identifier(tx, &service.renderer, &email.key, true)
                            .await?
                            .is_some()
                        {
                            return Ok(response(None));
                        }
                        queue_token(
                            tx,
                            &service.renderer,
                            account.id,
                            "change_email",
                            &email.display,
                            &email.key,
                            now,
                            expires,
                            &issuer,
                        )
                        .await?;
                        Ok(response(None))
                    })
                },
            )
            .await?,
        )
    }

    pub async fn change_password(
        &self,
        identity: &SessionIdentity,
        csrf: &str,
        key: &str,
        expected_version: i64,
        password: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        check_csrf(identity, csrf)?;
        let password_hash = self.passwords.hash(password)?;
        let now = normalized_now(now);
        // Only the request digest reaches idempotency storage; the password never does.
        let canonical = serde_json::to_vec(&(expected_version, password))
            .map_err(|_| IdentityError::Rejected)?;
        let command = identity_command(
            identity.user_id,
            "identity.profile.password",
            key,
            &bound_request_hash(csrf, &canonical),
        );
        let service = self.clone();
        let identity = identity.clone();
        mutation(
            execute_command(
                &self.pool,
                &self.renderer,
                &command,
                &locks(identity.user_id),
                now,
                move |tx| {
                    let service = service.clone();
                    let identity = identity.clone();
                    let password_hash = password_hash.clone();
                    Box::pin(async move {
                        assert_flags(tx, &service.renderer, &[Flag::PublicRead]).await?;
                        let account =
                            repo::require_session(tx, &service.renderer, &identity, now, true)
                                .await?;
                        expect_version(&account, expected_version)?;
                        settings_repo::change_password(
                            tx,
                            &service.renderer,
                            &account,
                            &password_hash,
                            now,
                        )
                        .await?;
                        repo::revoke_sessions(tx, &service.renderer, account.id, None).await?;
                        repo::invalidate_tokens(tx, &service.renderer, account.id).await?;
                        queue_notice(tx, &service.renderer, &account, "password_changed", now)
                            .await?;
                        Ok(response(None))
                    })
                },
            )
            .await?,
        )
    }

    pub async fn confirm_email_change(
        &self,
        context: &RequestContext,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        let now = normalized_now(now);
        let hash = secret_hash(token);
        let mut read = DbTransaction::begin_read(&self.pool).await?;
        let user_id =
            settings_repo::email_change_owner(&mut read.connection(), &self.renderer, &hash)
                .await?
                .ok_or(IdentityError::Rejected)?;
        read.rollback().await?;
        let command = self
            .public_command(
                context,
                "identity.public.email_confirm",
                token.as_bytes(),
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
                &locks(user_id),
                now,
                move |tx| {
                    let service = service.clone();
                    let context = context.clone();
                    let hash = hash.clone();
                    Box::pin(async move {
                        repo::principal(tx, &service.renderer, &context, now).await?;
                        assert_flags(
                            tx,
                            &service.renderer,
                            &[Flag::PublicRead, Flag::EmailVerification],
                        )
                        .await?;
                        let token =
                            repo::token_by_hash(tx, &service.renderer, &hash, "change_email", now)
                                .await?
                                .ok_or(TransactionError::InvalidInput)?;
                        if token.user_id != user_id {
                            return Err(TransactionError::InvalidInput);
                        }
                        let account = repo::account_by_id(tx, &service.renderer, user_id)
                            .await?
                            .ok_or(TransactionError::InvalidInput)?;
                        if account.status != "active"
                            || token.target_email_key == account.email_key
                            || token.issued_credential_version != account.credential_version
                            || token.expires_at <= now
                        {
                            return Err(TransactionError::InvalidInput);
                        }
                        repo::consume_token(tx, &service.renderer, &token.token_id, now).await?;
                        settings_repo::change_email(
                            tx,
                            &service.renderer,
                            &account,
                            &token.target_email,
                            &token.target_email_key,
                        )
                        .await?;
                        repo::revoke_sessions(tx, &service.renderer, user_id, None).await?;
                        repo::invalidate_tokens(tx, &service.renderer, user_id).await?;
                        // Finish trust locks before any analytics or mail append begins.
                        trust::core::mark_email_verified(tx, &service.renderer, user_id, now)
                            .await
                            .map_err(|_| TransactionError::InvalidInput)?;
                        if !account.email_verified {
                            enqueue_analytics(
                                tx,
                                &service.renderer,
                                user_id,
                                platform::analytics::EventKind::EmailVerified,
                                now,
                            )
                            .await?;
                        }
                        queue_notice(tx, &service.renderer, &account, "email_changed", now).await?;
                        let mut updated = account;
                        updated.email = token.target_email;
                        updated.email_key = token.target_email_key;
                        queue_notice(tx, &service.renderer, &updated, "email_changed", now).await?;
                        Ok(response(None))
                    })
                },
            )
            .await?,
        )
    }

    pub async fn sessions(
        &self,
        identity: &SessionIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<DeviceSummary>, IdentityError> {
        let now = normalized_now(now);
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        repo::require_session(&mut tx.connection(), &self.renderer, identity, now, false).await?;
        let devices = settings_repo::devices(
            &mut tx.connection(),
            &self.renderer,
            identity.user_id,
            identity.session_id,
            now,
        )
        .await?;
        tx.rollback().await?;
        Ok(devices)
    }

    pub async fn revoke_session(
        &self,
        identity: &SessionIdentity,
        csrf: &str,
        key: &str,
        session_ref: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        check_csrf(identity, csrf)?;
        let now = normalized_now(now);
        let canonical = serde_json::to_vec(&(identity.session_ref(), session_ref))
            .map_err(|_| IdentityError::Rejected)?;
        let command = identity_command(
            identity.user_id,
            "identity.sessions.revoke",
            key,
            &canonical,
        );
        let service = self.clone();
        let identity = identity.clone();
        let session_ref = session_ref.to_owned();
        mutation(
            execute_command(
                &self.pool,
                &self.renderer,
                &command,
                &locks(identity.user_id),
                now,
                move |tx| {
                    let service = service.clone();
                    let identity = identity.clone();
                    let session_ref = session_ref.clone();
                    Box::pin(async move {
                        assert_flags(tx, &service.renderer, &[Flag::PublicRead]).await?;
                        repo::require_session(tx, &service.renderer, &identity, now, true).await?;
                        repo::revoke_one(tx, &service.renderer, identity.user_id, &session_ref)
                            .await?;
                        Ok(response(None))
                    })
                },
            )
            .await?,
        )
    }

    pub async fn revoke_other_sessions(
        &self,
        identity: &SessionIdentity,
        csrf: &str,
        key: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        check_csrf(identity, csrf)?;
        let now = normalized_now(now);
        let command = identity_command(
            identity.user_id,
            "identity.sessions.revoke_others",
            key,
            identity.session_ref.as_bytes(),
        );
        let service = self.clone();
        let identity = identity.clone();
        mutation(
            execute_command(
                &self.pool,
                &self.renderer,
                &command,
                &locks(identity.user_id),
                now,
                move |tx| {
                    let service = service.clone();
                    let identity = identity.clone();
                    Box::pin(async move {
                        assert_flags(tx, &service.renderer, &[Flag::PublicRead]).await?;
                        repo::require_session(tx, &service.renderer, &identity, now, true).await?;
                        repo::revoke_sessions(
                            tx,
                            &service.renderer,
                            identity.user_id,
                            Some(identity.session_id),
                        )
                        .await?;
                        Ok(response(None))
                    })
                },
            )
            .await?,
        )
    }
}
