//! A05 four-dimensional rate limiting; disabled Redis means explicit no protection.
//! Thresholds and network prefixes have no product defaults.
use crate::contract::IdentityError;
use platform::cache::{
    RateLimitKey, RateLimitOutcome, RateLimitPolicy, RateLimitUse, RateLimiter, check_rate_limit,
};
use std::{net::IpAddr, sync::Arc};

/// Injected by trusted application assembly, never accepted from an HTTP form.
/// No default key, persistent raw address, or reversible request metadata is supplied.
pub struct RiskSignalKey(crate::contract::Secret);
impl RiskSignalKey {
    pub fn new(secret: crate::contract::Secret) -> Result<Self, IdentityError> {
        if secret.expose().len() < 32 {
            return Err(IdentityError::PolicyPending);
        }
        Ok(Self(secret))
    }
    pub(crate) fn signals(
        &self,
        account: &str,
        ip: Option<IpAddr>,
        device: Option<&str>,
        invitation: Option<&str>,
    ) -> Result<trust::risk::RiskSignals, IdentityError> {
        use hmac::{Hmac, Mac};
        let digest = |dimension: &str, value: &str| {
            let mut mac = Hmac::<sha2::Sha256>::new_from_slice(self.0.expose().as_bytes())
                .map_err(|_| IdentityError::Unavailable)?;
            mac.update(b"bbbcm.identity.risk.v1\0");
            mac.update(dimension.as_bytes());
            mac.update(b"\0");
            mac.update(value.as_bytes());
            let hash = mac
                .finalize()
                .into_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            trust::risk::SignalDigest::from_keyed_hash(hash).map_err(|_| IdentityError::Unavailable)
        };
        Ok(trust::risk::RiskSignals {
            account: Some(digest("account", account)?),
            ip: ip.map(|v| digest("ip", &v.to_string())).transpose()?,
            device: device.map(|v| digest("device", v)).transpose()?,
            // Network aggregation remains absent until its product prefix is approved.
            network: None,
            invitation: invitation.map(|v| digest("invitation", v)).transpose()?,
        })
    }
}

#[derive(Clone)]
pub struct RatePolicy {
    pub account: RateLimitPolicy,
    pub ip: RateLimitPolicy,
    pub device: RateLimitPolicy,
    pub network: RateLimitPolicy,
    pub ipv4_prefix: u8,
    pub ipv6_prefix: u8,
}
#[derive(Clone)]
pub struct RateProtection {
    pub redis_enabled: bool,
    pub limiter: Arc<dyn RateLimiter>,
    pub registration: Option<RatePolicy>,
    pub login: Option<RatePolicy>,
    pub recovery: Option<RatePolicy>,
    pub invitation: Option<RatePolicy>,
}
impl RateProtection {
    pub async fn check(
        &self,
        action: &str,
        account: &str,
        ip: IpAddr,
        device: &str,
    ) -> Result<RateLimitOutcome, IdentityError> {
        if !self.redis_enabled {
            return Ok(RateLimitOutcome::NoProtection);
        }
        let policy = match action {
            "registration" => self.registration.as_ref(),
            "login" => self.login.as_ref(),
            "recovery" => self.recovery.as_ref(),
            "invitation" => self.invitation.as_ref(),
            _ => return Err(IdentityError::Rejected),
        }
        .ok_or(IdentityError::PolicyPending)?;
        let network = match ip {
            IpAddr::V4(value) if policy.ipv4_prefix <= 32 => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(policy.ipv4_prefix))
                    .unwrap_or(0);
                std::net::Ipv4Addr::from(u32::from(value) & mask).to_string()
            }
            IpAddr::V6(value) if policy.ipv6_prefix <= 128 => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(policy.ipv6_prefix))
                    .unwrap_or(0);
                std::net::Ipv6Addr::from(u128::from(value) & mask).to_string()
            }
            _ => return Err(IdentityError::PolicyPending),
        };
        let ip = ip.to_string();
        let mut limited = false;
        for (dimension, value, budget) in [
            ("account", account, policy.account),
            ("ip", ip.as_str(), policy.ip),
            ("device", device, policy.device),
            ("network", network.as_str(), policy.network),
        ] {
            let digest = crate::core::secret_hash(&format!("{action}\n{dimension}\n{value}"));
            let key =
                RateLimitKey::new(dimension, &digest).map_err(|_| IdentityError::Unavailable)?;
            match check_rate_limit(self.limiter.as_ref(), &key, budget, RateLimitUse::HighRisk)
                .await
                .map_err(|_| IdentityError::Unavailable)?
            {
                RateLimitOutcome::Allowed => {}
                RateLimitOutcome::Limited { .. } => limited = true,
                // A configured Redis dependency must never quietly become disabled.
                _ => return Err(IdentityError::Unavailable),
            }
        }
        if limited {
            Err(IdentityError::RateLimited)
        } else {
            Ok(RateLimitOutcome::Allowed)
        }
    }
}

impl crate::core::IdentityService {
    /// Registration has no proven account yet. Its account dimension is the normalized
    /// email input for both existing and absent users; it never resolves username aliases.
    pub(crate) fn registration_subject(&self, email: &str) -> String {
        match self.policy.email(email) {
            Ok(value) => format!("registration-email:{}", value.key),
            Err(_) => format!(
                "invalid-registration-email:{}",
                crate::core::secret_hash(email)
            ),
        }
    }
    pub(crate) async fn validate_public_context(
        &self,
        context: &crate::contract::RequestContext,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), IdentityError> {
        let mut transaction =
            platform_db::transaction::DbTransaction::begin_read(&self.pool).await?;
        let result =
            crate::repo::principal(&mut transaction.connection(), &self.renderer, context, now)
                .await;
        transaction.rollback().await?;
        result
            .map(|_| ())
            .map_err(|e| match IdentityError::from(e) {
                IdentityError::Rejected => IdentityError::Csrf,
                other => other,
            })
    }
    /// Resolve aliases to a stable authoritative account reference before selecting a bucket.
    /// Unknown identifiers retain a pseudonymous input bucket and reveal no lookup result.
    pub(crate) async fn rate_subject(
        &self,
        identifier: &str,
        token: Option<&str>,
        session: Option<&crate::contract::SessionIdentity>,
    ) -> Result<String, IdentityError> {
        if let Some(session) = session {
            return Ok(format!("account:{}", session.user_id()));
        }
        let mut transaction =
            platform_db::transaction::DbTransaction::begin_read(&self.pool).await?;
        let mut tx = transaction.connection();
        let subject = if let Some(token) = token {
            let digest = crate::core::secret_hash(token);
            crate::repo::token_owner(&mut tx, &self.renderer, &digest, "reset_password")
                .await?
                .map(|t| format!("account:{}", t.user_id))
                .unwrap_or_else(|| format!("unknown-token:{digest}"))
        } else {
            let email = identifier.contains('@');
            let normalized = if email {
                self.policy.email(identifier)
            } else {
                self.policy.username(identifier)
            };
            match normalized {
                Ok(normalized) => crate::repo::account_by_identifier(
                    &mut tx,
                    &self.renderer,
                    &normalized.key,
                    email,
                )
                .await?
                .map(|a| format!("account:{}", a.id))
                .unwrap_or_else(|| format!("unknown:{email}:{}", normalized.key)),
                Err(_) => format!("invalid:{}", crate::core::secret_hash(identifier)),
            }
        };
        transaction.rollback().await?;
        Ok(subject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform::cache::{BoxFuture, CacheError, RateLimitDecision};
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };
    #[test]
    fn risk_signals_are_keyed_separated_and_do_not_retain_raw_inputs() {
        let first =
            RiskSignalKey::new("synthetic-risk-key-aaaaaaaaaaaaaaaa".to_owned().into()).unwrap();
        let other =
            RiskSignalKey::new("synthetic-risk-key-bbbbbbbbbbbbbbbb".to_owned().into()).unwrap();
        let make = |key: &RiskSignalKey| {
            key.signals(
                "private@example.invalid",
                Some("192.0.2.18".parse().unwrap()),
                Some("synthetic-device"),
                Some("synthetic-invitation"),
            )
            .unwrap()
        };
        let json = serde_json::to_string(&make(&first)).unwrap();
        assert_eq!(json, serde_json::to_string(&make(&first)).unwrap());
        assert_ne!(json, serde_json::to_string(&make(&other)).unwrap());
        for raw in [
            "private@example.invalid",
            "192.0.2.18",
            "synthetic-device",
            "synthetic-invitation",
        ] {
            assert!(!json.contains(raw));
            assert!(!format!("{:?}", make(&first)).contains(raw));
        }
        let same = first.signals("same", None, Some("same"), None).unwrap();
        assert_ne!(
            serde_json::to_string(&same.account).unwrap(),
            serde_json::to_string(&same.device).unwrap()
        );
        assert!(same.network.is_none());
    }
    struct Counting {
        count: AtomicUsize,
        fail: bool,
    }
    impl RateLimiter for Counting {
        fn check<'a>(
            &'a self,
            _: &'a RateLimitKey,
            _: RateLimitPolicy,
        ) -> BoxFuture<'a, Result<RateLimitDecision, CacheError>> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if self.fail {
                    Err(CacheError::Unavailable)
                } else {
                    Ok(RateLimitDecision {
                        allowed: true,
                        retry_after: Duration::ZERO,
                    })
                }
            })
        }
    }
    fn synthetic(limiter: Arc<dyn RateLimiter>, enabled: bool) -> RateProtection {
        let budget = RateLimitPolicy::new(5, Duration::from_secs(10)).unwrap();
        let policy = RatePolicy {
            account: budget,
            ip: budget,
            device: budget,
            network: budget,
            ipv4_prefix: 24,
            ipv6_prefix: 64,
        };
        RateProtection {
            redis_enabled: enabled,
            limiter,
            registration: Some(policy.clone()),
            login: Some(policy.clone()),
            recovery: Some(policy.clone()),
            invitation: Some(policy),
        }
    }
    #[tokio::test]
    async fn four_checks_disabled_absence_and_enabled_failure() {
        let limiter = Arc::new(Counting {
            count: AtomicUsize::new(0),
            fail: false,
        });
        let ip = "192.0.2.25".parse().unwrap();
        assert_eq!(
            synthetic(limiter.clone(), false)
                .check("login", "synthetic", ip, "synthetic-device")
                .await
                .unwrap(),
            RateLimitOutcome::NoProtection
        );
        assert_eq!(limiter.count.load(Ordering::SeqCst), 0);
        assert_eq!(
            synthetic(limiter.clone(), true)
                .check("login", "synthetic", ip, "synthetic-device")
                .await
                .unwrap(),
            RateLimitOutcome::Allowed
        );
        assert_eq!(limiter.count.load(Ordering::SeqCst), 4);
        assert_eq!(
            synthetic(
                Arc::new(Counting {
                    count: AtomicUsize::new(0),
                    fail: true
                }),
                true
            )
            .check("login", "synthetic", ip, "synthetic-device")
            .await,
            Err(IdentityError::Unavailable)
        );
        let mut pending = synthetic(limiter, true);
        pending.login = None;
        assert_eq!(
            pending
                .check("login", "synthetic", ip, "synthetic-device")
                .await,
            Err(IdentityError::PolicyPending)
        );
    }
    #[tokio::test]
    async fn known_account_aliases_share_authoritative_bucket() {
        let f = crate::core_tests::Fixture::new().await;
        f.enable().await;
        let input = f.registration();
        let username = input.username.clone();
        let email = input.email.clone();
        f.service
            .register(&f.context().await, input, crate::core_tests::at())
            .await
            .unwrap();
        assert_eq!(
            f.service.rate_subject(&username, None, None).await.unwrap(),
            f.service.rate_subject(&email, None, None).await.unwrap()
        );
        assert_ne!(
            f.service.rate_subject(&username, None, None).await.unwrap(),
            f.service
                .rate_subject("absent_subject", None, None)
                .await
                .unwrap()
        );
    }
}
