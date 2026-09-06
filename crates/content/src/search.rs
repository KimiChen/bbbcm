//! `SEARCH-001`～`SEARCH-004`：搜索抽象与查询。
//!
//! `02` §13 规定搜索统一经应用层 `SearchBackend` 抽象：完整模式由 PostgreSQL 的
//! `pg_jieba + pg_trgm` 实现，SQLite 体验模式用 FTS5 或规范化标题列 `LIKE`。
//! 本模块交付抽象与 SQLite 体验模式实现；PostgreSQL 全文实现需要真实 PG 与已编译
//! 扩展，属统一硬化阶段。
//!
//! **过滤顺序是安全要求，不是性能选择**：可见性、审核、删除与版块过滤必须在生成
//! 摘要与高亮**之前**完成。反过来做的话，高亮片段会带出调用者无权看到的标题。
use chrono::{DateTime, Utc};

/// 排序白名单（`02` §13：排序白名单、签名 keyset 游标、日期和版块过滤必须绑定查询上下文）。
///
/// 用封闭枚举而不是接受任意列名：把排序键直接拼进 SQL 是注入面，
/// 而「看起来像列名的字符串」在 code review 里最容易被放过。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchOrder {
    /// 最新发布优先。体验模式下没有相关性打分，最新是唯一可解释的默认。
    Newest,
    /// 最早发布优先。
    Oldest,
}
impl SearchOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Oldest => "oldest",
        }
    }
    /// 解析用户输入。未知值回落到默认排序而不是报错——搜索是只读操作，
    /// 为一个排序参数拒绝整次查询没有意义。
    pub fn parse(value: &str) -> Self {
        match value {
            "oldest" => Self::Oldest,
            _ => Self::Newest,
        }
    }
}

/// 一次搜索请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// 已 trim 的查询词。
    pub terms: String,
    /// 限定版块；`None` 为全站。
    pub forum_id: Option<i64>,
    /// 时间范围下界（含）。
    pub since: Option<DateTime<Utc>>,
    /// 时间范围上界（不含）。
    pub until: Option<DateTime<Utc>>,
    pub order: SearchOrder,
    pub limit: i64,
}

/// 一条搜索结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub topic_id: i64,
    /// 原始标题。高亮由渲染层按 `highlight` 给出的区间标记，
    /// 这里不返回拼好的 HTML——拼 HTML 就等于把转义责任交给了搜索层。
    pub title: String,
    /// 命中区间在 `title` 中的字符下标 `[start, end)`，按 Unicode 标量计。
    pub highlight: Option<(usize, usize)>,
    pub forum_id: i64,
    pub created_at: DateTime<Utc>,
}

/// 搜索后端能力，供页面明示当前限制（`02` §13：搜索页明示「体验模式：仅支持标题搜索」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchCapability {
    /// 仅标题检索（SQLite 体验模式）。
    TitleOnly,
    /// 标题与正文全文检索（PostgreSQL 完整模式）。
    FullText,
}

/// 搜索后端抽象（`02` §13）。
pub trait SearchBackend: Send + Sync {
    fn capability(&self) -> SearchCapability;
}

/// SQLite 体验模式：规范化标题列的 `LIKE` 匹配。
#[derive(Debug, Clone, Copy, Default)]
pub struct SqliteTitleSearch;
impl SearchBackend for SqliteTitleSearch {
    fn capability(&self) -> SearchCapability {
        SearchCapability::TitleOnly
    }
}

/// PostgreSQL 完整模式：`pg_jieba` 中文分词的标题与正文全文检索（`02` §13）。
///
/// **构造是有条件的**：`new` 只在探测到 `pg_jieba` 时返回实例。装配层拿不到实例
/// 就继续用 `SqliteTitleSearch`，页面照旧明示「仅支持标题搜索」。这样写是为了让
/// 「扩展没装」表现为**能力降级并如实告知**，而不是运行期报错——扩展缺失是部署
/// 事实，不是用户的错。
#[derive(Debug, Clone, Copy)]
pub struct PostgresFullTextSearch;
impl PostgresFullTextSearch {
    /// `capabilities` 来自 `platform-db` 建连时的一次性探测。
    pub fn new(capabilities: platform::db::Capabilities) -> Option<Self> {
        capabilities.pg_jieba.then_some(Self)
    }
}
impl SearchBackend for PostgresFullTextSearch {
    fn capability(&self) -> SearchCapability {
        SearchCapability::FullText
    }
}

/// 规范化查询词，并判断它是否可用于检索。
///
/// 返回 `None` 表示这次查询不该执行：空串或纯空白会退化成「匹配全部」，
/// 那不是搜索结果而是一份内容清单，容易被当成搜索能力已经工作。
pub fn normalize_terms(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 128 {
        return None;
    }
    Some(trimmed.to_lowercase())
}

/// 把 `LIKE` 的通配符转义掉。
///
/// 不转义的话，一个 `%` 就能让查询变成「列出全部标题」，绕过用户本应输入的关键词。
/// 这不是注入（值仍走 bind 参数），但同样是把检索变成枚举。
///
/// 转义符用 `!` 而不是反斜杠：本项目的 SQL 静态守卫禁止 SQL 字面量里出现反斜杠转义
/// （不同后端对 `\` 的处理有差异），因此 `ESCAPE '!'` 是三库交集下的可移植写法。
pub fn escape_like(value: &str) -> String {
    value
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_")
}

/// 在标题中定位命中区间，供渲染层做高亮。
///
/// 按 Unicode 标量下标返回，避免中文标题上按字节切片导致的乱码。
/// 找不到时返回 `None`，渲染层原样输出标题——宁可不高亮，也不猜一个区间。
pub fn locate(title: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    let lower_title: Vec<char> = title.to_lowercase().chars().collect();
    let lower_needle: Vec<char> = needle.to_lowercase().chars().collect();
    if lower_needle.len() > lower_title.len() {
        return None;
    }
    (0..=lower_title.len() - lower_needle.len())
        .find(|start| lower_title[*start..*start + lower_needle.len()] == lower_needle[..])
        .map(|start| (start, start + lower_needle.len()))
}

impl SearchQuery {
    /// 校验并规范化一次查询。返回 `None` 表示查询词不可用。
    pub fn build(
        raw_terms: &str,
        forum_id: Option<i64>,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        order: &str,
        limit: i64,
    ) -> Option<Self> {
        let terms = normalize_terms(raw_terms)?;
        // 时间范围颠倒时直接丢弃两端，而不是返回空结果——空结果会被读成
        // 「这个时间段没有内容」，而实际上是参数写反了。
        let (since, until) = match (since, until) {
            (Some(a), Some(b)) if a >= b => (None, None),
            pair => pair,
        };
        Some(Self {
            terms,
            forum_id: forum_id.filter(|id| *id > 0),
            since,
            until,
            order: SearchOrder::parse(order),
            limit: limit.clamp(1, 50),
        })
    }
}

/// `SEO-006`：从分享链接里剥掉追踪参数，生成 Canonical。
///
/// 只保留白名单参数（分页游标与主题模式）。用白名单而不是黑名单：
/// 追踪参数的名字是无穷的，黑名单永远漏。
pub fn canonical_query(pairs: &[(String, String)]) -> String {
    let kept: Vec<String> = pairs
        .iter()
        .filter(|(key, _)| matches!(key.as_str(), "cursor" | "theme"))
        .map(|(key, value)| format!("{key}={}", crate::http::encode(value)))
        .collect();
    if kept.is_empty() {
        String::new()
    } else {
        format!("?{}", kept.join("&"))
    }
}

#[cfg(test)]
mod tests;
