//! Content risk preflight runs before the content transaction takes identity locks.
//! Policies and keys are explicitly injected. No product threshold is selected here.
use axum::http::{HeaderMap, header};
use content::{
    contract::{Action, ContentError, FutureResult},
    http::RequestContext,
};
use hmac::{Hmac, Mac};
use platform::cache::{
    RateLimitKey, RateLimitOutcome, RateLimitPolicy, RateLimitUse, RateLimiter, check_rate_limit,
};
use std::{
    any::Any,
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use trust::risk::{RiskSignals, SignalDigest};

#[derive(Clone)]
pub(crate) struct RateRule {
    pub account: RateLimitPolicy,
    pub ip: RateLimitPolicy,
    pub device: RateLimitPolicy,
    pub network: RateLimitPolicy,
    pub ipv4_prefix: u8,
    pub ipv6_prefix: u8,
}

#[derive(Clone)]
pub(crate) struct ContentRatePolicy {
    pub version: i64,
    pub actions: Vec<(Action, RateRule)>,
}
impl ContentRatePolicy {
    fn rule(&self, action: Action) -> Result<&RateRule, ContentError> {
        if self.version <= 0 {
            return Err(ContentError::PolicyPending);
        }
        let mut matches = self
            .actions
            .iter()
            .filter(|(candidate, _)| *candidate == action);
        let (_, rule) = matches.next().ok_or(ContentError::PolicyPending)?;
        if matches.next().is_some() || rule.ipv4_prefix > 32 || rule.ipv6_prefix > 128 {
            return Err(ContentError::PolicyPending);
        }
        Ok(rule)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RateProtection {
    Protected,
    NoProtection,
}

/// Only this preflight factory can construct the context. It is never deserialized
/// or accepted from HTTP, and the final transaction must check actor/action again.
pub(crate) struct TrustedContentRisk {
    actor_id: i64,
    action: Action,
    signals: RiskSignals,
    rule_version: i64,
    protection: RateProtection,
}
impl TrustedContentRisk {
    pub fn binding(&self, actor_id: i64, action: Action) -> Result<(), ContentError> {
        if self.actor_id == actor_id && self.action == action {
            Ok(())
        } else {
            Err(ContentError::Forbidden)
        }
    }
    pub fn signals(&self) -> &RiskSignals {
        &self.signals
    }
    pub fn rule_version(&self) -> i64 {
        self.rule_version
    }
    pub fn protection(&self) -> RateProtection {
        self.protection
    }
}
impl std::fmt::Debug for TrustedContentRisk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrustedContentRisk([REDACTED])")
    }
}

pub(crate) struct ContentRiskFactory {
    key: Vec<u8>,
    redis_enabled: bool,
    limiter: Arc<dyn RateLimiter>,
    policy: Option<ContentRatePolicy>,
    secure_cookies: bool,
}
impl ContentRiskFactory {
    pub fn new(
        key: Vec<u8>,
        redis_enabled: bool,
        limiter: Arc<dyn RateLimiter>,
        policy: Option<ContentRatePolicy>,
        secure_cookies: bool,
    ) -> Result<Self, ContentError> {
        if key.len() < 32 {
            return Err(ContentError::PolicyPending);
        }
        Ok(Self {
            key,
            redis_enabled,
            limiter,
            policy,
            secure_cookies,
        })
    }
    fn digest(
        &self,
        purpose: &[u8],
        version: i64,
        action: Action,
        dimension: &str,
        value: &str,
    ) -> Result<String, ContentError> {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&self.key)
            .map_err(|_| ContentError::Unavailable)?;
        mac.update(purpose);
        mac.update(&version.to_be_bytes());
        mac.update(action.as_str().as_bytes());
        mac.update(b"\0");
        mac.update(dimension.as_bytes());
        mac.update(b"\0");
        mac.update(value.as_bytes());
        Ok(mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    }
    async fn check(
        &self,
        headers: &HeaderMap,
        peer: Option<SocketAddr>,
        actor_id: i64,
        action: Action,
    ) -> Result<TrustedContentRisk, ContentError> {
        if actor_id <= 0 {
            return Err(ContentError::Forbidden);
        }
        let policy = self.policy.as_ref().ok_or(ContentError::PolicyPending)?;
        let rule = policy.rule(action)?;
        // Peer address is the server's socket peer; Forwarded and X-Forwarded-For
        // are intentionally not consulted. Device is the existing opaque cookie.
        let ip = peer.map(|peer| peer.ip());
        let device = device_cookie(headers, self.secure_cookies);
        if self.redis_enabled && (ip.is_none() || device.is_none()) {
            return Err(ContentError::Unavailable);
        }
        let actor = actor_id.to_string();
        let ip = ip.map(|ip| ip.to_string());
        let network = peer
            .map(|peer| network(peer.ip(), rule.ipv4_prefix, rule.ipv6_prefix))
            .transpose()?;
        let signal = |dimension: &str, value: &str| {
            self.digest(
                b"bbbcm.content.risk.v1\0",
                policy.version,
                action,
                dimension,
                value,
            )
            .and_then(|hash| {
                SignalDigest::from_keyed_hash(hash).map_err(|_| ContentError::Unavailable)
            })
        };
        let signals = RiskSignals {
            account: Some(signal("account", &actor)?),
            ip: ip.as_deref().map(|ip| signal("ip", ip)).transpose()?,
            device: device
                .as_deref()
                .map(|device| signal("device", device))
                .transpose()?,
            network: network
                .as_deref()
                .map(|network| signal("network", network))
                .transpose()?,
            invitation: None,
        };
        let protection = if self.redis_enabled {
            let dimensions = [
                ("account", actor.as_str(), rule.account),
                (
                    "ip",
                    ip.as_deref().ok_or(ContentError::Unavailable)?,
                    rule.ip,
                ),
                (
                    "device",
                    device.as_deref().ok_or(ContentError::Unavailable)?,
                    rule.device,
                ),
                (
                    "network",
                    network.as_deref().ok_or(ContentError::Unavailable)?,
                    rule.network,
                ),
            ];
            let mut limited = false;
            for (dimension, value, budget) in dimensions {
                let digest = self.digest(
                    b"bbbcm.content.rate.v1\0",
                    policy.version,
                    action,
                    dimension,
                    value,
                )?;
                let key =
                    RateLimitKey::new(dimension, &digest).map_err(|_| ContentError::Unavailable)?;
                match check_rate_limit(self.limiter.as_ref(), &key, budget, RateLimitUse::HighRisk)
                    .await
                    .map_err(|_| ContentError::Unavailable)?
                {
                    RateLimitOutcome::Allowed => {}
                    RateLimitOutcome::Limited { .. } => limited = true,
                    RateLimitOutcome::NoProtection | RateLimitOutcome::OriginFallback => {
                        return Err(ContentError::Unavailable);
                    }
                }
            }
            if limited {
                return Err(ContentError::RateLimited);
            }
            RateProtection::Protected
        } else {
            RateProtection::NoProtection
        };
        Ok(TrustedContentRisk {
            actor_id,
            action,
            signals,
            rule_version: policy.version,
            protection,
        })
    }
}
impl RequestContext for ContentRiskFactory {
    fn prepare<'a>(
        &'a self,
        headers: &'a HeaderMap,
        peer: Option<SocketAddr>,
        actor_id: i64,
        action: Action,
    ) -> FutureResult<'a, Arc<dyn Any + Send + Sync>> {
        Box::pin(async move {
            self.check(headers, peer, actor_id, action)
                .await
                .map(|context| Arc::new(context) as Arc<dyn Any + Send + Sync>)
        })
    }
}
impl std::fmt::Debug for ContentRiskFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ContentRiskFactory([REDACTED])")
    }
}
fn device_cookie(headers: &HeaderMap, secure: bool) -> Option<String> {
    let name = if secure {
        "__Host-bcm_device"
    } else {
        "bcm_device"
    };
    let mut values = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|v| v.trim().split_once('='))
        .filter(|(key, _)| *key == name)
        .map(|(_, value)| value);
    let value = values.next()?;
    if values.next().is_some()
        || !(1..=256).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    Some(value.into())
}
fn network(ip: IpAddr, ipv4_prefix: u8, ipv6_prefix: u8) -> Result<String, ContentError> {
    match ip {
        IpAddr::V4(value) if ipv4_prefix <= 32 => Ok(std::net::Ipv4Addr::from(
            u32::from(value)
                & u32::MAX
                    .checked_shl(32 - u32::from(ipv4_prefix))
                    .unwrap_or(0),
        )
        .to_string()),
        IpAddr::V6(value) if ipv6_prefix <= 128 => Ok(std::net::Ipv6Addr::from(
            u128::from(value)
                & u128::MAX
                    .checked_shl(128 - u32::from(ipv6_prefix))
                    .unwrap_or(0),
        )
        .to_string()),
        _ => Err(ContentError::PolicyPending),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform::cache::{CacheError, DisabledRateLimiter, RateLimitDecision, TokenBucketLimiter};
    use std::{sync::Mutex, time::Duration};
    fn policy() -> ContentRatePolicy {
        let budget = RateLimitPolicy::new(2, Duration::from_secs(60)).unwrap();
        let rule = RateRule {
            account: budget,
            ip: budget,
            device: budget,
            network: budget,
            ipv4_prefix: 24,
            ipv6_prefix: 64,
        };
        ContentRatePolicy {
            version: 71,
            actions: vec![
                (Action::CreateTopic, rule.clone()),
                (Action::CreateReply, rule),
            ],
        }
    }
    fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "bcm_device=synthetic_opaque_device_0001".parse().unwrap(),
        );
        headers
    }
    fn peer() -> Option<SocketAddr> {
        Some("192.0.2.41:40000".parse().unwrap())
    }

    #[tokio::test]
    async fn no_policy_is_pending_even_when_redis_is_disabled() {
        let factory = ContentRiskFactory::new(
            vec![7; 32],
            false,
            Arc::new(DisabledRateLimiter),
            None,
            false,
        )
        .unwrap();
        assert!(matches!(
            factory
                .check(&headers(), peer(), 1, Action::CreateTopic)
                .await,
            Err(ContentError::PolicyPending)
        ));
    }
    #[tokio::test]
    async fn disabled_is_explicit_and_context_is_bound_redacted_and_pseudonymous() {
        let factory = ContentRiskFactory::new(
            vec![7; 32],
            false,
            Arc::new(DisabledRateLimiter),
            Some(policy()),
            false,
        )
        .unwrap();
        let result = factory
            .check(&headers(), peer(), 19, Action::CreateTopic)
            .await
            .unwrap();
        assert_eq!(result.protection(), RateProtection::NoProtection);
        assert_eq!(result.rule_version(), 71);
        assert!(result.binding(19, Action::CreateTopic).is_ok());
        assert!(result.binding(20, Action::CreateTopic).is_err());
        assert!(result.binding(19, Action::CreateReply).is_err());
        let json = serde_json::to_string(result.signals()).unwrap();
        assert!(!json.contains("192.0.2"));
        assert!(!json.contains("synthetic_opaque"));
        assert!(!format!("{factory:?} {result:?}").contains("19"));
        let signal = factory
            .digest(
                b"bbbcm.content.risk.v1\0",
                71,
                Action::CreateTopic,
                "account",
                "19",
            )
            .unwrap();
        let bucket = factory
            .digest(
                b"bbbcm.content.rate.v1\0",
                71,
                Action::CreateTopic,
                "account",
                "19",
            )
            .unwrap();
        assert_ne!(signal, bucket);
    }
    #[tokio::test]
    async fn enabled_uses_four_dimensions_and_limit_is_separate_from_daily_quota() {
        let limiter = Arc::new(TokenBucketLimiter::new());
        let factory =
            ContentRiskFactory::new(vec![7; 32], true, limiter.clone(), Some(policy()), false)
                .unwrap();
        assert_eq!(
            factory
                .check(&headers(), peer(), 19, Action::CreateTopic)
                .await
                .unwrap()
                .protection(),
            RateProtection::Protected
        );
        assert_eq!(limiter.tracked_keys().unwrap(), 4);
        factory
            .check(&headers(), peer(), 19, Action::CreateTopic)
            .await
            .unwrap();
        assert!(matches!(
            factory
                .check(&headers(), peer(), 19, Action::CreateTopic)
                .await,
            Err(ContentError::RateLimited)
        ));
        factory
            .check(&headers(), peer(), 19, Action::CreateReply)
            .await
            .unwrap();
        assert_eq!(limiter.tracked_keys().unwrap(), 8);
    }
    struct Unavailable;
    impl RateLimiter for Unavailable {
        fn check<'a>(
            &'a self,
            _: &'a RateLimitKey,
            _: RateLimitPolicy,
        ) -> platform::cache::BoxFuture<'a, Result<RateLimitDecision, CacheError>> {
            Box::pin(async { Err(CacheError::Unavailable) })
        }
    }
    #[tokio::test]
    async fn enabled_failure_disabled_adapter_missing_peer_and_bad_device_fail_closed() {
        for limiter in [
            Arc::new(Unavailable) as Arc<dyn RateLimiter>,
            Arc::new(DisabledRateLimiter),
        ] {
            let factory =
                ContentRiskFactory::new(vec![7; 32], true, limiter, Some(policy()), false).unwrap();
            assert!(matches!(
                factory
                    .check(&headers(), peer(), 19, Action::CreateTopic)
                    .await,
                Err(ContentError::Unavailable)
            ));
        }
        let factory = ContentRiskFactory::new(
            vec![7; 32],
            true,
            Arc::new(TokenBucketLimiter::new()),
            Some(policy()),
            false,
        )
        .unwrap();
        assert!(
            factory
                .check(&headers(), None, 19, Action::CreateTopic)
                .await
                .is_err()
        );
        let mut duplicate = headers();
        duplicate.append(
            header::COOKIE,
            "bcm_device=synthetic_other_device_0002".parse().unwrap(),
        );
        assert!(
            factory
                .check(&duplicate, peer(), 19, Action::CreateTopic)
                .await
                .is_err()
        );
    }
    #[derive(Default)]
    struct Keys(Mutex<Vec<RateLimitKey>>);
    impl RateLimiter for Keys {
        fn check<'a>(
            &'a self,
            key: &'a RateLimitKey,
            _: RateLimitPolicy,
        ) -> platform::cache::BoxFuture<'a, Result<RateLimitDecision, CacheError>> {
            Box::pin(async move {
                self.0.lock().unwrap().push(key.clone());
                Ok(RateLimitDecision {
                    allowed: true,
                    retry_after: Duration::ZERO,
                })
            })
        }
    }
    #[tokio::test]
    async fn forwarded_headers_cannot_change_peer_ip_or_network_buckets() {
        let keys = Arc::new(Keys::default());
        let factory =
            ContentRiskFactory::new(vec![7; 32], true, keys.clone(), Some(policy()), false)
                .unwrap();
        factory
            .check(&headers(), peer(), 19, Action::CreateTopic)
            .await
            .unwrap();
        let mut forged = headers();
        forged.insert("x-forwarded-for", "198.51.100.200".parse().unwrap());
        forged.insert("forwarded", "for=198.51.100.200".parse().unwrap());
        factory
            .check(&forged, peer(), 19, Action::CreateTopic)
            .await
            .unwrap();
        let keys = keys.0.lock().unwrap();
        assert_eq!(&keys[..4], &keys[4..]);
    }
}
