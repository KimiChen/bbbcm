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

/// 成功读取数据库后的一致快照；缺项与注册默认值可被调用者明确区分。
#[derive(Debug, Clone)]
pub struct FlagSnapshot {
    values: Vec<FlagValue>,
}
#[derive(Debug, Clone, Copy)]
pub struct FlagValue {
    pub flag: Flag,
    pub enabled: bool,
    /// None 表示数据库缺项，按安全默认关闭；不是一次读取失败。
    pub version: Option<i64>,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FlagReadError {
    #[error("Feature Flag 存储不可用；未取得数据库快照")]
    Unavailable,
    #[error("Feature Flag 存储含未知项、非法值或损坏版本")]
    Corrupt,
}
impl FlagSnapshot {
    pub fn from_rows(rows: Vec<(String, bool, i64)>) -> Result<Self, FlagReadError> {
        let mut seen = HashSet::new();
        let mut values: Vec<_> = Flag::ALL
            .iter()
            .map(|flag| FlagValue {
                flag: *flag,
                enabled: false,
                version: None,
            })
            .collect();
        for (key, enabled, version) in rows {
            let Some(value) = values.iter_mut().find(|v| v.flag.key() == key) else {
                return Err(FlagReadError::Corrupt);
            };
            if version < 1 || !seen.insert(key) {
                return Err(FlagReadError::Corrupt);
            }
            value.enabled = enabled;
            value.version = Some(version);
        }
        Ok(Self { values })
    }
    pub fn values(&self) -> &[FlagValue] {
        &self.values
    }
    pub fn get(&self, flag: Flag) -> FlagValue {
        *self
            .values
            .iter()
            .find(|v| v.flag == flag)
            .expect("snapshot contains registry")
    }
}
/// 只读合同，不提供写入口或启动修复。
pub trait FlagReader: Send + Sync {
    fn load(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<FlagSnapshot, FlagReadError>> + Send + '_>,
    >;
}
