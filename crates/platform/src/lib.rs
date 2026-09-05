//! 平台基础库：配置、数据库后端抽象、标识符渲染、Feature Flag、缓存/限流、错误与通用类型。
//! 与业务无关；领域 crate 依赖本 crate，反向禁止。

pub mod cache;
pub mod config;
pub mod db;
pub mod error;
pub mod flags;
pub mod identifiers;
pub mod types;
