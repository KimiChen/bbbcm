//! Synthetic content policy tests in real temporary SQLite. These fixtures grant no runtime level.
use super::*;
use crate::policy::{SyntheticContentPolicy, UnconfiguredContentPolicy};
use platform::analytics::TrustedExclusions;
use platform_db::transaction::DbValue;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
struct SyntheticAuthority {
    deny: AtomicBool,
    fail_append: AtomicBool,
    pseudonym: String,
    /// 记录观察到的 DEC-054 贡献变更序列。
    ///
    /// 让测试可以直接断言「哪些发布构成有效贡献」，而不必反向读 trust 的仓储，
    /// 从而把内容领域的判定逻辑与信任领域的存储实现分开验证。
    contributions: std::sync::Mutex<Vec<(i64, Contribution)>>,
    /// 观察到的移帖审计调用：(操作者, 主题, 原版块, 新版块)。
    moves: std::sync::Mutex<Vec<(i64, i64, i64, i64)>>,
}
impl SyntheticAuthority {
    fn facts(&self, id: i64) -> AuthorityFacts {
        AuthorityFacts {
            user_id: id,
            created_at: at() - Duration::days(2),
            tier: if id == 3 {
                TrustTier::Pending
            } else {
                TrustTier::T0
            },
            email_verified: id != 4,
            allowed: !self.deny.load(Ordering::SeqCst),
            default_visibility: Visibility::Public,
            publication: if id == 5 {
                Publication::Pending
            } else {
                Publication::Published
            },
            analytics_pseudonym: self.pseudonym.clone(),
            analytics_exclusions: TrustedExclusions::from_authority(false, true, false, false),
        }
    }
    fn author(id: i64) -> Author {
        Author {
            user_id: id,
            public_id: format!("synthetic-{id}"),
            username: format!("fixture_{id}"),
            // 合成夹具里账号 8 代表已关闭 PRIV-002 的作者，用于验证正文扣留；
            // 账号 7 代表已关闭 PRIV-001 的作者，用于验证个人列表对访客不可见。
            body_visibility: if id == 8 {
                Visibility::Members
            } else {
                Visibility::Public
            },
            lists_visibility: if id == 7 {
                Visibility::Members
            } else {
                Visibility::Public
            },
        }
    }
}
impl ContentAuthority for SyntheticAuthority {
    /// 合成管理授权：只有账号 1 被授予管理能力，其余一律拒绝。
    /// `deny` 打开时连账号 1 也被拒，用于验证失败关闭路径。
    fn authorize_management<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        _capability: ManagementCapability,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ManagementActor> {
        Box::pin(async move {
            if self.deny.load(Ordering::SeqCst) {
                return Err(ContentError::PolicyPending);
            }
            let user_id: i64 = credentials
                .session_token
                .expose()
                .parse()
                .map_err(|_| ContentError::Forbidden)?;
            if user_id != 1 {
                return Err(ContentError::Forbidden);
            }
            Ok(ManagementActor { user_id })
        })
    }
    /// 合成审计：只记录调用事实，供测试断言移帖确实写了审计。
    fn append_move_audit<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        actor: &'a ManagementActor,
        topic_id: i64,
        from_forum_id: i64,
        to_forum_id: i64,
        _operation_id: &'a str,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async move {
            self.moves
                .lock()
                .map_err(|_| ContentError::Unavailable)?
                .push((actor.user_id, topic_id, from_forum_id, to_forum_id));
            Ok(())
        })
    }
    fn record_contribution<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        user_id: i64,
        contribution: Contribution,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async move {
            self.contributions
                .lock()
                .map_err(|_| ContentError::Unavailable)?
                .push((user_id, contribution));
            Ok(())
        })
    }
    fn read_viewer<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        raw: Option<&'a str>,
        _: DateTime<Utc>,
    ) -> FutureResult<'a, Viewer> {
        Box::pin(async move {
            let id = raw.and_then(|s| s.parse::<i64>().ok());
            Ok(Viewer {
                user_id: id,
                member: id.is_some_and(|id| id != 4),
                // 合成夹具里账号 9 代表被分析排除的账号，用于验证浏览量排除规则。
                excluded: id == Some(9),
            })
        })
    }
    fn authorize<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        c: &'a Credentials,
        _: Action,
        _: DateTime<Utc>,
    ) -> FutureResult<'a, AuthorityFacts> {
        Box::pin(async move {
            if c.csrf.expose() != "synthetic-csrf" || c.session_token.expose() == "revoked" {
                return Err(ContentError::Forbidden);
            }
            let id = c
                .session_token
                .expose()
                .parse::<i64>()
                .map_err(|_| ContentError::Forbidden)?;
            Ok(self.facts(id))
        })
    }
    fn authors<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        ids: &'a [i64],
        _: DateTime<Utc>,
    ) -> FutureResult<'a, Vec<Author>> {
        Box::pin(async move {
            assert!(ids.len() <= 256);
            Ok(ids.iter().copied().map(Self::author).collect())
        })
    }
    fn resolve_author<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        id: &'a str,
        _: DateTime<Utc>,
    ) -> FutureResult<'a, Option<Author>> {
        Box::pin(async move {
            Ok(id
                .strip_prefix("synthetic-")
                .and_then(|s| s.parse().ok())
                .map(Self::author))
        })
    }
    fn append_risk<'a>(
        &'a self,
        tx: &'a mut TransactionConnection<'_>,
        _: &'a Credentials,
        _: &'a AuthorityFacts,
        _: Action,
        _: RiskDisposition,
        _: &'a str,
        _: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async move {
            tx.enter_append_phase();
            if self.fail_append.load(Ordering::SeqCst) {
                Err(ContentError::Unavailable)
            } else {
                Ok(())
            }
        })
    }
}
fn at() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-06T12:00:00.000000Z").unwrap()
}
fn rules() -> ContentRules {
    ContentRules {
        version: 9001,
        allow_pending: false,
        allow_t0: true,
        allow_t1: true,
        minimum_title_chars: 1,
        maximum_title_chars: 200,
        minimum_body_chars: 1,
        maximum_body_chars: 100_000,
        minimum_reply_chars: 1,
        maximum_reply_chars: 100_000,
        maximum_tags: 5,
        edit_window_seconds: 3600,
        delete_window_seconds: 3600,
        first_post_deletion: crate::policy::FirstPostDeletion::HideTopic,
        t0_daily_topics: 1000,
        t0_daily_replies: 1000,
        t1_daily_topics: 1000,
        t1_daily_replies: 1000,
        early_topics: 1,
        early_replies: 5,
        day_utc_offset_minutes: 0,
        count_pending: true,
        max_external_links_t0: 10,
        max_external_links_t1: 20,
        show_structural_reply_count: true,
        show_last_public_reply: true,
    }
}
struct Fixture {
    dir: PathBuf,
    pool: Pool,
    renderer: SchemaRenderer,
    authority: Arc<SyntheticAuthority>,
    service: ContentService,
    forum: i64,
    /// 仅登录成员可见的版块，用于移帖后的权限收紧断言。
    members_forum: i64,
    tag: i64,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("bbbcm-content-{}", new_operation_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = platform::config::Config::default().database;
        config.sqlite_path = dir.join("content.sqlite3").to_string_lossy().into();
        config.schema_prefix = "content_probe_".into();
        platform_db::migrations::migrate(
            &config,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&config).await.unwrap();
        let renderer = SchemaRenderer::new(&config.schema_prefix).unwrap();
        let mut tx = DbTransaction::begin(&pool).await.unwrap();
        let mut c = tx.connection();
        let users = renderer.table(c.backend(), "identity", "users").unwrap();
        // 1..=5 是既有夹具账号（3 为 pending 等级、4 为非成员、5 为待审发布）。
        // 6..=9 为本轮新增：7 关闭 PRIV-001 列表公开、8 关闭 PRIV-002 正文公开、
        // 9 为分析排除账号，均由 SyntheticAuthority 按 ID 映射。
        for i in 1..=9 {
            let name = format!("synthetic_{i}");
            let email = format!("synthetic-{i}@example.invalid");
            c.execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"),&[new_operation_id().into(),new_operation_id().into(),DbValue::Bool(true),name.clone().into(),name.into(),email.clone().into(),email.into(),DbValue::Bool(true),DbValue::Time(at()),DbValue::Time(at())]).await.unwrap();
        }
        let forums = repo::table(&c, &renderer, "forums").unwrap();
        c.execute(&format!("INSERT INTO {forums} (slug, name, position, status, version) VALUES ('synthetic', '合成版块', 1, 'active', 1)"),&[]).await.unwrap();
        let forum = c.last_insert_id().await.unwrap();
        // 移帖测试需要第二个版块：一个公开、一个仅成员可见，用于验证移动后权限随新版块收紧。
        c.execute(&format!("INSERT INTO {forums} (slug, name, position, status, visibility, version) VALUES ('synthetic-members', '合成成员版块', 2, 'active', 'members', 1)"),&[]).await.unwrap();
        let members_forum = c.last_insert_id().await.unwrap();
        let tags = repo::table(&c, &renderer, "tags").unwrap();
        c.execute(&format!("INSERT INTO {tags} (slug, name, description, restricted, merged_into, version) VALUES ('fixture', '合成标签', '仅供测试', ?, 0, 1)"),&[DbValue::Bool(false)]).await.unwrap();
        let tag = c.last_insert_id().await.unwrap();
        let flags = renderer
            .table(c.backend(), "platform", "feature_flags")
            .unwrap();
        for flag in ["public_read", "topic_creation", "reply_creation"] {
            c.execute(
                &format!(
                    "UPDATE {flags} SET enabled = ?, version = version + 1 WHERE flag_key = ?"
                ),
                &[DbValue::Bool(true), flag.into()],
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
        let authority = Arc::new(SyntheticAuthority {
            deny: AtomicBool::new(false),
            fail_append: AtomicBool::new(false),
            pseudonym: new_operation_id(),
            contributions: std::sync::Mutex::new(Vec::new()),
            moves: std::sync::Mutex::new(Vec::new()),
        });
        let service = ContentService::new(
            pool.clone(),
            renderer.clone(),
            authority.clone(),
            Arc::new(SyntheticContentPolicy(rules())),
            CursorKey::new(vec![7; 32]).unwrap(),
        );
        Self {
            dir,
            pool,
            renderer,
            authority,
            service,
            forum,
            members_forum,
            tag,
        }
    }
    fn credentials(&self, id: i64) -> Credentials {
        Credentials::new(id.to_string(), "synthetic-csrf", new_operation_id())
    }
    fn topic_input(&self, label: &str) -> NewTopic {
        NewTopic {
            forum_id: self.forum,
            title: label.into(),
            markdown: "合成正文 **safe**".into(),
            tag_ids: vec![self.tag],
            visibility: Visibility::Public,
        }
    }
    async fn create(&self, label: &str) -> MutationResult {
        self.service
            .create_topic(&self.credentials(1), self.topic_input(label), at())
            .await
            .unwrap()
    }
    async fn topic(&self, id: i64) -> TopicDetail {
        self.service.topic(id, None, Some("1"), at()).await.unwrap()
    }
    async fn reply(&self, id: i64, label: &str) -> MutationResult {
        let v = self.topic(id).await.topic.version;
        self.service
            .reply(
                &self.credentials(1),
                NewReply {
                    topic_id: id,
                    markdown: label.into(),
                    quote_post_id: None,
                    expected_topic_version: v,
                },
                at(),
            )
            .await
            .unwrap()
    }
    async fn count(&self, schema: &str, table: &str) -> i64 {
        let mut tx = DbTransaction::begin_read(&self.pool).await.unwrap();
        let mut c = tx.connection();
        let t = self.renderer.table(c.backend(), schema, table).unwrap();
        let count = c
            .fetch_optional(&format!("SELECT COUNT(*) AS n FROM {t}"), &[])
            .await
            .unwrap()
            .unwrap()
            .i64("n")
            .unwrap();
        tx.rollback().await.unwrap();
        count
    }
}
#[tokio::test]
async fn create_edit_delete_share_projection_revisions_events_and_idempotency() {
    let f = Fixture::new().await;
    let credentials = f.credentials(1);
    let input = f.topic_input("初始标题");
    let created = f
        .service
        .create_topic(&credentials, input.clone(), at())
        .await
        .unwrap();
    assert_eq!(created.floor_no, 1);
    let replay = f
        .service
        .create_topic(&credentials, input.clone(), at())
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.post_id, created.post_id);
    assert_eq!(f.count("community", "topics").await, 1);
    assert_eq!(f.count("community", "post_revisions").await, 1);
    assert_eq!(f.count("community", "search_documents").await, 1);
    assert_eq!(f.count("analytics", "event_facts").await, 1);
    assert_eq!(f.count("platform", "outbox_events").await, 3);
    let mut changed = input;
    changed.title = "异载荷".into();
    assert_eq!(
        f.service
            .create_topic(&credentials, changed, at())
            .await
            .unwrap_err(),
        ContentError::Conflict
    );
    let edited = f
        .service
        .edit(
            &f.credentials(1),
            EditPost {
                post_id: created.post_id,
                expected_version: 1,
                title: Some("已编辑".into()),
                markdown: "新正文<script>alert(1)</script>".into(),
                tag_ids: None,
                visibility: None,
            },
            at() + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(edited.version, 2);
    let detail = f.topic(created.topic_id).await;
    assert_eq!(detail.topic.title, "已编辑");
    assert!(detail.first_post.html.contains("&lt;script&gt;"));
    assert!(detail.first_post.edited_at.is_some());
    assert_eq!(f.count("community", "post_revisions").await, 2);
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let mut c = tx.connection();
    let revisions = repo::table(&c, &f.renderer, "post_revisions").unwrap();
    let original = c
        .fetch_optional(
            &format!("SELECT title, body FROM {revisions} WHERE post_id = ? AND revision_no = 1"),
            &[created.post_id.into()],
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.text("title").unwrap(), "初始标题");
    assert_eq!(original.text("body").unwrap(), "合成正文 **safe**");
    tx.rollback().await.unwrap();
    f.service
        .delete(
            &f.credentials(1),
            DeletePost {
                post_id: created.post_id,
                expected_version: 2,
            },
            at() + Duration::seconds(2),
        )
        .await
        .unwrap();
    // DEC-061：作者删除首帖后主题页仍可打开，首帖正文以占位呈现，回复继续可读。
    let deleted = f
        .service
        .topic(created.topic_id, None, None, at())
        .await
        .unwrap();
    assert_eq!(deleted.first_post.body_state, BodyState::Deleted);
    assert!(deleted.first_post.markdown.is_empty() && deleted.first_post.html.is_empty());
    assert_eq!(
        deleted.first_post.body_state.placeholder(),
        "该内容已删除。"
    );
    assert_eq!(f.count("community", "post_revisions").await, 3);
    assert_eq!(f.count("community", "publication_receipts").await, 1);
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let c = &mut tx.connection();
    let docs = repo::table(c, &f.renderer, "search_documents").unwrap();
    let row = c
        .fetch_optional(
            &format!("SELECT deleted_at, body FROM {docs} WHERE post_id = ?"),
            &[created.post_id.into()],
        )
        .await
        .unwrap()
        .unwrap();
    assert_ne!(row.time("deleted_at").unwrap(), DateTime::UNIX_EPOCH);
    assert_eq!(row.text("body").unwrap(), "");
    tx.rollback().await.unwrap();
}
#[tokio::test]
async fn authority_policy_cross_owner_window_and_version_fail_closed() {
    let f = Fixture::new().await;
    for actor in [3, 4] {
        assert_eq!(
            f.service
                .create_topic(&f.credentials(actor), f.topic_input("拒绝"), at())
                .await
                .unwrap_err(),
            ContentError::Forbidden
        );
    }
    assert_eq!(f.count("community", "topics").await, 0);
    let created = f.create("owner").await;
    let edit = EditPost {
        post_id: created.post_id,
        expected_version: 1,
        title: None,
        markdown: "changed".into(),
        tag_ids: None,
        visibility: None,
    };
    assert_eq!(
        f.service
            .edit(&f.credentials(2), edit.clone(), at())
            .await
            .unwrap_err(),
        ContentError::Forbidden
    );
    assert_eq!(
        f.service
            .edit(&f.credentials(1), edit.clone(), at() + Duration::hours(1))
            .await
            .unwrap_err(),
        ContentError::Forbidden
    );
    let mut stale = edit.clone();
    stale.expected_version = 10;
    assert_eq!(
        f.service
            .edit(&f.credentials(1), stale, at())
            .await
            .unwrap_err(),
        ContentError::Conflict
    );
    let c = f.credentials(1);
    let input = f.topic_input("replay checks");
    f.service
        .create_topic(&c, input.clone(), at())
        .await
        .unwrap();
    let mut bad = c.clone();
    bad.csrf = SecretText::new("wrong");
    assert_eq!(
        f.service
            .create_topic(&bad, input.clone(), at())
            .await
            .unwrap_err(),
        ContentError::Forbidden
    );
    f.authority.deny.store(true, Ordering::SeqCst);
    assert_eq!(
        f.service.create_topic(&c, input, at()).await.unwrap_err(),
        ContentError::Forbidden
    );
    let mut closed = f.service.clone();
    closed.policy = Arc::new(UnconfiguredContentPolicy);
    assert!(
        closed
            .list(&ListFilter::default(), None, None, at())
            .await
            .is_ok()
    );
    let list = closed
        .list(&ListFilter::default(), None, None, at())
        .await
        .unwrap();
    // 回复数受发布策略控制，策略未装配时不展示；浏览量是独立的存量计数，
    // 与发布策略无关，因此仍然给出（此处尚无浏览，故为 0）。
    assert!(
        list.topics
            .iter()
            .all(|t| t.reply_count.is_none() && t.view_count == Some(0))
    );
}
#[tokio::test]
async fn rollback_at_final_append_removes_all_business_and_projection() {
    let f = Fixture::new().await;
    f.authority.fail_append.store(true, Ordering::SeqCst);
    let err = f
        .service
        .create_topic(&f.credentials(1), f.topic_input("must rollback"), at())
        .await
        .unwrap_err();
    assert_eq!(err, ContentError::Unavailable);
    for (s, t) in [
        ("community", "topics"),
        ("community", "posts"),
        ("community", "post_revisions"),
        ("community", "search_documents"),
        ("community", "publication_receipts"),
        ("community", "topic_history"),
        ("platform", "idempotency_records"),
        ("platform", "outbox_events"),
    ] {
        assert_eq!(f.count(s, t).await, 0, "{s}.{t}");
    }
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    assert_eq!(
        repo::watermark(&mut tx.connection(), &f.renderer)
            .await
            .unwrap(),
        0
    );
    tx.rollback().await.unwrap();
}
#[tokio::test]
async fn stable_reply_floors_fixed_watermark_and_permalinks() {
    let f = Fixture::new().await;
    let topic = f.create("stable floors").await;
    let mut ids = Vec::new();
    for i in 0..53 {
        ids.push(f.reply(topic.topic_id, &format!("reply {i}")).await);
    }
    let first = f
        .service
        .topic(topic.topic_id, None, None, at())
        .await
        .unwrap();
    assert_eq!(first.replies.len(), 50);
    assert_eq!(first.replies[0].floor_no, 2);
    let cursor = first.next_cursor.unwrap();
    let added = f.reply(topic.topic_id, "new after page").await;
    assert_eq!(added.floor_no, 55);
    f.service
        .delete(
            &f.credentials(1),
            DeletePost {
                post_id: ids[10].post_id,
                expected_version: 1,
            },
            at() + Duration::seconds(1),
        )
        .await
        .unwrap();
    let second = f
        .service
        .topic(topic.topic_id, Some(&cursor), None, at())
        .await
        .unwrap();
    assert_eq!(
        second
            .replies
            .iter()
            .map(|r| r.floor_no)
            .collect::<Vec<_>>(),
        vec![52, 53, 54]
    );
    assert!(second.next_cursor.is_none());
    assert_eq!(
        f.service
            .locate(ids[10].post_id, None, at())
            .await
            .unwrap_err(),
        ContentError::NotFound
    );
    let loc = f.service.locate(ids[52].post_id, None, at()).await.unwrap();
    let detail = f
        .service
        .topic(loc.topic_id, loc.cursor.as_deref(), None, at())
        .await
        .unwrap();
    assert_eq!(detail.replies[0].id, ids[52].post_id);
    assert_eq!(detail.replies[0].floor_no, 54);
    // DEC-054 有效回复排除主题作者本人。本夹具里 53 条回复全部由主题作者发出，
    // 因此有效回复数为 0，而楼层号与分页定位不受影响——两者是不同口径。
    assert_eq!(detail.topic.reply_count, Some(0));
}
#[tokio::test]
async fn latest_reply_list_history_does_not_move_old_topics_across_cursor() {
    let f = Fixture::new().await;
    let mut all = Vec::new();
    for i in 0..55 {
        all.push(f.create(&format!("topic {i}")).await.topic_id);
    }
    let first = f
        .service
        .list(&ListFilter::default(), None, None, at())
        .await
        .unwrap();
    let cursor = first.next_cursor.unwrap();
    assert_eq!(first.topics.len(), 50);
    f.reply(all[1], "old topic bumped after watermark").await;
    let fresh = f.create("newly inserted").await;
    let second = f
        .service
        .list(&ListFilter::default(), Some(&cursor), None, at())
        .await
        .unwrap();
    let observed = first
        .topics
        .iter()
        .chain(second.topics.iter())
        .map(|t| t.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(observed, all.iter().copied().collect());
    assert!(!observed.contains(&fresh.topic_id));
    let filter = ListFilter {
        order: ListOrder::LatestTopic,
        ..ListFilter::default()
    };
    assert_eq!(
        f.service
            .list(&filter, Some(&cursor), None, at())
            .await
            .unwrap_err(),
        ContentError::InvalidCursor
    );
    assert_eq!(
        f.service
            .list(&ListFilter::default(), Some(&cursor), Some("1"), at())
            .await
            .unwrap_err(),
        ContentError::InvalidCursor
    );
    let mut tampered = cursor;
    tampered.push('0');
    assert_eq!(
        f.service
            .list(&ListFilter::default(), Some(&tampered), None, at())
            .await
            .unwrap_err(),
        ContentError::InvalidCursor
    );
}
#[tokio::test]
async fn visibility_member_quote_tightening_and_safe_preview_share_rules() {
    let f = Fixture::new().await;
    let target = f.create("target").await;
    let mut source = f.topic_input("source");
    source.markdown = format!(
        "My own label [reference](/posts/{}) <img src=x onerror=alert(1)>",
        target.post_id
    );
    let source = f
        .service
        .create_topic(&f.credentials(1), source, at())
        .await
        .unwrap();
    let before = f
        .service
        .topic(source.topic_id, None, None, at())
        .await
        .unwrap();
    assert!(
        before
            .first_post
            .html
            .contains(&format!("href=\"/posts/{}\"", target.post_id))
    );
    let preview = f
        .service
        .preview(&f.credentials(1), &before.first_post.markdown, at())
        .await
        .unwrap();
    assert_eq!(preview.html, before.first_post.html);
    f.service
        .edit(
            &f.credentials(1),
            EditPost {
                post_id: target.post_id,
                expected_version: 1,
                title: None,
                markdown: "restricted body".into(),
                tag_ids: None,
                visibility: Some(Visibility::Members),
            },
            at() + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(
        f.service
            .topic(target.topic_id, None, None, at())
            .await
            .unwrap_err(),
        ContentError::NotFound
    );
    let after = f
        .service
        .topic(source.topic_id, None, None, at())
        .await
        .unwrap();
    assert!(after.first_post.html.contains("My own label"));
    assert!(
        !after
            .first_post
            .html
            .contains(&format!("href=\"/posts/{}\"", target.post_id))
    );
    let member = f.topic(source.topic_id).await;
    assert!(
        member
            .first_post
            .html
            .contains(&format!("href=\"/posts/{}\"", target.post_id))
    );
    let public = f
        .service
        .list(&ListFilter::default(), None, None, at())
        .await
        .unwrap();
    assert_eq!(public.topics.len(), 1);
    assert_eq!(public.topics[0].id, source.topic_id);
    let version = f.topic(source.topic_id).await.topic.version;
    assert_eq!(
        f.service
            .reply(
                &f.credentials(1),
                NewReply {
                    topic_id: source.topic_id,
                    markdown: "no reference disclosure".into(),
                    quote_post_id: Some(target.post_id),
                    expected_topic_version: version
                },
                at()
            )
            .await
            .unwrap_err(),
        ContentError::Forbidden
    );
}
#[tokio::test]
async fn strict_daily_quota_survives_delete_and_pending_has_private_owner_read() {
    let f = Fixture::new().await;
    let mut r = rules();
    r.t0_daily_topics = 1;
    let mut limited = f.service.clone();
    limited.policy = Arc::new(SyntheticContentPolicy(r));
    let first = limited
        .create_topic(&f.credentials(1), f.topic_input("daily"), at())
        .await
        .unwrap();
    limited
        .delete(
            &f.credentials(1),
            DeletePost {
                post_id: first.post_id,
                expected_version: 1,
            },
            at() + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(
        limited
            .create_topic(
                &f.credentials(1),
                f.topic_input("delete does not refund"),
                at()
            )
            .await
            .unwrap_err(),
        ContentError::QuotaExceeded
    );
    assert!(
        limited
            .create_topic(
                &f.credentials(1),
                f.topic_input("next day"),
                at() + Duration::days(1)
            )
            .await
            .is_ok()
    );
    let pending = f
        .service
        .create_topic(&f.credentials(5), f.topic_input("pending"), at())
        .await
        .unwrap();
    assert_eq!(
        f.service
            .topic(pending.topic_id, None, None, at())
            .await
            .unwrap_err(),
        ContentError::NotFound
    );
    assert_eq!(
        f.service
            .topic(pending.topic_id, None, Some("1"), at())
            .await
            .unwrap_err(),
        ContentError::NotFound
    );
    assert!(
        f.service
            .topic(pending.topic_id, None, Some("5"), at())
            .await
            .is_ok()
    );
}
#[test]
fn unsigned_policy_offset_and_cursor_fail_without_panics() {
    let mut r = rules();
    r.day_utc_offset_minutes = i32::MIN;
    assert_eq!(r.validate(), Err(ContentError::PolicyPending));
    let key = CursorKey::new(vec![3; 32]).unwrap();
    let c = Cursor {
        version: 1,
        context: "test".into(),
        watermark: 1,
        sort: 1,
        id: 1,
    };
    let encoded = key.encode(&c).unwrap();
    assert_eq!(
        key.decode(&encoded, "other").unwrap_err(),
        ContentError::InvalidCursor
    );
    assert!(key.decode(&encoded, "test").is_ok());
}
#[tokio::test]
async fn cache_uses_current_authority_and_versions_and_ignores_private_pages() {
    use platform::cache::{CacheEntity, CacheStore, CacheVisibility, MemoryCache, PublicCacheKey};
    let f = Fixture::new().await;
    let created = f.create("cache").await;
    let cache = Arc::new(MemoryCache::default());
    let service = f.service.clone().with_cache(cache.clone());
    let original = service
        .topic(created.topic_id, None, None, at())
        .await
        .unwrap();
    let key = PublicCacheKey::new(
        CacheEntity::new("post", &created.post_id.to_string()).unwrap(),
        1,
        CacheVisibility::Public,
    )
    .unwrap();
    assert_eq!(
        cache.get(&key).await.unwrap().unwrap(),
        original.first_post.html
    );
    cache
        .set(
            &key,
            "<p>synthetic cached proof</p>".into(),
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .topic(created.topic_id, None, None, at())
            .await
            .unwrap()
            .first_post
            .html,
        "<p>synthetic cached proof</p>"
    );
    service
        .edit(
            &f.credentials(1),
            EditPost {
                post_id: created.post_id,
                expected_version: 1,
                title: None,
                markdown: "new authoritative version".into(),
                tag_ids: None,
                visibility: None,
            },
            at() + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert!(
        service
            .topic(created.topic_id, None, None, at())
            .await
            .unwrap()
            .first_post
            .html
            .contains("new authoritative version")
    );
    service
        .edit(
            &f.credentials(1),
            EditPost {
                post_id: created.post_id,
                expected_version: 2,
                title: None,
                markdown: "private body".into(),
                tag_ids: None,
                visibility: Some(Visibility::Members),
            },
            at() + Duration::seconds(2),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .topic(created.topic_id, None, None, at())
            .await
            .unwrap_err(),
        ContentError::NotFound
    );
    assert!(
        service
            .topic(created.topic_id, None, Some("1"), at())
            .await
            .unwrap()
            .first_post
            .html
            .contains("private body")
    );
    let private = PublicCacheKey::new(
        CacheEntity::new("post", &created.post_id.to_string()).unwrap(),
        3,
        CacheVisibility::Public,
    )
    .unwrap();
    assert!(cache.get(&private).await.unwrap().is_none());
}
#[tokio::test]
async fn operation_queries_remain_actor_key_bound_and_flags_reject_old_requests() {
    let f = Fixture::new().await;
    let credentials = f.credentials(1);
    let input = f.topic_input("operation");
    let created = f
        .service
        .create_topic(&credentials, input.clone(), at())
        .await
        .unwrap();
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let mut c = tx.connection();
    let receipts = repo::table(&c, &f.renderer, "publication_receipts").unwrap();
    let operation = c
        .fetch_optional(
            &format!("SELECT operation_id FROM {receipts} WHERE post_id = ?"),
            &[created.post_id.into()],
        )
        .await
        .unwrap()
        .unwrap()
        .text("operation_id")
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(matches!(
        f.service
            .operation(&credentials, Action::CreateTopic, &operation, at())
            .await
            .unwrap(),
        OperationStatus::Completed(_)
    ));
    let mut wrong = credentials.clone();
    wrong.session_token = SecretText::new("2");
    assert!(matches!(
        f.service
            .operation(&wrong, Action::CreateTopic, &operation, at())
            .await
            .unwrap(),
        OperationStatus::NotObserved
    ));
    wrong = credentials.clone();
    wrong.idempotency_key = new_operation_id();
    assert!(matches!(
        f.service
            .operation(&wrong, Action::CreateTopic, &operation, at())
            .await
            .unwrap(),
        OperationStatus::NotObserved
    ));
    wrong = credentials.clone();
    wrong.session_token = SecretText::new("revoked");
    assert_eq!(
        f.service
            .operation(&wrong, Action::CreateTopic, &operation, at())
            .await
            .unwrap_err(),
        ContentError::Forbidden
    );
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let c = &mut tx.connection();
    let flags = f
        .renderer
        .table(c.backend(), "platform", "feature_flags")
        .unwrap();
    c.execute(&format!("UPDATE {flags} SET enabled = ?, version = version + 1 WHERE flag_key = 'topic_creation'"),&[DbValue::Bool(false)]).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        f.service
            .create_topic(&credentials, input, at())
            .await
            .unwrap_err(),
        ContentError::Forbidden
    );
    assert_eq!(f.count("community", "topics").await, 1);
}
#[tokio::test]
async fn server_state_matrix_blocks_hidden_locked_deleted_and_leaves_archive_orthogonal() {
    let f = Fixture::new().await;
    let created = f.create("state matrix").await;
    // Both SQLite flags must have the same Boolean domain as PostgreSQL BOOLEAN.
    for table in ["topics", "topic_history"] {
        let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
        let table = repo::table(&tx.connection(), &f.renderer, table).unwrap();
        assert!(
            tx.connection()
                .execute(
                    &format!("UPDATE {table} SET globally_pinned = ?"),
                    &[2.into()]
                )
                .await
                .is_err()
        );
        tx.rollback().await.unwrap();
    }
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    let topics = repo::table(&c, &f.renderer, "topics").unwrap();
    c.execute(
        &format!("UPDATE {topics} SET locked = ?, version = version + 1 WHERE id = ?"),
        &[DbValue::Bool(true), created.topic_id.into()],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let locked = f.topic(created.topic_id).await;
    assert!(locked.topic.state.locked);
    assert_eq!(
        f.service
            .reply(
                &f.credentials(1),
                NewReply {
                    topic_id: created.topic_id,
                    markdown: "stale form".into(),
                    quote_post_id: None,
                    expected_topic_version: locked.topic.version
                },
                at()
            )
            .await
            .unwrap_err(),
        ContentError::Forbidden
    );
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let c = &mut tx.connection();
    c.execute(
        &format!("UPDATE {topics} SET moderation = 'hidden', version = version + 1 WHERE id = ?"),
        &[created.topic_id.into()],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        f.service
            .topic(created.topic_id, None, None, at())
            .await
            .unwrap_err(),
        ContentError::NotFound
    );
    assert!(
        f.service
            .list(&ListFilter::default(), None, Some("1"), at())
            .await
            .unwrap()
            .topics
            .is_empty()
    );
    let mut orthogonal = ContentState::published(Visibility::Public);
    orthogonal.archived_at = Some(at());
    assert!(orthogonal.validate().is_ok());
    assert!(!orthogonal.locked);
    assert!(!orthogonal.permits_write());
    let state = ContentState::published(Visibility::Public);
    let private = state
        .apply_owner_change(OwnerStateChange::Visibility(Visibility::Members))
        .unwrap();
    assert_eq!(private.publication, state.publication);
    assert_eq!(private.moderation, state.moderation);
    assert_eq!(private.locked, state.locked);
    assert!(private.deleted_at.is_none() && private.archived_at.is_none());
}
#[tokio::test]
async fn tag_aliases_and_merge_chains_resolve_current_canonical_visibility() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let c = &mut tx.connection();
    let tags = repo::table(c, &f.renderer, "tags").unwrap();
    let aliases = repo::table(c, &f.renderer, "tag_aliases").unwrap();
    c.execute(&format!("INSERT INTO {tags} (slug, name, description, restricted, merged_into, version) VALUES ('canonical', '合成规范标签', '', ?, 0, 1)"),&[DbValue::Bool(false)]).await.unwrap();
    let target = c.last_insert_id().await.unwrap();
    c.execute(
        &format!("INSERT INTO {aliases} (alias, tag_id) VALUES ('legacy', ?)"),
        &[f.tag.into()],
    )
    .await
    .unwrap();
    c.execute(
        &format!("UPDATE {tags} SET merged_into = ?, version = version + 1 WHERE id = ?"),
        &[target.into(), f.tag.into()],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    for slug in ["fixture", "legacy", "canonical"] {
        let tag = f
            .service
            .resolve_tag(slug, None, at())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tag.id, target);
        assert_eq!(tag.slug, "canonical");
    }
    assert!(
        f.service
            .resolve_tag("absent", None, at())
            .await
            .unwrap()
            .is_none()
    );
    let mut input = f.topic_input("canonical filter");
    input.tag_ids = vec![target];
    let created = f
        .service
        .create_topic(&f.credentials(1), input, at())
        .await
        .unwrap();
    let canonical = f
        .service
        .resolve_tag("legacy", None, at())
        .await
        .unwrap()
        .unwrap();
    let list = f
        .service
        .list(
            &ListFilter {
                tag_id: Some(canonical.id),
                ..ListFilter::default()
            },
            None,
            None,
            at(),
        )
        .await
        .unwrap();
    assert_eq!(list.topics.len(), 1);
    assert_eq!(list.topics[0].id, created.topic_id);
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {tags} SET restricted = ?, version = version + 1 WHERE id = ?"),
            &[DbValue::Bool(true), target.into()],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        f.service
            .resolve_tag("legacy", None, at())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.service
            .resolve_tag("legacy", Some("1"), at())
            .await
            .unwrap()
            .is_some()
    );
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {tags} SET merged_into = ?, version = version + 1 WHERE id = ?"),
            &[f.tag.into(), target.into()],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        f.service
            .resolve_tag("legacy", Some("1"), at())
            .await
            .unwrap_err(),
        ContentError::Unavailable
    );
}

/// DEC-054 有效贡献只在合格时、且每帖恰好一次地离开发布事务。
///
/// 断言四种情形：已发布主题计入；主题作者自己的回复不计入（防自问自答刷贡献）；
/// 其他账号的回复计入；本人删除该回复时产生一条撤销记录。
/// 这里直接观察 content 交给授权层的调用序列，因此不依赖 trust 的仓储实现。
#[tokio::test]
async fn only_approved_valid_contributions_leave_the_publishing_transaction() {
    let f = Fixture::new().await;
    let topic = f.create("贡献事实").await;
    let self_reply = f.reply(topic.topic_id, "作者自己的回复").await;
    let version = f.topic(topic.topic_id).await.topic.version;
    let other_reply = f
        .service
        .reply(
            &f.credentials(2),
            NewReply {
                topic_id: topic.topic_id,
                markdown: "另一个账号的回复".into(),
                quote_post_id: None,
                expected_topic_version: version,
            },
            at(),
        )
        .await
        .unwrap();
    let observed = f.authority.contributions.lock().unwrap().clone();
    assert_eq!(
        observed,
        vec![
            (
                1,
                Contribution::Recorded {
                    post_id: topic.post_id,
                    is_reply: false,
                    body_chars: "合成正文 **safe**".chars().count() as i64,
                }
            ),
            (
                2,
                Contribution::Recorded {
                    post_id: other_reply.post_id,
                    is_reply: true,
                    body_chars: "另一个账号的回复".chars().count() as i64,
                }
            ),
        ],
        "self reply {} must not qualify",
        self_reply.post_id
    );

    // 删除那条合格回复时，恰好撤销对应的一条事实，不牵连其他贡献。
    f.service
        .delete(
            &f.credentials(2),
            DeletePost {
                post_id: other_reply.post_id,
                expected_version: 1,
            },
            at() + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(
        f.authority.contributions.lock().unwrap().last(),
        Some(&(
            2,
            Contribution::Withdrawn {
                post_id: other_reply.post_id
            }
        ))
    );
}

/// DEC-054 浏览量：SSR 成功后的计数、30 分钟去重窗口、作者本人排除。
///
/// 覆盖四点：
///   1. 首次浏览累加，阅读量随之可读；
///   2. 窗口内重复浏览不累加，且台账时间戳不被刷新（固定窗口而非滑动窗口）；
///   3. 跨过窗口后再次累加；
///   4. 主题作者本人的访问不计入自己的阅读量。
#[tokio::test]
async fn view_counting_deduplicates_within_the_window_and_excludes_the_author() {
    let f = Fixture::new().await;
    let topic = f.create("浏览量").await;
    let guest = ViewSubject {
        kind: ViewSubjectKind::Session,
        hash: "synthetic-guest-digest".into(),
    };

    assert!(
        f.service
            .record_view(topic.topic_id, &guest, None, at())
            .await
            .unwrap()
    );
    assert_eq!(f.topic(topic.topic_id).await.topic.view_count, Some(1));

    // 窗口内重复访问不累加。第 29 分钟仍在 30 分钟窗口内。
    assert!(
        !f.service
            .record_view(topic.topic_id, &guest, None, at() + Duration::minutes(29))
            .await
            .unwrap()
    );
    assert_eq!(f.topic(topic.topic_id).await.topic.view_count, Some(1));

    // 窗口自「上一次计数」起算：第 31 分钟越过窗口，重新计数。
    assert!(
        f.service
            .record_view(topic.topic_id, &guest, None, at() + Duration::minutes(31))
            .await
            .unwrap()
    );
    assert_eq!(f.topic(topic.topic_id).await.topic.view_count, Some(2));

    // 主题作者本人（夹具里是账号 1）的访问不计数。
    let author = ViewSubject {
        kind: ViewSubjectKind::Account,
        hash: "synthetic-author-digest".into(),
    };
    assert!(
        !f.service
            .record_view(topic.topic_id, &author, Some(1), at() + Duration::hours(2))
            .await
            .unwrap()
    );
    assert_eq!(f.topic(topic.topic_id).await.topic.view_count, Some(2));

    // 空摘要没有稳定主体，拒绝而不是记一笔无法去重的浏览。
    assert_eq!(
        f.service
            .record_view(
                topic.topic_id,
                &ViewSubject {
                    kind: ViewSubjectKind::Device,
                    hash: String::new(),
                },
                None,
                at(),
            )
            .await
            .unwrap_err(),
        ContentError::InvalidInput
    );
}

/// DEC-054 的 PRIV-001／PRIV-002：作者关闭后未登录访客的投影收敛，登录成员与本人不受影响。
///
/// 合成夹具约定：账号 8 关闭了正文公开（PRIV-002），账号 7 关闭了列表公开（PRIV-001）。
#[tokio::test]
async fn author_privacy_withholds_body_and_lists_from_guests_only() {
    let f = Fixture::new().await;
    let version = f.topic(f.create("隐私作者").await.topic_id).await.topic.id;
    let private = f
        .service
        .create_topic(&f.credentials(8), f.topic_input("正文不公开"), at())
        .await
        .unwrap();
    assert!(version > 0);

    // 未登录访客：标题与元信息保留，正文被扣留并给出占位提示。
    let guest = f
        .service
        .topic(private.topic_id, None, None, at())
        .await
        .unwrap();
    assert_eq!(guest.first_post.body_state, BodyState::WithheldByAuthor);
    assert!(guest.first_post.markdown.is_empty() && guest.first_post.html.is_empty());
    assert!(!guest.topic.title.is_empty(), "标题按 DEC-054 保留");

    // 登录成员：正文正常展示。
    let member = f
        .service
        .topic(private.topic_id, None, Some("2"), at())
        .await
        .unwrap();
    assert_eq!(member.first_post.body_state, BodyState::Visible);
    assert!(!member.first_post.html.is_empty());

    // PRIV-001：账号 7 的个人列表对访客整体不可见，对登录成员可见。
    let owner_filter = ListFilter {
        owner_id: Some(7),
        ..ListFilter::default()
    };
    assert_eq!(
        f.service
            .list(&owner_filter, None, None, at())
            .await
            .unwrap_err(),
        ContentError::NotFound
    );
    assert!(
        f.service
            .list(&owner_filter, None, Some("2"), at())
            .await
            .is_ok()
    );
}

/// CAT-014／DEC-056：移帖是同对象换版块，主题 ID、楼层与回复全部不变。
///
/// 覆盖：未授权时整体失败关闭；移到原版块被拒；移动后 ID/楼层/回复不变、版本递增；
/// 移到仅成员可见版块后访客即刻看不到、登录成员仍可见；审计带操作者与前后版块。
#[tokio::test]
async fn moving_a_topic_keeps_its_identity_and_reapplies_the_target_forum_permissions() {
    let f = Fixture::new().await;
    let created = f.create("移帖").await;
    let reply = f.reply(created.topic_id, "移动前的回复").await;
    let before = f.topic(created.topic_id).await;

    // 未获管理授权时整体失败关闭，不产生任何状态变化。
    f.authority.deny.store(true, Ordering::SeqCst);
    assert_eq!(
        f.service
            .move_topic(
                &f.credentials(1),
                created.topic_id,
                f.members_forum,
                before.topic.version,
                at(),
            )
            .await
            .unwrap_err(),
        ContentError::PolicyPending
    );
    f.authority.deny.store(false, Ordering::SeqCst);
    assert!(f.authority.moves.lock().unwrap().is_empty());

    // 非管理账号被拒。
    assert_eq!(
        f.service
            .move_topic(
                &f.credentials(2),
                created.topic_id,
                f.members_forum,
                before.topic.version,
                at(),
            )
            .await
            .unwrap_err(),
        ContentError::Forbidden
    );

    // 移到原版块没有状态变化，直接拒绝而不是写一条空审计。
    assert_eq!(
        f.service
            .move_topic(
                &f.credentials(1),
                created.topic_id,
                f.forum,
                before.topic.version,
                at(),
            )
            .await
            .unwrap_err(),
        ContentError::InvalidInput
    );

    // 过期版本触发 CAS 冲突。
    assert_eq!(
        f.service
            .move_topic(
                &f.credentials(1),
                created.topic_id,
                f.members_forum,
                before.topic.version + 99,
                at(),
            )
            .await
            .unwrap_err(),
        ContentError::Conflict
    );

    let moved = f
        .service
        .move_topic(
            &f.credentials(1),
            created.topic_id,
            f.members_forum,
            before.topic.version,
            at(),
        )
        .await
        .unwrap();
    assert_eq!(moved.topic_id, created.topic_id, "主题 ID 必须不变");
    assert_eq!(moved.version, before.topic.version + 1);

    // 审计事实带操作者与前后版块。
    assert_eq!(
        f.authority.moves.lock().unwrap().as_slice(),
        &[(1, created.topic_id, f.forum, f.members_forum)]
    );

    // 登录成员视角：ID、楼层、回复与永久链接全部不变，只有版块变了。
    let after = f.topic(created.topic_id).await;
    assert_eq!(after.topic.id, before.topic.id);
    assert_eq!(after.topic.forum.id, f.members_forum);
    assert_eq!(after.replies.len(), before.replies.len());
    assert_eq!(after.replies[0].id, reply.post_id);
    assert_eq!(after.replies[0].floor_no, 2);

    // 访客视角：新版块仅成员可见，移动后即刻收敛。
    assert_eq!(
        f.service
            .topic(created.topic_id, None, None, at())
            .await
            .unwrap_err(),
        ContentError::NotFound
    );
}
