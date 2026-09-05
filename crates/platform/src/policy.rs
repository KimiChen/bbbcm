//! Policy consent references shared without a domain or database dependency.
//! A policy reference is an immutable resource ID, never client-supplied policy text.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicySet {
    pub terms: i64,
    pub privacy: i64,
    pub community: i64,
}
impl PolicySet {
    pub fn validate(self) -> Result<(), PolicyError> {
        if self.terms <= 0
            || self.privacy <= 0
            || self.community <= 0
            || self.terms == self.privacy
            || self.terms == self.community
            || self.privacy == self.community
        {
            return Err(PolicyError::InvalidInput);
        }
        Ok(())
    }
    pub fn references(self) -> [(&'static str, i64); 3] {
        [
            ("terms", self.terms),
            ("privacy", self.privacy),
            ("community", self.community),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyVersion {
    pub id: i64,
    pub kind: &'static str,
    pub version: i64,
    pub body: String,
    pub synthetic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("政策版本引用无效")]
    InvalidInput,
    #[error("当前政策版本尚未就绪")]
    NotReady,
    #[error("政策版本已变化，请重新阅读并确认")]
    Changed,
    #[error("政策读取不可用")]
    Unavailable,
}
