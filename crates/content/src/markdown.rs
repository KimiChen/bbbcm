//! 唯一服务端 Markdown 路径：先转换不受信事件，再进行 HTML5 白名单消毒。
//! 预览与最终展示均调用 render。不读取数据库、不展开引用、不抓取 URL。

use std::collections::{HashMap, HashSet};
use std::fmt;

use ammonia::{Builder, UrlRelative};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};
use url::{Position, Url};

/// 仅能通过完整渲染/消毒创建，不能从持久化 HTML 反序列化。
/// html() 仅可放在 HTML 元素内容中，不能用作属性、脚本或样式。
#[derive(Clone, PartialEq, Eq)]
pub struct SafeMarkdown {
    html: String,
    plain_text: String,
    links: Vec<MarkdownLink>,
}

impl SafeMarkdown {
    pub fn html(&self) -> &str {
        &self.html
    }

    /// 同一解析结果的可见文本，供搜索源使用；不是有效贡献或最短长度口径。
    pub fn plain_text(&self) -> &str {
        &self.plain_text
    }

    /// 可点击 HTTP(S) 外链出现次数，重复链接分别计数。
    /// 不含代码、原始 HTML、被禁图片、未链接普通文本。绝对 URL 保守地算外链；
    /// 站内链接用根相对路径，避免依赖部署域名或不受信 Host 请求头。
    pub fn external_link_count(&self) -> usize {
        self.links.iter().filter(|link| link.external).count()
    }

    pub fn links(&self) -> &[MarkdownLink] {
        &self.links
    }
}

impl fmt::Debug for SafeMarkdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 正文和 URL 查询参数可能含隐私；调试日志不自动包含它们。
        f.debug_struct("SafeMarkdown")
            .field("html_bytes", &self.html.len())
            .field("links", &self.links.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MarkdownLink {
    destination: String,
    external: bool,
    target: Option<LocalLinkTarget>,
}

impl MarkdownLink {
    pub fn destination(&self) -> &str {
        &self.destination
    }

    pub fn is_external(&self) -> bool {
        self.external
    }

    /// 仅语法识别，不证明目标存在、公开或允许被引用。
    pub fn target(&self) -> Option<LocalLinkTarget> {
        self.target
    }
}

impl fmt::Debug for MarkdownLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarkdownLink")
            .field("external", &self.external)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LocalLinkTarget {
    Topic {
        topic_id: i64,
        floor: Option<i64>,
    },
    Post {
        post_id: i64,
    },
    /// 由内容服务结合当前帖子所属主题解析，不能从请求方提供的主题认定权限。
    CurrentTopicFloor {
        floor: i64,
    },
}

/// Markdown 和预览唯一入口。原始 HTML 显示为文本，图片仅保留替代文本。
/// 支持 CommonMark、删除线、表格、只读任务标记；代码仅提供安全语言标记，
/// 不声称完成语法高亮。提及、自动链接和引用正文展开留后续。
pub fn render(source: &str) -> SafeMarkdown {
    render_filtered(source, |_| true)
}

/// 以同一套 Markdown/消毒规则移除当前不可访问目标的超链接，保留作者输入的文字。
///
/// 内容服务先用 render() 得到全部目标，在当前只读事务中检查权限，再传入允许集合。
/// 不可见引用不会导致来源帖隐藏，也不会展开目标标题或正文。结果的 links() 仅包含
/// 实际输出的链接；发布政策须使用过滤前的 render() 结果，避免绕过外链/引用检查。
pub fn render_filtered(
    source: &str,
    mut allow_target: impl FnMut(LocalLinkTarget) -> bool,
) -> SafeMarkdown {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut events = Vec::new();
    let mut links = Vec::new();
    let mut active_links = Vec::new();
    let mut image_depth = 0_usize;
    let mut languages = HashSet::new();

    for event in Parser::new_ext(source, options) {
        // 图片子树不能重新启用链接或 HTML，仅保留文字供无障碍阅读。
        if image_depth > 0 {
            match event {
                Event::Start(Tag::Image { .. }) => image_depth += 1,
                Event::End(TagEnd::Image) => {
                    image_depth -= 1;
                    if image_depth == 0 {
                        events.push(Event::Text("]".into()));
                    }
                }
                Event::Text(text)
                | Event::Code(text)
                | Event::Html(text)
                | Event::InlineHtml(text) => {
                    events.push(Event::Text(text));
                }
                Event::SoftBreak | Event::HardBreak => events.push(Event::Text(" ".into())),
                _ => {}
            }
            continue;
        }

        match event {
            Event::Html(text) | Event::InlineHtml(text) => events.push(Event::Text(text)),
            Event::Start(Tag::Image { .. }) => {
                image_depth = 1;
                events.push(Event::Text("[图片未启用：".into()));
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let link =
                    parse_link(&dest_url).filter(|link| link.target.is_none_or(&mut allow_target));
                active_links.push(link.is_some());
                if let Some(link) = link {
                    events.push(Event::Start(Tag::Link {
                        link_type,
                        dest_url: link.destination.clone().into(),
                        title,
                        id,
                    }));
                    links.push(link);
                }
            }
            Event::End(TagEnd::Link) => {
                if active_links.pop() == Some(true) {
                    events.push(Event::End(TagEnd::Link));
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                let language = safe_language(&info);
                if !language.is_empty() {
                    languages.insert(format!("language-{language}"));
                }
                events.push(Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(
                    language.into(),
                ))));
            }
            Event::TaskListMarker(checked) => {
                events.push(Event::Text(if checked { "☑ " } else { "☐ " }.into()));
            }
            Event::InlineMath(text) | Event::DisplayMath(text) | Event::FootnoteReference(text) => {
                events.push(Event::Text(text));
            }
            event => events.push(event),
        }
    }

    let plain_text = extract_plain_text(&events);
    let mut rendered = String::new();
    html::push_html(&mut rendered, events.into_iter());
    SafeMarkdown {
        html: sanitize(&rendered, &languages),
        plain_text,
        links,
    }
}

fn safe_language(info: &str) -> String {
    let language = info.split_whitespace().next().unwrap_or_default();
    if language.len() <= 32
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+'))
    {
        language.to_ascii_lowercase()
    } else {
        String::new()
    }
}

fn sanitize(rendered: &str, languages: &HashSet<String>) -> String {
    let mut sanitizer = Builder::empty();
    sanitizer
        .tags(HashSet::from([
            "p",
            "br",
            "hr",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "strong",
            "em",
            "del",
            "blockquote",
            "ul",
            "ol",
            "li",
            "pre",
            "code",
            "a",
            "table",
            "thead",
            "tbody",
            "tr",
            "th",
            "td",
        ]))
        .tag_attributes(HashMap::from([
            ("a", HashSet::from(["href", "title"])),
            ("ol", HashSet::from(["start"])),
        ]))
        .allowed_classes(HashMap::from([(
            "code",
            languages.iter().map(String::as_str).collect(),
        )]))
        .url_schemes(HashSet::from(["http", "https"]))
        .url_relative(UrlRelative::PassThrough)
        .link_rel(Some("nofollow ugc noopener noreferrer"))
        .strip_comments(true)
        .attribute_filter(|tag, attribute, value| {
            // 与解析器采用同一 URL 规则，防 HTML5 重解析产生另一条授权路径。
            if tag == "a" && attribute == "href" && parse_link(value).is_none() {
                None
            } else {
                Some(value.into())
            }
        });
    sanitizer.clean(rendered).to_string()
}

fn extract_plain_text(events: &[Event<'_>]) -> String {
    let mut plain = String::new();
    for event in events {
        match event {
            Event::Text(text) | Event::Code(text) => plain.push_str(text),
            Event::SoftBreak | Event::HardBreak | Event::Rule => plain.push('\n'),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::HtmlBlock
                | TagEnd::Item
                | TagEnd::TableHead
                | TagEnd::TableRow,
            ) if !plain.ends_with('\n') => plain.push('\n'),
            Event::End(TagEnd::TableCell) => plain.push(' '),
            _ => {}
        }
    }
    plain.trim().to_owned()
}

fn parse_link(destination: &str) -> Option<MarkdownLink> {
    if destination.is_empty()
        || destination
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
        || contains_encoded_control(destination)
    {
        return None;
    }
    if destination.starts_with('/') && !destination.starts_with("//") {
        // 保留域仅用于无网络的 URL 标准化，不是运行时站点配置或外部依赖。
        let base = Url::parse("https://markdown.invalid/").ok()?;
        let parsed = base.join(destination).ok()?;
        if parsed.origin() != base.origin() || parsed.path().starts_with("//") {
            return None;
        }
        return Some(MarkdownLink {
            destination: parsed[Position::BeforePath..].to_owned(),
            external: false,
            target: local_target(&parsed),
        });
    }
    if destination.starts_with('#') {
        return Some(MarkdownLink {
            destination: destination.to_owned(),
            external: false,
            target: destination
                .strip_prefix("#post-")
                .and_then(positive_id)
                .map(|floor| LocalLinkTarget::CurrentTopicFloor { floor }),
        });
    }
    let parsed = Url::parse(destination).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !destination
            .get(..parsed.scheme().len() + 3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&format!("{}://", parsed.scheme())))
        || !parsed.has_host()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    Some(MarkdownLink {
        destination: parsed.to_string(),
        external: true,
        target: None,
    })
}

fn contains_encoded_control(value: &str) -> bool {
    value.as_bytes().windows(3).any(|window| {
        if window[0] != b'%' {
            return false;
        }
        let Some(high) = (window[1] as char).to_digit(16) else {
            return false;
        };
        let Some(low) = (window[2] as char).to_digit(16) else {
            return false;
        };
        let byte = high * 16 + low;
        byte <= 0x1f || byte == 0x7f || byte == u32::from(b'\\')
    })
}

fn local_target(parsed: &Url) -> Option<LocalLinkTarget> {
    let parts = parsed
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    match parts.as_slice() {
        ["topics", topic] => Some(LocalLinkTarget::Topic {
            topic_id: positive_id(topic)?,
            floor: parsed
                .fragment()
                .and_then(|f| f.strip_prefix("post-"))
                .and_then(positive_id),
        }),
        ["posts", post] => Some(LocalLinkTarget::Post {
            post_id: positive_id(post)?,
        }),
        _ => None,
    }
}

fn positive_id(value: &str) -> Option<i64> {
    if value.starts_with('0') || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|id| *id > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_tables_tasks_and_code_share_one_sanitized_render() {
        let body = "# 标题\n\n**粗体** *斜体* ~~删除~~ `行内<>`\n\n> 引用\n\n3. 顺序\n\n- [x] 已办\n- [ ] 未办\n\n| 名称 | 值 |\n|---|---|\n| 草莓 | 1 |\n\n```Rust\nlet x = \"<script>\";\n```\n";
        let result = render(body);
        for tag in [
            "<h1>",
            "<strong>",
            "<em>",
            "<del>",
            "<blockquote>",
            "<ol start=\"3\">",
            "<ul>",
            "<table>",
            "<thead>",
            "<tbody>",
        ] {
            assert!(result.html().contains(tag), "missing {tag}");
        }
        assert!(result.html().contains("☑ 已办"));
        assert!(result.html().contains("☐ 未办"));
        assert!(!result.html().contains("<input"));
        assert!(result.html().contains("<code class=\"language-rust\">"));
        assert!(result.html().contains("&lt;script&gt;"));
        assert!(result.plain_text().contains("行内<>"));
        assert_eq!(result.external_link_count(), 0);
        assert_eq!(
            render(body),
            result,
            "preview and display have one deterministic entry"
        );
    }

    #[test]
    fn all_raw_html_is_visible_text_including_events_embeds_and_comments() {
        for source in [
            "<script>alert('x')</script>",
            "<img src=x onerror=alert(1)>",
            "text <b onclick=alert(1)>bold</b>",
            "<iframe src='https://example.test/'></iframe>",
            "<svg><a xlink:href='javascript:alert(1)'>x</a></svg>",
            "<math><mtext><table><mglyph><style><!--</style><img title='--><img src=x onerror=alert(1)>'>",
            "<!-- private-looking comment -->",
        ] {
            let result = render(source);
            assert!(
                result.html().contains("&lt;"),
                "must display raw HTML: {source}"
            );
            for forbidden in [
                "<script", "<img", "<iframe", "<svg", "<math", "<style", "<!--", "<b ",
            ] {
                assert!(!result.html().contains(forbidden), "active {forbidden}");
            }
            assert!(result.plain_text().contains('<'));
            assert!(result.links().is_empty());
        }
    }

    #[test]
    fn unsafe_urls_never_become_clickable_in_any_markdown_link_form() {
        for destination in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "java&#x73;cript:alert(1)",
            "javascript&#58;alert(1)",
            "jav&#x09;ascript:alert(1)",
            "data:text/html;base64,PHNjcmlwdD4=",
            "vbscript:msgbox(1)",
            "file:///etc/passwd",
            "blob:https://example.test/token",
            "//example.test/x",
            "///example.test/x",
            "/%5cexample.test/x",
            "/a/..//example.test/x",
            "https://user:secret@example.test/",
            "https:example.test",
            "https:/example.test",
            "https://example.test/%0aevil",
            "mailto:user@example.test",
        ] {
            for source in [
                format!("[link]({destination})"),
                format!("[link][ref]\n\n[ref]: {destination}"),
            ] {
                let result = render(&source);
                assert!(
                    !result.html().contains("href="),
                    "unsafe clickable URL: {destination}"
                );
                assert_eq!(result.external_link_count(), 0);
            }
        }
    }

    #[test]
    fn safe_links_are_normalized_counted_and_local_targets_only_identified() {
        let result = render(
            "[主题](/topics/12) [楼层](/topics/12#post-51) [帖子](/posts/24) [本站](/forums/3) [外链](HTTPS://example.test?q=1&x=2) <https://example.test/a> [重复](https://example.test/a)",
        );
        assert_eq!(result.external_link_count(), 3);
        assert_eq!(result.links().len(), 7);
        assert_eq!(
            result.links()[0].target(),
            Some(LocalLinkTarget::Topic {
                topic_id: 12,
                floor: None
            })
        );
        assert_eq!(
            result.links()[1].target(),
            Some(LocalLinkTarget::Topic {
                topic_id: 12,
                floor: Some(51)
            })
        );
        assert_eq!(
            result.links()[2].target(),
            Some(LocalLinkTarget::Post { post_id: 24 })
        );
        assert_eq!(result.links()[3].target(), None);
        assert!(
            result
                .html()
                .contains("href=\"https://example.test/?q=1&amp;x=2\"")
        );
        assert!(
            result
                .html()
                .contains("rel=\"nofollow ugc noopener noreferrer\"")
        );
    }

    #[test]
    fn current_floor_and_permanent_link_targets_are_unambiguous() {
        let result = render("[本页](#post-2) [同主题](/topics/12#post-2) [永久](/posts/24)");
        assert_eq!(
            result
                .links()
                .iter()
                .map(MarkdownLink::target)
                .collect::<Vec<_>>(),
            vec![
                Some(LocalLinkTarget::CurrentTopicFloor { floor: 2 }),
                Some(LocalLinkTarget::Topic {
                    topic_id: 12,
                    floor: Some(2)
                }),
                Some(LocalLinkTarget::Post { post_id: 24 }),
            ]
        );
        assert!(result.links().iter().all(|link| !link.is_external()));
    }

    #[test]
    fn tightening_link_permissions_preserves_source_text_and_cannot_restore_a_link() {
        let source = "来源 **[已隐藏目标](/posts/3)** [会员](#post-2) [可见](/topics/12#post-4) [外链](https://example.test/)\n\n> 作者自己输入的引文。";
        let original = render(source);
        let narrowed = render_filtered(source, |target| {
            target
                == LocalLinkTarget::Topic {
                    topic_id: 12,
                    floor: Some(4),
                }
        });
        assert_eq!(original.plain_text(), narrowed.plain_text());
        assert!(!narrowed.html().contains("href=\"/posts/3\""));
        assert!(!narrowed.html().contains("href=\"#post-2\""));
        assert!(narrowed.html().contains("<strong>已隐藏目标</strong>"));
        assert!(narrowed.html().contains("href=\"/topics/12#post-4\""));
        assert!(narrowed.html().contains("作者自己输入的引文。"));
        assert_eq!(narrowed.links().len(), 2);
        assert_eq!(narrowed.external_link_count(), 1);
        assert_eq!(render_filtered(source, |_| true), original);
    }

    #[test]
    fn disabled_images_cannot_load_or_reactivate_alt_links() {
        let result = render(
            "![头像 <img src=x onerror=alert(1)>](https://example.test/pixel?secret=1) ![local](/static/photo.png) ![[嵌套](https://example.test/)](data:image/svg+xml,x)",
        );
        for forbidden in ["<img", "href=", "secret=1", "data:image"] {
            assert!(
                !result.html().contains(forbidden),
                "image escaped policy: {forbidden}"
            );
        }
        assert!(result.html().contains("[图片未启用："));
        assert!(result.links().is_empty());
    }

    #[test]
    fn code_and_link_titles_cannot_inject_attributes_or_break_out() {
        let result = render(
            "```rust\"onmouseover=alert(1)\n</code><script>alert(1)</script>\n```\n\n[x](https://example.test/ \"quote &quot; onclick=alert(1)\")\n\n`[x](https://example.test/)`\n",
        );
        assert!(!result.html().contains("<script"));
        assert!(!result.html().contains("language-rust"));
        assert!(!result.html().contains("\" onclick="));
        assert_eq!(result.external_link_count(), 1);
        assert!(result.html().contains("&lt;/code&gt;"));
    }

    #[test]
    fn sanitizer_defense_removes_every_attribute_and_tag_outside_whitelist() {
        let raw = "<p id=x class=hidden style='color:red' onclick=x>safe</p><a href='javascript:alert(1)' target='_blank' download rel=opener>x</a><img src=x><iframe src=x></iframe><input type=checkbox><code class='language-rust hidden' onmouseover=x>code</code>";
        let cleaned = sanitize(raw, &HashSet::from(["language-rust".to_owned()]));
        for forbidden in [
            " id=",
            " hidden",
            " style=",
            " onclick=",
            "javascript:",
            " target=",
            " download",
            "<img",
            "<iframe",
            "<input",
            "onmouseover",
        ] {
            assert!(
                !cleaned.contains(forbidden),
                "sanitizer retained {forbidden}"
            );
        }
        assert!(cleaned.contains("<code class=\"language-rust\">code</code>"));
    }

    #[test]
    fn empty_and_plain_text_keep_search_text_without_metadata_or_hidden_fetches() {
        assert_eq!(render("").html(), "");
        assert_eq!(render("").plain_text(), "");
        let result = render(
            "# 草莓\n\n[标签](https://example.test/?secret=query \"hidden title\")\n\n```rust\ncode\n```",
        );
        assert_eq!(result.plain_text(), "草莓\n标签\ncode");
        assert!(!format!("{result:?}").contains("草莓"));
        assert!(!format!("{:?}", result.links()).contains("secret"));
    }
}
