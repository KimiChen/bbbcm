//! App composition keeps content independent of identity, moderation and trust.
//! Only core services receive the caller-owned transaction; no foreign repo access.
use crate::content_risk::{ContentRiskFactory, RateProtection, TrustedContentRisk};
use chrono::{DateTime, Utc};
use content::contract::*;
use platform::{analytics::TrustedExclusions, identifiers::SchemaRenderer};
use platform_db::transaction::TransactionConnection;
use std::sync::Arc;

pub struct ApplicationContentAuthority {
    renderer: SchemaRenderer,
    /// No default content policy is approved. Only explicit synthetic tests set this.
    risk_rule_version: Option<i64>,
    /// PRIV-001/002 default is unapproved; do not invent a Public author fact.
    author_default: Option<Visibility>,
}
impl ApplicationContentAuthority {
    pub fn new(renderer: SchemaRenderer) -> Self {
        Self {
            renderer,
            risk_rule_version: None,
            author_default: None,
        }
    }
    #[cfg(test)]
    pub fn synthetic(renderer: SchemaRenderer) -> Self {
        Self {
            renderer,
            risk_rule_version: Some(1),
            author_default: Some(Visibility::Public),
        }
    }
}
fn failure(error: identity::contract::IdentityError) -> ContentError {
    use identity::contract::IdentityError as E;
    match error {
        E::Csrf | E::Unauthenticated | E::Rejected => ContentError::Forbidden,
        E::PolicyPending => ContentError::PolicyPending,
        _ => ContentError::Unavailable,
    }
}
fn capability(action: Action) -> moderation::contract::ContentCapability {
    use moderation::contract::ContentCapability as C;
    match action {
        Action::CreateTopic => C::Topic,
        Action::CreateReply => C::Reply,
        Action::EditPost => C::Edit,
        Action::DeletePost => C::Delete,
    }
}
impl ContentAuthority for ApplicationContentAuthority {
    fn forum_rules<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        ids: &'a [i64],
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Vec<(i64, String)>> {
        Box::pin(async move {
            moderation::core::content_forum_rules(tx, &self.renderer, ids)
                .await
                .map_err(Into::into)
        })
    }
    fn read_viewer<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        session: Option<&'a str>,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, Viewer> {
        Box::pin(async move {
            let Some(raw) = session else {
                return Ok(Viewer::default());
            };
            let Some(user) = identity::core::read_content_identity(tx, &self.renderer, raw, now)
                .await
                .map_err(failure)?
            else {
                return Ok(Viewer::default());
            };
            Ok(Viewer {
                user_id: Some(user.user_id()),
                member: user.email_verified(),
            })
        })
    }
    fn authorize<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        action: Action,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, AuthorityFacts> {
        Box::pin(async move {
            let user = identity::core::authorize_content_identity(
                tx,
                &self.renderer,
                credentials.session_token.expose(),
                credentials.csrf.expose(),
                now,
            )
            .await
            .map_err(failure)?;
            if let Some(context) = credentials.trusted_context.as_ref() {
                let context = context
                    .downcast_ref::<TrustedContentRisk>()
                    .ok_or(ContentError::Forbidden)?;
                context.binding(user.user_id(), action)?;
                if self.risk_rule_version != Some(context.rule_version()) {
                    return Err(ContentError::PolicyPending);
                }
                // This observation distinguishes disabled protection from an allowed bucket.
                match context.protection() {
                    RateProtection::Protected => {
                        tracing::debug!(protection = "protected", "content rate preflight")
                    }
                    RateProtection::NoProtection => {
                        tracing::debug!(protection = "disabled", "content rate preflight")
                    }
                }
            } else if self.risk_rule_version.is_none() {
                return Err(ContentError::PolicyPending);
            }
            let restricted = moderation::core::content_restricted(
                tx,
                &self.renderer,
                user.user_id(),
                capability(action),
                now,
            )
            .await?;
            let snapshot = user.trust().ok_or(ContentError::Unavailable)?;
            let tier = match snapshot.level() {
                trust::contract::TrustLevel::Pending => TrustTier::Pending,
                trust::contract::TrustLevel::T0 => TrustTier::T0,
                trust::contract::TrustLevel::T1 => TrustTier::T1,
            };
            // No privilege is inferred from invitation eligibility or verification. Tier
            // permissions are evaluated separately by the content policy on these facts.
            Ok(AuthorityFacts {
                user_id: user.user_id(),
                created_at: user.created_at(),
                tier,
                email_verified: user.email_verified(),
                allowed: !restricted,
                default_visibility: self.author_default.ok_or(ContentError::PolicyPending)?,
                publication: Publication::Published,
                analytics_pseudonym: user.analytics_pseudonym().into(),
                analytics_exclusions: TrustedExclusions::from_authority(
                    false,
                    user.analytics_excluded(),
                    false,
                    false,
                ),
            })
        })
    }
    fn authors<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        ids: &'a [i64],
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Vec<Author>> {
        Box::pin(async move {
            let default_visibility = self.author_default.ok_or(ContentError::PolicyPending)?;
            identity::core::content_authors(tx, &self.renderer, ids)
                .await
                .map_err(failure)
                .map(|authors| {
                    authors
                        .into_iter()
                        .map(|a| Author {
                            user_id: a.user_id,
                            public_id: a.public_ref,
                            username: a.username,
                            default_visibility,
                        })
                        .collect()
                })
        })
    }
    fn resolve_author<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        public_id: &'a str,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, Option<Author>> {
        Box::pin(async move {
            let Some(id) = identity::core::content_profile_id(tx, &self.renderer, public_id)
                .await
                .map_err(failure)?
            else {
                return Ok(None);
            };
            Ok(self.authors(tx, &[id], now).await?.into_iter().next())
        })
    }
    fn append_risk<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        facts: &'a AuthorityFacts,
        action: Action,
        disposition: RiskDisposition,
        event_id: &'a str,
        now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async move {
            use trust::risk::{
                RiskAction as R, RiskDecision, RiskEventKind as K, RiskReason as Why,
            };
            let signals = credentials
                .trusted_context
                .as_ref()
                .and_then(|v| v.downcast_ref::<TrustedContentRisk>())
                .map(|v| v.signals().clone())
                .unwrap_or_default();
            let decision = match self.risk_rule_version {
                Some(version) => RiskDecision::new(
                    version,
                    match disposition {
                        RiskDisposition::Allowed => R::Allow,
                        RiskDisposition::Denied => R::Deny,
                        RiskDisposition::Unavailable => R::Unavailable,
                    },
                    if disposition == RiskDisposition::Denied {
                        Why::ExplicitDeny
                    } else {
                        Why::ApprovedPolicy
                    },
                ),
                None => RiskDecision::new(0, R::Unavailable, Why::PolicyNotConfigured),
            }
            .map_err(|_| ContentError::Unavailable)?;
            let kind = match action {
                Action::CreateTopic => K::Topic,
                Action::CreateReply => K::Reply,
                Action::EditPost => K::Edit,
                Action::DeletePost => K::Delete,
            };
            trust::core::record_content_decision(
                tx,
                &self.renderer,
                event_id,
                facts.user_id,
                kind,
                signals,
                decision,
                now,
            )
            .await
            .map_err(|_| ContentError::Unavailable)?;
            if !decision.allowed() && disposition == RiskDisposition::Allowed {
                return Err(ContentError::PolicyPending);
            }
            Ok(())
        })
    }
}

pub async fn connect(state: &mut crate::app_state::AppState) -> Result<(), String> {
    state.content.flags = state.identity.flags.clone();
    let Some(key) = &state.config.content.cursor_key else {
        return Ok(());
    };
    let bytes = key
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|p| u8::from_str_radix(p, 16).ok())
                .ok_or("invalid content cursor key")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let cursor =
        content::cursor::CursorKey::new(bytes).map_err(|_| "invalid content cursor key")?;
    let renderer = SchemaRenderer::new(&state.config.database.schema_prefix)
        .map_err(|_| "content renderer unavailable")?;
    let pool = platform_db::Pool::connect_existing(&state.config.database)
        .await
        .map_err(|_| "content database unavailable")?;
    let mut service = content::core::ContentService::new(
        pool,
        renderer.clone(),
        Arc::new(ApplicationContentAuthority::new(renderer)),
        Arc::new(content::policy::UnconfiguredContentPolicy),
        cursor,
    );
    if let Some(runtime) = state.runtime.as_ref() {
        service = service.with_cache(runtime.cache.clone());
    }
    state.content.service = Some(Arc::new(service));
    if let (Some(key), Some(runtime)) = (&state.config.identity.risk_key, state.runtime.as_ref()) {
        state.content.request_context = Some(Arc::new(
            ContentRiskFactory::new(
                key.as_bytes().to_vec(),
                state.config.features.redis,
                runtime.rate_limiter.clone(),
                None,
                state.config.server.environment.is_production_like(),
            )
            .map_err(|_| "content risk assembly unavailable")?,
        ));
    }
    Ok(())
}
