//! PostgreSQL 初始化角色 SQL 的受信渲染；命令只输出，不自动取得集群权限。
use platform::identifiers::{DATABASE_ROLES, SchemaError, SchemaRenderer};

/// 由数据库管理员在新的隔离数据库执行一次。无登录口令、IAM 或 SiteOwner。
/// 所有物理角色/schema 名由 renderer 生成，数据库名由 psql 当前连接读取。
pub fn bootstrap_sql(renderer: &SchemaRenderer) -> Result<String, SchemaError> {
    let mut lines = vec![
        "-- Run once as a database administrator in a NEW isolated database using psql -v ON_ERROR_STOP=1.".to_owned(),
        "-- Roles are NOLOGIN groups. Provision login credentials outside this file; never use runtime logins for migration.".to_owned(),
        "BEGIN;".to_owned(),
    ];
    for role in DATABASE_ROLES {
        lines.push(format!("CREATE ROLE {} NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;", renderer.role(role)?));
    }
    // 当前数据库限定，不接受配置文本拼接库名。psql 的 :"..." 负责系统库标识符引用。
    lines.push("SELECT current_database() AS bcm_bootstrap_database \\gset".to_owned());
    lines.push(
        "REVOKE CREATE, TEMPORARY ON DATABASE :\"bcm_bootstrap_database\" FROM PUBLIC;".to_owned(),
    );
    lines.push("REVOKE CREATE ON SCHEMA public FROM PUBLIC;".to_owned());
    lines.push(format!(
        "GRANT CREATE ON DATABASE :\"bcm_bootstrap_database\" TO {};",
        renderer.role("migrator")?
    ));
    lines.push(format!(
        "GRANT CONNECT ON DATABASE :\"bcm_bootstrap_database\" TO {};",
        DATABASE_ROLES
            .iter()
            .map(|r| renderer.role(r))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ")
    ));
    lines.push("COMMIT;".to_owned());
    Ok(format!("{}\n", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform::db::Backend;
    #[test]
    fn all_initial_objects_have_explicit_acl_and_future_defaults_are_empty() {
        let source = [
            include_str!("../../../db/migrations/postgres/0001_init.sql"),
            include_str!("../../../db/migrations/postgres/0002_engine.sql"),
            include_str!("../../../db/migrations/postgres/0003_flags.sql"),
            include_str!("../../../db/migrations/postgres/0004_status.sql"),
            include_str!("../../../db/migrations/postgres/0005_analytics.sql"),
            include_str!("../../../db/migrations/postgres/0006_adapters.sql"),
            include_str!("../../../db/migrations/postgres/0007_identity.sql"),
            include_str!("../../../db/migrations/postgres/0008_identity_challenges.sql"),
            include_str!("../../../db/migrations/postgres/0009_content.sql"),
            include_str!("../../../db/migrations/postgres/0010_content_access.sql"),
            include_str!("../../../db/migrations/postgres/0011_trust_contributions.sql"),
            include_str!("../../../db/migrations/postgres/0012_privacy_and_views.sql"),
            include_str!("../../../db/migrations/postgres/0013_mfa_and_management.sql"),
            include_str!("../../../db/migrations/postgres/0014_reports_and_cases.sql"),
            include_str!("../../../db/migrations/postgres/0015_enforcement_and_appeals.sql"),
            include_str!("../../../db/migrations/postgres/0016_iam.sql"),
            include_str!("../../../db/migrations/postgres/0017_iam_seed.sql"),
            include_str!("../../../db/migrations/postgres/0018_search_normalized_column.sql"),
            include_str!("../../../db/migrations/postgres/0019_iam_schema_usage.sql"),
            include_str!("../../../db/migrations/postgres/0020_feedback.sql"),
            include_str!("../../../db/migrations/postgres/0021_notifications.sql"),
        ]
        .join("\n");
        for prefix in ["bcm_", "review42_"] {
            let r = SchemaRenderer::new(prefix).unwrap();
            let sql = r.migration(Backend::Postgres, &source).unwrap();
            let bootstrap = bootstrap_sql(&r).unwrap();
            for role in DATABASE_ROLES {
                assert!(bootstrap.contains(&format!("CREATE ROLE {} NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS", r.role(role).unwrap())));
            }
            for s in platform::identifiers::LOGICAL_SCHEMAS {
                assert!(sql.contains(&format!(
                    "REVOKE ALL ON SCHEMA {} FROM PUBLIC",
                    r.schema(s).unwrap()
                )));
                for kind in ["TABLES", "SEQUENCES", "FUNCTIONS", "TYPES"] {
                    assert!(sql.contains(&format!(
                        "IN SCHEMA {} REVOKE ALL ON {kind} FROM PUBLIC",
                        r.schema(s).unwrap()
                    )));
                }
            }
            for t in platform::identifiers::TABLES {
                assert!(sql.contains(&format!(
                    "REVOKE ALL ON TABLE {} FROM PUBLIC",
                    r.table(Backend::Postgres, t.schema, t.table).unwrap()
                )));
                if let Some(column) = t.identity_column {
                    assert!(sql.contains(&format!(
                        "REVOKE ALL ON SEQUENCE {} FROM PUBLIC",
                        r.sequence(t.schema, t.table, column).unwrap()
                    )));
                }
            }
            for kind in ["TABLES", "SEQUENCES", "FUNCTIONS", "TYPES", "SCHEMAS"] {
                assert!(sql.contains(&format!(
                    "ALTER DEFAULT PRIVILEGES FOR ROLE {} REVOKE ALL ON {kind} FROM PUBLIC",
                    r.role("migrator").unwrap()
                )));
            }
            assert!(!sql.contains("GRANT UPDATE"));
            assert!(!sql.contains("GRANT DELETE"));
            assert!(!sql.contains("GRANT ALL"));
            assert!(sql.contains(&format!(
                "GRANT USAGE ON SCHEMA {} TO {}",
                r.schema("operations").unwrap(),
                r.role("app_rw").unwrap()
            )));
        }
    }
}
