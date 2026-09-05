//! A03 开发种子的 Repository；只借当前初始化事务连接，不开启/提交事务。
use crate::connection::{DatabaseConnection, DatabaseError};
use platform::db::render_sql;
use platform::identifiers::SchemaRenderer;

const SELECT_FORUM: &str = "SELECT id FROM {table} WHERE slug = ?";
const INSERT_FORUM: &str =
    "INSERT INTO {table} (slug, name, position, status, version) VALUES (?, ?, ?, ?, ?)";
const SELECT_RULE: &str = "SELECT id FROM {table} WHERE forum_id = ? AND version = ?";
const INSERT_RULE: &str =
    "INSERT INTO {table} (forum_id, version, body, status) VALUES (?, ?, ?, ?)";

pub const FORUMS: &[(&str, &str)] = &[
    ("practice", "问答/实战"),
    ("ai", "AI"),
    ("domains-hosting", "域名与主机"),
    ("site", "站务"),
];
const RULE_PLACEHOLDER: &str =
    "开发规则占位版本：内容范围、商业规则和发帖模板待项目负责人批准；不用于公开发布。";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeedSummary {
    pub forums_created: usize,
    pub rules_created: usize,
}

pub async fn insert_missing(
    conn: &mut DatabaseConnection,
    renderer: &SchemaRenderer,
) -> Result<SeedSummary, DatabaseError> {
    let backend = conn.backend();
    let table = renderer.table(backend, "community", "forums")?;
    let forum_select = render_sql(backend, &SELECT_FORUM.replace("{table}", &table))
        .map_err(|_| DatabaseError::InvalidConfiguration)?;
    let forum_insert = render_sql(backend, &INSERT_FORUM.replace("{table}", &table))
        .map_err(|_| DatabaseError::InvalidConfiguration)?;
    let table = renderer.table(backend, "moderation", "rule_versions")?;
    let rule_select = render_sql(backend, &SELECT_RULE.replace("{table}", &table))
        .map_err(|_| DatabaseError::InvalidConfiguration)?;
    let rule_insert = render_sql(backend, &INSERT_RULE.replace("{table}", &table))
        .map_err(|_| DatabaseError::InvalidConfiguration)?;
    let mut result = SeedSummary::default();
    for (position, (slug, name)) in FORUMS.iter().enumerate() {
        let id: Option<i64> = match conn {
            DatabaseConnection::Postgres(c) => {
                sqlx::query_scalar(&forum_select)
                    .bind(slug)
                    .fetch_optional(&mut *c)
                    .await?
            }
            DatabaseConnection::Sqlite(c) => {
                sqlx::query_scalar(&forum_select)
                    .bind(slug)
                    .fetch_optional(&mut *c)
                    .await?
            }
        };
        if id.is_none() {
            match conn {
                DatabaseConnection::Postgres(c) => {
                    sqlx::query(&forum_insert)
                        .bind(slug)
                        .bind(name)
                        .bind((position + 1) as i32)
                        .bind("draft")
                        .bind(1_i64)
                        .execute(&mut *c)
                        .await?;
                }
                DatabaseConnection::Sqlite(c) => {
                    sqlx::query(&forum_insert)
                        .bind(slug)
                        .bind(name)
                        .bind((position + 1) as i32)
                        .bind("draft")
                        .bind(1_i64)
                        .execute(&mut *c)
                        .await?;
                }
            }
            result.forums_created += 1;
        }
        // 批量初始化预先确定稳定唯一键，再按键回读，不调用方言自增函数。
        let forum_id: i64 = match conn {
            DatabaseConnection::Postgres(c) => {
                sqlx::query_scalar(&forum_select)
                    .bind(slug)
                    .fetch_one(&mut *c)
                    .await?
            }
            DatabaseConnection::Sqlite(c) => {
                sqlx::query_scalar(&forum_select)
                    .bind(slug)
                    .fetch_one(&mut *c)
                    .await?
            }
        };
        let rule: Option<i64> = match conn {
            DatabaseConnection::Postgres(c) => {
                sqlx::query_scalar(&rule_select)
                    .bind(forum_id)
                    .bind(1_i32)
                    .fetch_optional(&mut *c)
                    .await?
            }
            DatabaseConnection::Sqlite(c) => {
                sqlx::query_scalar(&rule_select)
                    .bind(forum_id)
                    .bind(1_i32)
                    .fetch_optional(&mut *c)
                    .await?
            }
        };
        if rule.is_none() {
            match conn {
                DatabaseConnection::Postgres(c) => {
                    sqlx::query(&rule_insert)
                        .bind(forum_id)
                        .bind(1_i32)
                        .bind(RULE_PLACEHOLDER)
                        .bind("placeholder")
                        .execute(&mut *c)
                        .await?;
                }
                DatabaseConnection::Sqlite(c) => {
                    sqlx::query(&rule_insert)
                        .bind(forum_id)
                        .bind(1_i32)
                        .bind(RULE_PLACEHOLDER)
                        .bind("placeholder")
                        .execute(&mut *c)
                        .await?;
                }
            }
            result.rules_created += 1;
        }
    }
    Ok(result)
}
