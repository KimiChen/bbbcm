//! 数据库后端抽象、SQL 占位符渲染与跨库类型编解码。
//! 连接建立、事务模板与迁移执行在 `platform-db`。

use serde::Deserialize;
use thiserror::Error;

/// 部署后端：运行时二选一，不做跨后端数据迁移。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Postgres,
    #[default]
    Sqlite,
    /// 预留值：规范先行，尚未实现。配置该值时拒绝启动（`DEC-022`／`DEC-033`）。
    Mysql8,
}

impl Backend {
    /// 是否已实现连接与迁移。`mysql8` 为规范预留，装配时必须拒绝启动。
    pub fn is_implemented(self) -> bool {
        !matches!(self, Backend::Mysql8)
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Backend::Postgres => "postgres",
            Backend::Sqlite => "sqlite",
            Backend::Mysql8 => "mysql8",
        })
    }
}

/// 后端能力集：启动时探测一次并缓存，运行期只读。
#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    /// SQLite 是否带 FTS5；探测实现在 platform-db 建立连接后执行。
    pub sqlite_fts5: bool,
    /// PostgreSQL 是否装有 `pg_jieba`（完整模式全文检索的前提）。
    pub pg_jieba: bool,
    /// PostgreSQL 是否装有 `pg_trgm`。
    pub pg_trgm: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    #[error("共享 SQL 不得直接出现 PostgreSQL 位置参数 `${0}`：一律写 `?` 占位符")]
    ExplicitPlaceholder(usize),
    #[error("共享 SQL 不得使用双引号标识符：标识符一律经 SchemaRenderer 渲染")]
    DoubleQuotedIdentifier,
    #[error("SQL 结束时仍处于未闭合的{0}中")]
    Unterminated(&'static str),
}

/// repo 统一写 `?` 占位符：PostgreSQL 转换为 `$1..$n`，SQLite/MySQL 原样。
///
/// 扫描器识别单引号字符串、行注释（`--`）与块注释（`/* */`，PostgreSQL 允许嵌套），
/// 三者内部的 `?` 一律原样输出、不参与编号——注释里的一个问号就会让其后所有参数错位，
/// 而这类错误在运行期才暴露。按 `02` 第 6.1 节的语义差异禁用清单，双引号标识符与
/// 已经写死的 `$N` 直接拒绝。
pub fn render_sql(backend: Backend, sql: &str) -> Result<String, RenderError> {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut index = 0usize;
    let mut in_single = false;
    let mut line_comment = false;
    let mut block_depth = 0usize;
    let bytes: Vec<char> = sql.chars().collect();
    let mut i = 0usize;

    while i < bytes.len() {
        let ch = bytes[i];
        let next = bytes.get(i + 1).copied();

        if line_comment {
            out.push(ch);
            if ch == '\n' {
                line_comment = false;
            }
            i += 1;
            continue;
        }
        if block_depth > 0 {
            if ch == '*' && next == Some('/') {
                block_depth -= 1;
                out.push(ch);
                out.push('/');
                i += 2;
                continue;
            }
            if ch == '/' && next == Some('*') {
                block_depth += 1;
                out.push(ch);
                out.push('*');
                i += 2;
                continue;
            }
            out.push(ch);
            i += 1;
            continue;
        }
        if in_single {
            out.push(ch);
            if ch == '\'' {
                // `''` 是转义的单引号：连续两个引号相当于留在字符串内。
                in_single = false;
            }
            i += 1;
            continue;
        }

        match ch {
            '\'' => {
                in_single = true;
                out.push(ch);
                i += 1;
            }
            '"' => return Err(RenderError::DoubleQuotedIdentifier),
            '-' if next == Some('-') => {
                line_comment = true;
                out.push_str("--");
                i += 2;
            }
            '/' if next == Some('*') => {
                block_depth = 1;
                out.push_str("/*");
                i += 2;
            }
            '$' if next.is_some_and(|c| c.is_ascii_digit()) => {
                let mut j = i + 1;
                let mut n = 0usize;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    n = n * 10 + bytes[j].to_digit(10).expect("已检查是数字") as usize;
                    j += 1;
                }
                return Err(RenderError::ExplicitPlaceholder(n));
            }
            '?' => {
                index += 1;
                if backend == Backend::Postgres {
                    out.push('$');
                    out.push_str(&index.to_string());
                } else {
                    out.push('?');
                }
                i += 1;
            }
            _ => {
                out.push(ch);
                i += 1;
            }
        }
    }

    if in_single {
        return Err(RenderError::Unterminated("字符串字面量"));
    }
    if block_depth > 0 {
        return Err(RenderError::Unterminated("块注释"));
    }
    Ok(out)
}

/// 跨库时间文本编码：固定 27 字节、6 位小数、大写 `Z`，UTC。
///
/// 见 `02` 第 6.2 节「时间文本编码规范」。固定宽度保证字典序等于时间序，
/// 否则 SQLite 的 `TEXT` 时间列会破坏 keyset 游标。
pub fn encode_time(ts: chrono::DateTime<chrono::Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

/// 解析 [`encode_time`] 的输出；也接受其他合法 RFC3339，一律归一到 UTC。
pub fn decode_time(text: &str) -> Result<chrono::DateTime<chrono::Utc>, chrono::ParseError> {
    Ok(chrono::DateTime::parse_from_rfc3339(text)?.with_timezone(&chrono::Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg(sql: &str) -> String {
        render_sql(Backend::Postgres, sql).expect("应渲染成功")
    }

    #[test]
    fn postgres_renders_numbered_placeholders() {
        assert_eq!(
            pg("UPDATE t SET a = ?, b = ? WHERE id = ?"),
            "UPDATE t SET a = $1, b = $2 WHERE id = $3"
        );
    }

    #[test]
    fn sqlite_keeps_question_marks() {
        assert_eq!(
            render_sql(Backend::Sqlite, "SELECT * FROM t WHERE a = ?").expect("应渲染成功"),
            "SELECT * FROM t WHERE a = ?"
        );
    }

    #[test]
    fn quoted_placeholders_are_literal() {
        assert_eq!(
            pg("SELECT * FROM t WHERE name = '? 帮助' AND id = ?"),
            "SELECT * FROM t WHERE name = '? 帮助' AND id = $1"
        );
    }

    #[test]
    fn escaped_quote_does_not_leak_out_of_string() {
        assert_eq!(
            pg("SELECT * FROM t WHERE name = 'it''s ?' AND id = ?"),
            "SELECT * FROM t WHERE name = 'it''s ?' AND id = $1"
        );
    }

    #[test]
    fn line_comment_question_mark_is_not_a_placeholder() {
        assert_eq!(
            pg("SELECT id FROM topics -- 已读?\nWHERE forum_id = ? AND id > ?"),
            "SELECT id FROM topics -- 已读?\nWHERE forum_id = $1 AND id > $2"
        );
    }

    #[test]
    fn block_comment_question_mark_is_not_a_placeholder() {
        assert_eq!(
            pg("/* 是否置顶? */ UPDATE topics SET pinned = ? WHERE id = ?"),
            "/* 是否置顶? */ UPDATE topics SET pinned = $1 WHERE id = $2"
        );
    }

    #[test]
    fn nested_block_comment_is_tracked() {
        assert_eq!(
            pg("/* a /* b ? */ c ? */ SELECT ?"),
            "/* a /* b ? */ c ? */ SELECT $1"
        );
    }

    #[test]
    fn explicit_dollar_placeholder_is_rejected() {
        assert_eq!(
            render_sql(Backend::Postgres, "SELECT * FROM t WHERE a = $1 AND b = ?"),
            Err(RenderError::ExplicitPlaceholder(1))
        );
    }

    #[test]
    fn double_quoted_identifier_is_rejected() {
        assert_eq!(
            render_sql(Backend::Sqlite, r#"SELECT "Id" FROM t"#),
            Err(RenderError::DoubleQuotedIdentifier)
        );
    }

    #[test]
    fn unterminated_string_is_rejected() {
        assert_eq!(
            render_sql(Backend::Sqlite, "SELECT 'abc"),
            Err(RenderError::Unterminated("字符串字面量"))
        );
    }

    #[test]
    fn mysql8_is_reserved_but_not_implemented() {
        assert!(!Backend::Mysql8.is_implemented());
        assert!(Backend::Postgres.is_implemented());
        assert_eq!(Backend::Mysql8.to_string(), "mysql8");
    }

    #[test]
    fn time_encoding_is_fixed_width_and_roundtrips() {
        let t = decode_time("2026-09-05T04:00:00Z").expect("应解析成功");
        let encoded = encode_time(t);
        assert_eq!(encoded, "2026-09-05T04:00:00.000000Z");
        assert_eq!(encoded.len(), 27, "固定 27 字节，字典序才等于时间序");
        assert_eq!(decode_time(&encoded).expect("应解析成功"), t);
    }

    #[test]
    fn encoded_times_sort_lexicographically() {
        let a = encode_time(decode_time("2026-09-05T04:00:00Z").expect("应解析成功"));
        let b = encode_time(decode_time("2026-09-05T04:00:00.000001Z").expect("应解析成功"));
        let c = encode_time(decode_time("2026-09-05T12:00:00Z").expect("应解析成功"));
        assert!(a < b && b < c);
    }
}
