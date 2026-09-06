//! Restricted identity services. The caller owns the transaction, account authorization,
//! complete lock plan and commit. There is no promotion or invitation-grant endpoint.
use crate::{
    contract::{INVITATION_POLICY_VERSION, InvitationAuthorization, TrustError, TrustSnapshot},
    risk::{
        RISK_EVENT_VERSION, RiskEvent, RiskEventKind, RiskPolicy, RiskSignals, TrustedRiskContext,
    },
};
use chrono::{DateTime, Utc};
use platform::{
    identifiers::SchemaRenderer,
    transaction::{LockPlan, LockTarget},
};
use platform_db::transaction::TransactionConnection;

fn valid_id(id: i64) -> Result<(), TrustError> {
    if id > 0 {
        Ok(())
    } else {
        Err(TrustError::InvalidInput)
    }
}
fn canonical(now: DateTime<Utc>) -> Result<DateTime<Utc>, TrustError> {
    if now < DateTime::UNIX_EPOCH {
        return Err(TrustError::InvalidInput);
    }
    platform::db::decode_time(&platform::db::encode_time(now)).map_err(|_| TrustError::InvalidInput)
}

pub async fn register_initial(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
    invitation_id: i64,
    inviter_id: i64,
    now: DateTime<Utc>,
) -> Result<TrustSnapshot, TrustError> {
    valid_id(user_id)?;
    valid_id(invitation_id)?;
    valid_id(inviter_id)?;
    if user_id == inviter_id {
        return Err(TrustError::InvalidInput);
    }
    crate::repo::insert_initial(
        tx,
        renderer,
        user_id,
        invitation_id,
        inviter_id,
        canonical(now)?,
    )
    .await?;
    crate::repo::load_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::CorruptData)
}

pub async fn load_identity_snapshot(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
) -> Result<Option<TrustSnapshot>, TrustError> {
    valid_id(user_id)?;
    crate::repo::load_snapshot(tx, renderer, user_id).await
}

pub async fn mark_email_verified(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
    now: DateTime<Utc>,
) -> Result<TrustSnapshot, TrustError> {
    let now = canonical(now)?;
    let before = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    let mut targets = vec![
        LockTarget::new("trust", "identity_snapshots", before.id).ok_or(TrustError::Unavailable)?,
    ];
    if let Some(id) = crate::repo::invite_use_id(tx, renderer, user_id).await? {
        targets.push(LockTarget::new("trust", "invite_uses", id).ok_or(TrustError::Unavailable)?);
    }
    for target in LockPlan::new(targets).targets() {
        tx.lock(renderer, target).await?;
    }
    let current = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    if now < current.updated_at {
        return Err(TrustError::InvalidInput);
    }
    if current.email_verified
        && current.invitation_allowed
        && current.policy_version == INVITATION_POLICY_VERSION
    {
        return Ok(current);
    }
    // DEC-051 makes verified mail sufficient for invitation eligibility. The
    // existing level remains unchanged; this never grants T0/T1 or other privileges.
    crate::repo::confirm_email(tx, renderer, &current, now).await?;
    load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::CorruptData)
}

pub async fn authorize_invitation(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
) -> Result<InvitationAuthorization, TrustError> {
    let found = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    let target =
        LockTarget::new("trust", "identity_snapshots", found.id).ok_or(TrustError::Unavailable)?;
    tx.lock(renderer, &target).await?;
    let snapshot = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    if !snapshot.email_verified {
        return Ok(InvitationAuthorization::EmailUnverified);
    }
    if snapshot.invitation_allowed && snapshot.policy_version > 0 {
        return Ok(InvitationAuthorization::Allowed {
            policy_version: snapshot.policy_version,
        });
    }
    Ok(InvitationAuthorization::NotGranted)
}

/// Read the current stored trust facts, then evaluate a separately approved policy.
/// This function appends the decision in the same transaction and never commits.
/// Invoke after all mutable domain locks; Outbox/analytics appends may follow it.
#[allow(clippy::too_many_arguments)]
pub async fn record_identity_event(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    event_id: &str,
    actor_id: Option<i64>,
    kind: RiskEventKind,
    signals: RiskSignals,
    policy: &dyn RiskPolicy,
    now: DateTime<Utc>,
) -> Result<RiskEvent, TrustError> {
    if !platform::transaction::validate_operation_id(event_id) {
        return Err(TrustError::InvalidInput);
    }
    let snapshot = match actor_id {
        Some(id) => Some(
            load_identity_snapshot(tx, renderer, id)
                .await?
                .ok_or(TrustError::NotFound)?,
        ),
        None => None,
    };
    let context = TrustedRiskContext {
        actor_id,
        snapshot,
        kind,
        signals,
    };
    let decision = policy.assess(&context)?;
    let event = RiskEvent {
        event_version: RISK_EVENT_VERSION,
        event_id: event_id.into(),
        actor_id,
        kind,
        occurred_at: canonical(now)?,
        signals: context.signals,
        decision,
    };
    crate::repo::append_risk(tx, renderer, &event).await?;
    Ok(event)
}

#[cfg(test)]
mod tests;

/// Called under the identity user root lock, before any community lock.
/// No grant, default level or invitation-derived publishing permission is created.
pub async fn lock_content_snapshot(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
) -> Result<TrustSnapshot, TrustError> {
    let snapshot = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    tx.lock(
        renderer,
        &LockTarget::new("trust", "identity_snapshots", snapshot.id)
            .ok_or(TrustError::Unavailable)?,
    )
    .await?;
    load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)
}

/// Append a server-evaluated content decision after all mutable locks are held.
/// The owning command owns policy evaluation and commit; this is not a public grant API.
#[allow(clippy::too_many_arguments)]
pub async fn record_content_decision(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    event_id: &str,
    user_id: i64,
    kind: RiskEventKind,
    signals: RiskSignals,
    decision: crate::risk::RiskDecision,
    now: DateTime<Utc>,
) -> Result<RiskEvent, TrustError> {
    valid_id(user_id)?;
    if !platform::transaction::validate_operation_id(event_id)
        || !matches!(
            kind,
            RiskEventKind::Topic
                | RiskEventKind::Reply
                | RiskEventKind::Edit
                | RiskEventKind::Delete
        )
    {
        return Err(TrustError::InvalidInput);
    }
    let snapshot = load_identity_snapshot(tx, renderer, user_id)
        .await?
        .ok_or(TrustError::NotFound)?;
    if decision.allowed() && !snapshot.email_verified() {
        return Err(TrustError::InvalidInput);
    }
    let event = RiskEvent {
        event_version: RISK_EVENT_VERSION,
        event_id: event_id.into(),
        actor_id: Some(user_id),
        kind,
        occurred_at: canonical(now)?,
        signals,
        decision,
    };
    crate::repo::append_risk(tx, renderer, &event).await?;
    Ok(event)
}
