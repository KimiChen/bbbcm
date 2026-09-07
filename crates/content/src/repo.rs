//! Content tables only. Every function borrows the caller's uncommittable connection.
use crate::contract::*;
use chrono::{DateTime, Utc};
use platform::{identifiers::SchemaRenderer, transaction::LockTarget};
use platform_db::transaction::{DbRow, DbValue, TransactionConnection};
use std::collections::BTreeMap;
type Result<T> = std::result::Result<T, ContentError>;
pub(crate) fn table(
    tx: &TransactionConnection<'_>,
    r: &SchemaRenderer,
    name: &str,
) -> Result<String> {
    r.table(tx.backend(), "community", name)
        .map_err(|_| ContentError::Unavailable)
}
fn text(s: &str) -> DbValue {
    s.into()
}
fn date(t: DateTime<Utc>) -> DbValue {
    DbValue::Time(t)
}
fn optional_time(row: &DbRow, key: &str) -> Result<Option<DateTime<Utc>>> {
    let time = row.time(key)?;
    Ok((time != DateTime::UNIX_EPOCH).then_some(time))
}
fn optional_id(id: i64) -> Option<i64> {
    (id > 0).then_some(id)
}
fn state(row: &DbRow) -> Result<ContentState> {
    let value = ContentState {
        publication: Publication::parse(&row.text("publication")?)?,
        moderation: Moderation::parse(&row.text("moderation")?)?,
        locked: row.boolean("locked")?,
        visibility: Visibility::parse(&row.text("visibility")?)?,
        deleted_at: optional_time(row, "deleted_at")?,
        archived_at: optional_time(row, "archived_at")?,
    };
    value.validate()?;
    Ok(value)
}
#[derive(Clone)]
pub(crate) struct Topic {
    pub id: i64,
    pub forum_id: i64,
    pub author_id: i64,
    pub title: String,
    pub state: ContentState,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub create_seq: i64,
    pub current_seq: i64,
    pub last_reply_seq: i64,
    pub next_floor: i64,
    pub raw_reply_count: i64,
    /// NAV-003／NAV-004 的累计阅读量。由浏览计数事务单独累加，读路径只读不写。
    pub view_count: i64,
    pub last_reply_author_id: Option<i64>,
    pub last_reply_at: Option<DateTime<Utc>>,
    pub pinned: bool,
    pub globally_pinned: bool,
    pub featured: bool,
    pub version: i64,
}
#[derive(Clone)]
pub(crate) struct Post {
    pub id: i64,
    pub topic_id: i64,
    pub author_id: i64,
    pub floor_no: i64,
    pub body: String,
    pub quote_post_id: Option<i64>,
    pub publication: Publication,
    pub moderation: Moderation,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub create_seq: i64,
    pub version: i64,
}
impl Post {
    pub fn can_read(&self, viewer: &Viewer) -> bool {
        self.deleted_at.is_none()
            && self.moderation == Moderation::Normal
            && (self.publication == Publication::Published
                || viewer.user_id == Some(self.author_id))
    }
}
/// 在 DEC-054 的 30 分钟去重窗口内登记一次浏览，返回是否应当累加计数。
///
/// 去重台账按 (topic_id, subject_hash) 唯一：
///   * 台账无记录 → 首次浏览，插入并返回真；
///   * 记录存在但上次浏览已超出窗口 → 刷新时间戳并返回真；
///   * 记录存在且仍在窗口内 → 返回假。
///
/// 注意：返回假时调用方会回滚整个事务，因此窗口内的重复访问**不会**刷新时间戳。
/// 这样窗口是从「上一次真正计数的浏览」起算的固定窗口，而不是滑动窗口——否则每 29 分钟
/// 刷新一次的访问者将永远只被计 1 次，与 DEC-054「30 分钟内只计 1 次」的读法不符。
///
/// `subject_hash` 是调用方派生的不可逆摘要，原始用户 ID、Cookie 与设备指纹不进入本表。
/// 这里刻意先查后写而不是依赖唯一键冲突，因为 DEC-022 的三库交集子集禁用 `ON CONFLICT`。
pub(crate) async fn register_view(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic_id: i64,
    subject_hash: &str,
    window: chrono::Duration,
    now: DateTime<Utc>,
) -> Result<bool> {
    let views = r
        .table(tx.backend(), "community", "topic_views")
        .map_err(|_| ContentError::Unavailable)?;
    let existing = tx
        .fetch_optional(
            &format!(
                "SELECT id, last_viewed_at, version FROM {views} WHERE topic_id = ? AND subject_hash = ?"
            ),
            &[topic_id.into(), text(subject_hash)],
        )
        .await?;
    let Some(row) = existing else {
        tx.execute(
            &format!(
                "INSERT INTO {views} (topic_id, subject_hash, first_viewed_at, last_viewed_at, version) VALUES (?, ?, ?, ?, ?)"
            ),
            &[
                topic_id.into(),
                text(subject_hash),
                date(now),
                date(now),
                DbValue::I64(1),
            ],
        )
        .await?;
        return Ok(true);
    };
    let id = row.i64("id")?;
    let last = row.time("last_viewed_at")?;
    let version = row.i64("version")?;
    let fresh = now
        .checked_sub_signed(window)
        .is_some_and(|threshold| last <= threshold);
    tx.execute_versioned(
        &format!(
            "UPDATE {views} SET last_viewed_at = ?, version = version + 1 WHERE id = ? AND version = ?"
        ),
        &[date(now), id.into(), version.into()],
    )
    .await?;
    Ok(fresh)
}

/// 累加主题的累计阅读量。
///
/// 单表单行更新，不带版本 CAS：阅读量是可累加的计数而不是需要防覆盖的业务状态，
/// 并发累加彼此不冲突。刻意不改 topics.version，避免阅读量把内容版本推高、
/// 进而让公开缓存和乐观并发校验被无意义地击穿。
pub(crate) async fn bump_view_count(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic_id: i64,
) -> Result<()> {
    let t = table(tx, r, "topics")?;
    tx.execute(
        &format!("UPDATE {t} SET view_count = view_count + 1 WHERE id = ?"),
        &[topic_id.into()],
    )
    .await?;
    Ok(())
}

fn topic_row(row: &DbRow) -> Result<Topic> {
    Ok(Topic {
        id: row.i64("id")?,
        forum_id: row.i64("forum_id")?,
        author_id: row.i64("author_id")?,
        title: row.text("title")?,
        state: state(row)?,
        created_at: row.time("created_at")?,
        edited_at: optional_time(row, "edited_at")?,
        create_seq: row.i64("create_seq")?,
        current_seq: row.i64("current_seq")?,
        last_reply_seq: row.i64("last_reply_seq")?,
        next_floor: row.i64("next_floor")?,
        raw_reply_count: row.i64("raw_reply_count")?,
        view_count: row.i64("view_count")?,
        last_reply_author_id: optional_id(row.i64("last_reply_author_id")?),
        last_reply_at: optional_time(row, "last_reply_at")?,
        pinned: row.boolean("pinned")?,
        globally_pinned: row.boolean("globally_pinned")?,
        featured: row.boolean("featured")?,
        version: row.i64("version")?,
    })
}
fn post_row(row: &DbRow) -> Result<Post> {
    Ok(Post {
        id: row.i64("id")?,
        topic_id: row.i64("topic_id")?,
        author_id: row.i64("author_id")?,
        floor_no: row.i64("floor_no")?,
        body: row.text("body")?,
        quote_post_id: optional_id(row.i64("quote_post_id")?),
        publication: Publication::parse(&row.text("publication")?)?,
        moderation: Moderation::parse(&row.text("moderation")?)?,
        deleted_at: optional_time(row, "deleted_at")?,
        created_at: row.time("created_at")?,
        edited_at: optional_time(row, "edited_at")?,
        create_seq: row.i64("create_seq")?,
        version: row.i64("version")?,
    })
}
pub(crate) async fn topic(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
) -> Result<Option<Topic>> {
    let t = table(tx, r, "topics")?;
    tx.fetch_optional(&format!("SELECT * FROM {t} WHERE id = ?"), &[id.into()])
        .await?
        .as_ref()
        .map(topic_row)
        .transpose()
}
pub(crate) async fn post(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
) -> Result<Option<Post>> {
    let t = table(tx, r, "posts")?;
    tx.fetch_optional(&format!("SELECT * FROM {t} WHERE id = ?"), &[id.into()])
        .await?
        .as_ref()
        .map(post_row)
        .transpose()
}
pub(crate) async fn post_at_floor(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic: i64,
    floor: i64,
) -> Result<Option<Post>> {
    let t = table(tx, r, "posts")?;
    tx.fetch_optional(
        &format!("SELECT * FROM {t} WHERE topic_id = ? AND floor_no = ?"),
        &[topic.into(), floor.into()],
    )
    .await?
    .as_ref()
    .map(post_row)
    .transpose()
}
pub(crate) async fn posts(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic_id: i64,
    watermark: i64,
    after_floor: i64,
    limit: i64,
) -> Result<Vec<Post>> {
    let t = table(tx, r, "posts")?;
    tx.fetch_all(&format!("SELECT * FROM {t} WHERE topic_id = ? AND create_seq <= ? AND floor_no > ? ORDER BY floor_no LIMIT ?"),&[topic_id.into(),watermark.into(),after_floor.into(),limit.into()]).await?.iter().map(post_row).collect()
}
pub(crate) async fn forums(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
) -> Result<Vec<Forum>> {
    let t = table(tx, r, "forums")?;
    tx.fetch_all(&format!("SELECT id, parent_id, slug, name, description, icon, position, visibility, status, version FROM {t} ORDER BY position, id"),&[]).await?.iter().map(|row|Ok(Forum{id:row.i64("id")?,parent_id:optional_id(row.i64("parent_id")?),slug:row.text("slug")?,name:row.text("name")?,description:row.text("description")?,icon:row.text("icon")?,position:row.i64("position")?,visibility:Visibility::parse(&row.text("visibility")?)?,status:row.text("status")?,rule:String::new(),version:row.i64("version")?})).collect()
}
pub(crate) async fn tags(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
) -> Result<Vec<Tag>> {
    let t = table(tx, r, "tags")?;
    tx.fetch_all(&format!("SELECT id, slug, name, description, restricted, merged_into, version FROM {t} ORDER BY id"),&[]).await?.iter().map(|row|Ok(Tag{id:row.i64("id")?,slug:row.text("slug")?,name:row.text("name")?,description:row.text("description")?,restricted:row.boolean("restricted")?,merged_into:optional_id(row.i64("merged_into")?),version:row.i64("version")?})).collect()
}
/// Resolve either registered canonical slug or alias; ambiguous cross-catalogue data
/// is corrupt rather than choosing a convenient target.
pub(crate) async fn tag_id_for_slug(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    slug: &str,
) -> Result<Option<i64>> {
    let tags = table(tx, r, "tags")?;
    let aliases = table(tx, r, "tag_aliases")?;
    let canonical = tx
        .fetch_optional(
            &format!("SELECT id FROM {tags} WHERE slug = ?"),
            &[text(slug)],
        )
        .await?
        .map(|row| row.i64("id"))
        .transpose()?;
    let alias = tx
        .fetch_optional(
            &format!("SELECT tag_id FROM {aliases} WHERE alias = ?"),
            &[text(slug)],
        )
        .await?
        .map(|row| row.i64("tag_id"))
        .transpose()?;
    match (canonical, alias) {
        (Some(a), Some(b)) if a != b => Err(ContentError::Unavailable),
        (Some(id), _) | (_, Some(id)) => Ok(Some(id)),
        (None, None) => Ok(None),
    }
}
/// 批量取主题。一次 `IN (...)` 代替 N 次按 id 查询。
///
/// 列表页渲染 50 行时逐条查主题是 N+1 的主要来源之一（`99` §5.44）。
/// 占位符按实际个数生成，值仍全部走 bind——`02` 的「模板占位只允许出现在表位置，
/// 值一律 bind」在这里同样成立：拼进 SQL 的只有 `?` 的个数，没有任何取值。
pub(crate) async fn topics_by_ids(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    ids: &[i64],
) -> Result<Vec<Topic>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = platform::db::topics_by_ids_sql(r, tx.backend(), ids.len())
        .map_err(|_| ContentError::Unavailable)?;
    let values: Vec<DbValue> = ids.iter().map(|id| (*id).into()).collect();
    tx.fetch_all(&sql, &values)
        .await?
        .iter()
        .map(topic_row)
        .collect()
}

/// 批量取指定楼层的帖子（列表页用 floor_no = 1 取首帖）。
pub(crate) async fn posts_at_floor_by_topics(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic_ids: &[i64],
    floor: i64,
) -> Result<Vec<Post>> {
    if topic_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = platform::db::posts_at_floor_by_topics_sql(r, tx.backend(), topic_ids.len())
        .map_err(|_| ContentError::Unavailable)?;
    // 用 `chain` 而不是 `push`：`sql_lint` 禁止 Repository 出现 `.push` 一类调用，
    // 它无法区分「往 SQL 里拼一段」和「往 bind 值数组里加一项」。规则宁可误伤也不
    // 放松，这里按它的口径写。顺序是「先全部 topic_id，最后 floor」，与
    // `posts_at_floor_by_topics_sql` 的占位符顺序一一对应。
    let values: Vec<DbValue> = topic_ids
        .iter()
        .map(|id| (*id).into())
        .chain(std::iter::once(floor.into()))
        .collect();
    tx.fetch_all(&sql, &values)
        .await?
        .iter()
        .map(post_row)
        .collect()
}

/// 批量取主题标签，返回 `(topic_id, tag_id)` 对。
pub(crate) async fn topic_tag_ids_by_topics(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic_ids: &[i64],
) -> Result<Vec<(i64, i64)>> {
    if topic_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = platform::db::topic_tags_by_topics_sql(r, tx.backend(), topic_ids.len())
        .map_err(|_| ContentError::Unavailable)?;
    let values: Vec<DbValue> = topic_ids.iter().map(|id| (*id).into()).collect();
    tx.fetch_all(&sql, &values)
        .await?
        .iter()
        .map(|row| Ok((row.i64("topic_id")?, row.i64("tag_id")?)))
        .collect()
}

pub(crate) async fn topic_tag_ids(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic_id: i64,
) -> Result<Vec<i64>> {
    let t = table(tx, r, "topic_tags")?;
    tx.fetch_all(
        &format!("SELECT tag_id FROM {t} WHERE topic_id = ? ORDER BY tag_id"),
        &[topic_id.into()],
    )
    .await?
    .iter()
    .map(|row| row.i64("tag_id").map_err(Into::into))
    .collect()
}
pub(crate) async fn set_tags(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic_id: i64,
    ids: &[i64],
) -> Result<()> {
    let t = table(tx, r, "topic_tags")?;
    tx.execute(
        &format!("DELETE FROM {t} WHERE topic_id = ?"),
        &[topic_id.into()],
    )
    .await?;
    for id in ids {
        tx.execute(
            &format!("INSERT INTO {t} (topic_id, tag_id) VALUES (?, ?)"),
            &[topic_id.into(), (*id).into()],
        )
        .await?;
    }
    Ok(())
}
pub(crate) async fn watermark(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
) -> Result<i64> {
    let t = table(tx, r, "commit_state")?;
    Ok(tx
        .fetch_optional(&format!("SELECT sequence_no FROM {t} WHERE id = 1"), &[])
        .await?
        .ok_or(ContentError::Unavailable)?
        .i64("sequence_no")?)
}
/// CAT-014／DEC-056：把主题移动到另一个版块。
///
/// 只更新 `forum_id` 与排序水位 `current_seq`，其余列一概不动——主题 ID、楼层、
/// 回复、修订与发布回执必须原样保留，这正是「同对象换版块」与「克隆到新 ID」的区别。
/// 带版本 CAS，避免与并发编辑互相覆盖。
pub(crate) async fn move_topic(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic: &Topic,
) -> Result<()> {
    let t = table(tx, r, "topics")?;
    tx.execute_versioned(
        &format!(
            "UPDATE {t} SET forum_id = ?, current_seq = ?, version = version + 1 WHERE id = ? AND version = ?"
        ),
        &[
            topic.forum_id.into(),
            topic.current_seq.into(),
            topic.id.into(),
            topic.version.into(),
        ],
    )
    .await?;
    Ok(())
}

pub(crate) async fn lock_root(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
) -> Result<i64> {
    lock(tx, r, "commit_state", 1).await?;
    let t = table(tx, r, "commit_state")?;
    let row = tx
        .fetch_optional(
            &format!("SELECT sequence_no, version FROM {t} WHERE id = 1"),
            &[],
        )
        .await?
        .ok_or(ContentError::Unavailable)?;
    let seq = row
        .i64("sequence_no")?
        .checked_add(1)
        .ok_or(ContentError::Unavailable)?;
    let v = row.i64("version")?;
    tx.execute_versioned(
        &format!(
            "UPDATE {t} SET sequence_no = ?, version = version + 1 WHERE id = 1 AND version = ?"
        ),
        &[seq.into(), v.into()],
    )
    .await?;
    Ok(seq)
}
pub(crate) async fn lock(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    name: &str,
    id: i64,
) -> Result<()> {
    tx.lock(
        r,
        &LockTarget::new("community", name, id).ok_or(ContentError::InvalidInput)?,
    )
    .await?;
    Ok(())
}
pub(crate) async fn insert_topic(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    input: &NewTopic,
    facts: &AuthorityFacts,
    seq: i64,
    now: DateTime<Utc>,
) -> Result<i64> {
    let t = table(tx, r, "topics")?;
    let no = DbValue::Bool(false);
    tx.execute(&format!("INSERT INTO {t} (forum_id, author_id, title, publication, moderation, locked, visibility, deleted_at, archived_at, created_at, edited_at, create_seq, current_seq, last_reply_seq, next_floor, raw_reply_count, last_reply_author_id, last_reply_at, pinned, globally_pinned, featured, version) VALUES (?, ?, ?, ?, 'normal', ?, ?, ?, ?, ?, ?, ?, ?, ?, 2, 0, 0, ?, ?, ?, ?, 1)"),&[input.forum_id.into(),facts.user_id.into(),text(input.title.trim()),text(facts.publication.as_str()),no.clone(),text(input.visibility.as_str()),date(DateTime::UNIX_EPOCH),date(DateTime::UNIX_EPOCH),date(now),date(DateTime::UNIX_EPOCH),seq.into(),seq.into(),seq.into(),date(DateTime::UNIX_EPOCH),no.clone(),no.clone(),no]).await?;
    Ok(tx.last_insert_id().await?)
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_post(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic_id: i64,
    facts: &AuthorityFacts,
    floor: i64,
    body: &str,
    quote: Option<i64>,
    seq: i64,
    now: DateTime<Utc>,
) -> Result<i64> {
    let t = table(tx, r, "posts")?;
    tx.execute(&format!("INSERT INTO {t} (topic_id, author_id, floor_no, body, quote_post_id, publication, moderation, deleted_at, created_at, edited_at, create_seq, version) VALUES (?, ?, ?, ?, ?, ?, 'normal', ?, ?, ?, ?, 1)"),&[topic_id.into(),facts.user_id.into(),floor.into(),text(body),quote.unwrap_or(0).into(),text(facts.publication.as_str()),date(DateTime::UNIX_EPOCH),date(now),date(DateTime::UNIX_EPOCH),seq.into()]).await?;
    Ok(tx.last_insert_id().await?)
}
pub(crate) async fn save_topic(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic: &Topic,
) -> Result<()> {
    let t = table(tx, r, "topics")?;
    tx.execute_versioned(&format!("UPDATE {t} SET title = ?, publication = ?, moderation = ?, locked = ?, visibility = ?, deleted_at = ?, archived_at = ?, edited_at = ?, current_seq = ?, last_reply_seq = ?, next_floor = ?, raw_reply_count = ?, last_reply_author_id = ?, last_reply_at = ?, pinned = ?, globally_pinned = ?, featured = ?, version = version + 1 WHERE id = ? AND version = ?"),&[text(&topic.title),text(topic.state.publication.as_str()),text(topic.state.moderation.as_str()),DbValue::Bool(topic.state.locked),text(topic.state.visibility.as_str()),date(topic.state.deleted_at.unwrap_or(DateTime::UNIX_EPOCH)),date(topic.state.archived_at.unwrap_or(DateTime::UNIX_EPOCH)),date(topic.edited_at.unwrap_or(DateTime::UNIX_EPOCH)),topic.current_seq.into(),topic.last_reply_seq.into(),topic.next_floor.into(),topic.raw_reply_count.into(),topic.last_reply_author_id.unwrap_or(0).into(),date(topic.last_reply_at.unwrap_or(DateTime::UNIX_EPOCH)),DbValue::Bool(topic.pinned),DbValue::Bool(topic.globally_pinned),DbValue::Bool(topic.featured),topic.id.into(),topic.version.into()]).await?;
    Ok(())
}
pub(crate) async fn save_post(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    post: &Post,
) -> Result<()> {
    let t = table(tx, r, "posts")?;
    tx.execute_versioned(&format!("UPDATE {t} SET body = ?, publication = ?, moderation = ?, deleted_at = ?, edited_at = ?, version = version + 1 WHERE id = ? AND version = ?"),&[text(&post.body),text(post.publication.as_str()),text(post.moderation.as_str()),date(post.deleted_at.unwrap_or(DateTime::UNIX_EPOCH)),date(post.edited_at.unwrap_or(DateTime::UNIX_EPOCH)),post.id.into(),post.version.into()]).await?;
    Ok(())
}
pub(crate) async fn revision(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic: &Topic,
    post: &Post,
    tags: &[i64],
    editor: i64,
    now: DateTime<Utc>,
) -> Result<()> {
    let t = table(tx, r, "post_revisions")?;
    let state = serde_json::to_string(&topic.state).map_err(|_| ContentError::Unavailable)?;
    let tags = serde_json::to_string(tags).map_err(|_| ContentError::Unavailable)?;
    tx.execute(&format!("INSERT INTO {t} (post_id, revision_no, title, body, state_json, tags_json, editor_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"),&[post.id.into(),post.version.into(),text(&topic.title),text(&post.body),text(&state),text(&tags),editor.into(),date(now)]).await?;
    Ok(())
}
pub(crate) async fn history(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic: &Topic,
) -> Result<()> {
    let t = table(tx, r, "topic_history")?;
    tx.execute(&format!("INSERT INTO {t} (sequence_no, topic_id, create_seq, last_reply_seq, pinned, globally_pinned, featured) VALUES (?, ?, ?, ?, ?, ?, ?)"),&[topic.current_seq.into(),topic.id.into(),topic.create_seq.into(),topic.last_reply_seq.into(),DbValue::Bool(topic.pinned),DbValue::Bool(topic.globally_pinned),DbValue::Bool(topic.featured)]).await?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn charge(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    operation: &str,
    actor: i64,
    action: Action,
    post: i64,
    charged: bool,
    policy: i64,
    now: DateTime<Utc>,
) -> Result<()> {
    let t = table(tx, r, "publication_receipts")?;
    tx.execute(&format!("INSERT INTO {t} (operation_id, actor_id, action, post_id, occurred_at, charged, policy_version) VALUES (?, ?, ?, ?, ?, ?, ?)"),&[text(operation),actor.into(),text(action.as_str()),post.into(),date(now),DbValue::Bool(charged),policy.into()]).await?;
    Ok(())
}
pub(crate) async fn charges(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    actor: i64,
    action: Action,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<i64> {
    let t = table(tx, r, "publication_receipts")?;
    Ok(tx.fetch_optional(&format!("SELECT COUNT(*) AS count FROM {t} WHERE actor_id = ? AND action = ? AND occurred_at >= ? AND occurred_at < ? AND charged = ?"),&[actor.into(),text(action.as_str()),date(start),date(end),DbValue::Bool(true)]).await?.ok_or(ContentError::Unavailable)?.i64("count")?)
}
/// DEC-030 热门榜候选：按 (view_count, id) 降序的 keyset 分页。
///
/// 排除条件与 DEC-030 一致：系统账号与确认作弊流量已在浏览计数阶段排除（不会进 view_count），
/// 这里再排除已删除内容与未发布内容。活动参与回复属 P2，当前不存在。
async fn hot_candidates(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    filter: &ListFilter,
    sort: i64,
    id: i64,
    limit: i64,
) -> Result<Vec<(i64, i64)>> {
    let topics = table(tx, r, "topics")?;
    let topic_tags = table(tx, r, "topic_tags")?;
    let forum = filter.forum_id.unwrap_or(0);
    let tag = filter.tag_id.unwrap_or(0);
    let owner = filter.owner_id.unwrap_or(0);
    let rows = tx.fetch_all(&format!("SELECT t.id AS resource_id, t.view_count AS sort_seq FROM {topics} t WHERE (t.view_count < ? OR (t.view_count = ? AND t.id < ?)) AND t.publication = ? AND t.moderation = ? AND t.deleted_at = ? AND (? = 0 OR t.forum_id = ?) AND (? = 0 OR EXISTS (SELECT 1 FROM {topic_tags} tt WHERE tt.topic_id = t.id AND tt.tag_id = ?)) AND (? = 0 OR t.author_id = ?) ORDER BY t.view_count DESC, t.id DESC LIMIT ?"),
        &[sort.into(), sort.into(), id.into(), text("published"), text("normal"), date(DateTime::UNIX_EPOCH),
          forum.into(), forum.into(), tag.into(), tag.into(), owner.into(), owner.into(), limit.into()]).await?;
    rows.iter()
        .map(|row| Ok((row.i64("resource_id")?, row.i64("sort_seq")?)))
        .collect()
}

/// NAV-004 站点统计：当前可见口径下的真实计数。
pub(crate) async fn site_statistics(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
) -> Result<crate::contract::SiteStatistics> {
    let topics = table(tx, r, "topics")?;
    let posts = table(tx, r, "posts")?;
    let users = r
        .table(tx.backend(), "identity", "users")
        .map_err(|_| ContentError::Unavailable)?;
    let count = |row: Option<platform_db::transaction::DbRow>| -> Result<i64> {
        Ok(row.ok_or(ContentError::Unavailable)?.i64("count")?)
    };
    // 主题与回复只算已发布、未被审核处置、未删除的；回复排除首帖（floor_no = 1），
    // 否则「主题 12 / 回复 24」里的回复数会把每个主题的首帖重复算进去。
    let topics = count(tx.fetch_optional(&format!("SELECT COUNT(*) AS count FROM {topics} WHERE publication = ? AND moderation = ? AND deleted_at = ?"),
        &[text("published"), text("normal"), date(DateTime::UNIX_EPOCH)]).await?)?;
    let replies = count(tx.fetch_optional(&format!("SELECT COUNT(*) AS count FROM {posts} WHERE floor_no > 1 AND publication = ? AND moderation = ? AND deleted_at = ?"),
        &[text("published"), text("normal"), date(DateTime::UNIX_EPOCH)]).await?)?;
    let users = count(
        tx.fetch_optional(
            &format!("SELECT COUNT(*) AS count FROM {users} WHERE status = ?"),
            &[text("active")],
        )
        .await?,
    )?;
    Ok(crate::contract::SiteStatistics {
        topics,
        replies,
        users,
    })
}

/// `SEARCH-001`／`SEARCH-004`：SQLite 体验模式的标题检索。
///
/// **可见性过滤在 SQL 里完成，先于任何摘要与高亮**（`02` §13）：只返回已发布、
/// 未被审核处置、未删除，且主题与版块都对当前访问者可见的行。把过滤放到返回之后
/// 再做，高亮片段会先一步带出调用者无权看到的标题。
///
/// `member` 为真表示访问者是登录成员，可看到 `members` 可见性的内容。
/// 通配符已由调用方转义，这里再用 `ESCAPE` 显式声明转义符。
/// 完整模式全文检索（`WP-MVP-C05`）。
///
/// 语句本身来自 `platform::db::full_text_search_sql`——它用 `@@`／`to_tsvector`
/// 这类 PostgreSQL 专有语法，共享 SQL 静态守卫会正确地拒绝它们出现在 Repository
/// 里。这里只负责绑值与取行，方言归方言模块。
///
/// 绑定顺序必须与语句里 `?` 的出现顺序严格一致：查询词出现两次（标题一次、正文
/// 一次），因此绑两次同一个值。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn search_full_text(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    terms: &str,
    forum_id: i64,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    newest_first: bool,
    member: bool,
    limit: i64,
) -> Result<Vec<(i64, String, i64, DateTime<Utc>)>> {
    let Some(sql) = platform::db::full_text_search_sql(r, tx.backend(), newest_first)
        .map_err(|_| ContentError::Unavailable)?
    else {
        // 该后端没有全文方言。调用方只应在能力为 FullText 时走到这里，
        // 走到了说明装配与能力判断不一致——宁可失败，也不静默降级成空结果。
        return Err(ContentError::Unavailable);
    };
    let rows = tx
        .fetch_all(
            &sql,
            &[
                text(terms),
                text(terms),
                text("published"),
                text("normal"),
                date(DateTime::UNIX_EPOCH),
                DbValue::I64(i64::from(member)),
                text("public"),
                text("public"),
                forum_id.into(),
                forum_id.into(),
                date(since),
                date(DateTime::UNIX_EPOCH),
                date(since),
                date(until),
                date(DateTime::UNIX_EPOCH),
                date(until),
                limit.into(),
            ],
        )
        .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.i64("topic_id")?,
                row.text("title")?,
                row.i64("forum_id")?,
                row.time("created_at")?,
            ))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn search_titles(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    pattern: &str,
    forum_id: i64,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    newest_first: bool,
    member: bool,
    limit: i64,
) -> Result<Vec<(i64, String, i64, DateTime<Utc>)>> {
    let docs = table(tx, r, "search_documents")?;
    let topics = table(tx, r, "topics")?;
    // 两条完整语句，而不是把 ORDER BY 作为片段插进模板：
    // SQL 模板占位只允许出现在表位置，值一律 bind。排序方向来自封闭枚举，
    // 但「来自枚举」不等于可以拼接——拼接一旦成为惯例，下一处就会拼上用户输入。
    let values = [
        text(pattern),
        text("published"),
        text("normal"),
        date(DateTime::UNIX_EPOCH),
        DbValue::I64(i64::from(member)),
        text("public"),
        text("public"),
        forum_id.into(),
        forum_id.into(),
        date(since),
        date(DateTime::UNIX_EPOCH),
        date(since),
        date(until),
        date(DateTime::UNIX_EPOCH),
        date(until),
        limit.into(),
    ];
    let rows = if newest_first {
        tx.fetch_all(&format!("SELECT DISTINCT d.topic_id AS topic_id, d.title AS title, d.forum_id AS forum_id, t.created_at AS created_at FROM {docs} d JOIN {topics} t ON t.id = d.topic_id WHERE d.title_normalized LIKE ? ESCAPE '!' AND d.publication = ? AND d.moderation = ? AND d.deleted_at = ? AND (? = 1 OR (d.visibility = ? AND d.forum_visibility = ?)) AND (? = 0 OR d.forum_id = ?) AND (? = ? OR t.created_at >= ?) AND (? = ? OR t.created_at < ?) ORDER BY t.created_at DESC, d.topic_id DESC LIMIT ?"), &values).await?
    } else {
        tx.fetch_all(&format!("SELECT DISTINCT d.topic_id AS topic_id, d.title AS title, d.forum_id AS forum_id, t.created_at AS created_at FROM {docs} d JOIN {topics} t ON t.id = d.topic_id WHERE d.title_normalized LIKE ? ESCAPE '!' AND d.publication = ? AND d.moderation = ? AND d.deleted_at = ? AND (? = 1 OR (d.visibility = ? AND d.forum_visibility = ?)) AND (? = 0 OR d.forum_id = ?) AND (? = ? OR t.created_at >= ?) AND (? = ? OR t.created_at < ?) ORDER BY t.created_at ASC, d.topic_id ASC LIMIT ?"), &values).await?
    };
    rows.iter()
        .map(|row| {
            Ok((
                row.i64("topic_id")?,
                row.text("title")?,
                row.i64("forum_id")?,
                row.time("created_at")?,
            ))
        })
        .collect()
}

/// `SEO-004`／`SEO-005`：可公开索引的主题清单，供 sitemap 与 RSS 使用。
///
/// 只收录**公开且已发布未删除未遮蔽**的主题：成员可见、待审、草稿、删除与被审核
/// 处置的内容一律不出现。这条过滤与搜索共用同一组条件，但**额外排除了成员可见**——
/// sitemap 与 RSS 面向匿名抓取方，没有「登录成员」这个概念。
pub(crate) async fn public_index(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    forum_id: i64,
    limit: i64,
) -> Result<Vec<(i64, String, DateTime<Utc>)>> {
    let t = table(tx, r, "topics")?;
    let f = table(tx, r, "forums")?;
    let rows = tx.fetch_all(&format!(
        "SELECT t.id AS id, t.title AS title, t.created_at AS created_at          FROM {t} t JOIN {f} f ON f.id = t.forum_id          WHERE t.publication = ? AND t.moderation = ? AND t.deleted_at = ?            AND t.visibility = ? AND f.visibility = ?            AND (? = 0 OR t.forum_id = ?)          ORDER BY t.created_at DESC, t.id DESC LIMIT ?"),
        &[
            text("published"), text("normal"), date(DateTime::UNIX_EPOCH),
            text("public"), text("public"),
            forum_id.into(), forum_id.into(), limit.into(),
        ]).await?;
    rows.iter()
        .map(|row| Ok((row.i64("id")?, row.text("title")?, row.time("created_at")?)))
        .collect()
}

pub(crate) async fn candidates(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    filter: &ListFilter,
    watermark: i64,
    sort: i64,
    id: i64,
    limit: i64,
) -> Result<Vec<(i64, i64)>> {
    let topics = table(tx, r, "topics")?;
    let history = table(tx, r, "topic_history")?;
    let posts = table(tx, r, "posts")?;
    let topic_tags = table(tx, r, "topic_tags")?;
    let forum = filter.forum_id.unwrap_or(0);
    let tag = filter.tag_id.unwrap_or(0);
    let owner = filter.owner_id.unwrap_or(0);
    let (sql, values) = if filter.kind == ListKind::Topics {
        // DEC-030 的「热门」：累计阅读量降序，(阅读量, id) 组成 keyset 游标。
        //
        // 与其他视图不同，热门的排序键取自 topics.view_count 而不是不可变的
        // topic_history：阅读量本来就会随时间变化，把它冻进历史表反而会让「热门」
        // 停在过去某一刻。NAV-005 要求的是「不因新内容插入导致重复/漏项」——
        // keyset 游标对插入是稳定的；一个主题的阅读量在翻页途中变化时可能被重复
        // 或跳过，这是 DEC-030 选择「最简可解释口径」时接受的代价，不是缺陷。
        //
        // 已删除主题不进热门榜：它的正文已是占位（DEC-061），把它排上榜没有意义。
        if filter.order == ListOrder::Hot {
            return hot_candidates(tx, r, filter, sort, id, limit).await;
        }
        let latest = i64::from(filter.order == ListOrder::LatestReply);
        let pinned = i64::from(filter.order == ListOrder::Pinned);
        let featured = i64::from(filter.order == ListOrder::Featured);
        (
            format!(
                "SELECT t.id AS resource_id, CASE WHEN ? = 1 THEN h.last_reply_seq ELSE h.create_seq END AS sort_seq FROM {topics} t JOIN {history} h ON h.topic_id = t.id WHERE h.sequence_no = (SELECT MAX(h2.sequence_no) FROM {history} h2 WHERE h2.topic_id = t.id AND h2.sequence_no <= ?) AND (CASE WHEN ? = 1 THEN h.last_reply_seq ELSE h.create_seq END < ? OR (CASE WHEN ? = 1 THEN h.last_reply_seq ELSE h.create_seq END = ? AND t.id < ?)) AND (? = 0 OR t.forum_id = ?) AND (? = 0 OR EXISTS (SELECT 1 FROM {topic_tags} tt WHERE tt.topic_id = t.id AND tt.tag_id = ?)) AND (? = 0 OR t.author_id = ?) AND (? = 0 OR h.pinned = ? OR h.globally_pinned = ?) AND (? = 0 OR h.featured = ?) ORDER BY sort_seq DESC, t.id DESC LIMIT ?"
            ),
            vec![
                latest.into(),
                watermark.into(),
                latest.into(),
                sort.into(),
                latest.into(),
                sort.into(),
                id.into(),
                forum.into(),
                forum.into(),
                tag.into(),
                tag.into(),
                owner.into(),
                owner.into(),
                pinned.into(),
                DbValue::Bool(true),
                DbValue::Bool(true),
                featured.into(),
                DbValue::Bool(true),
                limit.into(),
            ],
        )
    } else {
        (
            format!(
                "SELECT p.id AS resource_id, p.create_seq AS sort_seq FROM {posts} p JOIN {topics} t ON t.id = p.topic_id WHERE p.create_seq <= ? AND p.floor_no > 1 AND (p.create_seq < ? OR (p.create_seq = ? AND p.id < ?)) AND (? = 0 OR t.forum_id = ?) AND (? = 0 OR EXISTS (SELECT 1 FROM {topic_tags} tt WHERE tt.topic_id = t.id AND tt.tag_id = ?)) AND (? = 0 OR p.author_id = ?) ORDER BY p.create_seq DESC, p.id DESC LIMIT ?"
            ),
            vec![
                watermark.into(),
                sort.into(),
                sort.into(),
                id.into(),
                forum.into(),
                forum.into(),
                tag.into(),
                tag.into(),
                owner.into(),
                owner.into(),
                limit.into(),
            ],
        )
    };
    tx.fetch_all(&sql, &values)
        .await?
        .iter()
        .map(|row| Ok((row.i64("resource_id")?, row.i64("sort_seq")?)))
        .collect()
}
pub(crate) async fn set_links(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    post: i64,
    targets: &[i64],
) -> Result<()> {
    let t = table(tx, r, "post_links")?;
    tx.execute(
        &format!("DELETE FROM {t} WHERE post_id = ?"),
        &[post.into()],
    )
    .await?;
    for id in targets {
        tx.execute(
            &format!("INSERT INTO {t} (post_id, target_post_id) VALUES (?, ?)"),
            &[post.into(), (*id).into()],
        )
        .await?;
    }
    Ok(())
}
pub(crate) async fn refresh_projection(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    topic: &Topic,
    forum_visibility: Visibility,
    authors: &BTreeMap<i64, Visibility>,
) -> Result<()> {
    let t = table(tx, r, "search_documents")?;
    let mut floor = 0;
    loop {
        let batch = posts(tx, r, topic.id, i64::MAX, floor, 100).await?;
        if batch.is_empty() {
            break;
        }
        for p in batch {
            floor = p.floor_no;
            let title = topic.title.clone();
            let body = if tx.backend() == platform::db::Backend::Postgres {
                p.body.clone()
            } else {
                String::new()
            };
            let visibility = topic
                .state
                .visibility
                .strictest(forum_visibility)
                .strictest(
                    authors
                        .get(&topic.author_id)
                        .copied()
                        .unwrap_or(Visibility::Members),
                )
                .strictest(
                    authors
                        .get(&p.author_id)
                        .copied()
                        .unwrap_or(Visibility::Members),
                );
            let publication = if topic.state.publication == Publication::Published {
                p.publication
            } else {
                topic.state.publication
            };
            let moderation = if topic.state.moderation == Moderation::Normal {
                p.moderation
            } else {
                topic.state.moderation
            };
            let deleted = topic
                .state
                .deleted_at
                .or(p.deleted_at)
                .unwrap_or(DateTime::UNIX_EPOCH);
            let values = vec![
                topic.id.into(),
                topic.forum_id.into(),
                text(&title),
                text(&title.to_lowercase()),
                text(&body),
                text(visibility.as_str()),
                text(forum_visibility.as_str()),
                text(publication.as_str()),
                text(moderation.as_str()),
                date(deleted),
                p.version.into(),
                topic.version.into(),
                p.create_seq.into(),
                // 物化排序键（`99` §5.39）。取主题的 created_at 而不是帖子的：
                // 搜索结果按主题排序，取帖子时间会让同一主题的多行给出不同的
                // 排序位置，DISTINCT 之后顺序不确定。
                date(topic.created_at),
                p.id.into(),
            ];
            let affected=tx.execute(&format!("UPDATE {t} SET topic_id = ?, forum_id = ?, title = ?, title_normalized = ?, body = ?, visibility = ?, forum_visibility = ?, publication = ?, moderation = ?, deleted_at = ?, entity_version = ?, topic_version = ?, create_seq = ?, created_at = ? WHERE post_id = ?"),&values).await?;
            if affected == 0 {
                tx.execute(&format!("INSERT INTO {t} (topic_id, forum_id, title, title_normalized, body, visibility, forum_visibility, publication, moderation, deleted_at, entity_version, topic_version, create_seq, created_at, post_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),&values).await?;
            }
        }
    }
    Ok(())
}
