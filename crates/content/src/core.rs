//! Authoritative content services: identity injection, current visibility and one commit.
use crate::{
    contract::*,
    cursor::{Cursor, CursorKey},
    markdown,
    policy::{ContentPolicy, ContentRules},
    repo,
};
use chrono::{DateTime, Duration, Utc};
use platform::{
    flags::{Flag, FlagSnapshot},
    identifiers::SchemaRenderer,
    outbox::{OutboxEvent, Outcome, SafePayload},
    transaction::{
        CommandIdentity, CommitOutcome, LockPlan, OperationQuery, OperationStatus, StoredResponse,
        new_operation_id, request_hash,
    },
};
use platform_db::{
    Pool,
    command::execute_command,
    transaction::{DbTransaction, TransactionConnection, TransactionError},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
const PAGE_SIZE: usize = 50;
#[derive(Clone)]
pub struct ContentService {
    pool: Pool,
    renderer: SchemaRenderer,
    authority: Arc<dyn ContentAuthority>,
    policy: Arc<dyn ContentPolicy>,
    cursor: CursorKey,
    cache: Option<Arc<dyn platform::cache::CacheStore>>,
    /// 当前检索后端。由装配层按后端能力注入，不由配置声称。
    search: Arc<dyn crate::search::SearchBackend>,
    /// `NAV-004` 站点统计的进程内短 TTL 缓存。
    ///
    /// 统计挂在每个公开页面的侧栏上，而它由三条**全表 COUNT** 算出
    /// （主题、回复、用户）。E02 吞吐实测（`99` §5.43）里，10 万主题 + 10 万帖子
    /// 下这三条构成首页请求的主要数据库开销，200 req/s 时系统饱和在约 130 req/s。
    ///
    /// 缓存的是聚合公开计数，不含任何按人可见性——统计只数已发布、未处置、未删除
    /// 的行，对所有访客同一份，因此不存在跨用户串味的问题。
    statistics: StatisticsCache,
}

/// 站点统计缓存的存活时间。
///
/// 取 30 秒与 `02` §12 的公开片段缓存 TTL 一致，并落在 `DEC-030` 规定的
/// 「删除／隐藏／权限收紧向搜索与缓存传播 ≤60 秒」之内——统计里的一条主题被删除
/// 后，最迟 30 秒该计数就会重算。
const STATISTICS_TTL_SECONDS: i64 = 30;

/// 站点统计缓存：`(算出的时刻, 计数)`。`Arc` 使 `ContentService` 的克隆共享同一份。
type StatisticsCache = Arc<std::sync::Mutex<Option<(DateTime<Utc>, SiteStatistics)>>>;
#[derive(Clone, serde::Serialize)]
enum Mutation {
    Topic(NewTopic),
    Reply(NewReply),
    Edit(EditPost),
    Delete(DeletePost),
}
impl Mutation {
    fn action(&self) -> Action {
        match self {
            Self::Topic(_) => Action::CreateTopic,
            Self::Reply(_) => Action::CreateReply,
            Self::Edit(_) => Action::EditPost,
            Self::Delete(_) => Action::DeletePost,
        }
    }
}
struct ReadContext {
    viewer: Viewer,
    forums: Vec<Forum>,
    tags: Vec<Tag>,
    now: DateTime<Utc>,
    /// 单次读请求内的备忘缓存：作者与主题。
    ///
    /// 列表页渲染 50 行时，同一个主题会被查两次（调用方一次、`basic_post_visible`
    /// 再查一次），同一个作者会被查三次（主题作者、可见性判定、首帖作者），而作者
    /// 在一页里往往高度重复。E02 实测：修复前首页一次渲染发约 820 条查询，其中
    /// 300 条是用户、200 条是主题（`99` §5.44）。
    ///
    /// 只在**一次读事务内**有效：`ReadContext` 随请求创建、随事务结束丢弃，因此不会
    /// 跨请求返回陈旧数据，也不需要失效策略。事务本身就是一致性快照，同一事务内
    /// 重复读到同一行是正确行为。
    authors: Memo<Author>,
    topics: Memo<repo::Topic>,
    /// 按主题预取的首帖与标签。列表路径在取回候选 id 后一次性填好，
    /// 详情页路径不填——那里只渲染一个主题，批量没有收益。
    first_posts: Memo<repo::Post>,
    topic_tags: Memo<Vec<i64>>,
}

/// 请求内备忘缓存。用 `std::sync::Mutex` 而不是 `RefCell`：读路径是 async，
/// 值可能跨线程移动；锁只在取值/存值的瞬间持有，绝不跨 `.await`。
type Memo<T> = std::sync::Mutex<std::collections::BTreeMap<i64, T>>;

fn memo_get<T: Clone>(memo: &Memo<T>, id: i64) -> Option<T> {
    memo.lock().ok()?.get(&id).cloned()
}

fn memo_put<T>(memo: &Memo<T>, id: i64, value: T) {
    if let Ok(mut guard) = memo.lock() {
        guard.insert(id, value);
    }
}
fn canonical(now: DateTime<Utc>) -> Result<DateTime<Utc>, ContentError> {
    if now < DateTime::UNIX_EPOCH {
        return Err(ContentError::InvalidInput);
    }
    platform::db::decode_time(&platform::db::encode_time(now))
        .map_err(|_| ContentError::InvalidInput)
}
fn positive(id: i64) -> Result<(), ContentError> {
    if id > 0 {
        Ok(())
    } else {
        Err(ContentError::InvalidInput)
    }
}
impl ContentService {
    pub fn new(
        pool: Pool,
        renderer: SchemaRenderer,
        authority: Arc<dyn ContentAuthority>,
        policy: Arc<dyn ContentPolicy>,
        cursor: CursorKey,
    ) -> Self {
        Self {
            pool,
            renderer,
            authority,
            policy,
            cursor,
            cache: None,
            // 默认体验模式。完整模式由装配层在探测到 pg_jieba 后显式注入——
            // 默认值取「能力更弱的那个」，漏接线时页面会如实说仅支持标题搜索，
            // 而不是声称支持全文却搜不到正文。
            search: Arc::new(crate::search::SqliteTitleSearch),
            statistics: Arc::new(std::sync::Mutex::new(None)),
        }
    }
    pub fn with_cache(mut self, cache: Arc<dyn platform::cache::CacheStore>) -> Self {
        self.cache = Some(cache);
        self
    }
    async fn flags(
        &self,
        tx: &mut TransactionConnection<'_>,
        action: Option<Action>,
    ) -> Result<(), ContentError> {
        let t = self
            .renderer
            .table(tx.backend(), "platform", "feature_flags")
            .map_err(|_| ContentError::Unavailable)?;
        let rows = tx
            .fetch_all(
                &format!("SELECT flag_key, enabled, version FROM {t} ORDER BY flag_key"),
                &[],
            )
            .await?;
        let facts = rows
            .iter()
            .map(|r| {
                Ok((
                    r.text("flag_key")?,
                    r.boolean("enabled")?,
                    r.i64("version")?,
                ))
            })
            .collect::<Result<Vec<_>, TransactionError>>()?;
        let flags = FlagSnapshot::from_rows(facts).map_err(|_| ContentError::Unavailable)?;
        let required = match action {
            Some(Action::CreateTopic) => Some(Flag::TopicCreation),
            Some(Action::CreateReply) => Some(Flag::ReplyCreation),
            _ => None,
        };
        if !flags.get(Flag::PublicRead).enabled
            || required.is_some_and(|f| !flags.get(f).enabled)
            || action.is_some() && flags.get(Flag::ReadOnlyMode).enabled
        {
            return Err(ContentError::Forbidden);
        }
        Ok(())
    }
    async fn context(
        &self,
        tx: &mut TransactionConnection<'_>,
        session: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<ReadContext, ContentError> {
        self.flags(tx, None).await?;
        let mut forums = repo::forums(tx, &self.renderer).await?;
        let ids = forums.iter().map(|f| f.id).collect::<Vec<_>>();
        for chunk in ids.chunks(256) {
            for (id, rule) in self.authority.forum_rules(tx, chunk, now).await? {
                if let Some(forum) = forums.iter_mut().find(|f| f.id == id) {
                    forum.rule = rule;
                }
            }
        }
        Ok(ReadContext {
            viewer: self.authority.read_viewer(tx, session, now).await?,
            forums,
            tags: repo::tags(tx, &self.renderer).await?,
            now,
            authors: Memo::default(),
            topics: Memo::default(),
            first_posts: Memo::default(),
            topic_tags: Memo::default(),
        })
    }
    /// 注入完整模式检索后端（`WP-MVP-C05`）。
    ///
    /// 装配层只在探测到 `pg_jieba` 时调用；探测不到就不调用，服务保持体验模式。
    pub fn with_search_backend(mut self, backend: Arc<dyn crate::search::SearchBackend>) -> Self {
        self.search = backend;
        self
    }

    pub async fn viewer(
        &self,
        session: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Viewer, ContentError> {
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        self.flags(&mut tx.connection(), None).await?;
        let result = self
            .authority
            .read_viewer(&mut tx.connection(), session, canonical(now)?)
            .await;
        tx.rollback().await?;
        result
    }
    pub async fn author(
        &self,
        user_id: i64,
        now: DateTime<Utc>,
    ) -> Result<Option<Author>, ContentError> {
        positive(user_id)?;
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        self.flags(&mut tx.connection(), None).await?;
        let result = self
            .authority
            .authors(&mut tx.connection(), &[user_id], canonical(now)?)
            .await?
            .into_iter()
            .next();
        tx.rollback().await?;
        Ok(result)
    }
    pub async fn resolve_author(
        &self,
        public_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Author>, ContentError> {
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        self.flags(&mut tx.connection(), None).await?;
        let result = self
            .authority
            .resolve_author(&mut tx.connection(), public_id, canonical(now)?)
            .await?;
        tx.rollback().await?;
        Ok(result)
    }
    /// Read-only alias/merge resolution. Every hop is filtered using the current
    /// trusted viewer; no management operation or implied grant is exposed.
    pub async fn resolve_tag(
        &self,
        slug: &str,
        session: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Option<Tag>, ContentError> {
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let mut c = tx.connection();
        let ctx = self.context(&mut c, session, canonical(now)?).await?;
        let Some(mut id) = repo::tag_id_for_slug(&mut c, &self.renderer, slug).await? else {
            tx.rollback().await?;
            return Ok(None);
        };
        let mut seen = BTreeSet::new();
        let result = loop {
            if !seen.insert(id) {
                return Err(ContentError::Unavailable);
            }
            let tag = ctx
                .tags
                .iter()
                .find(|tag| tag.id == id)
                .ok_or(ContentError::Unavailable)?;
            if tag.restricted && !ctx.viewer.member {
                break None;
            }
            match tag.merged_into {
                Some(next) => id = next,
                None => break Some(tag.clone()),
            }
        };
        tx.rollback().await?;
        Ok(result)
    }
    async fn author_in(
        &self,
        tx: &mut TransactionConnection<'_>,
        id: i64,
        now: DateTime<Utc>,
    ) -> Result<Author, ContentError> {
        self.authority
            .authors(tx, &[id], now)
            .await?
            .into_iter()
            .find(|a| a.user_id == id)
            .ok_or(ContentError::NotFound)
    }

    /// 带请求内备忘的作者读取。语义与 `author_in` 完全一致，只是同一事务内
    /// 对同一 id 不再重复查库。
    async fn author_memo(
        &self,
        tx: &mut TransactionConnection<'_>,
        ctx: &ReadContext,
        id: i64,
    ) -> Result<Author, ContentError> {
        if let Some(found) = memo_get(&ctx.authors, id) {
            return Ok(found);
        }
        let author = self.author_in(tx, id, ctx.now).await?;
        memo_put(&ctx.authors, id, author.clone());
        Ok(author)
    }

    /// 批量预取一批主题及其首帖、标签与作者，填入 `ReadContext` 的备忘表。
    ///
    /// 这是把列表页 N+1 压平的核心：原先每行要 8 条查询（主题 ×2、首帖、回复扫描、
    /// 标签、作者 ×3），预取后整批只要 4 条（主题、首帖、标签、作者），逐行判定
    /// 全部命中备忘。可见性与权限判定**不在这里做**——仍由 `topic_view` 逐行执行，
    /// 预取只负责把数据一次取回来。
    async fn prefetch_topics(
        &self,
        tx: &mut TransactionConnection<'_>,
        ctx: &ReadContext,
        ids: &[i64],
    ) -> Result<(), ContentError> {
        let missing: Vec<i64> = ids
            .iter()
            .copied()
            .filter(|id| memo_get(&ctx.topics, *id).is_none())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let topics = repo::topics_by_ids(tx, &self.renderer, &missing).await?;
        let present: Vec<i64> = topics.iter().map(|t| t.id).collect();
        // 作者取主题作者与首帖作者的并集，一次问全。
        let mut authors: Vec<i64> = topics.iter().map(|t| t.author_id).collect();
        let posts = repo::posts_at_floor_by_topics(tx, &self.renderer, &present, 1).await?;
        authors.extend(posts.iter().map(|p| p.author_id));
        authors.sort_unstable();
        authors.dedup();
        let tags = repo::topic_tag_ids_by_topics(tx, &self.renderer, &present).await?;

        for topic in topics {
            memo_put(&ctx.topics, topic.id, topic);
        }
        for post in posts {
            memo_put(&ctx.first_posts, post.topic_id, post);
        }
        // 没有标签的主题也要落一条空记录，否则它每行都会回落到单条查询。
        let mut grouped: std::collections::BTreeMap<i64, Vec<i64>> =
            present.iter().map(|id| (*id, Vec::new())).collect();
        for (topic_id, tag_id) in tags {
            grouped.entry(topic_id).or_default().push(tag_id);
        }
        for (topic_id, tag_ids) in grouped {
            memo_put(&ctx.topic_tags, topic_id, tag_ids);
        }
        if !authors.is_empty() {
            for author in self.authority.authors(tx, &authors, ctx.now).await? {
                memo_put(&ctx.authors, author.user_id, author);
            }
        }
        Ok(())
    }

    /// 带请求内备忘的主题读取。
    async fn topic_memo(
        &self,
        tx: &mut TransactionConnection<'_>,
        ctx: &ReadContext,
        id: i64,
    ) -> Result<Option<repo::Topic>, ContentError> {
        if let Some(found) = memo_get(&ctx.topics, id) {
            return Ok(Some(found));
        }
        let topic = repo::topic(tx, &self.renderer, id).await?;
        if let Some(found) = topic.as_ref() {
            memo_put(&ctx.topics, id, found.clone());
        }
        Ok(topic)
    }
    async fn topic_visible(
        &self,
        tx: &mut TransactionConnection<'_>,
        ctx: &ReadContext,
        topic: &repo::Topic,
    ) -> Result<(Forum, Author, Visibility), ContentError> {
        let forum = ctx
            .forums
            .iter()
            .find(|f| f.id == topic.forum_id)
            .ok_or(ContentError::NotFound)?
            .clone();
        let fv = forum_visibility(&ctx.forums, forum.id)?;
        let author = self.author_memo(tx, ctx, topic.author_id).await?;
        // PRIV-002（作者关闭正文公开）只扣留正文，不整体隐藏主题：标题与元信息按 DEC-054
        // 「标题按规则保留」继续展示。因此这里的主题级可见性只由版块与主题自身决定，
        // 作者隐私在 `post_view` 产出正文时单独生效。
        let visibility = fv.strictest(topic.state.visibility);
        if !visibility.permits(&ctx.viewer) || !topic.state.can_read(&ctx.viewer, topic.author_id) {
            return Err(ContentError::NotFound);
        }
        Ok((forum, author, visibility))
    }
    async fn basic_post_visible(
        &self,
        tx: &mut TransactionConnection<'_>,
        ctx: &ReadContext,
        post: &repo::Post,
    ) -> Result<(repo::Topic, Author, Visibility), ContentError> {
        // 主题走请求内备忘：调用方通常刚刚读过同一个主题，列表页每行都会重复一次。
        let topic = self
            .topic_memo(tx, ctx, post.topic_id)
            .await?
            .ok_or(ContentError::NotFound)?;
        let (_, _, visibility) = self.topic_visible(tx, ctx, &topic).await?;
        let author = self.author_memo(tx, ctx, post.author_id).await?;
        // PRIV-002 只覆盖「本人主题正文」，不覆盖回复，因此这里不再折入作者隐私开关。
        //
        // DEC-061：删除态主题的首帖本身也带墓碑，但必须保留可读性才能渲染占位提示。
        // 放行范围严格限定为「首帖 + 主题处于删除态 + 该帖未被审核处置且已发布」，
        // 普通回复的删除语义完全不变。
        let tombstoned_opening = post.floor_no == 1
            && topic.state.deleted_at.is_some()
            && post.moderation == Moderation::Normal
            && post.publication == Publication::Published;
        if (!post.can_read(&ctx.viewer) && !tombstoned_opening) || !visibility.permits(&ctx.viewer)
        {
            return Err(ContentError::NotFound);
        }
        Ok((topic, author, visibility))
    }
    async fn post_view(
        &self,
        tx: &mut TransactionConnection<'_>,
        ctx: &ReadContext,
        post: &repo::Post,
    ) -> Result<PostView, ContentError> {
        let (topic, author, visibility) = self.basic_post_visible(tx, ctx, post).await?;
        // 正文状态优先级：删除 > 作者隐私 > 正常。
        // 只有主题首帖适用这两条扣留规则：DEC-061 的删除占位与 PRIV-002 的作者隐私都以
        // 「主题正文」为对象，单条回复的删除仍按既有规则整体不展示。
        let body_state = if post.floor_no == 1 && topic.state.deleted_at.is_some() {
            BodyState::Deleted
        } else if post.floor_no == 1 && !author.body_visibility.permits(&ctx.viewer) {
            BodyState::WithheldByAuthor
        } else {
            BodyState::Visible
        };
        let rendered = self
            .filtered_markup(tx, ctx, &post.body, Some(post.topic_id))
            .await?;
        let quote = if let Some(id) = post.quote_post_id {
            match repo::post(tx, &self.renderer, id).await? {
                Some(target) => match self.basic_post_visible(tx, ctx, &target).await {
                    Ok(_) => Some(id),
                    Err(ContentError::NotFound) => None,
                    Err(e) => return Err(e),
                },
                None => None,
            }
        } else {
            None
        };
        let html = if visibility == Visibility::Public
            && post.publication == Publication::Published
            && topic.state.publication == Publication::Published
            && post.quote_post_id.is_none()
            && markdown::render(&post.body)
                .links()
                .iter()
                .all(|l| l.target().is_none())
        {
            if let Some(cache) = &self.cache {
                let key = platform::cache::PublicCacheKey::new(
                    platform::cache::CacheEntity::new("post", &post.id.to_string())
                        .map_err(|_| ContentError::Unavailable)?,
                    u64::try_from(post.version).map_err(|_| ContentError::Unavailable)?,
                    platform::cache::CacheVisibility::Public,
                )
                .map_err(|_| ContentError::Unavailable)?;
                platform::cache::read_public(cache.as_ref(), &key, || async {
                    Ok::<_, ContentError>(rendered.html().to_string())
                })
                .await?
                .value
            } else {
                rendered.html().to_string()
            }
        } else {
            rendered.html().to_string()
        };
        // 扣留发生在服务端投影阶段：正文与 HTML 一律清空，绝不交给模板去「隐藏」。
        let (markdown, html) = if body_state.visible() {
            (post.body.clone(), html)
        } else {
            (String::new(), String::new())
        };
        Ok(PostView {
            id: post.id,
            topic_id: post.topic_id,
            floor_no: post.floor_no,
            author,
            markdown,
            html,
            body_state,
            created_at: post.created_at,
            edited_at: post.edited_at,
            version: post.version,
            quote_post_id: quote,
            publication: post.publication,
        })
    }
    async fn topic_view(
        &self,
        tx: &mut TransactionConnection<'_>,
        ctx: &ReadContext,
        topic: &repo::Topic,
    ) -> Result<TopicView, ContentError> {
        let (forum, author, _) = self.topic_visible(tx, ctx, topic).await?;
        // 首帖与标签优先取预取表；列表路径已批量填好，详情页路径回落到单条查询。
        // 回落分支保留是必要的：预取缺项时必须仍然正确，而不是渲染出缺内容的页面。
        let first = match memo_get(&ctx.first_posts, topic.id) {
            Some(found) => found,
            None => repo::post_at_floor(tx, &self.renderer, topic.id, 1)
                .await?
                .ok_or(ContentError::NotFound)?,
        };
        self.basic_post_visible(tx, ctx, &first).await?;
        let ids = match memo_get(&ctx.topic_tags, topic.id) {
            Some(found) => found,
            None => repo::topic_tag_ids(tx, &self.renderer, topic.id).await?,
        };
        let tags = ctx
            .tags
            .iter()
            .filter(|tag| ids.contains(&tag.id) && (!tag.restricted || ctx.viewer.member))
            .cloned()
            .collect();
        let rules = self.policy.rules().ok();
        let mut last_reply_author = None;
        let mut last_reply_at = None;
        let mut reply_count = None;
        if rules
            .as_ref()
            .is_some_and(|r| r.show_structural_reply_count || r.show_last_public_reply)
        {
            let mut floor = 1;
            let mut count = 0;
            loop {
                let rows = repo::posts(tx, &self.renderer, topic.id, i64::MAX, floor, 100).await?;
                if rows.is_empty() {
                    break;
                }
                for post in rows {
                    floor = post.floor_no;
                    if post.publication != Publication::Published {
                        continue;
                    }
                    match self.post_view(tx, ctx, &post).await {
                        Ok(view) => {
                            // DEC-054 有效回复：非主题作者本人发出、正文 ≥ 最低回复长度、
                            // 未删除、未被判违规（后两项已由 `post_view` 的可见性过滤保证）。
                            // 同一用户在同一主题内可重复计入，不做按人去重。
                            // 首帖不是回复，不参与计数。
                            let minimum = rules
                                .as_ref()
                                .map(|r| r.minimum_reply_chars)
                                .unwrap_or_default();
                            if view.floor_no != 1
                                && view.author.user_id != topic.author_id
                                && view.markdown.trim().chars().count() >= minimum
                            {
                                count += 1;
                            }
                            // 最后回复与有效回复是两套口径：最后回复取最新一条未删回复，
                            // 含作者本人，也不看长度（DEC-054）。
                            if view.floor_no != 1 {
                                last_reply_at = Some(view.created_at);
                                last_reply_author = Some(view.author);
                            }
                        }
                        Err(ContentError::NotFound) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
            if rules
                .as_ref()
                .is_some_and(|r| r.show_structural_reply_count)
            {
                reply_count = Some(count)
            }
            if !rules.as_ref().is_some_and(|r| r.show_last_public_reply) {
                last_reply_at = None;
                last_reply_author = None;
            }
        }
        Ok(TopicView {
            id: topic.id,
            title: topic.title.clone(),
            forum,
            tags,
            author,
            state: topic.state.clone(),
            created_at: topic.created_at,
            edited_at: topic.edited_at,
            version: topic.version,
            pinned: topic.pinned,
            globally_pinned: topic.globally_pinned,
            featured: topic.featured,
            reply_count,
            // NAV-003／NAV-004 的累计阅读量。计数在 SSR 成功后的独立事务里累加，
            // 这里只读当前值，不在读路径产生写入。
            view_count: Some(topic.view_count),
            last_reply_author,
            last_reply_at,
        })
    }
    /// CAT-014／DEC-056：把主题移动到另一个版块。
    ///
    /// 语义要点（与 DEC-056 逐条对应）：
    ///   * **同一对象换版块**：只改 `forum_id`。主题 ID、楼层号、回复、修订、发布回执
    ///     全部原样保留，不克隆、不新建 ID、不建别名表——权威永久链接 `/t/{id}` 不含
    ///     版块段，因此移动根本不会让旧链接失效，`CR-012` 的 ID 冲突不复存在。
    ///   * **权限随新版块重算**：移动后按目标版块的可见性刷新权威搜索投影（`CAT-015`），
    ///     移到成员可见版块后，访客即刻看不到。
    ///   * **必须审计**：同事务追加一条带操作者、前后版块与操作 ID 的审计事实。
    ///
    /// 授权由应用装配层按 DEC-058 的权限 registry 判定；未装配时整体失败关闭。
    /// 锁序沿用 DEC-052：先源与目标版块（按 ID 升序，避免两次移动互相死锁），
    /// 再主题行，最后才进入审计与 Outbox 追加段。
    pub async fn move_topic(
        &self,
        credentials: &Credentials,
        topic_id: i64,
        target_forum_id: i64,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, ContentError> {
        positive(topic_id)?;
        positive(target_forum_id)?;
        let now = canonical(now)?;
        let mut tx = DbTransaction::begin(&self.pool).await?;
        let result = self
            .move_topic_in(
                &mut tx.connection(),
                credentials,
                topic_id,
                target_forum_id,
                expected_version,
                now,
            )
            .await;
        match result {
            Ok(moved) => {
                tx.commit().await?;
                Ok(moved)
            }
            Err(e) => {
                tx.rollback().await?;
                Err(e)
            }
        }
    }

    async fn move_topic_in(
        &self,
        tx: &mut TransactionConnection<'_>,
        credentials: &Credentials,
        topic_id: i64,
        target_forum_id: i64,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, ContentError> {
        self.flags(tx, None).await?;
        // 先读出目标当前所属版块，再做授权：ABAC 的版块范围要按「目标此刻在哪」判断，
        // 而不是按调用方声称的版块（`03` §5）。这次读取不加锁，随后的写路径会在
        // 取得版块与主题锁后重读并做版本 CAS，因此不存在 TOCTOU 放行。
        let source = repo::topic(tx, &self.renderer, topic_id)
            .await?
            .ok_or(ContentError::NotFound)?;
        let actor = self
            .authority
            .authorize_management(
                tx,
                credentials,
                ManagementCapability::MoveTopic,
                ManagementTarget {
                    resource_id: topic_id,
                    forum_id: Some(source.forum_id),
                },
                now,
            )
            .await?;
        let ctx = self
            .context(tx, Some(credentials.session_token.expose()), now)
            .await?;
        if source.forum_id == target_forum_id {
            // 移到原版块没有任何状态变化，直接拒绝而不是写一条空审计。
            return Err(ContentError::InvalidInput);
        }
        // DEC-052/DEC-064 锁序：community 根锁必须早于主题锁，同时取得新的排序水位。
        let seq = repo::lock_root(tx, &self.renderer).await?;
        // 两个版块链都要校验（`DEC-064` 后不再加锁）。仍按 ID 升序遍历：顺序不再影响
        // 死锁，但保持确定的校验次序，让失败信息在并发下也可复现。
        let mut forums = [source.forum_id, target_forum_id];
        forums.sort_unstable();
        for id in forums {
            check_forum_chain(&ctx.forums, id)?;
        }
        repo::lock(tx, &self.renderer, "topics", topic_id).await?;
        let mut topic = repo::topic(tx, &self.renderer, topic_id)
            .await?
            .ok_or(ContentError::NotFound)?;
        if topic.version != expected_version {
            return Err(ContentError::Conflict);
        }
        // 目标版块必须存在且可写；已归档或已删除的主题不移动。
        let target_visibility = forum_visibility(&ctx.forums, target_forum_id)?;
        if !topic.state.permits_write() {
            return Err(ContentError::Forbidden);
        }
        let previous_forum = topic.forum_id;
        topic.forum_id = target_forum_id;
        topic.current_seq = seq;
        repo::move_topic(tx, &self.renderer, &topic).await?;
        topic.version += 1;
        repo::history(tx, &self.renderer, &topic).await?;

        // CAT-015：同事务把新版块的可见性推进权威搜索投影，避免移动后出现
        // 「列表已按新版块收紧、搜索还按旧版块可见」的窗口。
        let mut author_ids = BTreeSet::from([topic.author_id]);
        let mut floor = 0;
        loop {
            let batch = repo::posts(tx, &self.renderer, topic.id, i64::MAX, floor, 100).await?;
            if batch.is_empty() {
                break;
            }
            for row in batch {
                floor = row.floor_no;
                author_ids.insert(row.author_id);
            }
        }
        let ids = author_ids.into_iter().collect::<Vec<_>>();
        let mut authors = BTreeMap::new();
        for chunk in ids.chunks(256) {
            authors.extend(
                self.authority
                    .authors(tx, chunk, now)
                    .await?
                    .into_iter()
                    .map(|a| (a.user_id, a.body_visibility)),
            );
        }
        repo::refresh_projection(tx, &self.renderer, &topic, target_visibility, &authors).await?;

        // 以下为追加段：审计与缓存失效，不再获取任何领域锁。
        tx.enter_append_phase();
        let operation_id = new_operation_id();
        self.authority
            .append_move_audit(
                tx,
                &actor,
                topic.id,
                previous_forum,
                target_forum_id,
                &operation_id,
                now,
            )
            .await?;
        let event = OutboxEvent {
            event_id: new_operation_id(),
            aggregate_type: "topic".into(),
            aggregate_id: topic.id.to_string(),
            event_type: "cache.invalidate".into(),
            version: 1,
            occurred_at: now,
            payload: SafePayload::empty()
                .with_reference("resource_id", &topic.id.to_string())
                .map_err(|_| ContentError::Unavailable)?,
            trace_id: String::new(),
        };
        platform_db::outbox_repo::enqueue(tx, &self.renderer, &event, &["cache.invalidate"])
            .await
            .map_err(|_| ContentError::Unavailable)?;
        Ok(MutationResult {
            topic_id: topic.id,
            post_id: 0,
            floor_no: 1,
            version: topic.version,
            replayed: false,
        })
    }

    /// DEC-054 的浏览计数窗口：同一主体对同一主题 30 分钟内只计 1 次。
    const VIEW_DEDUPLICATION_MINUTES: i64 = 30;

    /// 在主题详情页服务端渲染成功返回 200 之后登记一次浏览。
    ///
    /// 触发点与边界（DEC-054）：
    ///   * 只有 SSR 成功才调用，失败或 404 不计数；
    ///   * 单独的短写事务，不进入读事务，也不阻塞渲染——调用方应忽略错误只记观测，
    ///     浏览量丢一次远好过让一次统计失败把正常阅读变成错误页；
    ///   * 排除作者本人；
    ///   * 排除被标记为分析排除的账号（系统账号与已确认作弊流量）与已知爬虫，
    ///     这两类由调用方在派生主体时直接返回 `None`，压根不会到达这里；
    ///   * 同一主体 30 分钟内重复访问不累加计数，且整个事务回滚，台账时间戳保持为
    ///     上一次真正计数的时刻，使窗口是固定窗口而非滑动窗口。
    ///
    /// 返回是否实际累加，供测试与观测断言。
    pub async fn record_view(
        &self,
        topic_id: i64,
        subject: &ViewSubject,
        viewer_id: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<bool, ContentError> {
        positive(topic_id)?;
        if subject.hash.is_empty() {
            return Err(ContentError::InvalidInput);
        }
        let now = canonical(now)?;
        let window = Duration::try_minutes(Self::VIEW_DEDUPLICATION_MINUTES)
            .ok_or(ContentError::Unavailable)?;
        let mut tx = DbTransaction::begin(&self.pool).await?;
        let counted = {
            let mut c = tx.connection();
            let topic = repo::topic(&mut c, &self.renderer, topic_id)
                .await?
                .ok_or(ContentError::NotFound)?;
            // 作者本人的访问不计入自己的阅读量。
            if viewer_id == Some(topic.author_id) {
                false
            } else {
                repo::lock(&mut c, &self.renderer, "topics", topic_id).await?;
                if repo::register_view(&mut c, &self.renderer, topic_id, &subject.hash, window, now)
                    .await?
                {
                    repo::bump_view_count(&mut c, &self.renderer, topic_id).await?;
                    true
                } else {
                    false
                }
            }
        };
        if counted {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        Ok(counted)
    }

    /// `SEARCH-001`～`SEARCH-004`：执行一次搜索。
    ///
    /// 可见性过滤在 SQL 内完成，先于高亮定位（`02` §13）。返回的 `SearchHit` 只带
    /// 原始标题与命中区间，不带拼好的 HTML——渲染层负责转义，搜索层不接管这份责任。
    pub async fn search(
        &self,
        query: &crate::search::SearchQuery,
        session: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(Viewer, Vec<crate::search::SearchHit>), ContentError> {
        let now = canonical(now)?;
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let mut c = tx.connection();
        let ctx = self.context(&mut c, session, now).await?;
        let forum_id = query.forum_id.unwrap_or(0);
        let since = query.since.unwrap_or(DateTime::UNIX_EPOCH);
        let until = query.until.unwrap_or(DateTime::UNIX_EPOCH);
        let newest = query.order == crate::search::SearchOrder::Newest;
        let member = ctx.viewer.member;
        // 按注入的后端能力分派。两条路径的权限过滤完全一致——全文检索不放宽
        // 任何一条，否则就是用搜索把权限过滤绕过去。
        let rows = match self.search.capability() {
            crate::search::SearchCapability::FullText => {
                repo::search_full_text(
                    &mut c,
                    &self.renderer,
                    &query.terms,
                    forum_id,
                    since,
                    until,
                    newest,
                    member,
                    query.limit,
                )
                .await
            }
            crate::search::SearchCapability::TitleOnly => {
                // 通配符转义后再包上 %：不转义的话一个 % 就能把检索变成枚举全部标题。
                let pattern = format!("%{}%", crate::search::escape_like(&query.terms));
                repo::search_titles(
                    &mut c,
                    &self.renderer,
                    &pattern,
                    forum_id,
                    since,
                    until,
                    newest,
                    member,
                    query.limit,
                )
                .await
            }
        };
        tx.rollback().await?;
        let hits = rows?
            .into_iter()
            .map(|(topic_id, title, forum_id, created_at)| {
                let highlight = crate::search::locate(&title, &query.terms);
                crate::search::SearchHit {
                    topic_id,
                    title,
                    highlight,
                    forum_id,
                    created_at,
                }
            })
            .collect();
        Ok((ctx.viewer, hits))
    }

    /// 当前搜索后端的能力，供页面明示限制（`02` §13）。
    ///
    /// 由数据库后端决定：SQLite 体验模式只支持标题检索。写死在这里而不是配置项，
    /// 因为它是后端的客观能力，不该由配置声称。
    pub fn search_capability(&self) -> crate::search::SearchCapability {
        self.search.capability()
    }

    /// `SEO-004`／`SEO-005`：可公开索引的主题清单，供 sitemap 与 RSS 使用。
    ///
    /// 不接受访问者身份：它面向匿名抓取方，只返回公开内容。传入 session 反而会让
    /// 「登录后抓到的 sitemap 更全」，那正是 `SEO-005` 要避免的。
    pub async fn public_index(
        &self,
        forum_id: Option<i64>,
        limit: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<(i64, String, DateTime<Utc>)>, ContentError> {
        let _ = canonical(now)?;
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let mut c = tx.connection();
        self.flags(&mut c, None).await?;
        let rows = repo::public_index(
            &mut c,
            &self.renderer,
            forum_id.filter(|id| *id > 0).unwrap_or(0),
            limit.clamp(1, 500),
        )
        .await;
        tx.rollback().await?;
        rows
    }

    /// 版块目录，供管理侧的目标版块选择使用。
    ///
    /// 返回全部版块（含仅成员可见的），因为调用方已经通过管理授权——
    /// 管理动作的可见性由权限与 ABAC 范围决定，不由公开可见性决定。
    pub async fn catalog(&self, now: DateTime<Utc>) -> Result<Vec<Forum>, ContentError> {
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let mut c = tx.connection();
        self.flags(&mut c, None).await?;
        let _ = canonical(now)?;
        let forums = repo::forums(&mut c, &self.renderer).await;
        tx.rollback().await?;
        forums
    }

    /// 主题的管理侧摘要：(标题, 当前版块 ID, 当前版本)。
    ///
    /// 版本号是移帖的乐观并发 CAS 令牌，由这里读出后填进表单隐藏字段，
    /// 而不是让操作者手填——手填既难用，填错时又只会得到一次静默的冲突失败。
    pub async fn topic_summary(
        &self,
        topic_id: i64,
        now: DateTime<Utc>,
    ) -> Result<(String, i64, i64), ContentError> {
        positive(topic_id)?;
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let mut c = tx.connection();
        self.flags(&mut c, None).await?;
        let _ = canonical(now)?;
        let topic = repo::topic(&mut c, &self.renderer, topic_id).await;
        tx.rollback().await?;
        let topic = topic?.ok_or(ContentError::NotFound)?;
        Ok((topic.title, topic.forum_id, topic.version))
    }

    /// `NAV-004` 站点统计。
    ///
    /// 独立只读事务：它挂在每个页面的侧栏上，不应该被卷进任何写路径，
    /// 也不值得为它扩大读事务的持有时间。
    pub async fn site_statistics(
        &self,
        now: DateTime<Utc>,
    ) -> Result<SiteStatistics, ContentError> {
        let now = canonical(now)?;
        // 命中未过期缓存就不碰数据库。锁只护一个小结构，持有时间是常数级，
        // 不跨 await——跨 await 持有 std 互斥锁会在多线程运行时上死锁。
        if let Ok(guard) = self.statistics.lock()
            && let Some((computed_at, cached)) = guard.as_ref()
            && (now - *computed_at).num_seconds() < STATISTICS_TTL_SECONDS
            && now >= *computed_at
        {
            return Ok(*cached);
        }
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let mut c = tx.connection();
        self.flags(&mut c, None).await?;
        let stats = repo::site_statistics(&mut c, &self.renderer).await;
        tx.rollback().await?;
        let stats = stats?;
        // 只缓存成功结果：把失败缓存下来会让一次瞬时故障持续 30 秒。
        if let Ok(mut guard) = self.statistics.lock() {
            *guard = Some((now, stats));
        }
        Ok(stats)
    }

    pub async fn list(
        &self,
        filter: &ListFilter,
        cursor: Option<&str>,
        session: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<ContentList, ContentError> {
        for id in [filter.forum_id, filter.tag_id, filter.owner_id]
            .into_iter()
            .flatten()
        {
            positive(id)?;
        }
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let mut c = tx.connection();
        let ctx = self.context(&mut c, session, canonical(now)?).await?;
        if let Some(id) = filter.forum_id
            && !forum_visibility(&ctx.forums, id)?.permits(&ctx.viewer)
        {
            return Err(ContentError::NotFound);
        }
        if let Some(id) = filter.tag_id
            && !ctx
                .tags
                .iter()
                .any(|t| t.id == id && (!t.restricted || ctx.viewer.member))
        {
            return Err(ContentError::NotFound);
        }
        // PRIV-001：作者关闭「向未登录访客展示主题和回帖列表」后，个人主页的这两个集合
        // 对访客整体不可见。判定放在取数之前，避免先查出内容再过滤——那样容易从分页
        // 条数、游标或耗时上反推出被隐藏内容的存在。本人与登录成员不受影响。
        if let Some(owner) = filter.owner_id {
            let author = self
                .authority
                .authors(&mut c, &[owner], ctx.now)
                .await?
                .into_iter()
                .next()
                .ok_or(ContentError::NotFound)?;
            if !author.lists_visibility.permits(&ctx.viewer) {
                return Err(ContentError::NotFound);
            }
        }
        let binding = list_binding(filter, &ctx.viewer)?;
        let mut position = match cursor {
            Some(text) => self.cursor.decode(text, &binding)?,
            None => Cursor {
                version: 1,
                context: binding,
                watermark: repo::watermark(&mut c, &self.renderer).await?,
                sort: i64::MAX,
                id: i64::MAX,
            },
        };
        let mut topics = Vec::new();
        let mut replies = Vec::new();
        let mut next = None;
        'scan: loop {
            let rows = repo::candidates(
                &mut c,
                &self.renderer,
                filter,
                position.watermark,
                position.sort,
                position.id,
                100,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            // 批量预取本批候选的主题、首帖、标签与作者，把逐行的 N+1 压成常数条。
            // 预取只填备忘表，不改变任何可见性判定——判定仍在 `topic_view` 里逐行
            // 执行，读到的值与逐条查询完全相同。
            if filter.kind == ListKind::Topics {
                let ids: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
                self.prefetch_topics(&mut c, &ctx, &ids).await?;
            }
            for (id, sort) in rows {
                position.id = id;
                position.sort = sort;
                let found = if filter.kind == ListKind::Topics {
                    if let Some(topic) = self.topic_memo(&mut c, &ctx, id).await? {
                        match self.topic_view(&mut c, &ctx, &topic).await {
                            Ok(topic) => {
                                topics.push(topic);
                                true
                            }
                            Err(ContentError::NotFound) => false,
                            Err(e) => return Err(e),
                        }
                    } else {
                        false
                    }
                } else if let Some(post) = repo::post(&mut c, &self.renderer, id).await? {
                    match self.post_view(&mut c, &ctx, &post).await {
                        Ok(post) => {
                            replies.push(post);
                            true
                        }
                        Err(ContentError::NotFound) => false,
                        Err(e) => return Err(e),
                    }
                } else {
                    false
                };
                if found && topics.len() + replies.len() == PAGE_SIZE {
                    if !repo::candidates(
                        &mut c,
                        &self.renderer,
                        filter,
                        position.watermark,
                        position.sort,
                        position.id,
                        1,
                    )
                    .await?
                    .is_empty()
                    {
                        next = Some(self.cursor.encode(&position)?)
                    }
                    break 'scan;
                }
            }
        }
        let forums = ctx
            .forums
            .iter()
            .filter(|f| forum_visibility(&ctx.forums, f.id).is_ok_and(|v| v.permits(&ctx.viewer)))
            .cloned()
            .collect();
        let tags = ctx
            .tags
            .into_iter()
            .filter(|t| t.merged_into.is_none() && (!t.restricted || ctx.viewer.member))
            .collect();
        let result = ContentList {
            viewer: ctx.viewer,
            topics,
            replies,
            forums,
            tags,
            next_cursor: next,
        };
        tx.rollback().await?;
        Ok(result)
    }
    pub async fn topic(
        &self,
        id: i64,
        cursor: Option<&str>,
        session: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<TopicDetail, ContentError> {
        positive(id)?;
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let mut c = tx.connection();
        let ctx = self.context(&mut c, session, canonical(now)?).await?;
        let topic = repo::topic(&mut c, &self.renderer, id)
            .await?
            .ok_or(ContentError::NotFound)?;
        let topic = self.topic_view(&mut c, &ctx, &topic).await?;
        let first = repo::post_at_floor(&mut c, &self.renderer, id, 1)
            .await?
            .ok_or(ContentError::NotFound)?;
        let first = self.post_view(&mut c, &ctx, &first).await?;
        let binding = topic_binding(id, &ctx.viewer);
        let mut position = match cursor {
            Some(text) => self.cursor.decode(text, &binding)?,
            None => Cursor {
                version: 1,
                context: binding,
                watermark: repo::watermark(&mut c, &self.renderer).await?,
                sort: 1,
                id,
            },
        };
        let mut replies = Vec::new();
        let mut next = None;
        'scan: loop {
            let rows = repo::posts(
                &mut c,
                &self.renderer,
                id,
                position.watermark,
                position.sort,
                100,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            for post in rows {
                position.sort = post.floor_no;
                match self.post_view(&mut c, &ctx, &post).await {
                    Ok(view) => replies.push(view),
                    Err(ContentError::NotFound) => {}
                    Err(e) => return Err(e),
                }
                if replies.len() == PAGE_SIZE {
                    if !repo::posts(
                        &mut c,
                        &self.renderer,
                        id,
                        position.watermark,
                        position.sort,
                        1,
                    )
                    .await?
                    .is_empty()
                    {
                        next = Some(self.cursor.encode(&position)?)
                    }
                    break 'scan;
                }
            }
        }
        let result = TopicDetail {
            viewer: ctx.viewer,
            topic,
            first_post: first,
            replies,
            next_cursor: next,
        };
        tx.rollback().await?;
        Ok(result)
    }
    pub async fn locate(
        &self,
        post_id: i64,
        session: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<PostLocation, ContentError> {
        positive(post_id)?;
        let mut tx = DbTransaction::begin_read(&self.pool).await?;
        let mut c = tx.connection();
        let ctx = self.context(&mut c, session, canonical(now)?).await?;
        let post = repo::post(&mut c, &self.renderer, post_id)
            .await?
            .ok_or(ContentError::NotFound)?;
        self.post_view(&mut c, &ctx, &post).await?;
        let cursor = if post.floor_no > 1 {
            Some(self.cursor.encode(&Cursor {
                version: 1,
                context: topic_binding(post.topic_id, &ctx.viewer),
                watermark: repo::watermark(&mut c, &self.renderer).await?,
                sort: post.floor_no - 1,
                id: post.topic_id,
            })?)
        } else {
            None
        };
        let result = PostLocation {
            topic_id: post.topic_id,
            floor_no: post.floor_no,
            cursor,
        };
        tx.rollback().await?;
        Ok(result)
    }
    pub async fn preview(
        &self,
        credentials: &Credentials,
        source: &str,
        now: DateTime<Utc>,
    ) -> Result<Preview, ContentError> {
        if source.len() > 1_048_576 {
            return Err(ContentError::InvalidInput);
        }
        let now = canonical(now)?;
        let mut tx = DbTransaction::begin(&self.pool).await?;
        let mut c = tx.connection();
        self.flags(&mut c, None).await?;
        let facts = self
            .authority
            .authorize(&mut c, credentials, Action::EditPost, now)
            .await?;
        if !facts.email_verified || !facts.allowed {
            return Err(ContentError::Forbidden);
        }
        let ctx = self
            .context(&mut c, Some(credentials.session_token.expose()), now)
            .await?;
        let rendered = self.filtered_markup(&mut c, &ctx, source, None).await?;
        let result = Preview {
            html: rendered.html().into(),
            external_links: rendered.external_link_count(),
        };
        tx.rollback().await?;
        Ok(result)
    }
    pub async fn create_topic(
        &self,
        c: &Credentials,
        input: NewTopic,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, ContentError> {
        self.mutate(c, Mutation::Topic(input), now).await
    }
    pub async fn reply(
        &self,
        c: &Credentials,
        input: NewReply,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, ContentError> {
        self.mutate(c, Mutation::Reply(input), now).await
    }
    pub async fn edit(
        &self,
        c: &Credentials,
        input: EditPost,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, ContentError> {
        self.mutate(c, Mutation::Edit(input), now).await
    }
    pub async fn delete(
        &self,
        c: &Credentials,
        input: DeletePost,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, ContentError> {
        self.mutate(c, Mutation::Delete(input), now).await
    }
    async fn mutate(
        &self,
        credentials: &Credentials,
        input: Mutation,
        now: DateTime<Utc>,
    ) -> Result<MutationResult, ContentError> {
        let now = canonical(now)?;
        let mut preflight = DbTransaction::begin(&self.pool).await?;
        self.flags(&mut preflight.connection(), Some(input.action()))
            .await?;
        let preflight_facts = self
            .authority
            .authorize(
                &mut preflight.connection(),
                credentials,
                input.action(),
                now,
            )
            .await?;
        if !preflight_facts.allowed || !preflight_facts.email_verified {
            return Err(ContentError::Forbidden);
        }
        let actor = preflight_facts.user_id;
        preflight.rollback().await?;
        let canonical = serde_json::to_vec(&input).map_err(|_| ContentError::InvalidInput)?;
        let command = CommandIdentity {
            scope: format!("content.{}", input.action().as_str()),
            actor_id: actor,
            idempotency_key: credentials.idempotency_key.clone(),
            request_hash: request_hash(&canonical),
            operation_id: new_operation_id(),
        };
        let service = self.clone();
        let credentials = credentials.clone();
        let command_copy = command.clone();
        let error = Arc::new(Mutex::new(None));
        let inner_error = error.clone();
        let outcome = execute_command(
            &self.pool,
            &self.renderer,
            &command,
            &LockPlan::default(),
            now,
            move |tx| {
                let service = service.clone();
                let credentials = credentials.clone();
                let input = input.clone();
                let command = command_copy.clone();
                let error = inner_error.clone();
                Box::pin(async move {
                    match service.write(tx, &credentials, &input, &command, now).await {
                        Ok(response) => Ok(response),
                        Err(ContentError::RetryableDatabase(class)) => {
                            Err(TransactionError::Database(class))
                        }
                        Err(e) => {
                            *error.lock().map_err(|_| TransactionError::CorruptData)? = Some(e);
                            Err(TransactionError::InvalidInput)
                        }
                    }
                })
            },
        )
        .await;
        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => {
                return Err(error
                    .lock()
                    .map_err(|_| ContentError::Unavailable)?
                    .take()
                    .unwrap_or_else(|| e.into()));
            }
        };
        match outcome {
            CommitOutcome::Unknown { operation_id } => {
                Err(ContentError::CommitUnknown { operation_id })
            }
            CommitOutcome::RolledBack => Err(ContentError::Unavailable),
            CommitOutcome::Committed(result) => {
                if result.response.status_code != 200 {
                    return Err(if result.response.status_code == 503 {
                        ContentError::PolicyPending
                    } else {
                        ContentError::Forbidden
                    });
                }
                let payload = SafePayload::from_json(&result.response.body)
                    .map_err(|_| ContentError::Unavailable)?;
                let id = payload
                    .reference("resource_id")
                    .and_then(|s| s.parse().ok())
                    .ok_or(ContentError::Unavailable)?;
                let version = payload
                    .reference("version")
                    .and_then(|s| s.parse().ok())
                    .ok_or(ContentError::Unavailable)?;
                let mut tx = DbTransaction::begin_read(&self.pool).await?;
                let post = repo::post(&mut tx.connection(), &self.renderer, id)
                    .await?
                    .ok_or(ContentError::Unavailable)?;
                tx.rollback().await?;
                Ok(MutationResult {
                    topic_id: post.topic_id,
                    post_id: post.id,
                    floor_no: post.floor_no,
                    version,
                    replayed: result.replayed,
                })
            }
        }
    }
    async fn write(
        &self,
        tx: &mut TransactionConnection<'_>,
        credentials: &Credentials,
        input: &Mutation,
        command: &CommandIdentity,
        now: DateTime<Utc>,
    ) -> Result<StoredResponse, ContentError> {
        let action = input.action();
        self.flags(tx, Some(action)).await?;
        let facts = self
            .authority
            .authorize(tx, credentials, action, now)
            .await?;
        if facts.user_id != command.actor_id {
            return Err(ContentError::Forbidden);
        }
        let rules = match self.policy.rules() {
            Ok(r) => r,
            Err(ContentError::PolicyPending) => {
                self.authority
                    .append_risk(
                        tx,
                        credentials,
                        &facts,
                        action,
                        RiskDisposition::Unavailable,
                        &command.operation_id,
                        now,
                    )
                    .await?;
                return stored_failure(503);
            }
            Err(e) => return Err(e),
        };
        if rules.authorize(&facts).is_err() {
            self.authority
                .append_risk(
                    tx,
                    credentials,
                    &facts,
                    action,
                    RiskDisposition::Denied,
                    &command.operation_id,
                    now,
                )
                .await?;
            return stored_failure(403);
        }
        if facts.created_at > now {
            return Err(ContentError::Forbidden);
        }
        let seq = repo::lock_root(tx, &self.renderer).await?;
        // Root serializes all community changes; catalogue and targets are reread after it.
        let ctx = self
            .context(tx, Some(credentials.session_token.expose()), now)
            .await?;
        let (topic, post, tag_ids, links) = match input {
            Mutation::Topic(new) => {
                positive(new.forum_id)?;
                let tags = validate_tags(&ctx.tags, &new.tag_ids)?;
                let rendered = markdown::render(&new.markdown);
                rules.validate_text(
                    Some(&new.title),
                    &new.markdown,
                    false,
                    tags.len(),
                    rendered.external_link_count(),
                    facts.tier,
                )?;
                let fv = check_forum_chain(&ctx.forums, new.forum_id)?;
                if !fv.permits(&ctx.viewer) {
                    return Err(ContentError::Forbidden);
                }
                let destination = fv
                    .strictest(new.visibility)
                    .strictest(facts.default_visibility);
                let links = self
                    .target_links(tx, &ctx, &rendered, None, destination, None)
                    .await?;
                self.quota(tx, &rules, &facts, action, now).await?;
                let id = repo::insert_topic(tx, &self.renderer, new, &facts, seq, now).await?;
                let post_id = repo::insert_post(
                    tx,
                    &self.renderer,
                    id,
                    &facts,
                    1,
                    &new.markdown,
                    None,
                    seq,
                    now,
                )
                .await?;
                repo::set_tags(tx, &self.renderer, id, &tags).await?;
                let topic = repo::topic(tx, &self.renderer, id)
                    .await?
                    .ok_or(ContentError::Unavailable)?;
                let post = repo::post(tx, &self.renderer, post_id)
                    .await?
                    .ok_or(ContentError::Unavailable)?;
                (topic, post, tags, Some(links))
            }
            Mutation::Reply(new) => {
                positive(new.topic_id)?;
                let mut topic = repo::topic(tx, &self.renderer, new.topic_id)
                    .await?
                    .ok_or(ContentError::NotFound)?;
                check_forum_chain(&ctx.forums, topic.forum_id)?;
                repo::lock(tx, &self.renderer, "topics", topic.id).await?;
                topic = repo::topic(tx, &self.renderer, topic.id)
                    .await?
                    .ok_or(ContentError::NotFound)?;
                let (_, _, destination) = self.topic_visible(tx, &ctx, &topic).await?;
                if !topic.state.permits_write() {
                    return Err(ContentError::Forbidden);
                }
                if topic.version != new.expected_topic_version {
                    return Err(ContentError::Conflict);
                }
                let rendered = markdown::render(&new.markdown);
                rules.validate_text(
                    None,
                    &new.markdown,
                    true,
                    0,
                    rendered.external_link_count(),
                    facts.tier,
                )?;
                let links = self
                    .target_links(
                        tx,
                        &ctx,
                        &rendered,
                        new.quote_post_id,
                        destination.strictest(facts.default_visibility),
                        Some(topic.id),
                    )
                    .await?;
                self.quota(tx, &rules, &facts, action, now).await?;
                let floor = topic.next_floor;
                topic.next_floor = topic
                    .next_floor
                    .checked_add(1)
                    .ok_or(ContentError::Unavailable)?;
                topic.current_seq = seq;
                if facts.publication == Publication::Published {
                    topic.raw_reply_count = topic
                        .raw_reply_count
                        .checked_add(1)
                        .ok_or(ContentError::Unavailable)?;
                    topic.last_reply_author_id = Some(facts.user_id);
                    topic.last_reply_at = Some(now);
                    topic.last_reply_seq = seq;
                }
                let id = repo::insert_post(
                    tx,
                    &self.renderer,
                    topic.id,
                    &facts,
                    floor,
                    &new.markdown,
                    new.quote_post_id,
                    seq,
                    now,
                )
                .await?;
                repo::save_topic(tx, &self.renderer, &topic).await?;
                topic.version += 1;
                let tags = repo::topic_tag_ids(tx, &self.renderer, topic.id).await?;
                let post = repo::post(tx, &self.renderer, id)
                    .await?
                    .ok_or(ContentError::Unavailable)?;
                (topic, post, tags, Some(links))
            }
            Mutation::Edit(edit) => {
                positive(edit.post_id)?;
                let post = repo::post(tx, &self.renderer, edit.post_id)
                    .await?
                    .ok_or(ContentError::NotFound)?;
                let mut topic = repo::topic(tx, &self.renderer, post.topic_id)
                    .await?
                    .ok_or(ContentError::NotFound)?;
                check_forum_chain(&ctx.forums, topic.forum_id)?;
                repo::lock(tx, &self.renderer, "topics", topic.id).await?;
                repo::lock(tx, &self.renderer, "posts", post.id).await?;
                let mut post = repo::post(tx, &self.renderer, post.id)
                    .await?
                    .ok_or(ContentError::NotFound)?;
                self.basic_post_visible(tx, &ctx, &post).await?;
                if post.author_id != facts.user_id
                    || !topic.state.permits_write()
                    || post.deleted_at.is_some()
                    || post.moderation != Moderation::Normal
                {
                    return Err(ContentError::Forbidden);
                }
                if post.version != edit.expected_version {
                    return Err(ContentError::Conflict);
                }
                check_window(post.created_at, now, rules.edit_window_seconds)?;
                let mut tags = repo::topic_tag_ids(tx, &self.renderer, topic.id).await?;
                if post.floor_no == 1 {
                    if let Some(title) = &edit.title {
                        topic.title = title.trim().into()
                    }
                    if let Some(visibility) = edit.visibility {
                        topic.state = topic
                            .state
                            .apply_owner_change(OwnerStateChange::Visibility(visibility))?
                    }
                    if let Some(ids) = &edit.tag_ids {
                        tags = validate_tags(&ctx.tags, ids)?;
                    }
                } else if edit.title.is_some()
                    || edit.visibility.is_some()
                    || edit.tag_ids.is_some()
                {
                    return Err(ContentError::Forbidden);
                }
                let rendered = markdown::render(&edit.markdown);
                rules.validate_text(
                    (post.floor_no == 1).then_some(topic.title.as_str()),
                    &edit.markdown,
                    post.floor_no > 1,
                    tags.len(),
                    rendered.external_link_count(),
                    facts.tier,
                )?;
                let destination = forum_visibility(&ctx.forums, topic.forum_id)?
                    .strictest(topic.state.visibility)
                    .strictest(facts.default_visibility);
                let links = self
                    .target_links(
                        tx,
                        &ctx,
                        &rendered,
                        post.quote_post_id,
                        destination,
                        Some(topic.id),
                    )
                    .await?;
                post.body = edit.markdown.clone();
                post.edited_at = Some(now);
                repo::save_post(tx, &self.renderer, &post).await?;
                post.version += 1;
                topic.current_seq = seq;
                if post.floor_no == 1 {
                    topic.edited_at = Some(now);
                    repo::set_tags(tx, &self.renderer, topic.id, &tags).await?;
                }
                repo::save_topic(tx, &self.renderer, &topic).await?;
                topic.version += 1;
                (topic, post, tags, Some(links))
            }
            Mutation::Delete(delete) => {
                positive(delete.post_id)?;
                let post = repo::post(tx, &self.renderer, delete.post_id)
                    .await?
                    .ok_or(ContentError::NotFound)?;
                let mut topic = repo::topic(tx, &self.renderer, post.topic_id)
                    .await?
                    .ok_or(ContentError::NotFound)?;
                check_forum_chain(&ctx.forums, topic.forum_id)?;
                repo::lock(tx, &self.renderer, "topics", topic.id).await?;
                repo::lock(tx, &self.renderer, "posts", post.id).await?;
                let mut post = repo::post(tx, &self.renderer, post.id)
                    .await?
                    .ok_or(ContentError::NotFound)?;
                self.basic_post_visible(tx, &ctx, &post).await?;
                if post.author_id != facts.user_id || !topic.state.permits_write() {
                    return Err(ContentError::Forbidden);
                }
                if post.version != delete.expected_version {
                    return Err(ContentError::Conflict);
                }
                check_window(post.created_at, now, rules.delete_window_seconds)?;
                post.deleted_at = Some(now);
                repo::save_post(tx, &self.renderer, &post).await?;
                post.version += 1;
                topic.current_seq = seq;
                if post.floor_no == 1 {
                    if rules.first_post_deletion == crate::policy::FirstPostDeletion::PolicyPending
                    {
                        return Err(ContentError::PolicyPending);
                    }
                    topic.state = topic
                        .state
                        .apply_owner_change(OwnerStateChange::Delete { at: now })?
                } else {
                    let mut floor = 1;
                    let mut count = 0;
                    topic.last_reply_at = None;
                    topic.last_reply_author_id = None;
                    topic.last_reply_seq = topic.create_seq;
                    loop {
                        let rows =
                            repo::posts(tx, &self.renderer, topic.id, i64::MAX, floor, 100).await?;
                        if rows.is_empty() {
                            break;
                        }
                        for reply in rows {
                            floor = reply.floor_no;
                            if reply.deleted_at.is_none()
                                && reply.moderation == Moderation::Normal
                                && reply.publication == Publication::Published
                            {
                                count += 1;
                                topic.last_reply_at = Some(reply.created_at);
                                topic.last_reply_author_id = Some(reply.author_id);
                                topic.last_reply_seq = reply.create_seq;
                            }
                        }
                    }
                    topic.raw_reply_count = count;
                }
                repo::save_topic(tx, &self.renderer, &topic).await?;
                topic.version += 1;
                let tags = repo::topic_tag_ids(tx, &self.renderer, topic.id).await?;
                (topic, post, tags, None)
            }
        };
        // Every mutation retains a new immutable revision. Original revisions never change.
        repo::revision(
            tx,
            &self.renderer,
            &topic,
            &post,
            &tag_ids,
            facts.user_id,
            now,
        )
        .await?;
        if let Some(links) = links {
            repo::set_links(tx, &self.renderer, post.id, &links).await?;
        }
        if matches!(action, Action::CreateTopic | Action::CreateReply) {
            repo::charge(
                tx,
                &self.renderer,
                &command.operation_id,
                facts.user_id,
                action,
                post.id,
                facts.publication == Publication::Published || rules.count_pending,
                rules.version,
                now,
            )
            .await?;
        }
        repo::history(tx, &self.renderer, &topic).await?;
        let mut author_ids = BTreeSet::from([topic.author_id]);
        let mut floor = 0;
        loop {
            let batch = repo::posts(tx, &self.renderer, topic.id, i64::MAX, floor, 100).await?;
            if batch.is_empty() {
                break;
            }
            for row in batch {
                floor = row.floor_no;
                author_ids.insert(row.author_id);
            }
        }
        let ids = author_ids.into_iter().collect::<Vec<_>>();
        let mut authors = BTreeMap::new();
        for ids in ids.chunks(256) {
            authors.extend(
                self.authority
                    .authors(tx, ids, now)
                    .await?
                    .into_iter()
                    // 搜索文档按作者正文可见性收紧：正文不可公开时不进入公开检索面，
                    // 这比读路径更严，宁可少检出也不泄露正文。
                    .map(|a| (a.user_id, a.body_visibility)),
            );
        }
        repo::refresh_projection(
            tx,
            &self.renderer,
            &topic,
            forum_visibility(&ctx.forums, topic.forum_id)?,
            &authors,
        )
        .await?;
        // No new mutable lock follows risk/analytics/Outbox append phase.
        // DEC-054 有效贡献判定，供信任等级 T1 规则统计：
        //   - 主题：已发布且正文达到最低长度即计入；
        //   - 回复：已发布、达到最低长度，且发布者不是主题作者（防止自问自答刷贡献）；
        //   - 删除：本人删除后该条不再计入，但事实行保留，贡献数仍可从历史重算。
        // 最低长度在前面的 `validate_text` 已经强制过，所以这里只需再判断发布状态，
        // 以及回复场景下作者是否与主题作者相同。
        // 时机：本段位于追加段，两个调用都是纯插入／更新，不获取任何新锁。
        let contribution = match action {
            Action::CreateTopic | Action::CreateReply
                if facts.publication == Publication::Published =>
            {
                let is_reply = action == Action::CreateReply;
                if is_reply && post.author_id == topic.author_id {
                    None
                } else {
                    Some(Contribution::Recorded {
                        post_id: post.id,
                        is_reply,
                        body_chars: i64::try_from(post.body.trim().chars().count())
                            .map_err(|_| ContentError::InvalidInput)?,
                    })
                }
            }
            Action::DeletePost => Some(Contribution::Withdrawn { post_id: post.id }),
            _ => None,
        };
        if let Some(contribution) = contribution {
            self.authority
                .record_contribution(tx, facts.user_id, contribution, now)
                .await?;
        }
        self.authority
            .append_risk(
                tx,
                credentials,
                &facts,
                action,
                RiskDisposition::Allowed,
                &command.operation_id,
                now,
            )
            .await?;
        if facts.publication == Publication::Published
            && matches!(action, Action::CreateTopic | Action::CreateReply)
        {
            let kind = if action == Action::CreateTopic {
                platform::analytics::EventKind::TopicPublished
            } else {
                platform::analytics::EventKind::ReplyPublished
            };
            let fact = platform::analytics::AnalyticsFact::from_authority(
                new_operation_id(),
                kind,
                now,
                facts.analytics_pseudonym.clone(),
                Some(if action == Action::CreateTopic {
                    topic.id
                } else {
                    post.id
                }),
                platform::analytics::Dimensions {
                    forum_id: Some(topic.forum_id),
                    tag_ids: tag_ids.iter().copied().collect(),
                    channel_id: None,
                    device: platform::analytics::Device::Unknown,
                },
                facts.analytics_exclusions,
            )
            .map_err(|_| ContentError::Unavailable)?;
            platform_db::analytics_repo::enqueue(
                tx,
                &self.renderer,
                &fact,
                now,
                new_operation_id(),
            )
            .await
            .map_err(|_| ContentError::Unavailable)?;
        }
        let event = OutboxEvent {
            event_id: new_operation_id(),
            aggregate_type: "topic".into(),
            aggregate_id: topic.id.to_string(),
            event_type: "cache.invalidate".into(),
            version: 1,
            occurred_at: now,
            payload: SafePayload::empty()
                .with_reference("resource_id", &topic.id.to_string())
                .map_err(|_| ContentError::Unavailable)?,
            trace_id: String::new(),
        };
        platform_db::outbox_repo::enqueue(tx, &self.renderer, &event, &["cache.invalidate"])
            .await
            .map_err(|_| ContentError::Unavailable)?;
        let post_event = OutboxEvent {
            event_id: new_operation_id(),
            aggregate_type: "post".into(),
            aggregate_id: post.id.to_string(),
            event_type: "cache.invalidate".into(),
            version: 1,
            occurred_at: now,
            payload: SafePayload::empty(),
            trace_id: String::new(),
        };
        platform_db::outbox_repo::enqueue(tx, &self.renderer, &post_event, &["cache.invalidate"])
            .await
            .map_err(|_| ContentError::Unavailable)?;
        stored_success(post.id, post.version)
    }
    async fn filtered_markup(
        &self,
        tx: &mut TransactionConnection<'_>,
        ctx: &ReadContext,
        source: &str,
        current_topic: Option<i64>,
    ) -> Result<markdown::SafeMarkdown, ContentError> {
        let rendered = markdown::render(source);
        let mut allowed = BTreeSet::new();
        for target in rendered
            .links()
            .iter()
            .filter_map(|l| l.target())
            .collect::<BTreeSet<_>>()
        {
            let post = match target {
                markdown::LocalLinkTarget::Post { post_id } => {
                    repo::post(tx, &self.renderer, post_id).await?
                }
                markdown::LocalLinkTarget::Topic { topic_id, floor } => {
                    repo::post_at_floor(tx, &self.renderer, topic_id, floor.unwrap_or(1)).await?
                }
                markdown::LocalLinkTarget::CurrentTopicFloor { floor } => match current_topic {
                    Some(topic_id) => {
                        repo::post_at_floor(tx, &self.renderer, topic_id, floor).await?
                    }
                    None => None,
                },
            };
            if let Some(post) = post {
                match self.basic_post_visible(tx, ctx, &post).await {
                    Ok(_) => {
                        allowed.insert(target);
                    }
                    Err(ContentError::NotFound) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(markdown::render_filtered(source, |target| {
            allowed.contains(&target)
        }))
    }
    async fn target_links(
        &self,
        tx: &mut TransactionConnection<'_>,
        ctx: &ReadContext,
        rendered: &markdown::SafeMarkdown,
        quote: Option<i64>,
        destination: Visibility,
        current_topic: Option<i64>,
    ) -> Result<Vec<i64>, ContentError> {
        let mut targets = BTreeSet::new();
        if let Some(quote) = quote {
            positive(quote)?;
            targets.insert(quote);
        }
        for target in rendered.links().iter().filter_map(|link| link.target()) {
            let id = match target {
                markdown::LocalLinkTarget::CurrentTopicFloor { floor } => {
                    let Some(topic_id) = current_topic else {
                        return Err(ContentError::InvalidInput);
                    };
                    repo::post_at_floor(tx, &self.renderer, topic_id, floor)
                        .await?
                        .ok_or(ContentError::NotFound)?
                        .id
                }
                markdown::LocalLinkTarget::Post { post_id } => post_id,
                markdown::LocalLinkTarget::Topic { topic_id, floor } => {
                    repo::post_at_floor(tx, &self.renderer, topic_id, floor.unwrap_or(1))
                        .await?
                        .ok_or(ContentError::NotFound)?
                        .id
                }
            };
            targets.insert(id);
        }
        for id in &targets {
            let post = repo::post(tx, &self.renderer, *id)
                .await?
                .ok_or(ContentError::NotFound)?;
            let (topic, _, visibility) = self.basic_post_visible(tx, ctx, &post).await?;
            if post.publication != Publication::Published
                || topic.state.publication != Publication::Published
                || destination == Visibility::Public && visibility == Visibility::Members
            {
                return Err(ContentError::Forbidden);
            }
        }
        Ok(targets.into_iter().collect())
    }
    async fn quota(
        &self,
        tx: &mut TransactionConnection<'_>,
        rules: &ContentRules,
        facts: &AuthorityFacts,
        action: Action,
        now: DateTime<Utc>,
    ) -> Result<(), ContentError> {
        if facts.publication != Publication::Published && !rules.count_pending {
            return Ok(());
        }
        let offset = Duration::minutes(i64::from(rules.day_utc_offset_minutes));
        let local = now
            .checked_add_signed(offset)
            .ok_or(ContentError::InvalidInput)?;
        let start = local
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .ok_or(ContentError::InvalidInput)?
            .and_utc()
            - offset;
        let until = start + Duration::days(1);
        let topic = action == Action::CreateTopic;
        let daily = match (facts.tier, topic) {
            (TrustTier::T1, true) => rules.t1_daily_topics,
            (TrustTier::T1, false) => rules.t1_daily_replies,
            (_, true) => rules.t0_daily_topics,
            (_, false) => rules.t0_daily_replies,
        };
        if repo::charges(tx, &self.renderer, facts.user_id, action, start, until).await? >= daily {
            return Err(ContentError::QuotaExceeded);
        }
        if facts.tier == TrustTier::T0 && now < facts.created_at + Duration::hours(24) {
            let maximum = if topic {
                rules.early_topics
            } else {
                rules.early_replies
            };
            if repo::charges(
                tx,
                &self.renderer,
                facts.user_id,
                action,
                facts.created_at,
                facts.created_at + Duration::hours(24),
            )
            .await?
                >= maximum
            {
                return Err(ContentError::QuotaExceeded);
            }
        }
        Ok(())
    }
    pub async fn operation(
        &self,
        credentials: &Credentials,
        action: Action,
        operation_id: &str,
        now: DateTime<Utc>,
    ) -> Result<OperationStatus, ContentError> {
        let now = canonical(now)?;
        let mut tx = DbTransaction::begin(&self.pool).await?;
        let mut c = tx.connection();
        self.flags(&mut c, None).await?;
        let facts = self
            .authority
            .authorize(&mut c, credentials, action, now)
            .await?;
        if !facts.allowed || !facts.email_verified {
            return Err(ContentError::Forbidden);
        }
        let result = platform_db::idempotency_repo::query_result(
            &mut c,
            &self.renderer,
            &OperationQuery {
                actor_id: facts.user_id,
                scope: format!("content.{}", action.as_str()),
                idempotency_key: credentials.idempotency_key.clone(),
                operation_id: operation_id.into(),
            },
            now,
        )
        .await?;
        tx.rollback().await?;
        Ok(result)
    }
}
fn list_binding(filter: &ListFilter, viewer: &Viewer) -> Result<String, ContentError> {
    let value = serde_json::to_vec(&(filter, viewer.user_id, viewer.member))
        .map_err(|_| ContentError::InvalidCursor)?;
    Ok(request_hash(&value))
}
fn topic_binding(id: i64, viewer: &Viewer) -> String {
    request_hash(format!("topic:{id}:{:?}:{}", viewer.user_id, viewer.member).as_bytes())
}
fn forum_visibility(forums: &[Forum], id: i64) -> Result<Visibility, ContentError> {
    let mut id = id;
    let mut seen = BTreeSet::new();
    let mut visibility = Visibility::Public;
    loop {
        if !seen.insert(id) || seen.len() > 64 {
            return Err(ContentError::Unavailable);
        }
        let forum = forums
            .iter()
            .find(|f| f.id == id)
            .ok_or(ContentError::NotFound)?;
        if forum.status == "draft" {
            return Err(ContentError::NotFound);
        }
        visibility = visibility.strictest(forum.visibility);
        match forum.parent_id {
            Some(parent) => id = parent,
            None => return Ok(visibility),
        }
    }
}
/// 校验版块链并返回目标版块的可见性：整条父链都必须存在且 `active`，且不得成环。
///
/// **不加锁**（`DEC-064`）。这里原本对链上每个版块取 `SELECT … FOR UPDATE`，但那与
/// `0001_init.sql` 只授予运行角色 `SELECT ON community.forums` 的最小权限 ACL 直接
/// 冲突——PostgreSQL 的 `FOR UPDATE` 需要 `UPDATE`/`DELETE` 权限，结果是完整模式下
/// 所有内容写入必然失败。SQLite 侧看不到：`lock_sql` 对 SQLite 返回不带 `FOR UPDATE`
/// 的原始 SELECT，SQLite 也没有表级授权。
///
/// 去掉行锁是安全的：版块是迁移/种子维护的参照数据，运行角色本就无权改写，运行期
/// 不存在并发修改；而调用方已经先取得 `community.commit_state` 根锁，所有社区写都
/// 已被串行化，版块行锁不提供额外互斥。校验本身一条不少地保留。
fn check_forum_chain(forums: &[Forum], id: i64) -> Result<Visibility, ContentError> {
    let visibility = forum_visibility(forums, id)?;
    let mut ids = BTreeSet::new();
    let mut current = id;
    loop {
        // 父链成环会让下面的遍历永不终止；见到重复 ID 直接判为数据损坏。
        if !ids.insert(current) {
            return Err(ContentError::Unavailable);
        }
        let forum = forums
            .iter()
            .find(|f| f.id == current)
            .ok_or(ContentError::NotFound)?;
        if forum.status != "active" {
            return Err(ContentError::Forbidden);
        }
        match forum.parent_id {
            Some(id) => current = id,
            None => break,
        }
    }
    Ok(visibility)
}
fn validate_tags(tags: &[Tag], input: &[i64]) -> Result<Vec<i64>, ContentError> {
    let ids = input.iter().copied().collect::<BTreeSet<_>>();
    if ids.len() != input.len() {
        return Err(ContentError::InvalidInput);
    }
    for id in &ids {
        positive(*id)?;
        if !tags
            .iter()
            .any(|t| t.id == *id && !t.restricted && t.merged_into.is_none())
        {
            return Err(ContentError::Forbidden);
        }
    }
    Ok(ids.into_iter().collect())
}
fn check_window(
    created: DateTime<Utc>,
    now: DateTime<Utc>,
    seconds: i64,
) -> Result<(), ContentError> {
    if now < created
        || now
            >= created
                .checked_add_signed(Duration::seconds(seconds))
                .ok_or(ContentError::PolicyPending)?
    {
        Err(ContentError::Forbidden)
    } else {
        Ok(())
    }
}
fn stored_failure(status_code: u16) -> Result<StoredResponse, ContentError> {
    Ok(StoredResponse {
        status_code,
        body: SafePayload::empty()
            .with_outcome(Outcome::Failure)
            .to_json()
            .map_err(|_| ContentError::Unavailable)?,
    })
}
fn stored_success(post: i64, version: i64) -> Result<StoredResponse, ContentError> {
    Ok(StoredResponse {
        status_code: 200,
        body: SafePayload::empty()
            .with_outcome(Outcome::Success)
            .with_reference("resource_id", &post.to_string())
            .and_then(|p| p.with_reference("version", &version.to_string()))
            .and_then(|p| p.to_json())
            .map_err(|_| ContentError::Unavailable)?,
    })
}
#[cfg(test)]
mod tests;
