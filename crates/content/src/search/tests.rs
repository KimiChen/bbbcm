use super::*;

/// 查询词规范化：空串与超长一律拒绝，避免退化成「列出全部」。
#[test]
fn blank_or_oversized_terms_are_refused_rather_than_matching_everything() {
    assert_eq!(normalize_terms("  草莓  ").as_deref(), Some("草莓"));
    // 大小写归一，与投影里的 normalized_title 口径一致。
    assert_eq!(normalize_terms("Rust").as_deref(), Some("rust"));
    for blank in ["", "   ", "\t\n"] {
        assert!(normalize_terms(blank).is_none(), "{blank:?} 应被拒绝");
    }
    assert!(normalize_terms(&"长".repeat(129)).is_none());
    assert!(normalize_terms(&"长".repeat(128)).is_some());
}

/// LIKE 通配符必须转义：不转义时一个 % 就能把检索变成枚举全部标题。
#[test]
fn like_wildcards_are_escaped_so_search_cannot_become_an_enumeration() {
    assert_eq!(escape_like("100%"), "100!%");
    assert_eq!(escape_like("a_b"), "a!_b");
    // 转义符本身也要转义，否则用户输入的 `!` 会吃掉后一个字符。
    assert_eq!(escape_like("a!b"), "a!!b");
    // 纯通配输入转义后不再是通配。
    assert_eq!(escape_like("%"), "!%");
    assert_eq!(escape_like("%_%"), "!%!_!%");
    // 普通中文不受影响。
    assert_eq!(escape_like("草莓"), "草莓");
}

/// 命中区间按 Unicode 标量下标返回，中文标题不会被按字节切坏。
#[test]
fn highlight_positions_are_scalar_indexed_and_case_insensitive() {
    assert_eq!(locate("草莓社区论坛", "社区"), Some((2, 4)));
    // 若按字节下标，"社区" 在 UTF-8 里从第 6 字节开始——按标量则是第 2 个字符。
    let (start, end) = locate("草莓社区论坛", "社区").unwrap();
    let sliced: String = "草莓社区论坛"
        .chars()
        .skip(start)
        .take(end - start)
        .collect();
    assert_eq!(sliced, "社区");
    // 大小写不敏感。
    assert_eq!(locate("Rust 语言", "rust"), Some((0, 4)));
    // 找不到时返回 None，渲染层原样输出标题，不猜区间。
    assert!(locate("草莓社区", "咖啡").is_none());
    assert!(locate("短", "很长的关键词").is_none());
    assert!(locate("任意", "").is_none());
}

/// 排序只接受白名单值；未知值回落默认而不是拒绝整次查询。
#[test]
fn sort_order_is_a_closed_whitelist() {
    assert_eq!(SearchOrder::parse("newest"), SearchOrder::Newest);
    assert_eq!(SearchOrder::parse("oldest"), SearchOrder::Oldest);
    for bogus in ["", "relevance", "created_at DESC; DROP TABLE", "views"] {
        assert_eq!(
            SearchOrder::parse(bogus),
            SearchOrder::Newest,
            "{bogus:?} 应回落默认排序"
        );
    }
}

/// 查询构造：limit 收敛、版块非正丢弃、时间范围颠倒时两端一并丢弃。
#[test]
fn query_building_clamps_and_discards_contradictory_ranges() {
    let now = platform::db::decode_time("2026-09-07T00:00:00.000000Z").unwrap();
    let later = now + chrono::Duration::days(1);

    let q = SearchQuery::build("草莓", Some(3), Some(now), Some(later), "oldest", 999).unwrap();
    assert_eq!(q.limit, 50, "上限收敛");
    assert_eq!(q.forum_id, Some(3));
    assert_eq!(q.order, SearchOrder::Oldest);

    // 时间范围写反时两端一并丢弃：返回空结果会被读成「这段时间没有内容」，
    // 而实际上只是参数写反了。
    let reversed = SearchQuery::build("草莓", None, Some(later), Some(now), "", 10).unwrap();
    assert!(reversed.since.is_none() && reversed.until.is_none());

    // 非正版块 ID 丢弃，等同全站。
    assert_eq!(
        SearchQuery::build("草莓", Some(0), None, None, "", 10)
            .unwrap()
            .forum_id,
        None
    );
    assert_eq!(SearchQuery::build("  ", None, None, None, "", 10), None);
}

/// `SEO-006`：Canonical 用白名单保留参数，追踪参数一律剥除。
#[test]
fn canonical_keeps_only_whitelisted_parameters() {
    let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
        v.iter()
            .map(|(k, val)| ((*k).to_owned(), (*val).to_owned()))
            .collect()
    };
    assert_eq!(canonical_query(&pairs(&[])), "");
    assert_eq!(
        canonical_query(&pairs(&[("cursor", "abc"), ("theme", "dark")])),
        "?cursor=abc&theme=dark"
    );
    // 追踪参数全部剥除。用白名单而不是黑名单：追踪参数的名字是无穷的。
    assert_eq!(
        canonical_query(&pairs(&[
            ("utm_source", "weibo"),
            ("utm_campaign", "spring"),
            ("ref", "friend"),
            ("fbclid", "x"),
            ("gclid", "y"),
            ("cursor", "abc"),
        ])),
        "?cursor=abc"
    );
    // 保留值仍然经过编码，不会把分隔符带进 URL。
    assert_eq!(
        canonical_query(&pairs(&[("cursor", "a&b=c")])),
        "?cursor=a%26b%3Dc"
    );
}

/// 体验模式必须能被页面读出来，用于明示「仅支持标题搜索」（`02` §13）。
#[test]
fn sqlite_backend_reports_title_only_capability() {
    assert_eq!(
        SqliteTitleSearch.capability(),
        SearchCapability::TitleOnly,
        "SQLite 体验模式不得声称具备全文能力"
    );
}
