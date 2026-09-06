//! 身份领域：用户、凭证、会话、邀请、邮箱验证与 MFA。
//! 业务实现随 WP-MVP-B01 至 B04 接入。

pub mod challenge;
mod challenge_repo;
#[cfg(test)]
mod challenge_tests;
pub mod contract;
pub mod core;
pub mod http;
pub mod management;
pub mod management_repo;
pub mod mfa;
pub mod password_screen;
pub mod policy;
pub mod repo;
pub mod settings;
mod settings_repo;
mod token_reissue;
#[cfg(test)]
mod token_reissue_tests;
pub mod web_view;

#[cfg(test)]
mod settings_tests;

pub mod mail_source;

pub mod rate;

#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod core_tests;

#[cfg(test)]
mod http_tests;

#[cfg(test)]
mod approved_tests;

#[cfg(test)]
mod content_access_tests;
