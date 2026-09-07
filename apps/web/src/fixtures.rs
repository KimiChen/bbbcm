//! WP-MVP-A00：只由固定常量构造的公开视觉夹具。
//!
//! 无时钟、随机数、数据库或请求内容；页型与主题是仅有的展示选项。

use serde::Deserialize;

use crate::contract::{A00Template, ListView, PageFrame, TopicView};

pub const FIXTURE_VERSION: &str = "a00-synthetic-v1";
pub const PAGES: [&str; 14] = [
    "home", "new", "pinned", "featured", "forum", "topic", "login", "register", "reset", "policy",
    "search", "empty", "error", "profile",
];

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct FixtureQuery {
    #[serde(default)]
    pub theme: Theme,
}

/// 页型必须来自白名单，未知路径不得回退成一个看似成功的页面。
pub fn page(name: &str, theme: Theme) -> Option<A00Template> {
    let page = PAGES.into_iter().find(|page| *page == name)?;
    let (title, active_nav) = match page {
        "home" => ("最新回复", "home"),
        "new" => ("最新主题", "new"),
        "pinned" => ("置顶主题", "pinned"),
        "featured" => ("精华主题", "featured"),
        "forum" => ("日常", "forum"),
        "topic" => ("合成主题：草莓论坛视觉夹具", "home"),
        "login" => ("登录", "login"),
        "register" => ("注册", "register"),
        "reset" => ("找回密码", "reset"),
        "policy" => ("社区条款", "policy"),
        "profile" => ("合成公开资料", "profile"),
        "search" => ("搜索", "search"),
        "empty" => ("空状态", "home"),
        "error" => ("错误状态", "home"),
        _ => unreachable!("PAGES 的每一个值均有明确页型"),
    };

    let frame = PageFrame {
        flash: None,
        error: (page == "error").then_some("合成错误：暂时无法显示内容，请稍后重试。"),
        empty: (page == "empty").then_some("暂无合成主题。"),
    };
    let mut items = if matches!(page, "empty" | "error") {
        Vec::new()
    } else {
        topics()
    };

    if matches!(page, "pinned" | "featured") {
        for item in &mut items {
            item.pinned = false;
        }
    }

    Some(A00Template {
        frame,
        fixture_version: FIXTURE_VERSION,
        page,
        title,
        active_nav,
        theme: theme.as_str(),
        topics: ListView {
            items,
            empty: "暂无合成主题。",
        },
    })
}

fn topics() -> Vec<TopicView> {
    const TITLES: [&str; 12] = [
        "合成主题 01：欢迎来到草莓论坛",
        "合成主题 02：一起整理实践笔记",
        "合成主题 03：分享一个小问题",
        "合成主题 04：每周学习记录",
        "合成主题 05：从零开始的实验",
        "合成主题 06：今天的新发现",
        "合成主题 07：如何整理常用资料",
        "合成主题 08：一次简单的复盘",
        "合成主题 09：工具使用体验",
        "合成主题 10：长标题换行示例——用固定内容检查列表行的布局与间距",
        "合成主题 11：欢迎交流想法",
        "合成主题 12：下一步学习计划",
    ];
    const IDS: [&str; 12] = [
        "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12",
    ];
    const CATEGORIES: [&str; 6] = ["日常", "酷品", "AI", "主机", "版本", "站务"];
    const AUTHORS: [&str; 4] = ["合成果农甲", "合成果农乙", "合成果农丙", "合成果农丁"];
    const REPLIES: [&str; 4] = ["0", "2", "8", "16"];
    const VIEWS: [&str; 4] = ["12", "24", "48", "96"];
    const AGES: [&str; 4] = ["1 分钟前", "5 分钟前", "1 小时前", "1 天前"];

    TITLES
        .into_iter()
        .enumerate()
        .map(|(index, title)| TopicView {
            id: IDS[index],
            title,
            category: CATEGORIES[index % CATEGORIES.len()],
            author: AUTHORS[index % AUTHORS.len()],
            replies: REPLIES[index % REPLIES.len()],
            views: VIEWS[index % VIEWS.len()],
            age: AGES[index % AGES.len()],
            pinned: index < 2,
        })
        .collect()
}
