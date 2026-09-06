//! Explicit local simulation of a challenge provider. Proofs never enter SQL or HTTP.
use crate::{
    challenge_repo,
    contract::*,
    core::{
        IdentityService, TokenIssuer, bound_request_hash, constant_equal, normalized_now,
        random_secret, secret_hash,
    },
    repo,
};
use chrono::{DateTime, Duration, Utc};
use platform::{flags::Flag, integrations::local::LocalRoot, transaction::new_operation_id};
use platform_db::transaction::{DbTransaction, TransactionConnection, TransactionError};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct ChallengeSubmission {
    pub challenge_id: String,
    pub proof: Secret,
}

#[derive(Clone, Copy)]
pub(crate) enum ChallengeAction {
    Register,
    Login,
    Recovery,
    VerificationResend,
}
impl ChallengeAction {
    fn name(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::Login => "login",
            Self::Recovery => "recovery",
            Self::VerificationResend => "verification_resend",
        }
    }
    fn flags(self) -> &'static [Flag] {
        match self {
            Self::Register => &[Flag::Registration, Flag::EmailVerification],
            Self::VerificationResend => &[Flag::EmailVerification],
            _ => &[Flag::PublicRead],
        }
    }
}

/// Constructible only with the existing development/test private local root.
/// The active mail key is reused exclusively through a separate HMAC purpose.
pub struct LocalChallengeProvider {
    root: LocalRoot,
    issuer: Arc<TokenIssuer>,
    retained: Vec<Arc<TokenIssuer>>,
}
impl LocalChallengeProvider {
    pub fn new(root: LocalRoot, issuer: Arc<TokenIssuer>) -> Self {
        Self {
            root,
            retained: vec![issuer.clone()],
            issuer,
        }
    }
    pub fn with_retained_issuers(
        mut self,
        issuers: Vec<Arc<TokenIssuer>>,
    ) -> Result<Self, IdentityError> {
        for issuer in issuers {
            if let Some(existing) = self
                .retained
                .iter()
                .find(|key| key.version == issuer.version)
            {
                if !Arc::ptr_eq(existing, &issuer) {
                    return Err(IdentityError::Rejected);
                }
            } else {
                self.retained.push(issuer);
            }
        }
        Ok(self)
    }
    fn proof(&self, record: &challenge_repo::ChallengeRecord) -> Result<Secret, IdentityError> {
        let issuer = self
            .retained
            .iter()
            .find(|issuer| issuer.version == record.key_version)
            .ok_or(IdentityError::Unavailable)?;
        Ok(issuer.derive(
            &record.challenge_id,
            &record.nonce,
            record.principal_id,
            &format!("local_challenge.{}", record.action),
            &record.request_hash,
            record.expires_at,
            0,
        ))
    }
    async fn publish(&self, record: &challenge_repo::ChallengeRecord) -> Result<(), IdentityError> {
        let proof = self.proof(record)?;
        if !constant_equal(&secret_hash(proof.expose()), &record.proof_hash) {
            return Err(IdentityError::Unavailable);
        }
        let bytes = serde_json::to_vec(&serde_json::json!({
            "provider": "LOCAL TEST ONLY — no anti-bot protection",
            "challenge_id": record.challenge_id,
            "action": record.action,
            "expires_at": platform::db::encode_time(record.expires_at),
            "proof": proof.expose(),
        }))
        .map_err(|_| IdentityError::Unavailable)?;
        self.root
            .write("processed", &record.challenge_id, "json", bytes)
            .await
            .map_err(|_| IdentityError::Unavailable)
    }
}

#[derive(Clone)]
pub(crate) struct ChallengeTicket {
    challenge_id: String,
    principal_id: i64,
    action: String,
    request_hash: String,
    proof_hash: String,
    consumed: bool,
}
impl ChallengeTicket {
    pub(crate) fn was_consumed(&self) -> bool {
        self.consumed
    }
}
impl IdentityService {
    /// A proof is prepared outside the business transaction, then atomically consumed
    /// inside it. A receipt replay skips consumption, preserving normal idempotency.
    pub(crate) async fn prepare_challenge(
        &self,
        context: &RequestContext,
        action: ChallengeAction,
        canonical: &[u8],
        now: DateTime<Utc>,
    ) -> Result<Option<ChallengeTicket>, IdentityError> {
        if !context.challenge_required && context.challenge_submission.is_none() {
            return Ok(None);
        }
        let provider = self
            .challenge_provider
            .as_ref()
            .ok_or(IdentityError::Unavailable)?;
        let now = normalized_now(now);
        let mut tx = DbTransaction::begin(&self.pool).await?;
        let principal_id =
            repo::principal(&mut tx.connection(), &self.renderer, context, now).await?;
        crate::core::assert_flags(&mut tx.connection(), &self.renderer, action.flags()).await?;
        if matches!(action, ChallengeAction::Register) {
            repo::assert_unlocked(&mut tx.connection(), &self.renderer, Flag::RegistrationLock)
                .await?;
            repo::assert_unlocked(&mut tx.connection(), &self.renderer, Flag::InvitationLock)
                .await?;
        }
        let binding = serde_json::to_vec(&(
            "identity.challenge.v1",
            action.name(),
            &context.idempotency_key,
            canonical,
        ))
        .map_err(|_| IdentityError::Rejected)?;
        let request_hash = platform::transaction::request_hash(&bound_request_hash(
            context.preauth.expose(),
            &binding,
        ));
        if let Some(submission) = &context.challenge_submission {
            let record = challenge_repo::by_reference(
                &mut tx.connection(),
                &self.renderer,
                &submission.challenge_id,
            )
            .await?
            .ok_or(IdentityError::Rejected)?;
            let proof_hash = secret_hash(submission.proof.expose());
            if record.principal_id != principal_id
                || record.action != action.name()
                || !constant_equal(&record.request_hash, &request_hash)
                || !constant_equal(&record.proof_hash, &proof_hash)
            {
                return Err(IdentityError::Rejected);
            }
            if !record.consumed && record.expires_at <= now {
                return Err(IdentityError::Rejected);
            }
            // Expiry and single-use are checked in the command closure. An original
            // committed receipt remains replayable even after challenge expiry/use.
            tx.rollback().await?;
            return Ok(Some(ChallengeTicket {
                challenge_id: record.challenge_id,
                principal_id,
                action: action.name().into(),
                request_hash,
                proof_hash,
                consumed: record.consumed,
            }));
        }
        let existing = challenge_repo::by_binding(
            &mut tx.connection(),
            &self.renderer,
            principal_id,
            action.name(),
            &request_hash,
        )
        .await?;
        let record = if let Some(record) = existing {
            if record.expires_at <= now || record.consumed {
                return Err(IdentityError::Rejected);
            }
            record
        } else {
            let mut record = challenge_repo::ChallengeRecord {
                challenge_id: new_operation_id(),
                principal_id,
                action: action.name().into(),
                request_hash,
                proof_hash: String::new(),
                nonce: random_secret().expose().to_owned(),
                key_version: provider.issuer.version.clone(),
                created_at: now,
                expires_at: now + Duration::minutes(5),
                consumed: false,
            };
            record.proof_hash = secret_hash(provider.proof(&record)?.expose());
            let inserted =
                challenge_repo::insert(&mut tx.connection(), &self.renderer, &record).await?;
            let kind = match action {
                ChallengeAction::Register => Some(trust::risk::RiskEventKind::Registration),
                ChallengeAction::Login => Some(trust::risk::RiskEventKind::Login),
                _ => None,
            };
            if let Some(kind) = kind.filter(|_| inserted) {
                trust::core::record_identity_event(
                    &mut tx.connection(),
                    &self.renderer,
                    &record.challenge_id,
                    None,
                    kind,
                    context.risk_signals.clone(),
                    &ChallengeRiskPolicy,
                    now,
                )
                .await
                .map_err(|_| IdentityError::Unavailable)?;
            }
            // A concurrent request can have installed the same binding first.
            challenge_repo::by_binding(
                &mut tx.connection(),
                &self.renderer,
                principal_id,
                action.name(),
                &record.request_hash,
            )
            .await?
            .ok_or(IdentityError::Unavailable)?
        };
        tx.commit().await?;
        // Synchronous local provider publication, retried from immutable DB context
        // on the same request. No business command or password data is stored here.
        provider.publish(&record).await?;
        Err(IdentityError::ChallengeRequired {
            challenge_id: record.challenge_id,
        })
    }
    pub(crate) async fn consume_challenge(
        &self,
        tx: &mut TransactionConnection<'_>,
        ticket: Option<&ChallengeTicket>,
        now: DateTime<Utc>,
    ) -> Result<(), TransactionError> {
        if let Some(ticket) = ticket {
            challenge_repo::consume(
                tx,
                &self.renderer,
                &ticket.challenge_id,
                ticket.principal_id,
                &ticket.action,
                &ticket.request_hash,
                &ticket.proof_hash,
                now,
            )
            .await?;
        }
        Ok(())
    }
}

pub(crate) fn hard_blocked(context: &RequestContext, ticket: Option<&ChallengeTicket>) -> bool {
    // A consumed proof is atomic with its receipt. Preserve that original response
    // even when a private hard bucket has since emptied. Every business closure also
    // guards hard_rate_limited, so a missing/corrupt receipt can never execute work.
    context.hard_rate_limited && !ticket.is_some_and(ChallengeTicket::was_consumed)
}

struct ChallengeRiskPolicy;
impl trust::risk::RiskPolicy for ChallengeRiskPolicy {
    fn assess(
        &self,
        _: &trust::risk::TrustedRiskContext,
    ) -> Result<trust::risk::RiskDecision, trust::contract::TrustError> {
        trust::risk::RiskDecision::new(
            1,
            trust::risk::RiskAction::Challenge,
            trust::risk::RiskReason::ExplicitChallenge,
        )
    }
}
