//! 页面模板使用的视图合同；不暴露数据库行或领域内部结构。

use askama::Template;

/// 每个页面都显式携带提示、错误与空状态，模板在编译期检查这些字段。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageFrame {
    pub flash: Option<&'static str>,
    pub error: Option<&'static str>,
    pub empty: Option<&'static str>,
}

/// 列表的空状态文案属于视图合同，而不是模板自行猜测的默认值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListView<T> {
    pub items: Vec<T>,
    pub empty: &'static str,
}

#[derive(Template)]
#[template(path = "home/index.html")]
pub struct HomeTemplate {
    pub frame: PageFrame,
}

/// A00 的每一行均为固定合成数据，不对应任何账号、主题或业务统计。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicView {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub author: &'static str,
    pub replies: &'static str,
    pub views: &'static str,
    pub age: &'static str,
    pub pinned: bool,
}

#[derive(Template)]
#[template(path = "a00/page.html")]
pub struct A00Template {
    pub frame: PageFrame,
    pub fixture_version: &'static str,
    pub page: &'static str,
    pub title: &'static str,
    pub active_nav: &'static str,
    pub theme: &'static str,
    pub topics: ListView<TopicView>,
}
