//! Public verification resend shares recovery's fixed-cost, uniform acceptance contract.
use crate::{
    challenge::ChallengeAction,
    contract::*,
    core::{IdentityService, assert_flags, locks, mutation, normalized_now, queue_token, response},
    repo,
};
use chrono::{DateTime, Duration, Utc};
use platform::flags::Flag;
use platform_db::{
    command::execute_command,
    transaction::{DbTransaction, TransactionError},
};

impl IdentityService {
    pub async fn resend_verification(
        &self,
        context: &RequestContext,
        identifier: &str,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, IdentityError> {
        let now = normalized_now(now);
        let issuer = self
            .token_issuer
            .clone()
            .ok_or(IdentityError::Unavailable)?;
        self.passwords
            .verify("verification-resend-fixed-cost", None);
        let challenge = self
            .prepare_challenge(
                context,
                ChallengeAction::VerificationResend,
                identifier.as_bytes(),
                now,
            )
            .await?;
        let command = self
            .public_command(
                context,
                "identity.public.resend_verification",
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
        let is_email = identifier.contains('@');
        let normalized = if is_email {
            self.policy.email(identifier)
        } else {
            self.policy.username(identifier)
        };
        let mut read = DbTransaction::begin_read(&self.pool).await?;
        let account = if let Ok(ref key) = normalized {
            repo::account_by_identifier(&mut read.connection(), &self.renderer, &key.key, is_email)
                .await?
        } else {
            None
        };
        read.rollback().await?;
        let lock_plan = account
            .as_ref()
            .map(|account| locks(account.id))
            .unwrap_or_default();
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
                        assert_flags(tx, &service.renderer, &[Flag::EmailVerification]).await?;
                        service
                            .consume_challenge(tx, challenge.as_ref(), now)
                            .await?;
                        if let Some(account) = account {
                            let current = repo::account_by_id(tx, &service.renderer, account.id)
                                .await?
                                .ok_or(TransactionError::InvalidInput)?;
                            if current.status == "active" && !current.email_verified {
                                queue_token(
                                    tx,
                                    &service.renderer,
                                    current.id,
                                    "verify_email",
                                    &current.email,
                                    &current.email_key,
                                    now,
                                    now + Duration::hours(24),
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
}
