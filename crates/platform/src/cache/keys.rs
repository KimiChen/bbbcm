use super::CacheError;
use crate::config::Environment;
use sha2::{Digest, Sha256};

/// Environment is an enum, never a request field or raw namespace string.
#[derive(Debug, Clone, Copy)]
pub struct Namespace(Environment);
impl Namespace {
    pub fn new(environment: Environment) -> Self {
        Self(environment)
    }
    pub fn prefix(&self) -> String {
        format!("bcm:{}:v1:", self.0)
    }
    pub fn cache_key(&self, key: &PublicCacheKey) -> String {
        format!("{}cache:{}", self.prefix(), key.suffix())
    }
    pub fn cache_entity_prefix(&self, entity: &CacheEntity) -> String {
        format!("{}cache:{}:", self.prefix(), entity.suffix())
    }
    pub fn lease_key(&self, key: &LeaseKey) -> String {
        format!("{}lease:{}", self.prefix(), key.0)
    }
    pub fn rate_limit_key(&self, key: &RateLimitKey) -> String {
        format!("{}ratelimit:{}", self.prefix(), key.0)
    }
    pub fn wakeup_key(&self) -> String {
        format!("{}wakeup:outbox", self.prefix())
    }
    pub fn contains(&self, key: &str) -> bool {
        key.starts_with(&self.prefix())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheEntity {
    kind: String,
    id: String,
}
impl CacheEntity {
    /// kind is supplied by trusted application code / a validated Outbox producer;
    /// id must be an opaque positive ID or operation UUID, never a name or address.
    pub fn new(kind: &str, id: &str) -> Result<Self, CacheError> {
        validate_kind(kind)?;
        if !(crate::transaction::validate_operation_id(id)
            || id
                .parse::<i64>()
                .is_ok_and(|n| n > 0 && n.to_string() == id))
        {
            return Err(CacheError::InvalidInput);
        }
        Ok(Self {
            kind: kind.into(),
            id: digest(id),
        })
    }
    pub(super) fn suffix(&self) -> String {
        format!("{}:{}", self.kind, self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheVisibility {
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PublicCacheKey {
    entity: CacheEntity,
    version: u64,
}
impl PublicCacheKey {
    pub fn new(
        entity: CacheEntity,
        version: u64,
        visibility: CacheVisibility,
    ) -> Result<Self, CacheError> {
        if version == 0 || visibility != CacheVisibility::Public {
            return Err(CacheError::InvalidInput);
        }
        Ok(Self { entity, version })
    }
    pub fn entity(&self) -> &CacheEntity {
        &self.entity
    }
    pub(super) fn suffix(&self) -> String {
        format!("{}:public:{}", self.entity.suffix(), self.version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LeaseKey(pub(super) String);
impl LeaseKey {
    pub fn new(kind: &str, opaque_identity: &str) -> Result<Self, CacheError> {
        Ok(Self(safe_key(kind, opaque_identity)?))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RateLimitKey(pub(super) String);
impl RateLimitKey {
    /// Callers pass a trusted dimension and normalized identity; only its digest enters
    /// Redis. Neither identity nor digest is appropriate as a metrics label.
    pub fn new(dimension: &str, identity: &str) -> Result<Self, CacheError> {
        Ok(Self(safe_key(dimension, identity)?))
    }
}
fn validate_kind(kind: &str) -> Result<(), CacheError> {
    if kind.len() > 32 || !crate::outbox::is_machine_name(kind) {
        return Err(CacheError::InvalidInput);
    }
    Ok(())
}
fn safe_key(kind: &str, identity: &str) -> Result<String, CacheError> {
    validate_kind(kind)?;
    if identity.is_empty() || identity.len() > 512 || identity.chars().any(char::is_control) {
        return Err(CacheError::InvalidInput);
    }
    Ok(format!("{kind}:{}", digest(identity)))
}
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
