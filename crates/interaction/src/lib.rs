//! 互动领域：收藏、正向反馈、关注、阅读位置与站内通知（`WP-MVP-C04`）。
//!
//! 本轮交付 `SOCIAL-002`／`SOCIAL-003` 的正向反馈；其余需求随后续切片接入。
pub mod contract;
pub mod core;
pub mod http;
pub mod notification;
pub mod repo;

#[cfg(test)]
mod feedback_tests;
