//! 身份领域：用户、凭证、会话、邀请、邮箱验证与 MFA。
//! 业务实现随 WP-MVP-B01 至 B04 接入。

pub mod contract;
pub mod core;
pub mod http;
pub mod repo;
pub mod settings;
mod settings_repo;
pub mod web_view;

#[cfg(test)]
mod settings_tests;

pub mod mail_source;

pub mod rate;

#[cfg(test)]
mod core_tests;

#[cfg(test)]
mod http_tests;
