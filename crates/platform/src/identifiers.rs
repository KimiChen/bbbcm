//! SchemaRenderer：物理对象名的唯一渲染入口。
//!
//! PostgreSQL 渲染 `<前缀><逻辑 schema>.<表名>`，SQLite/MySQL 渲染
//! `<前缀><逻辑 schema>_<表名>`（`DEC-029`）。扁平后仍带逻辑 schema，
//! 是因为 15 个逻辑域里 `sessions`、`jobs`、`evidence` 这类表名必然跨域重名，
//! 而共用一个 SQLite 文件或 MySQL 库时也需要与其他应用隔离。
//!
//! 「受信」体现在两处：前缀在构造时校验一次，`(schema, table)` 在每次渲染时
//! 对受信注册表校验——不在册的逻辑 schema 或非法表名一律拒绝，杜绝拼接不受信标识符。

use crate::db::Backend;
use thiserror::Error;

pub const MAX_PREFIX_LEN: usize = 32;
/// PostgreSQL 标识符上限；渲染结果超出会被静默截断，必须提前拒绝。
pub const MAX_IDENTIFIER_LEN: usize = 63;

/// `02` 第 7 节的 15 个逻辑 schema，是唯一受信集合。
pub const LOGICAL_SCHEMAS: &[&str] = &[
    "platform",
    "identity",
    "iam",
    "community",
    "media",
    "interaction",
    "trust",
    "campaign",
    "commerce",
    "moderation",
    "support",
    "privacy",
    "operations",
    "audit",
    "analytics",
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SchemaError {
    #[error("前缀不能为空")]
    Empty,
    #[error("前缀必须以 ASCII 小写字母开头: {0}")]
    BadStart(String),
    #[error("前缀只能包含 [a-z0-9_]，且必须以 _ 结尾: {0}")]
    BadChars(String),
    #[error("前缀超过 {MAX_PREFIX_LEN} 个字符: {0}")]
    TooLong(String),
    #[error("逻辑 schema 不在受信注册表中: {0}")]
    UnknownSchema(String),
    #[error("表名必须匹配 [a-z][a-z0-9_]*: {0}")]
    BadTable(String),
    #[error("渲染结果 {0} 超过 {MAX_IDENTIFIER_LEN} 字节")]
    IdentifierTooLong(String),
}

#[derive(Debug, Clone)]
pub struct SchemaRenderer {
    prefix: String,
}

fn is_snake_lower(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl SchemaRenderer {
    /// 校验并固定前缀；默认及生产固定为 `bcm_`。
    pub fn new(prefix: &str) -> Result<Self, SchemaError> {
        if prefix.is_empty() {
            return Err(SchemaError::Empty);
        }
        let first = prefix.chars().next().expect("已检查非空");
        if !first.is_ascii_lowercase() {
            return Err(SchemaError::BadStart(prefix.to_string()));
        }
        if !prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(SchemaError::BadChars(prefix.to_string()));
        }
        if !prefix.ends_with('_') {
            return Err(SchemaError::BadChars(prefix.to_string()));
        }
        if prefix.chars().count() > MAX_PREFIX_LEN {
            return Err(SchemaError::TooLong(prefix.to_string()));
        }
        Ok(Self {
            prefix: prefix.to_string(),
        })
    }

    /// 按后端渲染物理表名；查询值一律使用 bind 参数，不经此渲染。
    pub fn table(
        &self,
        backend: Backend,
        schema: &str,
        table: &str,
    ) -> Result<String, SchemaError> {
        if !LOGICAL_SCHEMAS.contains(&schema) {
            return Err(SchemaError::UnknownSchema(schema.to_string()));
        }
        if !is_snake_lower(table) {
            return Err(SchemaError::BadTable(table.to_string()));
        }
        let rendered = match backend {
            Backend::Postgres => format!("{}{}.{}", self.prefix, schema, table),
            Backend::Sqlite | Backend::Mysql8 => {
                format!("{}{}_{}", self.prefix, schema, table)
            }
        };
        // PostgreSQL 按 schema 与 table 分别限长，扁平形态则整体是一个标识符。
        let longest = match backend {
            Backend::Postgres => (self.prefix.len() + schema.len()).max(table.len()),
            Backend::Sqlite | Backend::Mysql8 => rendered.len(),
        };
        if longest > MAX_IDENTIFIER_LEN {
            return Err(SchemaError::IdentifierTooLong(rendered));
        }
        Ok(rendered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> SchemaRenderer {
        SchemaRenderer::new("bcm_").expect("bcm_ 必须合法")
    }

    #[test]
    fn accepts_default_prefix() {
        let r = renderer();
        assert_eq!(
            r.table(Backend::Postgres, "identity", "users")
                .expect("应渲染成功"),
            "bcm_identity.users"
        );
        assert_eq!(
            r.table(Backend::Sqlite, "identity", "users")
                .expect("应渲染成功"),
            "bcm_identity_users"
        );
        assert_eq!(
            r.table(Backend::Mysql8, "identity", "users")
                .expect("应渲染成功"),
            "bcm_identity_users"
        );
    }

    #[test]
    fn rejects_invalid_prefixes() {
        assert!(matches!(SchemaRenderer::new(""), Err(SchemaError::Empty)));
        assert!(matches!(
            SchemaRenderer::new("Bcm_"),
            Err(SchemaError::BadStart(_))
        ));
        assert!(matches!(
            SchemaRenderer::new("bcm"),
            Err(SchemaError::BadChars(_))
        ));
        assert!(matches!(
            SchemaRenderer::new("bcm-1_"),
            Err(SchemaError::BadChars(_))
        ));
        assert!(matches!(
            SchemaRenderer::new(&format!("{}_", "a".repeat(32))),
            Err(SchemaError::TooLong(_))
        ));
    }

    #[test]
    fn rejects_unknown_schema() {
        assert_eq!(
            renderer().table(Backend::Postgres, "not_a_schema", "users"),
            Err(SchemaError::UnknownSchema("not_a_schema".into()))
        );
    }

    #[test]
    fn rejects_untrusted_table_names() {
        let r = renderer();
        for bad in ["Users", "users; drop table x", "1users", "users-x", ""] {
            assert!(
                matches!(
                    r.table(Backend::Postgres, "identity", bad),
                    Err(SchemaError::BadTable(_))
                ),
                "表名 {bad:?} 应被拒绝"
            );
        }
    }

    #[test]
    fn rejects_over_long_identifier() {
        let r = renderer();
        let long = "a".repeat(60);
        assert!(matches!(
            r.table(Backend::Sqlite, "interaction", &long),
            Err(SchemaError::IdentifierTooLong(_))
        ));
    }

    /// 扁平渲染必须全局唯一：这正是 `DEC-029` 要解决的问题。
    #[test]
    fn flat_rendering_is_unique_across_schemas() {
        let r = renderer();
        let a = r
            .table(Backend::Sqlite, "identity", "sessions")
            .expect("应渲染成功");
        let b = r
            .table(Backend::Sqlite, "iam", "sessions")
            .expect("应渲染成功");
        assert_ne!(a, b, "同名表在不同逻辑 schema 下必须渲染为不同物理名");
    }

    #[test]
    fn every_logical_schema_renders() {
        let r = renderer();
        for s in LOGICAL_SCHEMAS {
            r.table(Backend::Postgres, s, "t")
                .unwrap_or_else(|e| panic!("{s} 应可渲染: {e}"));
        }
        assert_eq!(LOGICAL_SCHEMAS.len(), 15, "02 第 7 节定义 15 个逻辑 schema");
    }
}
