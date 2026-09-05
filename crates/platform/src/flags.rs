//! Feature Flag 唯一注册点：新增 flag 必须先在此登记再引用。
//! MVP 约定：所有能力 flag 默认关闭，紧急锁（RegistrationLock 等）同样默认不启用，
//! 由受控管理命令按需开启。

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flag {
    PublicRead,
    Registration,
    EmailVerification,
    TopicCreation,
    ReplyCreation,
    Attachments,
    Search,
    Rss,
    Reporting,
    Support,
    ManageConsole,
    RegistrationLock,
    InvitationLock,
    ReadOnlyMode,
}

impl Flag {
    pub const ALL: &'static [Flag] = &[
        Flag::PublicRead,
        Flag::Registration,
        Flag::EmailVerification,
        Flag::TopicCreation,
        Flag::ReplyCreation,
        Flag::Attachments,
        Flag::Search,
        Flag::Rss,
        Flag::Reporting,
        Flag::Support,
        Flag::ManageConsole,
        Flag::RegistrationLock,
        Flag::InvitationLock,
        Flag::ReadOnlyMode,
    ];

    /// 持久化与配置使用的稳定 key。
    pub fn key(self) -> &'static str {
        match self {
            Flag::PublicRead => "public_read",
            Flag::Registration => "registration",
            Flag::EmailVerification => "email_verification",
            Flag::TopicCreation => "topic_creation",
            Flag::ReplyCreation => "reply_creation",
            Flag::Attachments => "attachments",
            Flag::Search => "search",
            Flag::Rss => "rss",
            Flag::Reporting => "reporting",
            Flag::Support => "support",
            Flag::ManageConsole => "manage_console",
            Flag::RegistrationLock => "registration_lock",
            Flag::InvitationLock => "invitation_lock",
            Flag::ReadOnlyMode => "read_only_mode",
        }
    }

    /// 所有 flag 默认关闭。
    pub fn default_enabled(self) -> bool {
        false
    }
}

/// 进程内 flag 视图；持久化存储与受控修改命令随后续工作包接入。
#[derive(Debug, Clone, Default)]
pub struct FlagSet {
    enabled: HashSet<&'static str>,
}

impl FlagSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enable(&mut self, flag: Flag) {
        self.enabled.insert(flag.key());
    }

    pub fn disable(&mut self, flag: Flag) {
        self.enabled.remove(flag.key());
    }

    pub fn is_enabled(&self, flag: Flag) -> bool {
        self.enabled.contains(flag.key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_keys_are_unique() {
        let keys: HashSet<_> = Flag::ALL.iter().map(|flag| flag.key()).collect();
        assert_eq!(keys.len(), Flag::ALL.len(), "flag key 必须唯一");
    }

    #[test]
    fn all_flags_default_off() {
        assert!(Flag::ALL.iter().all(|flag| !flag.default_enabled()));
    }
}
