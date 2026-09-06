//! Versioned identity risk contracts. Raw request metadata never enters this module.
use crate::contract::{TrustError, TrustSnapshot};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const RISK_EVENT_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskEventKind {
    Registration,
    Login,
    Invitation,
}
impl RiskEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Registration => "registration",
            Self::Login => "login",
            Self::Invitation => "invitation",
        }
    }
}

/// Only a keyed, purpose-separated digest produced by the trusted request adapter is
/// accepted. No HTTP deserializer, raw IP, user agent, mailbox or invitation code.
#[derive(Clone, Serialize)]
pub struct SignalDigest(String);
impl SignalDigest {
    pub fn from_keyed_hash(value: String) -> Result<Self, TrustError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(TrustError::InvalidInput);
        }
        Ok(Self(value))
    }
}
impl std::fmt::Debug for SignalDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SignalDigest([REDACTED])")
    }
}
#[derive(Clone, Default, Serialize)]
pub struct RiskSignals {
    pub account: Option<SignalDigest>,
    pub ip: Option<SignalDigest>,
    pub device: Option<SignalDigest>,
    pub network: Option<SignalDigest>,
    pub invitation: Option<SignalDigest>,
}
impl std::fmt::Debug for RiskSignals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RiskSignals([REDACTED])")
    }
}

#[derive(Clone)]
pub struct TrustedRiskContext {
    pub(crate) actor_id: Option<i64>,
    pub(crate) snapshot: Option<TrustSnapshot>,
    pub(crate) kind: RiskEventKind,
    pub(crate) signals: RiskSignals,
}
impl TrustedRiskContext {
    pub fn actor_id(&self) -> Option<i64> {
        self.actor_id
    }
    pub fn snapshot(&self) -> Option<&TrustSnapshot> {
        self.snapshot.as_ref()
    }
    pub fn kind(&self) -> RiskEventKind {
        self.kind
    }
    pub fn signals(&self) -> &RiskSignals {
        &self.signals
    }
}
impl std::fmt::Debug for TrustedRiskContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustedRiskContext")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskAction {
    Allow,
    Deny,
    Challenge,
    Unavailable,
}
impl RiskAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Challenge => "challenge",
            Self::Unavailable => "unavailable",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskReason {
    PolicyNotConfigured,
    ApprovedPolicy,
    EmailUnverified,
    InvitationNotGranted,
    AccountUnavailable,
    ExplicitChallenge,
    ExplicitDeny,
}
impl RiskReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PolicyNotConfigured => "policy_not_configured",
            Self::ApprovedPolicy => "approved_policy",
            Self::EmailUnverified => "email_unverified",
            Self::InvitationNotGranted => "invitation_not_granted",
            Self::AccountUnavailable => "account_unavailable",
            Self::ExplicitChallenge => "explicit_challenge",
            Self::ExplicitDeny => "explicit_deny",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RiskDecision {
    rule_version: i64,
    action: RiskAction,
    reason: RiskReason,
}
impl RiskDecision {
    pub fn new(
        rule_version: i64,
        action: RiskAction,
        reason: RiskReason,
    ) -> Result<Self, TrustError> {
        if rule_version < 0
            || (rule_version == 0
                && (action != RiskAction::Unavailable || reason != RiskReason::PolicyNotConfigured))
            || (reason == RiskReason::PolicyNotConfigured
                && (rule_version != 0 || action != RiskAction::Unavailable))
        {
            return Err(TrustError::InvalidInput);
        }
        Ok(Self {
            rule_version,
            action,
            reason,
        })
    }
    pub fn rule_version(self) -> i64 {
        self.rule_version
    }
    pub fn action(self) -> RiskAction {
        self.action
    }
    pub fn reason(self) -> RiskReason {
        self.reason
    }
    pub fn allowed(self) -> bool {
        self.action == RiskAction::Allow
    }
}
pub trait RiskPolicy: Send + Sync {
    /// The trusted application selects the policy. There are no product thresholds
    /// or implicit single-IP banning rules in this identity slice.
    fn assess(&self, context: &TrustedRiskContext) -> Result<RiskDecision, TrustError>;
}
pub struct UnconfiguredRiskPolicy;
impl RiskPolicy for UnconfiguredRiskPolicy {
    fn assess(&self, _: &TrustedRiskContext) -> Result<RiskDecision, TrustError> {
        RiskDecision::new(0, RiskAction::Unavailable, RiskReason::PolicyNotConfigured)
    }
}

/// DEC-051 baseline. Password/invitation validity, active account and challenge
/// consumption are checked by the owning identity service, not inferred from this result.
pub struct ApprovedIdentityRiskPolicy;
impl RiskPolicy for ApprovedIdentityRiskPolicy {
    fn assess(&self, context: &TrustedRiskContext) -> Result<RiskDecision, TrustError> {
        if context.kind == RiskEventKind::Invitation {
            let Some(snapshot) = context.snapshot() else {
                return RiskDecision::new(1, RiskAction::Deny, RiskReason::AccountUnavailable);
            };
            if !snapshot.email_verified() {
                return RiskDecision::new(1, RiskAction::Deny, RiskReason::EmailUnverified);
            }
            if !snapshot.invitation_allowed() || snapshot.policy_version() == 0 {
                return RiskDecision::new(1, RiskAction::Deny, RiskReason::InvitationNotGranted);
            }
        }
        RiskDecision::new(1, RiskAction::Allow, RiskReason::ApprovedPolicy)
    }
}

#[derive(Clone, Serialize)]
pub struct RiskEvent {
    pub(crate) event_version: i64,
    pub(crate) event_id: String,
    pub(crate) actor_id: Option<i64>,
    pub(crate) kind: RiskEventKind,
    pub(crate) occurred_at: DateTime<Utc>,
    pub(crate) signals: RiskSignals,
    pub(crate) decision: RiskDecision,
}
impl std::fmt::Debug for RiskEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RiskEvent")
            .field("kind", &self.kind)
            .field("decision", &self.decision)
            .finish_non_exhaustive()
    }
}
impl RiskEvent {
    pub fn decision(&self) -> RiskDecision {
        self.decision
    }
    pub fn event_id(&self) -> &str {
        &self.event_id
    }
    pub(crate) fn fingerprint(&self) -> Result<String, TrustError> {
        let bytes = serde_json::to_vec(self).map_err(|_| TrustError::InvalidInput)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}
