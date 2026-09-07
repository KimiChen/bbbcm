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
    /// `postgres-search` 轨是否已应用（`search_documents.search_vector` 是否存在）。
    ///
    /// 与 `pg_jieba` 分开探测：扩展装了但轨没跑，是一个真实且容易发生的状态
    /// （两者由不同命令完成）。此时查询若引用不存在的列，会在每次搜索时抛错，
    /// 而不是按 `02` §6.5 干净地降级。
    pub search_vector: bool,
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

/// 事务级顾问锁的方言入口（`DEC-066`）。
///
/// 用途：为「先查再决定建不建」这类没有可锁行的合并逻辑提供序列化点。典型是
/// 举报合并——同一目标还没有案件时无行可锁，八路并发会同时读到「没有」。
///
/// - PostgreSQL：`pg_advisory_xact_lock`，随事务结束自动释放，无需显式解锁。
/// - SQLite：返回 `None`。SQLite 是单写者，写事务天然互斥，不需要也没有顾问锁。
///
/// 键由调用方在应用侧算出并以 bind 参数传入，**不使用 `hashtext`**——它不保证
/// 跨 PostgreSQL 版本稳定，升级后同一目标可能落到不同的键上，锁就失效了。
pub fn advisory_lock_sql(backend: Backend) -> Option<&'static str> {
    match backend {
        Backend::Postgres => Some("SELECT pg_advisory_xact_lock(?) AS locked"),
        // MySQL 的 GET_LOCK 是会话级不是事务级，语义不同；接入时再单独设计。
        Backend::Sqlite | Backend::Mysql8 => None,
    }
}

/// 把一个受信命名空间与整数标识折算成稳定的 64 位顾问锁键。
///
/// 命名空间参与哈希，避免不同领域用同一个整数 ID 撞到同一把锁。
pub fn advisory_lock_key(namespace: &str, id: i64) -> i64 {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update(b":");
    hasher.update(id.to_be_bytes());
    let digest = hasher.finalize();
    i64::from_be_bytes(digest[..8].try_into().expect("SHA-256 至少 8 字节"))
}

/// 方言入口：Repository 只提供共享 SELECT；SQLite 依赖 IMMEDIATE 单写者。
pub fn lock_sql(backend: Backend, select: &str, skip_locked: bool) -> String {
    match backend {
        Backend::Postgres | Backend::Mysql8 => {
            if skip_locked {
                format!("{select} FOR UPDATE SKIP LOCKED")
            } else {
                format!("{select} FOR UPDATE")
            }
        }
        Backend::Sqlite => select.to_owned(),
    }
}

/// 必须由事务适配器在同事务同连接上紧接单行 INSERT 调用。
pub fn last_insert_id_sql(backend: Backend) -> &'static str {
    match backend {
        Backend::Postgres => "SELECT lastval() AS id",
        Backend::Sqlite => "SELECT last_insert_rowid() AS id",
        Backend::Mysql8 => "SELECT LAST_INSERT_ID() AS id",
    }
}

pub fn begin_read_sql(backend: Backend) -> &'static str {
    match backend {
        Backend::Postgres => "BEGIN READ ONLY",
        Backend::Sqlite => "BEGIN",
        Backend::Mysql8 => "START TRANSACTION READ ONLY",
    }
}

pub fn begin_write_sql(backend: Backend) -> &'static str {
    match backend {
        Backend::Postgres => "BEGIN ISOLATION LEVEL READ COMMITTED",
        Backend::Sqlite => "BEGIN IMMEDIATE",
        Backend::Mysql8 => "START TRANSACTION",
    }
}

pub fn transaction_setup_sql(backend: Backend) -> Option<&'static str> {
    match backend {
        Backend::Postgres => Some("SET LOCAL lock_timeout = '500ms'"),
        Backend::Sqlite | Backend::Mysql8 => None,
    }
}

/// SQLite 租约三步算法与 PG 批量行锁共用显式 ID 列表；不产生 IN 子查询 LIMIT。
/// 完整模式全文检索语句（`02` §13，`WP-MVP-C05`）。
///
/// 只有 PostgreSQL 有这条方言实现，其他后端返回 `None`——调用方据此降级为标题检索。
///
/// **为什么在这里而不是 `repo.rs`**：这条语句用 `@@` 与 `to_tsvector`，都是
/// PostgreSQL 专有语法，共享 SQL 静态守卫会（正确地）拒绝它们。方言 SQL 的既定
/// 归处就是本模块，与 `lock_sql`、`advisory_lock_sql`、`outbox_claim_update_sql`
/// 同一处理方式：`repo.rs` 只负责绑定值，不持有方言字面量。
///
/// **为什么用 `plainto_tsquery` 而不是 `to_tsquery`**：`to_tsquery` 会把输入当成
/// 查询表达式解析，用户输入里的 `&`、`|`、`!`、`:*` 会变成运算符——轻则报错，
/// 重则被用来构造代价极高的查询。`plainto_tsquery` 把整段输入当纯文本分词后取
/// 合取，用户写什么就搜什么。
///
/// **为什么标题与正文分开匹配**：共享 SQL 禁止 `||` 字符串拼接（拼接必须在应用层
/// 完成），因此不把两列并成一个文档，而是各自 `to_tsvector` 后取并集。语义上等价，
/// 且两列将来可以各自建索引。
///
/// **为什么索引侧用 `jiebaqry`、查询侧用 `jiebacfg`**：`pg_jieba` 的两种模式粒度不同。
/// `jiebacfg` 是精确模式，「梧桐树下的公开主题」只切出 `梧桐树`；`jiebaqry` 是检索模式，
/// 会切出 `梧桐`／`桐树`／`梧桐树` 三个重叠词元。若两侧都用 `jiebacfg`，用户搜「梧桐」
/// 就搜不到「梧桐树」——中文里这种「查询词是索引词的前缀」极其常见，等于搜索大面积失灵。
/// 因此索引侧取高召回的 `jiebaqry`，查询侧取精确的 `jiebacfg`，实测短词、全词、正文词与
/// 多词查询均命中，不相关词不命中。
///
/// **为什么写 `public.jiebacfg` 而不是 `jiebacfg`**：运行连接的 `search_path` 被固定为
/// `pg_catalog`（`prepare_postgres` 刻意如此，强制所有标识符显式限定），不限定就会
/// 报 `text search configuration "jiebacfg" does not exist`。能力探测同样只认装在
/// `public` 的 `pg_jieba`，装在别处会被判为缺失并降级——两处必须一致，否则会出现
/// 「声称支持全文但每次查询都报错」。
///
/// 排序方向由布尔选择两条完整语句，不做片段插值——理由同 `repo::search_titles`。
pub fn full_text_search_sql(
    renderer: &crate::identifiers::SchemaRenderer,
    backend: Backend,
    newest_first: bool,
) -> Result<Option<String>, crate::identifiers::SchemaError> {
    if backend != Backend::Postgres {
        return Ok(None);
    }
    let docs = renderer.table(backend, "community", "search_documents")?;
    // 过滤条件与 `repo::search_titles` 逐条对齐：已发布、未被处置、未删除、
    // 可见性、版块、时间范围。全文检索**不放宽任何一条**——放宽等于用搜索
    // 把权限过滤绕过去。
    let filters = concat!(
        " AND d.publication = ? AND d.moderation = ? AND d.deleted_at = ?",
        " AND (? = 1 OR (d.visibility = ? AND d.forum_visibility = ?))",
        " AND (? = 0 OR d.forum_id = ?)",
        " AND (? = ? OR d.created_at >= ?) AND (? = ? OR d.created_at < ?)"
    );
    // 排序键取自 `search_documents` 自己的 `created_at`（迁移 0022 物化），
    // 因此**不再连接 `topics`**。这不是省一次连接那么简单：排序键在被连接的表上
    // 时，必须先取回全部匹配、连接、再全量排序才能取前 50，排序无法提前截断——
    // 实测十万篇语料下命中 1,000 条即超门槛 3.5 倍（`99` §5.39）。
    // 物化之后 `(created_at, topic_id)` 上的有序扫描可以凑满 50 条就停；低命中率
    // 仍走 GIN，高命中率走有序扫描，由规划器按选择性各取所需。
    // 判定读物化列 `search_vector`（`postgres-search` 轨 0002 的生成列），
    // 不再在查询里重算 `to_tsvector`。这不只是省一次函数调用：有序扫描的过滤
    // 路径上，每一行都会执行一次这个表达式——实测 1% 命中档因此扫 38,600 行、
    // 分词 38,600 次（`99` §5.40）。读物化列后 GIN 查找与过滤判定读同一份向量。
    //
    // 该列合并了 title 与 body，与原先 `title @@ q OR body @@ q` 等价，因此
    // 只需一个绑定值；调用方仍传两个查询词参数以保持绑定顺序不变，第二个用于
    // 兼容既有调用签名，值相同。
    let head = format!(
        "SELECT DISTINCT d.topic_id AS topic_id, d.title AS title, d.forum_id AS forum_id, \
         d.created_at AS created_at FROM {docs} d \
         WHERE (d.search_vector @@ plainto_tsquery('public.jiebacfg', ?) \
         OR d.search_vector @@ plainto_tsquery('public.jiebacfg', ?))"
    );
    let tail = if newest_first {
        " ORDER BY d.created_at DESC, d.topic_id DESC LIMIT ?"
    } else {
        " ORDER BY d.created_at ASC, d.topic_id ASC LIMIT ?"
    };
    Ok(Some(format!("{head}{filters}{tail}")))
}

pub fn outbox_claim_update_sql(
    renderer: &crate::identifiers::SchemaRenderer,
    backend: Backend,
    count: usize,
) -> Result<String, crate::identifiers::SchemaError> {
    if !(1..=100).contains(&count) {
        return Err(crate::identifiers::SchemaError::InvalidTemplate);
    }
    let table = renderer.table(backend, "platform", "outbox_events")?;
    let placeholders = vec!["?"; count].join(", ");
    Ok(format!(
        "UPDATE {table} SET state = 'leased', lease_owner = ?, lease_batch_id = ?, lease_until = ?, attempts = attempts + 1 WHERE event_id IN ({placeholders}) AND ((state = 'ready' AND available_at <= ?) OR (state = 'leased' AND lease_until <= ?))"
    ))
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
