//! 版块 slug 保留字校验（`02` §6.6，`01` `CAT-000-A`，`DEC-068`）。
//!
//! `DEC-068` 把版块页放到顶级 `/{slug}`，版块 slug 与站点级静态路由从此共用
//! 同一段命名空间。路由匹配时静态段优先于参数段，所以取用了已注册路径段的
//! 版块**不会报错**——它只是永远匹配不到自己的页面。没有异常、没有日志、
//! 没有 500，只有一个打不开的版块。这类失效必须在写入时挡住。

/// 已注册的顶级静态路径段，外加 `01` 第 10 节已登记但尚未实现的那些。
///
/// 未实现的也占位：日后为了让 `/top` 上线而去改一个已经在用的版块 slug，
/// 代价远高于现在多留几个保留字。
const RESERVED: &[&str] = &[
    // 平台与诊断
    "health",
    "status",
    "assets",
    "__fixtures",
    "__diagnostics",
    // 内容读取与聚合
    "search",
    "new",
    "pinned",
    "featured",
    "hot",
    "tags",
    "t",
    "topics",
    "posts",
    "forums",
    "content",
    "me",
    "u",
    // 抓取入口（带扩展名，仍按整段保留）
    "feed.xml",
    "robots.txt",
    "sitemap.xml",
    // 身份与设置
    "login",
    "register",
    "logout",
    "settings",
    // 后台
    "manage",
    // 其他领域已挂载的前缀
    "media",
    // `01` 第 10 节已登记、尚未实现
    "polls",
    "bounties",
    "lotteries",
    "red-packets",
    "top",
    "verified",
    "rules",
    "terms",
    "privacy",
];

/// slug 非法的原因。分开返回而不是笼统一个错误：三种原因的修法完全不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlugError {
    /// 空串。顶级路由下空 slug 等于站点根路径。
    Empty,
    /// 含 `[a-z0-9-]` 以外的字符。大写与百分号编码会让同一版块出现多个 URL。
    Charset,
    /// 撞上已注册的顶级静态路径段。
    Reserved,
}

/// 校验一个版块 slug 是否可以安全地作为顶级路由段。
pub fn check(slug: &str) -> Result<(), SlugError> {
    if slug.is_empty() {
        return Err(SlugError::Empty);
    }
    if !slug
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(SlugError::Charset);
    }
    if RESERVED.contains(&slug) {
        return Err(SlugError::Reserved);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_forums_are_all_routable() {
        // 种子里的六个 slug 必须全部通过——否则初始化就会失败，
        // 而这条断言让失败出现在测试里而不是第一次跑迁移的时候。
        for (slug, name) in crate::seed_repo::FORUMS {
            assert_eq!(check(slug), Ok(()), "版块 {name} 的 slug {slug} 不可路由");
        }
    }

    #[test]
    fn reserved_and_malformed_slugs_are_rejected() {
        // 撞静态路由：这是本模块存在的唯一理由，失败形态是「永远打不开」。
        assert_eq!(check("search"), Err(SlugError::Reserved));
        assert_eq!(check("manage"), Err(SlugError::Reserved));
        // 尚未实现但已登记的路由同样保留。
        assert_eq!(check("top"), Err(SlugError::Reserved));
        assert_eq!(check("forums"), Err(SlugError::Reserved));
        // 字符集与空串。
        assert_eq!(check(""), Err(SlugError::Empty));
        assert_eq!(check("Daily"), Err(SlugError::Charset));
        assert_eq!(check("a/b"), Err(SlugError::Charset));
        assert_eq!(check("日常"), Err(SlugError::Charset));
        // 只是以保留字开头不算冲突：路由按整段匹配，`/ai-tools` 不会撞 `/ai`。
        assert_eq!(check("new-hardware"), Ok(()));
    }
}
