//! Content SSR uses the frozen A00 assets and component classes.
use crate::assets::{Assets, filters};
use askama::Template;
use content::web_view::{Page, PageFrame, PageRenderer, RenderError};
use std::sync::Arc;

#[derive(Template)]
#[template(path = "content/page.html")]
struct ContentTemplate<'a> {
    page: &'a Page,
    frame: &'a PageFrame,
}

struct LocalBrandRenderer(Assets);
impl PageRenderer for LocalBrandRenderer {
    fn render(&self, page: &Page) -> Result<String, RenderError> {
        ContentTemplate {
            page,
            frame: &page.frame,
        }
        .render_with_values(&("assets", &self.0 as &dyn std::any::Any))
        .map_err(|_| RenderError)
    }
}

pub fn closed(
    config: &platform::config::Config,
    assets: &Assets,
) -> content::http::ContentWebState {
    content::http::ContentWebState {
        service: None,
        flags: None,
        renderer: Arc::new(LocalBrandRenderer(assets.clone())),
        secure_cookies: config.server.environment.is_production_like(),
        request_context: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use content::{
        markdown,
        web_view::{Editor, Link, PostView, TopicDetail, TopicRow},
    };

    fn renderer() -> LocalBrandRenderer {
        let missing = std::env::temp_dir().join(format!(
            "bbbcm-content-assets-{}",
            platform::transaction::new_operation_id()
        ));
        LocalBrandRenderer(
            Assets::load_for(platform::config::Environment::Test, &missing, false).unwrap(),
        )
    }

    #[test]
    fn content_ssr_escapes_user_fields_and_only_renders_sanitized_markdown() {
        let renderer = renderer();
        let attack = "<script>alert('synthetic')</script>";
        let mut page = Page::new(attack, "dark");
        page.username = Some(attack.into());
        page.profile_href = "/u/synthetic".into();
        page.show_list = true;
        page.topics.push(TopicRow {
            href: "/topics/1".into(),
            title: attack.into(),
            author: attack.into(),
            author_href: "/u/synthetic".into(),
            forum: "日常".into(),
            forum_href: "/daily".into(),
            created_at: "2026-01-01 00:00 UTC".into(),
            last_reply: None,
            replies: None,
            views: Some(0),
            title_parts: None,
            pinned: false,
            state: String::new(),
        });
        let safe = markdown::render(&format!(
            "**安全正文**\n\n{attack}\n\n[x](javascript:alert(1))"
        ));
        page.topic = Some(TopicDetail {
            title: attack.into(),
            forum: Link {
                label: "日常".into(),
                href: "/daily".into(),
                active: false,
            },
            tags: Vec::new(),
            state: String::new(),
            opening: PostView {
                author: attack.into(),
                author_href: "/u/synthetic".into(),
                created_at: "2026-01-01 00:00 UTC".into(),
                edited_at: Some("2026-01-01 01:00 UTC".into()),
                body_html: safe.html().into(),
                body_placeholder: None,
                floor: 1,
                permalink: "/posts/1".into(),
                quote_href: None,
                edit_href: None,
                delete_href: None,
                quote_target: None,
            },
            replies: Vec::new(),
            reply_href: None,
        });
        let html = renderer.render(&page).unwrap();
        assert!(!html.contains(attack));
        assert!(!html.contains("javascript:"));
        assert!(!html.contains("<script"));
        assert!(html.contains("<strong>安全正文</strong>"));
        assert!(html.contains("最后编辑"));
        assert!(html.contains("data-theme=\"dark\""));
        assert!(html.contains("id=\"post-1\""));
        assert!(!html.contains("合成浏览数"));
        assert!(!html.contains("今日热门"));
        assert!(!html.contains("固定合成数据"));
    }

    #[test]
    fn editor_preserves_raw_markdown_as_text_and_preview_matches_final_rules() {
        let renderer = renderer();
        let source = "</textarea><img src=x onerror=alert(1)>\n\n**安全**";
        let safe = markdown::render(source);
        let mut page = Page::new("编辑本人内容", "system");
        page.editor = Some(Editor {
            action: "/posts/1/edit".into(),
            csrf: "synthetic-csrf".into(),
            idempotency_key: "synthetic-key".into(),
            version: "1".into(),
            title: String::new(),
            body: source.into(),
            forum_id: String::new(),
            forum_choices: Vec::new(),
            tag_ids: String::new(),
            visibility: "public".into(),
            quote: String::new(),
            topic_fields: false,
            deleting: false,
            submit: "保存修改",
            preview_html: Some(safe.html().into()),
        });
        let html = renderer.render(&page).unwrap();
        assert!(!html.contains("</textarea><img"));
        assert!(!html.contains("<img src=x"));
        assert!(html.contains(safe.html()));
        assert!(html.contains("method=\"post\""));
        assert!(html.contains("value=\"preview\""));
        assert!(!html.contains("<script"));
    }
}
