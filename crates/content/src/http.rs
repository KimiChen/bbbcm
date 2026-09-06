//! Untrusted HTTP parsing and SSR projection; all authority and data access live in core.
use crate::{contract::*, core::ContentService, web_view as view};
use axum::{
    Form, Router,
    extract::{ConnectInfo, DefaultBodyLimit, OriginalUri, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use chrono::Utc;
use platform::flags::{Flag, FlagReader};
use serde::Deserialize;
use std::{any::Any, collections::HashMap, net::SocketAddr, sync::Arc};

pub trait RequestContext: Send + Sync {
    fn prepare<'a>(
        &'a self,
        headers: &'a HeaderMap,
        peer: Option<SocketAddr>,
        actor_id: i64,
        action: Action,
    ) -> FutureResult<'a, Arc<dyn Any + Send + Sync>>;

    /// 为一次主题详情页浏览派生去重主体（DEC-054）。
    ///
    /// 由应用装配层实现，因为只有它持有 HMAC 密钥；内容领域始终拿不到原始用户 ID、
    /// 会话 Cookie 或设备指纹。返回 `None` 表示这次访问**不应计数**——已知爬虫 UA、
    /// 被标记分析排除的账号，或者派生所需材料缺失，都走这一条，宁可少计不可错计。
    fn view_subject(
        &self,
        headers: &HeaderMap,
        peer: Option<SocketAddr>,
        viewer_id: Option<i64>,
        excluded: bool,
    ) -> Option<ViewSubject>;
}
#[derive(Clone)]
pub struct ContentWebState {
    pub service: Option<Arc<ContentService>>,
    pub flags: Option<Arc<dyn FlagReader>>,
    pub renderer: Arc<dyn view::PageRenderer>,
    pub secure_cookies: bool,
    pub request_context: Option<Arc<dyn RequestContext>>,
}
pub fn router<S: Clone + Send + Sync + 'static>(state: ContentWebState) -> Router<S> {
    let mut router = Router::new();
    for path in [
        "/",
        "/new",
        "/pinned",
        "/featured",
        "/forums/{slug}",
        "/tags/{slug}",
        "/topics/{id}",
        "/posts/{id}",
        "/me/topics",
        "/me/posts",
        "/me/replies",
        "/u/{id}/topics",
        "/u/{id}/posts",
        "/u/{id}/replies",
    ] {
        router = router.route(path, get(read));
    }
    for path in [
        "/topics/new",
        "/topics/{id}/reply",
        "/posts/{id}/edit",
        "/posts/{id}/delete",
    ] {
        router = router.route(path, get(editor).post(submit));
    }
    router
        .route("/content/operations", axum::routing::post(operation))
        .layer(DefaultBodyLimit::max(1_048_576))
        .with_state(state)
}

#[derive(Default, Deserialize)]
struct ReadQuery {
    #[serde(default)]
    theme: String,
    cursor: Option<String>,
    quote: Option<String>,
}
fn theme(query: &ReadQuery) -> Result<&'static str, ContentError> {
    match query.theme.as_str() {
        "" | "system" => Ok("system"),
        "light" => Ok("light"),
        "dark" => Ok("dark"),
        _ => Err(ContentError::InvalidInput),
    }
}
fn positive(value: &str) -> Result<i64, ContentError> {
    value
        .parse::<i64>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or(ContentError::InvalidInput)
}
fn cookie(headers: &HeaderMap, secure: bool, name: &str) -> Option<String> {
    let wanted = format!("{}bcm_{name}", if secure { "__Host-" } else { "" });
    let mut values = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .filter_map(|h| h.trim().split_once('='))
        .filter(|(name, _)| *name == wanted)
        .map(|(_, value)| value);
    let value = values.next()?;
    if values.next().is_some()
        || value.len() > 256
        || value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    Some(value.into())
}
fn origin(headers: &HeaderMap, secure: bool) -> Result<(), ContentError> {
    if headers
        .get("sec-fetch-site")
        .is_some_and(|h| h != "same-origin" && h != "none")
    {
        return Err(ContentError::Forbidden);
    }
    let origins: Vec<_> = headers.get_all(header::ORIGIN).iter().collect();
    if origins.len() > 1 {
        return Err(ContentError::Forbidden);
    }
    if let Some(origin) = origins.first() {
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .ok_or(ContentError::Forbidden)?;
        let expected = format!("{}://{host}", if secure { "https" } else { "http" });
        if origin.to_str().ok() != Some(expected.as_str()) {
            return Err(ContentError::Forbidden);
        }
    }
    Ok(())
}
fn credentials(
    state: &ContentWebState,
    headers: &HeaderMap,
    form: &HashMap<String, String>,
) -> Result<Credentials, ContentError> {
    origin(headers, state.secure_cookies)?;
    let session =
        cookie(headers, state.secure_cookies, "session").ok_or(ContentError::Forbidden)?;
    let csrf = cookie(headers, state.secure_cookies, "csrf").ok_or(ContentError::Forbidden)?;
    if value(form, "csrf")? != csrf {
        return Err(ContentError::Forbidden);
    }
    Ok(Credentials::new(
        session,
        csrf,
        value(form, "idempotency_key")?,
    ))
}
async fn gate(
    state: &ContentWebState,
    action: Option<Action>,
) -> Result<Arc<ContentService>, ContentError> {
    let flags = state
        .flags
        .as_ref()
        .ok_or(ContentError::NotFound)?
        .load()
        .await
        .map_err(|_| ContentError::Unavailable)?;
    if !flags.get(Flag::PublicRead).enabled {
        return Err(ContentError::NotFound);
    }
    if let Some(action) = action {
        let flag = if action == Action::CreateTopic {
            Flag::TopicCreation
        } else if action == Action::CreateReply {
            Flag::ReplyCreation
        } else {
            Flag::PublicRead
        };
        if !flags.get(flag).enabled || flags.get(Flag::ReadOnlyMode).enabled {
            return Err(ContentError::NotFound);
        }
    }
    state.service.clone().ok_or(ContentError::Unavailable)
}
fn response(state: &ContentWebState, page: &view::Page, status: StatusCode) -> Response {
    let mut result = match state.renderer.render(page) {
        Ok(html) => (status, Html(html)).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "服务暂不可用").into_response(),
    };
    secure_headers(&mut result);
    result
}
fn secure_headers(result: &mut Response) {
    let headers = result.headers_mut();
    headers.insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
    headers.insert("referrer-policy", "no-referrer".parse().unwrap());
    headers.insert(
        "x-robots-tag",
        "noindex, nofollow, noarchive".parse().unwrap(),
    );
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("content-security-policy","default-src 'self'; script-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'".parse().unwrap());
}
fn failure(state: &ContentWebState, error: ContentError) -> Response {
    let (status, message) = match error {
        ContentError::NotFound => (StatusCode::NOT_FOUND, "内容不存在、不可访问或功能尚未开放"),
        ContentError::Forbidden => (
            StatusCode::FORBIDDEN,
            "当前账号无法执行此操作，请重新登录或检查发布资格",
        ),
        ContentError::InvalidInput => (StatusCode::BAD_REQUEST, "输入无效，请检查内容后重新提交"),
        ContentError::Conflict => (StatusCode::CONFLICT, "内容已经变更或请求不一致，请重新加载"),
        ContentError::PolicyPending => (
            StatusCode::SERVICE_UNAVAILABLE,
            "内容政策尚未批准，发布与本人维护暂未开放",
        ),
        ContentError::QuotaExceeded => (StatusCode::TOO_MANY_REQUESTS, "发布额度已用完"),
        ContentError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "请求过于频繁，请稍后重试"),
        ContentError::Unavailable | ContentError::RetryableDatabase(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "请求暂未确认，请保留原请求重试",
        ),
        ContentError::InvalidCursor => {
            (StatusCode::BAD_REQUEST, "分页凭据无效，请从第一页重新开始")
        }
        ContentError::CommitUnknown { .. } => {
            (StatusCode::ACCEPTED, "提交结果尚待确认，请查询原操作")
        }
    };
    let mut page = view::Page::new("内容服务", "system");
    page.frame.error = Some(message);
    response(state, &page, status)
}
fn value<'a>(form: &'a HashMap<String, String>, name: &str) -> Result<&'a str, ContentError> {
    form.get(name)
        .map(String::as_str)
        .ok_or(ContentError::InvalidInput)
}
fn link(label: impl Into<String>, href: impl Into<String>) -> view::Link {
    view::Link {
        label: label.into(),
        href: href.into(),
        active: false,
    }
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn decode_segment(value: &str) -> Result<String, ContentError> {
    let mut bytes = Vec::new();
    let input = value.as_bytes();
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%' {
            let pair = input
                .get(index + 1..index + 3)
                .ok_or(ContentError::InvalidInput)?;
            let pair = std::str::from_utf8(pair).map_err(|_| ContentError::InvalidInput)?;
            bytes.push(u8::from_str_radix(pair, 16).map_err(|_| ContentError::InvalidInput)?);
            index += 3;
        } else {
            bytes.push(input[index]);
            index += 1;
        }
    }
    let result = String::from_utf8(bytes).map_err(|_| ContentError::InvalidInput)?;
    if result
        .chars()
        .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return Err(ContentError::InvalidInput);
    }
    Ok(result)
}
fn page_href(path: &str, theme: &str, cursor: Option<&str>) -> String {
    let mut href = format!("{path}?theme={theme}");
    if let Some(cursor) = cursor {
        href.push_str("&cursor=");
        href.push_str(&encode(cursor));
    }
    href
}
fn state_label(state: &ContentState) -> String {
    let mut labels = Vec::new();
    if state.publication == Publication::Pending {
        labels.push("待审")
    }
    if state.publication == Publication::Draft {
        labels.push("草稿")
    }
    if state.visibility == Visibility::Members {
        labels.push("仅会员")
    }
    if state.locked {
        labels.push("已锁定")
    }
    if state.archived_at.is_some() {
        labels.push("已归档")
    }
    labels.join(" · ")
}
async fn frame(
    service: &ContentService,
    headers: &HeaderMap,
    state: &ContentWebState,
    path: &str,
    query: &ReadQuery,
) -> Result<(view::Page, ContentList), ContentError> {
    let theme = theme(query)?;
    let session = cookie(headers, state.secure_cookies, "session");
    let catalog = service
        .list(
            &ListFilter {
                order: ListOrder::LatestTopic,
                ..ListFilter::default()
            },
            None,
            session.as_deref(),
            Utc::now(),
        )
        .await?;
    let mut page = view::Page::new("草莓论坛", theme);
    page.first = page_href(path, theme, None);
    for (label, selected) in [("浅色", "light"), ("深色", "dark"), ("跟随系统", "system")] {
        let mut choice = link(label, page_href(path, selected, query.cursor.as_deref()));
        choice.active = theme == selected;
        page.theme_links.push(choice)
    }
    page.forums
        .push(link("全部主题", page_href("/", theme, None)));
    page.forums.extend(catalog.forums.iter().map(|forum| {
        link(
            &forum.name,
            page_href(&format!("/forums/{}", encode(&forum.slug)), theme, None),
        )
    }));
    page.latest = catalog
        .topics
        .iter()
        .take(10)
        .map(|topic| {
            link(
                &topic.title,
                page_href(&format!("/topics/{}", topic.id), theme, None),
            )
        })
        .collect();
    if let Some(id) = catalog.viewer.user_id
        && let Some(author) = service.author(id, Utc::now()).await?
    {
        page.username = Some(author.username);
        page.profile_href = format!("/u/{}", encode(&author.public_id));
    }
    Ok((page, catalog))
}
fn row(topic: TopicView, theme: &str) -> view::TopicRow {
    let last_reply = topic
        .last_reply_author
        .zip(topic.last_reply_at)
        .map(|(author, at)| format!("{} · {}", author.username, at.format("%Y-%m-%d %H:%M UTC")));
    view::TopicRow {
        href: page_href(&format!("/topics/{}", topic.id), theme, None),
        title: topic.title,
        author: topic.author.username,
        author_href: format!("/u/{}", encode(&topic.author.public_id)),
        forum: topic.forum.name,
        forum_href: page_href(
            &format!("/forums/{}", encode(&topic.forum.slug)),
            theme,
            None,
        ),
        created_at: topic.created_at.format("%Y-%m-%d %H:%M UTC").to_string(),
        last_reply,
        replies: topic.reply_count,
        views: topic.view_count,
        pinned: topic.pinned || topic.globally_pinned,
        state: state_label(&topic.state),
    }
}
fn post(post: PostView, viewer: Viewer, theme: &str) -> view::PostView {
    let owner = viewer.user_id == Some(post.author.user_id);
    let id = post.id;
    view::PostView {
        author: post.author.username,
        author_href: format!("/u/{}", encode(&post.author.public_id)),
        created_at: post.created_at.format("%Y-%m-%d %H:%M UTC").to_string(),
        edited_at: post
            .edited_at
            .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string()),
        body_html: post.html,
        // 正文已在服务端投影阶段清空，这里只补一句占位文案，模板不做任何隐藏逻辑。
        body_placeholder: (!post.body_state.visible())
            .then(|| post.body_state.placeholder().to_owned()),
        floor: post.floor_no,
        permalink: format!("/posts/{id}?theme={theme}"),
        quote_href: viewer
            .user_id
            .map(|_| format!("/topics/{}/reply?theme={theme}&quote={id}", post.topic_id)),
        edit_href: owner.then(|| format!("/posts/{id}/edit?theme={theme}")),
        delete_href: owner.then(|| format!("/posts/{id}/delete?theme={theme}")),
        quote_target: post
            .quote_post_id
            .map(|id| link("查看引用目标", format!("/posts/{id}?theme={theme}"))),
    }
}
async fn read(
    State(state): State<ContentWebState>,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<ReadQuery>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let service = gate(&state, None).await?;
        let path = uri.path();
        let parts: Vec<_> = path.trim_matches('/').split('/').collect();
        let session = cookie(&headers, state.secure_cookies, "session");
        if parts.first() == Some(&"posts") {
            let location = service
                .locate(positive(parts[1])?, session.as_deref(), Utc::now())
                .await?;
            let href = format!(
                "{}#post-{}",
                page_href(
                    &format!("/topics/{}", location.topic_id),
                    theme(&query)?,
                    location.cursor.as_deref()
                ),
                location.floor_no
            );
            let mut response = Redirect::to(&href).into_response();
            secure_headers(&mut response);
            return Ok(response);
        }
        let (mut page, catalog) = frame(&service, &headers, &state, path, &query).await?;
        if parts.first() == Some(&"topics") {
            let detail = service
                .topic(
                    positive(parts[1])?,
                    query.cursor.as_deref(),
                    session.as_deref(),
                    Utc::now(),
                )
                .await?;
            // DEC-054：主题详情页服务端渲染成功后登记一次浏览。
            //
            // 放在这里而不是 `service.topic()` 内部，是因为触发条件是「SSR 成功」而不是
            // 「读到了主题」；同时读路径是只读事务，计数必须走独立的短写事务。
            // 任何失败只记观测并继续渲染：丢一次阅读量，远好过让统计把正常阅读变成错误页。
            if let Some(subject) = state.request_context.as_ref().and_then(|context| {
                context.view_subject(
                    &headers,
                    None,
                    detail.viewer.user_id,
                    detail.viewer.excluded,
                )
            }) {
                match service
                    .record_view(detail.topic.id, &subject, detail.viewer.user_id, Utc::now())
                    .await
                {
                    Ok(counted) => tracing::debug!(counted, "topic view registered"),
                    Err(_) => tracing::debug!("topic view registration skipped"),
                }
            }
            page.title = detail.topic.title.clone();
            page.next = detail
                .next_cursor
                .map(|c| page_href(path, page.theme, Some(&c)));
            let may_reply = detail.viewer.user_id.is_some() && detail.topic.state.permits_write();
            page.topic = Some(view::TopicDetail {
                title: detail.topic.title,
                forum: link(
                    detail.topic.forum.name,
                    page_href(
                        &format!("/forums/{}", encode(&detail.topic.forum.slug)),
                        page.theme,
                        None,
                    ),
                ),
                tags: detail
                    .topic
                    .tags
                    .into_iter()
                    .map(|tag| {
                        link(
                            tag.name,
                            page_href(&format!("/tags/{}", encode(&tag.slug)), page.theme, None),
                        )
                    })
                    .collect(),
                state: state_label(&detail.topic.state),
                opening: post(detail.first_post, detail.viewer, page.theme),
                replies: detail
                    .replies
                    .into_iter()
                    .map(|p| post(p, detail.viewer, page.theme))
                    .collect(),
                reply_href: may_reply.then(|| format!("{path}/reply?theme={}", page.theme)),
            });
        } else {
            let mut filter = ListFilter::default();
            page.show_list = true;
            match parts.first().copied().unwrap_or("") {
                "new" => {
                    filter.order = ListOrder::LatestTopic;
                    page.title = "最新主题".into()
                }
                "pinned" => {
                    filter.order = ListOrder::Pinned;
                    page.title = "置顶".into()
                }
                "featured" => {
                    filter.order = ListOrder::Featured;
                    page.title = "精华".into()
                }
                "forums" => {
                    let slug = decode_segment(parts[1])?;
                    let forum = catalog
                        .forums
                        .iter()
                        .find(|f| f.slug == slug)
                        .ok_or(ContentError::NotFound)?;
                    filter.forum_id = Some(forum.id);
                    page.title = forum.name.clone()
                }
                "tags" => {
                    let slug = decode_segment(parts[1])?;
                    let tag = service
                        .resolve_tag(&slug, session.as_deref(), Utc::now())
                        .await?
                        .ok_or(ContentError::NotFound)?;
                    filter.tag_id = Some(tag.id);
                    page.title = tag.name.clone()
                }
                "me" => {
                    filter.owner_id = Some(catalog.viewer.user_id.ok_or(ContentError::Forbidden)?);
                    filter.kind = if parts[1] == "topics" {
                        ListKind::Topics
                    } else {
                        ListKind::Replies
                    };
                    filter.order = ListOrder::LatestTopic;
                    page.title = if filter.kind == ListKind::Topics {
                        "我的主题"
                    } else {
                        "我的回复"
                    }
                    .into()
                }
                "u" => {
                    let author = service
                        .resolve_author(parts[1], Utc::now())
                        .await?
                        .ok_or(ContentError::NotFound)?;
                    filter.owner_id = Some(author.user_id);
                    filter.kind = if parts[2] == "topics" {
                        ListKind::Topics
                    } else {
                        ListKind::Replies
                    };
                    filter.order = ListOrder::LatestTopic;
                    page.title = format!(
                        "{}的{}",
                        author.username,
                        if filter.kind == ListKind::Topics {
                            "主题"
                        } else {
                            "回复"
                        }
                    )
                }
                _ => page.title = "最新回复".into(),
            }
            let list = service
                .list(
                    &filter,
                    query.cursor.as_deref(),
                    session.as_deref(),
                    Utc::now(),
                )
                .await?;
            page.next = list
                .next_cursor
                .map(|c| page_href(path, page.theme, Some(&c)));
            page.topics = list
                .topics
                .into_iter()
                .map(|t| row(t, page.theme))
                .collect();
            for reply in list.replies {
                page.topics.push(view::TopicRow {
                    href: format!("/posts/{}?theme={}", reply.id, page.theme),
                    title: format!(
                        "第 {} 楼 · {}",
                        reply.floor_no,
                        crate::markdown::render(&reply.markdown)
                            .plain_text()
                            .chars()
                            .take(120)
                            .collect::<String>()
                    ),
                    author: reply.author.username,
                    author_href: format!("/u/{}", encode(&reply.author.public_id)),
                    forum: String::new(),
                    forum_href: format!("/topics/{}", reply.topic_id),
                    created_at: reply.created_at.format("%Y-%m-%d %H:%M UTC").to_string(),
                    last_reply: None,
                    replies: None,
                    // 回复行展示的是单条回复本身，浏览数属于主题维度，这里不重复呈现。
                    views: None,
                    pinned: false,
                    state: if reply.publication == Publication::Pending {
                        "待审".into()
                    } else {
                        String::new()
                    },
                })
            }
            if filter.owner_id.is_none() {
                for (label, href, order) in [
                    ("最新回复", "/", ListOrder::LatestReply),
                    ("最新主题", "/new", ListOrder::LatestTopic),
                    ("置顶", "/pinned", ListOrder::Pinned),
                    ("精华", "/featured", ListOrder::Featured),
                ] {
                    let mut tab = link(label, page_href(href, page.theme, None));
                    tab.active = filter.order == order;
                    page.tabs.push(tab)
                }
            } else {
                let base = if parts[0] == "me" {
                    "/me".to_string()
                } else {
                    format!("/u/{}", parts[1])
                };
                page.tabs.push(link(
                    "主题",
                    page_href(&format!("{base}/topics"), page.theme, None),
                ));
                page.tabs.push(link(
                    "回复",
                    page_href(&format!("{base}/posts"), page.theme, None),
                ));
            }
        }
        Ok(response(&state, &page, StatusCode::OK))
    }
    .await;
    result.unwrap_or_else(|error| failure(&state, error))
}

fn action(path: &str) -> Result<Action, ContentError> {
    if path == "/topics/new" {
        Ok(Action::CreateTopic)
    } else if path.ends_with("/reply") {
        Ok(Action::CreateReply)
    } else if path.ends_with("/edit") {
        Ok(Action::EditPost)
    } else if path.ends_with("/delete") {
        Ok(Action::DeletePost)
    } else {
        Err(ContentError::NotFound)
    }
}
async fn edit_page(
    service: &ContentService,
    state: &ContentWebState,
    headers: &HeaderMap,
    path: &str,
    query: &ReadQuery,
) -> Result<view::Page, ContentError> {
    let (mut page, catalog) = frame(service, headers, state, path, query).await?;
    let owner = catalog.viewer.user_id.ok_or(ContentError::Forbidden)?;
    let session =
        cookie(headers, state.secure_cookies, "session").ok_or(ContentError::Forbidden)?;
    let csrf = cookie(headers, state.secure_cookies, "csrf").ok_or(ContentError::Forbidden)?;
    let action = action(path)?;
    page.title = match action {
        Action::CreateTopic => "发布主题",
        Action::CreateReply => "发表回复",
        Action::EditPost => "编辑本人内容",
        Action::DeletePost => "删除本人内容",
    }
    .into();
    let mut editor = view::Editor {
        action: path.into(),
        csrf,
        idempotency_key: platform::transaction::new_operation_id(),
        version: String::new(),
        title: String::new(),
        body: String::new(),
        forum_id: catalog
            .forums
            .first()
            .map(|f| f.id.to_string())
            .unwrap_or_default(),
        forum_choices: catalog
            .forums
            .iter()
            .map(|f| link(&f.name, f.id.to_string()))
            .collect(),
        tag_ids: String::new(),
        visibility: "public".into(),
        quote: String::new(),
        topic_fields: action == Action::CreateTopic,
        deleting: action == Action::DeletePost,
        submit: if action == Action::DeletePost {
            "确认删除"
        } else if action == Action::EditPost {
            "保存修改"
        } else {
            "发布"
        },
        preview_html: None,
    };
    if action != Action::CreateTopic {
        let id = positive(
            path.trim_matches('/')
                .split('/')
                .nth(1)
                .ok_or(ContentError::InvalidInput)?,
        )?;
        if action == Action::CreateReply {
            let detail = service.topic(id, None, Some(&session), Utc::now()).await?;
            if !detail.topic.state.permits_write() {
                return Err(ContentError::Forbidden);
            }
            editor.version = detail.topic.version.to_string();
            if let Some(quote) = query.quote.as_deref() {
                let quote_id = positive(quote)?;
                let location = service.locate(quote_id, Some(&session), Utc::now()).await?;
                if location.topic_id != id {
                    return Err(ContentError::Forbidden);
                }
                editor.quote = quote_id.to_string();
            }
        } else {
            let location = service.locate(id, Some(&session), Utc::now()).await?;
            let detail = service
                .topic(
                    location.topic_id,
                    location.cursor.as_deref(),
                    Some(&session),
                    Utc::now(),
                )
                .await?;
            let post = if detail.first_post.id == id {
                detail.first_post
            } else {
                detail
                    .replies
                    .into_iter()
                    .find(|p| p.id == id)
                    .ok_or(ContentError::NotFound)?
            };
            if post.author.user_id != owner {
                return Err(ContentError::Forbidden);
            }
            editor.version = post.version.to_string();
            editor.body = post.markdown;
            editor.topic_fields = post.floor_no == 1;
            if editor.topic_fields {
                editor.title = detail.topic.title;
                editor.forum_id = detail.topic.forum.id.to_string();
                editor.visibility = detail.topic.state.visibility.as_str().into();
                editor.tag_ids = detail
                    .topic
                    .tags
                    .into_iter()
                    .map(|t| t.name)
                    .collect::<Vec<_>>()
                    .join(", ");
            }
        }
    }
    page.editor = Some(editor);
    Ok(page)
}
async fn editor(
    State(state): State<ContentWebState>,
    OriginalUri(uri): OriginalUri,
    Query(query): Query<ReadQuery>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let service = gate(&state, Some(action(uri.path())?)).await?;
        let page = edit_page(&service, &state, &headers, uri.path(), &query).await?;
        Ok(response(&state, &page, StatusCode::OK))
    }
    .await;
    result.unwrap_or_else(|e| failure(&state, e))
}
async fn tags(
    service: &ContentService,
    session: &str,
    input: &str,
) -> Result<Vec<i64>, ContentError> {
    if input.trim().is_empty() {
        return Ok(Vec::new());
    }
    let list = service
        .list(&ListFilter::default(), None, Some(session), Utc::now())
        .await?;
    let mut tags = Vec::new();
    for name in input.split(',').map(str::trim) {
        let tag = list
            .tags
            .iter()
            .find(|tag| tag.name == name || tag.slug == name)
            .ok_or(ContentError::InvalidInput)?;
        if !tags.contains(&tag.id) {
            tags.push(tag.id)
        }
    }
    Ok(tags)
}
async fn submit(
    State(state): State<ContentWebState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let result = async {
        let path = uri.path();
        let action = action(path)?;
        let service = gate(&state, Some(action)).await?;
        let mut credentials = credentials(&state, &headers, &form)?;
        if let Some(factory) = state.request_context.as_ref() {
            let actor = service
                .viewer(Some(credentials.session_token.expose()), Utc::now())
                .await?
                .user_id
                .ok_or(ContentError::Forbidden)?;
            let checked_action = if form.get("intent").map(String::as_str) == Some("preview") {
                Action::EditPost
            } else {
                action
            };
            credentials.trusted_context = Some(
                factory
                    .prepare(&headers, peer.ok().map(|p| p.0), actor, checked_action)
                    .await?,
            );
        }
        let query = ReadQuery {
            theme: form.get("theme").cloned().unwrap_or_default(),
            cursor: None,
            quote: form.get("quote").filter(|s| !s.is_empty()).cloned(),
        };
        let theme = theme(&query)?;
        if form.get("intent").map(String::as_str) == Some("preview") {
            if action == Action::DeletePost {
                return Err(ContentError::InvalidInput);
            }
            let mut page = edit_page(&service, &state, &headers, path, &query).await?;
            let preview = service
                .preview(&credentials, value(&form, "body")?, Utc::now())
                .await?;
            let editor = page.editor.as_mut().ok_or(ContentError::Unavailable)?;
            editor.idempotency_key = credentials.idempotency_key.clone();
            editor.version = form.get("version").cloned().unwrap_or_default();
            editor.title = form.get("title").cloned().unwrap_or_default();
            editor.body = value(&form, "body")?.into();
            editor.tag_ids = form.get("tags").cloned().unwrap_or_default();
            editor.forum_id = form.get("forum_id").cloned().unwrap_or_default();
            editor.visibility = form
                .get("visibility")
                .cloned()
                .unwrap_or_else(|| "public".into());
            editor.preview_html = Some(preview.html);
            return Ok(response(&state, &page, StatusCode::OK));
        }
        let id = if action == Action::CreateTopic {
            0
        } else {
            positive(
                path.trim_matches('/')
                    .split('/')
                    .nth(1)
                    .ok_or(ContentError::InvalidInput)?,
            )?
        };
        let mutation = match action {
            Action::CreateTopic => {
                let tags = tags(
                    &service,
                    credentials.session_token.expose(),
                    form.get("tags").map(String::as_str).unwrap_or(""),
                )
                .await?;
                service
                    .create_topic(
                        &credentials,
                        NewTopic {
                            forum_id: positive(value(&form, "forum_id")?)?,
                            title: value(&form, "title")?.into(),
                            markdown: value(&form, "body")?.into(),
                            tag_ids: tags,
                            visibility: Visibility::parse(value(&form, "visibility")?)?,
                        },
                        Utc::now(),
                    )
                    .await
            }
            Action::CreateReply => {
                service
                    .reply(
                        &credentials,
                        NewReply {
                            topic_id: id,
                            markdown: value(&form, "body")?.into(),
                            quote_post_id: form
                                .get("quote")
                                .filter(|s| !s.is_empty())
                                .map(|s| positive(s))
                                .transpose()?,
                            expected_topic_version: positive(value(&form, "version")?)?,
                        },
                        Utc::now(),
                    )
                    .await
            }
            Action::EditPost => {
                let tag_ids = match form.get("tags") {
                    Some(tags_input) => {
                        Some(tags(&service, credentials.session_token.expose(), tags_input).await?)
                    }
                    None => None,
                };
                service
                    .edit(
                        &credentials,
                        EditPost {
                            post_id: id,
                            expected_version: positive(value(&form, "version")?)?,
                            title: form.get("title").cloned(),
                            markdown: value(&form, "body")?.into(),
                            tag_ids,
                            visibility: form
                                .get("visibility")
                                .map(|v| Visibility::parse(v))
                                .transpose()?,
                        },
                        Utc::now(),
                    )
                    .await
            }
            Action::DeletePost => {
                service
                    .delete(
                        &credentials,
                        DeletePost {
                            post_id: id,
                            expected_version: positive(value(&form, "version")?)?,
                        },
                        Utc::now(),
                    )
                    .await
            }
        };
        match mutation {
            Ok(result) => {
                let href = if action == Action::DeletePost {
                    format!("/me/topics?theme={theme}")
                } else {
                    format!("/posts/{}?theme={theme}", result.post_id)
                };
                let mut result = Redirect::to(&href).into_response();
                secure_headers(&mut result);
                Ok(result)
            }
            Err(ContentError::CommitUnknown { operation_id }) => {
                let mut page = view::Page::new("确认提交状态", theme);
                page.frame.flash =
                    Some("提交结果尚待确认。请查询原操作；未观察到记录不代表提交已回滚。");
                page.operation = Some(view::OperationQuery {
                    csrf: credentials.csrf.expose().into(),
                    idempotency_key: credentials.idempotency_key,
                    operation_id,
                    action: action.as_str().into(),
                });
                Ok(response(&state, &page, StatusCode::ACCEPTED))
            }
            Err(error) => Err(error),
        }
    }
    .await;
    result.unwrap_or_else(|e| failure(&state, e))
}
async fn operation(
    State(state): State<ContentWebState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let result = async {
        let service = gate(&state, None).await?;
        let credentials = credentials(&state, &headers, &form)?;
        let action = match value(&form, "action")? {
            "create_topic" => Action::CreateTopic,
            "create_reply" => Action::CreateReply,
            "edit_post" => Action::EditPost,
            "delete_post" => Action::DeletePost,
            _ => return Err(ContentError::InvalidInput),
        };
        let result = service
            .operation(
                &credentials,
                action,
                value(&form, "operation_id")?,
                Utc::now(),
            )
            .await?;
        let mut page = view::Page::new("提交状态", "system");
        page.frame.flash = Some(match result {
            platform::transaction::OperationStatus::Completed(_) => {
                "原操作已提交。请返回我的主题或回复查看内容。"
            }
            platform::transaction::OperationStatus::NotObserved => {
                "尚未观察到原操作。此结果不能证明已回滚，请保留原请求稍后查询。"
            }
            platform::transaction::OperationStatus::Pending => "原操作仍待确认，请稍后查询。",
            platform::transaction::OperationStatus::ResponseExpired => {
                "原操作回执已过期，请从我的内容核对结果。"
            }
        });
        page.operation = Some(view::OperationQuery {
            csrf: credentials.csrf.expose().into(),
            idempotency_key: credentials.idempotency_key,
            operation_id: value(&form, "operation_id")?.into(),
            action: action.as_str().into(),
        });
        Ok(response(&state, &page, StatusCode::OK))
    }
    .await;
    result.unwrap_or_else(|e| failure(&state, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_reject_duplicates_and_never_accept_a_secure_cookie_alias() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "bcm_session=abc123; bcm_csrf=csrf123".parse().unwrap(),
        );
        assert_eq!(
            cookie(&headers, false, "session").as_deref(),
            Some("abc123")
        );
        assert!(cookie(&headers, true, "session").is_none());
        headers.append(header::COOKIE, "bcm_session=second".parse().unwrap());
        assert!(cookie(&headers, false, "session").is_none());
        headers.insert(
            header::COOKIE,
            "bcm_session=contains%20space".parse().unwrap(),
        );
        assert!(cookie(&headers, false, "session").is_none());
    }

    #[test]
    fn origin_rejects_cross_origin_and_duplicate_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "localhost:18099".parse().unwrap());
        headers.insert(header::ORIGIN, "http://localhost:18099".parse().unwrap());
        assert!(origin(&headers, false).is_ok());
        assert!(origin(&headers, true).is_err());
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(origin(&headers, false).is_err());
        headers.remove("sec-fetch-site");
        headers.append(header::ORIGIN, "http://localhost:18099".parse().unwrap());
        assert!(origin(&headers, false).is_err());
    }

    #[test]
    fn theme_and_query_encoding_cannot_inject_attributes_or_change_the_cursor() {
        let query = ReadQuery {
            theme: "light\" onclick=bad".into(),
            ..ReadQuery::default()
        };
        assert!(theme(&query).is_err());
        assert_eq!(
            page_href("/topics/1", "dark", Some("a&theme=light#post-2")),
            "/topics/1?theme=dark&cursor=a%26theme%3Dlight%23post-2"
        );
    }

    struct Renderer;
    impl view::PageRenderer for Renderer {
        fn render(&self, _page: &view::Page) -> Result<String, view::RenderError> {
            Ok("closed".into())
        }
    }

    #[tokio::test]
    async fn default_closed_routes_never_expose_content_or_mutate() {
        use tower::ServiceExt;
        let state = ContentWebState {
            service: None,
            flags: None,
            renderer: Arc::new(Renderer),
            secure_cookies: false,
            request_context: None,
        };
        let router: Router = router(state);
        for route in [
            "/",
            "/new",
            "/pinned",
            "/featured",
            "/topics/1",
            "/topics/new",
            "/topics/1/reply",
            "/posts/1",
            "/posts/1/edit",
            "/posts/1/delete",
            "/me/topics",
            "/me/posts",
            "/u/synthetic/topics",
            "/u/synthetic/posts",
        ] {
            let response = router
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(route)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{route}");
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "private, no-store"
            );
            assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        }
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/topics/new")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from(
                        "body=private-body&csrf=secret&idempotency_key=private-key",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("private"));
    }
}
