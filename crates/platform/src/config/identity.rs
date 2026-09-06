//! Identity assembly inputs. Secrets never deserialize from TOML or appear in Debug.
use super::{ConfigError, Environment, SecretString};
use crate::integrations::AdapterMode;
use serde::{
    Deserialize,
    de::{MapAccess, Visitor},
};
use std::{collections::BTreeMap, fmt};

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IdentityConfig {
    pub password_minimum: usize,
    pub password_corpus_path: Option<String>,
    pub password_corpus_sha256: Option<String>,
    pub active_token_version: Option<String>,
    pub mail_from: Option<String>,
    pub mail_base_url: Option<String>,
    #[serde(skip)]
    pub token_keys: Option<SecretString>,
    #[serde(skip)]
    pub risk_key: Option<SecretString>,
    /// MFA 共享密钥的包裹密钥（`DEC-062`）。
    ///
    /// 与邮件、风险密钥彼此独立：任何一处泄露都不应横向解开另一处，
    /// 因此校验里显式拒绝三者取值相同。未注入时 TOTP 绑定与验签一律失败关闭。
    #[serde(skip)]
    pub mfa_key: Option<SecretString>,
}
impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            password_minimum: 8,
            password_corpus_path: None,
            password_corpus_sha256: None,
            active_token_version: None,
            mail_from: None,
            mail_base_url: None,
            token_keys: None,
            risk_key: None,
            mfa_key: None,
        }
    }
}
fn valid_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
fn valid_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || v == b'_')
}
struct UniqueKeys(BTreeMap<String, String>);
impl<'de> Deserialize<'de> for UniqueKeys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Keys;
        impl<'de> Visitor<'de> for Keys {
            type Value = UniqueKeys;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a unique versioned key map")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut keys = BTreeMap::new();
                while let Some((version, key)) = map.next_entry::<String, String>()? {
                    if keys.len() >= 16
                        || !valid_version(&version)
                        || !valid_hex(&key)
                        || keys.insert(version, key).is_some()
                    {
                        return Err(serde::de::Error::custom("invalid key map"));
                    }
                }
                Ok(UniqueKeys(keys))
            }
        }
        deserializer.deserialize_map(Keys)
    }
}
impl IdentityConfig {
    /// Strict duplicate rejection prevents silently replacing a retained version.
    pub fn token_keyring(&self) -> Result<BTreeMap<String, SecretString>, ConfigError> {
        let Some(raw) = self.token_keys.as_ref() else {
            return Ok(BTreeMap::new());
        };
        if raw.len() > 8192 {
            return Err(ConfigError::Invalid(
                "identity token keyring exceeds its input bound",
            ));
        }
        let keys = serde_json::from_str::<UniqueKeys>(raw)
            .map_err(|_| {
                ConfigError::Invalid("identity token keyring is malformed; values hidden")
            })?
            .0;
        if keys.is_empty() {
            return Err(ConfigError::Invalid(
                "identity token keyring cannot be empty",
            ));
        }
        Ok(keys
            .into_iter()
            .map(|(version, key)| (version, key.into()))
            .collect())
    }
    pub fn validate(&self, environment: Environment, mode: AdapterMode) -> Result<(), ConfigError> {
        if !(8..=256).contains(&self.password_minimum) {
            return Err(ConfigError::Invalid(
                "identity.password_minimum must be 8..256",
            ));
        }
        match (&self.password_corpus_path, &self.password_corpus_sha256) {
            (None, None) => {}
            (Some(path), Some(digest))
                if !path.trim().is_empty()
                    && !path.contains(['\0', '\r', '\n'])
                    && valid_hex(digest) => {}
            _ => {
                return Err(ConfigError::Invalid(
                    "identity offline corpus needs a path and SHA256",
                ));
            }
        }
        if let Some(key) = &self.risk_key
            && !valid_hex(key)
        {
            return Err(ConfigError::Invalid(
                "identity risk key must be 32 random bytes encoded as lowercase hex",
            ));
        }
        if let Some(key) = &self.mfa_key
            && !valid_hex(key)
        {
            return Err(ConfigError::Invalid(
                "identity mfa key must be 32 random bytes encoded as lowercase hex",
            ));
        }
        let keys = self.token_keyring()?;
        if let Some(risk) = &self.risk_key
            && keys.values().any(|key| **key == **risk)
        {
            return Err(ConfigError::Invalid(
                "identity mail and risk keys must be independent",
            ));
        }
        // DEC-062：MFA 包裹密钥必须与邮件、风险密钥彼此独立。
        // 取值相同等于「看起来隔离、实际同根」，比不隔离更容易误判安全性。
        if let Some(mfa) = &self.mfa_key
            && (self.risk_key.as_ref().is_some_and(|risk| **risk == **mfa)
                || keys.values().any(|key| **key == **mfa))
        {
            return Err(ConfigError::Invalid(
                "identity mfa key must be independent of mail and risk keys",
            ));
        }
        if self.active_token_version.is_some()
            || !keys.is_empty()
            || self.mail_from.is_some()
            || self.mail_base_url.is_some()
        {
            let Some(_) = self
                .active_token_version
                .as_ref()
                .filter(|v| valid_version(v) && keys.contains_key(*v))
            else {
                return Err(ConfigError::Invalid(
                    "identity active token version must name an injected key",
                ));
            };
            if self.risk_key.is_none() {
                return Err(ConfigError::Invalid(
                    "identity local assembly requires an independent risk key",
                ));
            }
            if environment.is_production_like() || mode != AdapterMode::LocalTest {
                return Err(ConfigError::Invalid(
                    "identity mail assembly currently requires development/test local_test provider",
                ));
            }
            let from = self
                .mail_from
                .as_deref()
                .ok_or(ConfigError::Invalid("identity local mail needs a sender"))?;
            if from.is_empty()
                || from.len() > 254
                || !from.is_ascii()
                || from.chars().any(char::is_control)
                || !from.strip_suffix("@example.invalid").is_some_and(|local| {
                    !local.is_empty()
                        && local.len() <= 64
                        && !local.starts_with('.')
                        && !local.ends_with('.')
                        && !local.contains("..")
                        && local
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"._%+-".contains(&c))
                })
            {
                return Err(ConfigError::Invalid(
                    "identity local sender must use example.invalid",
                ));
            }
            let base = self.mail_base_url.as_deref().ok_or(ConfigError::Invalid(
                "identity local mail needs a loopback URL",
            ))?;
            let authority = base
                .strip_prefix("http://")
                .map(|v| v.strip_suffix('/').unwrap_or(v));
            if !authority
                .and_then(|v| v.split_once(':'))
                .is_some_and(|(host, port)| {
                    matches!(host, "127.0.0.1" | "localhost")
                        && port.bytes().all(|b| b.is_ascii_digit())
                        && port.parse::<u16>().is_ok_and(|p| p > 0)
                })
            {
                return Err(ConfigError::Invalid(
                    "identity local mail URL must be a loopback HTTP origin with a port",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn configured() -> IdentityConfig {
        IdentityConfig {
            active_token_version: Some("v1".into()),
            mail_from: Some("noreply@example.invalid".into()),
            mail_base_url: Some("http://127.0.0.1:18099".into()),
            token_keys: Some(format!("{{\"v1\":\"{}\"}}", "a".repeat(64)).into()),
            risk_key: Some("b".repeat(64).into()),
            ..Default::default()
        }
    }
    #[test]
    fn keys_are_injected_redacted_distinct_and_versioned() {
        let config = configured();
        assert!(
            config
                .validate(Environment::Test, AdapterMode::LocalTest)
                .is_ok()
        );
        assert!(!format!("{config:?}").contains(&"a".repeat(64)));
        assert!(!format!("{:?}", config.token_keyring().unwrap()).contains(&"a".repeat(64)));
        assert!(
            config
                .validate(Environment::Production, AdapterMode::LocalTest)
                .is_err()
        );
        assert!(
            config
                .validate(Environment::Test, AdapterMode::Disabled)
                .is_err()
        );
        let mut invalid = configured();
        invalid.risk_key = Some("a".repeat(64).into());
        assert!(
            invalid
                .validate(Environment::Test, AdapterMode::LocalTest)
                .is_err()
        );
        let mut duplicate = configured();
        duplicate.token_keys = Some(
            format!(
                "{{\"v1\":\"{}\",\"v1\":\"{}\"}}",
                "a".repeat(64),
                "b".repeat(64)
            )
            .into(),
        );
        assert!(duplicate.token_keyring().is_err());
        let mut absent = configured();
        absent.active_token_version = Some("v2".into());
        assert!(
            absent
                .validate(Environment::Test, AdapterMode::LocalTest)
                .is_err()
        );
        let mut url = configured();
        url.mail_base_url = Some("http://127.0.0.1:1234@external.invalid".into());
        assert!(
            url.validate(Environment::Test, AdapterMode::LocalTest)
                .is_err()
        );
        let raw = format!("[identity]\ntoken_keys = '{}'\n", "private-input");
        assert!(toml::from_str::<super::super::Config>(&raw).is_err());
    }
}
