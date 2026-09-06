//! The MOD-010 identity slice only reads immutable consent versions.
pub use platform::policy::{PolicyError, PolicySet, PolicyVersion};

/// Closed content-only restriction scope, independent of unapproved staff permissions.
#[derive(Debug, Clone, Copy)]
pub enum ContentCapability {
    Topic,
    Reply,
    Edit,
    Delete,
}
impl ContentCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::Reply => "reply",
            Self::Edit => "edit",
            Self::Delete => "delete",
        }
    }
}
