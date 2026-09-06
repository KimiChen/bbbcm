//! Public SSR projections. Raw Markdown is escaped in forms; only the server's
//! sanitized output is placed in `body_html` / `preview_html`.

#[derive(Default)]
pub struct PageFrame {
    pub flash: Option<&'static str>,
    pub error: Option<&'static str>,
    pub empty: Option<&'static str>,
}

pub struct Link {
    pub label: String,
    pub href: String,
    pub active: bool,
}

pub struct TopicRow {
    pub href: String,
    pub title: String,
    pub author: String,
    pub author_href: String,
    pub forum: String,
    pub forum_href: String,
    pub created_at: String,
    pub last_reply: Option<String>,
    pub replies: Option<i64>,
    /// NAV-003 主题行的浏览数。
    pub views: Option<i64>,
    pub pinned: bool,
    pub state: String,
}

pub struct PostView {
    pub author: String,
    pub author_href: String,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub body_html: String,
    /// 正文被扣留时的占位提示；为空表示正常展示正文。
    /// 模板据此二选一渲染，永远不会同时拿到占位文案和原文。
    pub body_placeholder: Option<String>,
    pub floor: i64,
    pub permalink: String,
    pub quote_href: Option<String>,
    pub edit_href: Option<String>,
    pub delete_href: Option<String>,
    pub quote_target: Option<Link>,
}

pub struct TopicDetail {
    pub title: String,
    pub forum: Link,
    pub tags: Vec<Link>,
    pub state: String,
    pub opening: PostView,
    pub replies: Vec<PostView>,
    pub reply_href: Option<String>,
}

pub struct Editor {
    pub action: String,
    pub csrf: String,
    pub idempotency_key: String,
    pub version: String,
    pub title: String,
    pub body: String,
    pub forum_id: String,
    pub forum_choices: Vec<Link>,
    pub tag_ids: String,
    pub visibility: String,
    pub quote: String,
    pub topic_fields: bool,
    pub deleting: bool,
    pub submit: &'static str,
    pub preview_html: Option<String>,
}

pub struct Page {
    pub frame: PageFrame,
    pub title: String,
    pub theme: &'static str,
    pub theme_links: Vec<Link>,
    pub forums: Vec<Link>,
    pub tabs: Vec<Link>,
    pub username: Option<String>,
    pub profile_href: String,
    pub topics: Vec<TopicRow>,
    pub latest: Vec<Link>,
    pub next: Option<String>,
    pub first: String,
    pub empty: &'static str,
    pub show_list: bool,
    pub topic: Option<TopicDetail>,
    pub editor: Option<Editor>,
    pub notice: Option<&'static str>,
    pub operation: Option<OperationQuery>,
}

impl Page {
    pub fn new(title: impl Into<String>, theme: &'static str) -> Self {
        Self {
            frame: PageFrame::default(),
            title: title.into(),
            theme,
            theme_links: Vec::new(),
            forums: Vec::new(),
            tabs: Vec::new(),
            username: None,
            profile_href: "/login".into(),
            topics: Vec::new(),
            latest: Vec::new(),
            next: None,
            first: "/".into(),
            empty: "暂无可见内容。",
            show_list: false,
            topic: None,
            editor: None,
            notice: None,
            operation: None,
        }
    }
}

pub struct OperationQuery {
    pub csrf: String,
    pub idempotency_key: String,
    pub operation_id: String,
    pub action: String,
}

#[derive(Debug)]
pub struct RenderError;

pub trait PageRenderer: Send + Sync {
    fn render(&self, page: &Page) -> Result<String, RenderError>;
}
