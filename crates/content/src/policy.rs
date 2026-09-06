//! Explicit policy injection. No production publishing policy is approved by DEC-052.
use crate::contract::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstPostDeletion {
    PolicyPending,
    HideTopic,
}
#[derive(Debug, Clone)]
pub struct ContentRules {
    pub version: i64,
    pub allow_pending: bool,
    pub allow_t0: bool,
    pub allow_t1: bool,
    pub minimum_title_chars: usize,
    pub maximum_title_chars: usize,
    pub minimum_body_chars: usize,
    pub maximum_body_chars: usize,
    pub minimum_reply_chars: usize,
    pub maximum_reply_chars: usize,
    pub maximum_tags: usize,
    pub edit_window_seconds: i64,
    pub delete_window_seconds: i64,
    pub first_post_deletion: FirstPostDeletion,
    pub t0_daily_topics: i64,
    pub t0_daily_replies: i64,
    pub t1_daily_topics: i64,
    pub t1_daily_replies: i64,
    pub early_topics: i64,
    pub early_replies: i64,
    pub day_utc_offset_minutes: i32,
    pub count_pending: bool,
    pub max_external_links_t0: usize,
    pub max_external_links_t1: usize,
    /// Structural published replies; this does not declare valid-contribution semantics.
    pub show_structural_reply_count: bool,
    pub show_last_public_reply: bool,
}
pub trait ContentPolicy: Send + Sync {
    fn rules(&self) -> Result<ContentRules, ContentError>;
}
pub struct UnconfiguredContentPolicy;
impl ContentPolicy for UnconfiguredContentPolicy {
    fn rules(&self) -> Result<ContentRules, ContentError> {
        Err(ContentError::PolicyPending)
    }
}
/// Only instantiate with explicitly labelled synthetic test values. Never runtime default.
pub struct SyntheticContentPolicy(pub ContentRules);
impl ContentPolicy for SyntheticContentPolicy {
    fn rules(&self) -> Result<ContentRules, ContentError> {
        self.0.validate()?;
        Ok(self.0.clone())
    }
}
impl ContentRules {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.version <= 0
            || self.minimum_title_chars == 0
            || self.minimum_title_chars > self.maximum_title_chars
            || self.minimum_body_chars == 0
            || self.minimum_body_chars > self.maximum_body_chars
            || self.minimum_reply_chars == 0
            || self.minimum_reply_chars > self.maximum_reply_chars
            || self.edit_window_seconds < 0
            || self.delete_window_seconds < 0
            || chrono::Duration::try_seconds(self.edit_window_seconds).is_none()
            || chrono::Duration::try_seconds(self.delete_window_seconds).is_none()
            || self.day_utc_offset_minutes.unsigned_abs() > 840
            || [
                self.t0_daily_topics,
                self.t0_daily_replies,
                self.t1_daily_topics,
                self.t1_daily_replies,
                self.early_topics,
                self.early_replies,
            ]
            .iter()
            .any(|v| *v < 0)
        {
            Err(ContentError::PolicyPending)
        } else {
            Ok(())
        }
    }
    pub fn authorize(&self, facts: &AuthorityFacts) -> Result<(), ContentError> {
        self.validate()?;
        if !facts.allowed
            || !facts.email_verified
            || !match facts.tier {
                TrustTier::Pending => self.allow_pending,
                TrustTier::T0 => self.allow_t0,
                TrustTier::T1 => self.allow_t1,
            }
        {
            Err(ContentError::Forbidden)
        } else {
            Ok(())
        }
    }
    pub fn validate_text(
        &self,
        title: Option<&str>,
        body: &str,
        is_reply: bool,
        tags: usize,
        external: usize,
        tier: TrustTier,
    ) -> Result<(), ContentError> {
        let (min, max) = if is_reply {
            (self.minimum_reply_chars, self.maximum_reply_chars)
        } else {
            (self.minimum_body_chars, self.maximum_body_chars)
        };
        let count = body.trim().chars().count();
        if count < min
            || count > max
            || tags > self.maximum_tags
            || title.is_some_and(|s| {
                let n = s.trim().chars().count();
                n < self.minimum_title_chars
                    || n > self.maximum_title_chars
                    || s.chars().any(char::is_control)
            })
        {
            return Err(ContentError::InvalidInput);
        }
        let links = if tier == TrustTier::T1 {
            self.max_external_links_t1
        } else {
            self.max_external_links_t0
        };
        if external > links {
            return Err(ContentError::Forbidden);
        }
        Ok(())
    }
}
