//! 物理对象名的唯一入口；仅接受已登记的逻辑 schema、表、序列与数据库角色。
//! 规范见 plan/02 §6–8；注册结构随对应迁移交付，不为未来业务预注册空表。

use crate::db::Backend;
use thiserror::Error;

pub const MAX_PREFIX_LEN: usize = 32;
pub const MAX_IDENTIFIER_LEN: usize = 63;
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

#[derive(Debug, Clone, Copy)]
pub struct TableRegistration {
    pub schema: &'static str,
    pub table: &'static str,
    pub identity_column: Option<&'static str>,
}

pub const TABLES: &[TableRegistration] = &[
    TableRegistration {
        schema: "identity",
        table: "challenges",
        identity_column: None,
    },
    TableRegistration {
        schema: "identity",
        table: "mail_recipients",
        identity_column: None,
    },
    TableRegistration {
        schema: "identity",
        table: "users",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "identity",
        table: "credentials",
        identity_column: None,
    },
    TableRegistration {
        schema: "identity",
        table: "sessions",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "identity",
        table: "one_time_tokens",
        identity_column: None,
    },
    TableRegistration {
        schema: "identity",
        table: "invitations",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "identity",
        table: "policy_consents",
        identity_column: None,
    },
    TableRegistration {
        schema: "identity",
        table: "username_claims",
        identity_column: None,
    },
    TableRegistration {
        schema: "identity",
        table: "username_history",
        identity_column: None,
    },
    TableRegistration {
        schema: "identity",
        table: "mail_requests",
        identity_column: None,
    },
    TableRegistration {
        schema: "identity",
        table: "request_principals",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "trust",
        table: "identity_snapshots",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "trust",
        table: "invite_uses",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "trust",
        table: "risk_events",
        identity_column: None,
    },
    TableRegistration {
        schema: "moderation",
        table: "policy_versions",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "operations",
        table: "status_announcements",
        identity_column: None,
    },
    TableRegistration {
        schema: "analytics",
        table: "event_facts",
        identity_column: None,
    },
    TableRegistration {
        schema: "analytics",
        table: "projection_receipts",
        identity_column: None,
    },
    TableRegistration {
        schema: "analytics",
        table: "aggregates",
        identity_column: None,
    },
    TableRegistration {
        schema: "analytics",
        table: "watermarks",
        identity_column: None,
    },
    TableRegistration {
        schema: "platform",
        table: "mail_dispatches",
        identity_column: None,
    },
    TableRegistration {
        schema: "platform",
        table: "mail_feedback",
        identity_column: None,
    },
    TableRegistration {
        schema: "platform",
        table: "mail_suppressions",
        identity_column: None,
    },
    TableRegistration {
        schema: "platform",
        table: "mail_alerts",
        identity_column: None,
    },
    TableRegistration {
        schema: "platform",
        table: "idempotency_records",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "platform",
        table: "outbox_events",
        identity_column: None,
    },
    TableRegistration {
        schema: "platform",
        table: "sink_deliveries",
        identity_column: None,
    },
    TableRegistration {
        schema: "platform",
        table: "feature_flags",
        identity_column: None,
    },
    TableRegistration {
        schema: "audit",
        table: "audit_events",
        identity_column: None,
    },
    TableRegistration {
        schema: "audit",
        table: "sensitive_access_events",
        identity_column: None,
    },
    TableRegistration {
        schema: "platform",
        table: "schema_migrations",
        identity_column: None,
    },
    TableRegistration {
        schema: "community",
        table: "commit_state",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "community",
        table: "forums",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "community",
        table: "tags",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "community",
        table: "tag_aliases",
        identity_column: None,
    },
    TableRegistration {
        schema: "community",
        table: "topics",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "community",
        table: "posts",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "community",
        table: "post_revisions",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "community",
        table: "topic_tags",
        identity_column: None,
    },
    TableRegistration {
        schema: "community",
        table: "topic_history",
        identity_column: None,
    },
    TableRegistration {
        schema: "community",
        table: "publication_receipts",
        identity_column: None,
    },
    TableRegistration {
        schema: "community",
        table: "search_documents",
        identity_column: None,
    },
    TableRegistration {
        schema: "community",
        table: "post_links",
        identity_column: None,
    },
    TableRegistration {
        schema: "moderation",
        table: "content_restrictions",
        identity_column: Some("id"),
    },
    TableRegistration {
        schema: "moderation",
        table: "rule_versions",
        identity_column: Some("id"),
    },
];

/// 仅本轮引擎所需索引；迁移与表注册同时版本化。
pub const INDEXES: &[(&str, &str)] = &[
    ("community", "topic_forum_idx"),
    ("community", "topic_owner_idx"),
    ("community", "post_floor_idx"),
    ("community", "post_owner_idx"),
    ("community", "history_topic_idx"),
    ("community", "publication_owner_idx"),
    ("community", "search_topic_idx"),
    ("moderation", "content_restriction_owner_idx"),
    ("identity", "session_owner_idx"),
    ("identity", "invite_owner_idx"),
    ("identity", "token_owner_idx"),
    ("trust", "risk_actor_idx"),
    ("platform", "mail_recipient_idx"),
    ("platform", "mail_observed_idx"),
    ("platform", "mail_feedback_time_idx"),
    ("platform", "outbox_ready_idx"),
    ("platform", "outbox_lease_idx"),
    ("platform", "outbox_batch_idx"),
    ("platform", "sink_event_idx"),
    ("audit", "audit_actor_idx"),
    ("audit", "access_actor_idx"),
];

/// 数据库组角色，不是业务 IAM registry；不创建登录凭证或授予用户身份。
pub const DATABASE_ROLES: &[&str] = &["migrator", "app_rw", "worker_rw", "report_ro"];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SchemaError {
    #[error("前缀不能为空")]
    Empty,
    #[error("前缀必须以 ASCII 小写字母开头")]
    BadStart,
    #[error("前缀只能包含 [a-z0-9_]，且必须以 _ 结尾")]
    BadChars,
    #[error("前缀超过 {MAX_PREFIX_LEN} 个字符")]
    TooLong,
    #[error("逻辑 schema 不在受信注册表中")]
    UnknownSchema,
    #[error("表名必须匹配 [a-z][a-z0-9_]*")]
    BadTable,
    #[error("schema/table 不在受信注册表中")]
    UnknownTable,
    #[error("数据库角色不在受信注册表中")]
    UnknownRole,
    #[error("序列不在受信注册表中")]
    UnknownSequence,
    #[error("物理标识符超过 {MAX_IDENTIFIER_LEN} 字节")]
    IdentifierTooLong,
    #[error("迁移标识符模板非法或未登记")]
    InvalidTemplate,
}

#[derive(Debug, Clone)]
pub struct SchemaRenderer {
    prefix: String,
}

fn snake_lower(s: &str) -> bool {
    s.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn bounded(s: String) -> Result<String, SchemaError> {
    if s.len() > MAX_IDENTIFIER_LEN {
        Err(SchemaError::IdentifierTooLong)
    } else {
        Ok(s)
    }
}

impl SchemaRenderer {
    pub fn new(prefix: &str) -> Result<Self, SchemaError> {
        if prefix.is_empty() {
            return Err(SchemaError::Empty);
        }
        if !prefix.as_bytes()[0].is_ascii_lowercase() {
            return Err(SchemaError::BadStart);
        }
        if !snake_lower(prefix) || !prefix.ends_with('_') {
            return Err(SchemaError::BadChars);
        }
        if prefix.len() > MAX_PREFIX_LEN {
            return Err(SchemaError::TooLong);
        }
        let renderer = Self {
            prefix: prefix.to_owned(),
        };
        // 全注册表两种命名均须可用，不能等运行到某张表时才发现截断。
        for table in TABLES {
            renderer.table(Backend::Postgres, table.schema, table.table)?;
            renderer.table(Backend::Sqlite, table.schema, table.table)?;
        }
        Ok(renderer)
    }
    pub fn prefix(&self) -> &str {
        &self.prefix
    }
    pub fn schema(&self, logical: &str) -> Result<String, SchemaError> {
        if !LOGICAL_SCHEMAS.contains(&logical) {
            return Err(SchemaError::UnknownSchema);
        }
        bounded(format!("{}{logical}", self.prefix))
    }
    pub fn table(
        &self,
        backend: Backend,
        schema: &str,
        table: &str,
    ) -> Result<String, SchemaError> {
        let physical_schema = self.schema(schema)?;
        if !snake_lower(table) {
            return Err(SchemaError::BadTable);
        }
        if !TABLES
            .iter()
            .any(|t| t.schema == schema && t.table == table)
        {
            return Err(SchemaError::UnknownTable);
        }
        match backend {
            Backend::Postgres => {
                bounded(table.to_owned())?;
                Ok(format!("{physical_schema}.{table}"))
            }
            Backend::Sqlite | Backend::Mysql8 => bounded(format!("{physical_schema}_{table}")),
        }
    }
    pub fn index(&self, backend: Backend, schema: &str, name: &str) -> Result<String, SchemaError> {
        if !INDEXES.contains(&(schema, name)) {
            return Err(SchemaError::InvalidTemplate);
        }
        match backend {
            Backend::Postgres => bounded(name.to_owned()),
            Backend::Sqlite | Backend::Mysql8 => {
                bounded(format!("{}_{name}", self.schema(schema)?))
            }
        }
    }
    pub fn role(&self, role: &str) -> Result<String, SchemaError> {
        if !DATABASE_ROLES.contains(&role) {
            return Err(SchemaError::UnknownRole);
        }
        bounded(format!("{}{role}", self.prefix))
    }
    pub fn sequence(&self, schema: &str, table: &str, column: &str) -> Result<String, SchemaError> {
        self.table(Backend::Postgres, schema, table)?;
        if !TABLES
            .iter()
            .any(|t| t.schema == schema && t.table == table && t.identity_column == Some(column))
        {
            return Err(SchemaError::UnknownSequence);
        }
        let name = bounded(format!("{table}_{column}_seq"))?;
        Ok(format!("{}.{name}", self.schema(schema)?))
    }
    /// SQLite 重建用替代表：只能从已注册逻辑表派生固定后缀，仍做物理限长检查。
    pub fn rebuild_table(&self, schema: &str, table: &str) -> Result<String, SchemaError> {
        bounded(format!(
            "{}_rebuild",
            self.table(Backend::Sqlite, schema, table)?
        ))
    }
    /// 迁移模板只支持受信对象孔位，不接受自由 SQL 片段或请求输入。
    pub fn migration(&self, backend: Backend, sql: &str) -> Result<String, SchemaError> {
        let mut result = String::new();
        let mut rest = sql;
        while let Some(start) = rest.find("{{") {
            result.push_str(&rest[..start]);
            let tail = &rest[start + 2..];
            let end = tail.find("}}").ok_or(SchemaError::InvalidTemplate)?;
            let (kind, value) = tail[..end]
                .split_once(':')
                .ok_or(SchemaError::InvalidTemplate)?;
            let parts: Vec<_> = value.split('.').collect();
            let rendered = match (kind, parts.as_slice()) {
                ("schema", [schema]) if backend == Backend::Postgres => self.schema(schema)?,
                ("table", [schema, table]) => self.table(backend, schema, table)?,
                ("index", [schema, name]) => self.index(backend, schema, name)?,
                ("rebuild", [schema, table]) if backend == Backend::Sqlite => {
                    self.rebuild_table(schema, table)?
                }
                ("role", [role]) if backend == Backend::Postgres => self.role(role)?,
                ("sequence", [schema, table, column]) if backend == Backend::Postgres => {
                    self.sequence(schema, table, column)?
                }
                _ => return Err(SchemaError::InvalidTemplate),
            };
            result.push_str(&rendered);
            rest = &tail[end + 2..];
        }
        if rest.contains("}}") {
            return Err(SchemaError::InvalidTemplate);
        }
        result.push_str(rest);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_objects_render_under_both_prefixes() {
        for prefix in ["bcm_", "dev42_"] {
            let r = SchemaRenderer::new(prefix).unwrap();
            assert_eq!(LOGICAL_SCHEMAS.len(), 15);
            for s in LOGICAL_SCHEMAS {
                assert_eq!(r.schema(s).unwrap(), format!("{prefix}{s}"));
            }
            let mut flat = std::collections::BTreeSet::new();
            for t in TABLES {
                assert_eq!(
                    r.table(Backend::Postgres, t.schema, t.table).unwrap(),
                    format!("{prefix}{}.{}", t.schema, t.table)
                );
                assert!(flat.insert(r.table(Backend::Sqlite, t.schema, t.table).unwrap()));
            }
        }
    }
    #[test]
    fn refuses_unregistered_identifiers_and_invalid_prefixes() {
        for p in [
            "",
            "Bcm_",
            "bcm",
            "bcm-1_",
            "1b_",
            &format!("{}_", "a".repeat(32)),
        ] {
            assert!(SchemaRenderer::new(p).is_err());
        }
        let r = SchemaRenderer::new("bcm_").unwrap();
        assert_eq!(
            r.table(Backend::Sqlite, "identity", "unregistered_table"),
            Err(SchemaError::UnknownTable)
        );
        assert_eq!(
            r.table(Backend::Sqlite, "community", "valid_but_unknown"),
            Err(SchemaError::UnknownTable)
        );
        assert!(
            r.table(Backend::Postgres, "community", "forums; DROP TABLE t")
                .is_err()
        );
        assert!(r.schema("unregistered").is_err());
        assert!(r.role("site_owner").is_err());
        assert!(r.rebuild_table("community", "unregistered").is_err());
        assert_eq!(
            r.rebuild_table("community", "forums").unwrap(),
            "bcm_community_forums_rebuild"
        );
        assert!(r.sequence("community", "forums", "other").is_err());
    }
    #[test]
    fn template_replaces_only_registered_objects() {
        let r = SchemaRenderer::new("test_").unwrap();
        assert_eq!(
            r.migration(Backend::Sqlite, "SELECT * FROM {{table:community.forums}}")
                .unwrap(),
            "SELECT * FROM test_community_forums"
        );
        assert_eq!(
            r.sequence("community", "forums", "id").unwrap(),
            "test_community.forums_id_seq"
        );
        for bad in [
            "{{table:community.fake}}",
            "{{schema:fake}}",
            "{{table:community.forums}",
            "{{raw:DROP}}",
            "}}",
        ] {
            assert!(r.migration(Backend::Postgres, bad).is_err());
        }
    }
}
