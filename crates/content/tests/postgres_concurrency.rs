//! `WP-MVP-E01` 并发矩阵中的「主题楼层」一项，对应 `90` 第 5 节「并发」。
//!
//! 闸门与运行前置同 `docs/development/postgres.md`，默认 `#[ignore]`。
//!
//! 为什么必须在真实 PostgreSQL 上跑：楼层号由「读当前 `next_floor` → 写回 +1」构成，
//! 是教科书式的 read-modify-write 竞态。SQLite 的单写者会把并发串行化，让这条路径
//! **永远不会失败**——在那上面跑绿了，证明不了楼层分配是安全的。
use chrono::{DateTime, Utc};
use content::contract::*;
use content::core::ContentService;
use content::cursor::CursorKey;
use content::policy::ApprovedContentPolicy;
use platform::config::{Config, DatabaseConfig, SecretString};
use platform::db::Backend;
use platform::identifiers::SchemaRenderer;
use platform::transaction::new_operation_id;
use platform_db::Pool;
use platform_db::transaction::{DbTransaction, DbValue, TransactionConnection};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn endpoint(var: &str, prefix: &str) -> String {
    assert_eq!(
        std::env::var("POSTGRES_CONTRACT_ISOLATED").ok().as_deref(),
        Some("yes"),
        "必须显式确认目标库是隔离且可丢弃的"
    );
    let raw = std::env::var(var).unwrap_or_else(|_| panic!("需要 {var}"));
    let raw = raw.replace("{prefix}", prefix);
    let parsed = url::Url::parse(&raw).unwrap_or_else(|_| panic!("{var} 的 URL 非法"));
    assert!(
        parsed.host_str().is_some_and(|host| host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())),
        "{var} 只允许字面回环 IP"
    );
    raw
}

fn config(prefix: &str, url: &str, max_connections: u32) -> DatabaseConfig {
    let mut cfg = Config::default().database;
    cfg.backend = Backend::Postgres;
    cfg.schema_prefix = prefix.to_owned();
    cfg.postgres_url = Some(SecretString::new(url.to_owned()));
    cfg.max_connections = max_connections;
    cfg
}

fn migrations_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations")
}

fn at() -> DateTime<Utc> {
    Utc::now()
}

/// 本测试要压的是**楼层分配**，不是授权。这个实现一律放行，把授权变量固定下来，
/// 让失败只可能来自并发本身。它绝不能出现在非测试代码里：`authorize` 无条件返回
/// `allowed: true`，用在生产上等于没有权限系统。管理路径仍保持失败关闭，
/// 免得这个放行实现被顺手接到管理动作上。
struct AllowAll {
    default_user: i64,
    /// 每个账号一个稳定的分析化名。必须是合法的操作 ID（UUIDv7）——
    /// `platform::analytics` 会校验它的格式，随手拼个 `pseudo-3` 会让
    /// 整条写路径在构造分析事实时失败，且失败被归一成 `Unavailable`。
    pseudonyms: std::collections::BTreeMap<i64, String>,
}

impl AllowAll {
    fn facts(&self, user_id: i64) -> AuthorityFacts {
        AuthorityFacts {
            user_id,
            // 账龄取足够久，确保 T1 规则不因「账号太新」而拒绝。
            created_at: at() - chrono::Duration::days(30),
            tier: TrustTier::T1,
            email_verified: true,
            allowed: true,
            default_visibility: Visibility::Public,
            publication: Publication::Published,
            analytics_pseudonym: self
                .pseudonyms
                .get(&user_id)
                .cloned()
                .unwrap_or_else(new_operation_id),
            analytics_exclusions: platform::analytics::TrustedExclusions::from_authority(
                false, true, false, false,
            ),
        }
    }
}

impl ContentAuthority for AllowAll {
    fn read_viewer<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _session: Option<&'a str>,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Viewer> {
        // 必须按 session 判定成员身份。恒返回 `member: true` 会让所有「匿名不可见」
        // 的断言变成空断言——过滤条件里 `member = 1` 一路放行，测什么都通过。
        let member = _session.is_some();
        Box::pin(async move {
            Ok(Viewer {
                user_id: member.then_some(self.default_user),
                member,
                excluded: true,
            })
        })
    }

    fn authorize<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        credentials: &'a Credentials,
        _action: Action,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, AuthorityFacts> {
        // 夹具把用户 ID 直接放进 session token；这是测试自造的凭据，
        // 不经过任何真实会话解析，也不代表生产可以这样取身份。
        let user_id: i64 = credentials
            .session_token
            .expose()
            .parse()
            .unwrap_or(self.default_user);
        Box::pin(async move { Ok(self.facts(user_id)) })
    }

    fn resolve_author<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _public_id: &'a str,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Option<Author>> {
        Box::pin(async { Ok(None) })
    }

    fn authors<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        user_ids: &'a [i64],
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, Vec<Author>> {
        Box::pin(async move {
            Ok(user_ids
                .iter()
                .map(|id| Author {
                    user_id: *id,
                    public_id: format!("pub-{id}"),
                    username: format!("u{id}"),
                    body_visibility: Visibility::Public,
                    lists_visibility: Visibility::Public,
                })
                .collect())
        })
    }

    fn authorize_management<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _credentials: &'a Credentials,
        _capability: ManagementCapability,
        _target: ManagementTarget,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ManagementActor> {
        Box::pin(async { Err(ContentError::PolicyPending) })
    }

    fn append_move_audit<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _actor: &'a ManagementActor,
        _topic_id: i64,
        _from_forum_id: i64,
        _to_forum_id: i64,
        _operation_id: &'a str,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn record_contribution<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _user_id: i64,
        _contribution: Contribution,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn append_risk<'a>(
        &'a self,
        _tx: &'a mut TransactionConnection<'_>,
        _credentials: &'a Credentials,
        _facts: &'a AuthorityFacts,
        _action: Action,
        _disposition: RiskDisposition,
        _event_id: &'a str,
        _now: DateTime<Utc>,
    ) -> FutureResult<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

struct Fixture {
    pool: Pool,
    renderer: SchemaRenderer,
    service: ContentService,
    users: Vec<i64>,
}

async fn fixture(workers: u32) -> Fixture {
    // 每轮一个全新前缀（由运行手册生成并预置角色），避免与上一轮残留互相干扰。
    let prefix = std::env::var("POSTGRES_CONTRACT_ALT_PREFIX")
        .unwrap_or_else(|_| panic!("需要 POSTGRES_CONTRACT_ALT_PREFIX"));
    platform_db::migrations::migrate(
        &config(
            &prefix,
            &endpoint("POSTGRES_CONTRACT_MIGRATOR_URL", &prefix),
            4,
        ),
        &migrations_root(),
    )
    .await
    .expect("迁移");
    let pool = Pool::connect(&config(
        &prefix,
        &endpoint("POSTGRES_CONTRACT_URL", &prefix),
        workers + 2,
    ))
    .await
    .expect("连接");
    let renderer = SchemaRenderer::new(&prefix).expect("前缀");

    // 种子全部走**迁移身份**：Feature Flag 与版块一样是运维/迁移维护的数据，
    // 运行角色只被授予 SELECT。用运行身份去写它会被 ACL 拒绝——这本身就是
    // `app_role_cannot_write_reference_tables_but_can_read_them` 断言的那条边界。
    // 必须用 `Purpose::Migrator` 连接：`Pool::connect` 走 `Purpose::App`，会要求
    // 登录角色的组成员关系恰好是 `app_rw` 一条，而迁移登录角色属于 `migrator`。
    let seed_pool = Pool::connect_for(
        &config(
            &prefix,
            &endpoint("POSTGRES_CONTRACT_MIGRATOR_URL", &prefix),
            2,
        ),
        platform_db::connection::Purpose::Migrator,
    )
    .await
    .expect("以迁移身份连接");

    let mut users = Vec::new();
    let mut tx = DbTransaction::begin(&seed_pool).await.expect("事务");
    {
        let mut c = tx.connection();
        let table = renderer
            .table(c.backend(), "identity", "users")
            .expect("表名");
        for _ in 0..workers.max(1) {
            let m = new_operation_id().replace('-', "");
            c.execute(
                &format!(
                    "INSERT INTO {table} (public_id, analytics_pseudonym, analytics_excluded, \
                     username, username_key, email, email_key, status, email_verified, bio, \
                     created_at, username_changed_at, credential_version, version) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"
                ),
                &[
                    DbValue::Text(format!("pub-{m}")),
                    DbValue::Text(format!("anon-{m}")),
                    DbValue::Bool(true),
                    DbValue::Text(format!("u{m}")),
                    DbValue::Text(format!("u{m}")),
                    DbValue::Text(format!("{m}@example.invalid")),
                    DbValue::Text(format!("{m}@example.invalid")),
                    DbValue::Bool(true),
                    DbValue::Time(at() - chrono::Duration::days(30)),
                    DbValue::Time(at() - chrono::Duration::days(30)),
                ],
            )
            .await
            .expect("建用户");
            users.push(c.last_insert_id().await.expect("回读主键"));
        }
        // 版块同样由迁移身份建：运行角色对 `community.forums` 只有 SELECT。
        let forums = renderer
            .table(c.backend(), "community", "forums")
            .expect("表名");
        // slug 唯一：同一前缀可能被多个用例复用，固定 slug 第二次就撞唯一约束。
        c.execute(
            &format!(
                "INSERT INTO {forums} (slug, name, position, status, visibility, version) \
                 VALUES (?, '并发探针版块', 1, 'active', 'public', 1)"
            ),
            &[DbValue::Text(format!("probe-{}", new_operation_id()))],
        )
        .await
        .expect("建版块");

        let flags = renderer
            .table(c.backend(), "platform", "feature_flags")
            .expect("表名");
        for flag in ["public_read", "topic_creation", "reply_creation"] {
            c.execute(
                &format!(
                    "UPDATE {flags} SET enabled = ?, version = version + 1 WHERE flag_key = ?"
                ),
                &[DbValue::Bool(true), DbValue::Text(flag.into())],
            )
            .await
            .expect("开 Flag");
        }
    }
    tx.commit().await.expect("提交");
    seed_pool.close().await;

    let default_user = users[0];
    let service = ContentService::new(
        pool.clone(),
        renderer.clone(),
        Arc::new(AllowAll {
            default_user,
            pseudonyms: users.iter().map(|id| (*id, new_operation_id())).collect(),
        }),
        Arc::new(ApprovedContentPolicy::new().expect("已批准政策")),
        CursorKey::new(vec![7; 32]).expect("游标密钥"),
    );
    Fixture {
        pool,
        renderer,
        service,
        users,
    }
}

/// 取一个可用版块 ID。版块是迁移种子维护的参照数据，运行角色只读，不能自己造。
async fn seeded_forum(f: &Fixture) -> i64 {
    let mut tx = DbTransaction::begin_read(&f.pool).await.expect("事务");
    let id = {
        let mut c = tx.connection();
        let forums = f
            .renderer
            .table(c.backend(), "community", "forums")
            .expect("表名");
        c.fetch_optional(
            &format!("SELECT id FROM {forums} WHERE status = 'active' ORDER BY position LIMIT 1"),
            &[],
        )
        .await
        .expect("查版块")
        .expect("种子必须至少提供一个启用版块")
        .i64("id")
        .expect("id")
    };
    tx.rollback().await.expect("回滚");
    id
}

/// `90` §5 并发「主题楼层」：并发回复不得产生重复或跳空的楼层号。
///
/// 三条断言各自不可省：
///
/// 1. **不重复**：两条回复拿到同一楼层，等于帖子里出现两个「3 楼」，引用和锚点全错。
/// 2. **不跳空**：楼层必须从 2 起连续（1 楼是主题首帖）。跳空说明有人拿了号却没写成，
///    而计数器已经推进——那意味着失败路径没有正确回滚。
/// 3. **计数一致**：主题的 `next_floor` 与 `raw_reply_count` 必须与实际行数吻合，
///    否则下一条回复会拿到错号。
///
/// 每个 Worker 用**不同账号**：同账号连发会先撞上 `DEC-060` 的每小时回复预算，
/// 那样测的就成了限流而不是楼层分配。
///
/// # 首次执行发现的缺陷（`DEC-064`，已修复）
///
/// 本载体第一次在真实 PostgreSQL 上运行时，在建主题一步就失败：
/// `community.forums` 的 ACL 只授予运行角色 `SELECT`，而 `lock_forum` 执行
/// `SELECT id FROM forums WHERE id = ? FOR UPDATE`——PostgreSQL 的 `FOR UPDATE`
/// 需要 `UPDATE`/`DELETE` 权限。影响面是完整模式下所有内容写入必然失败，
/// 与并发无关，单请求也一样。SQLite 侧看不到：`lock_sql` 对 SQLite 返回不带
/// `FOR UPDATE` 的原始 SELECT，SQLite 也没有表级授权，所以单元测试与冒烟全绿。
///
/// 按 `DEC-064` 的裁决去掉了版块行锁（校验全部保留），ACL 保持最小权限不放宽。
/// 这条注释保留下来，是为了记住「SQLite 全绿」不能替代真实 PostgreSQL 验证。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn concurrent_replies_allocate_unique_contiguous_floors() {
    const WORKERS: u32 = 8;
    const PER_WORKER: usize = 5;
    let f = fixture(WORKERS).await;
    let forum = seeded_forum(&f).await;

    let author = Credentials::new(f.users[0].to_string(), "csrf", new_operation_id());
    let topic = f
        .service
        .create_topic(
            &author,
            NewTopic {
                forum_id: forum,
                title: "并发楼层探针".into(),
                markdown: "主题正文，长度需满足已批准内容政策的下限要求。".into(),
                tag_ids: Vec::new(),
                visibility: Visibility::Public,
            },
            at(),
        )
        .await
        .expect("建主题");

    let mut tasks = Vec::new();
    for worker in 0..WORKERS {
        let service = f.service.clone();
        let user = f.users[worker as usize % f.users.len()];
        let topic_id = topic.topic_id;
        tasks.push(tokio::spawn(async move {
            let mut floors = Vec::new();
            for round in 0..PER_WORKER {
                // 版本冲突在并发下是**正常**结果，客户端应重读后重试；
                // 这里重试而非失败，否则测的成了「有没有冲突」而不是「楼层对不对」。
                for _attempt in 0..128 {
                    let Ok((_, _, version)) = service.topic_summary(topic_id, at()).await else {
                        tokio::task::yield_now().await;
                        continue;
                    };
                    let c = Credentials::new(user.to_string(), "csrf", new_operation_id());
                    match service
                        .reply(
                            &c,
                            NewReply {
                                topic_id,
                                markdown: format!(
                                    "并发回复 w{worker} r{round}，正文长度满足政策下限。"
                                ),
                                quote_post_id: None,
                                expected_topic_version: version,
                            },
                            at(),
                        )
                        .await
                    {
                        Ok(result) => {
                            floors.push(result.floor_no);
                            break;
                        }
                        Err(
                            ContentError::Conflict
                            | ContentError::RetryableDatabase(_)
                            | ContentError::Unavailable,
                        ) => {
                            tokio::task::yield_now().await;
                            continue;
                        }
                        Err(other) => panic!("回复失败：{other:?}"),
                    }
                }
            }
            floors
        }));
    }

    let mut all = Vec::new();
    for task in tasks {
        all.extend(task.await.expect("任务不应 panic"));
    }
    let expected_total = WORKERS as usize * PER_WORKER;
    assert_eq!(all.len(), expected_total, "所有回复都应最终成功");

    let unique: BTreeSet<i64> = all.iter().copied().collect();
    assert_eq!(
        unique.len(),
        all.len(),
        "出现重复楼层：共 {} 条回复、去重后 {} 个楼层",
        all.len(),
        unique.len()
    );
    let expected: BTreeSet<i64> = (2..=(expected_total as i64 + 1)).collect();
    assert_eq!(unique, expected, "楼层必须从 2 起连续，不得跳空");

    let posts = f
        .renderer
        .table(Backend::Postgres, "community", "posts")
        .expect("表名");
    let topics = f
        .renderer
        .table(Backend::Postgres, "community", "topics")
        .expect("表名");
    let mut tx = DbTransaction::begin_read(&f.pool).await.expect("事务");
    let (rows, next_floor, raw_replies) = {
        let mut c = tx.connection();
        let n = c
            .fetch_optional(
                &format!("SELECT count(*) AS n FROM {posts} WHERE topic_id = ?"),
                &[DbValue::I64(topic.topic_id)],
            )
            .await
            .expect("查回复数")
            .expect("聚合必有行")
            .i64("n")
            .expect("n");
        let row = c
            .fetch_optional(
                &format!("SELECT next_floor, raw_reply_count FROM {topics} WHERE id = ?"),
                &[DbValue::I64(topic.topic_id)],
            )
            .await
            .expect("查主题")
            .expect("主题应存在");
        (
            n,
            row.i64("next_floor").expect("next_floor"),
            row.i64("raw_reply_count").expect("raw_reply_count"),
        )
    };
    tx.rollback().await.expect("回滚");
    assert_eq!(rows, expected_total as i64 + 1, "帖子行数应含首帖");
    assert_eq!(
        next_floor,
        expected_total as i64 + 2,
        "next_floor 必须指向下一个未用楼层"
    );
    assert_eq!(raw_replies, expected_total as i64, "回复计数必须与实际一致");
    f.pool.close().await;
}

// ---------------------------------------------------------------------------
// WP-MVP-C05：PostgreSQL 完整模式全文检索
// ---------------------------------------------------------------------------

/// `WP-MVP-C05` / `90` §5 搜索：`pg_jieba` 完整模式必须检索**正文**，
/// 且权限过滤与体验模式一字不差。
///
/// 这是体验模式与完整模式的分界线：体验模式搜不到正文是**已告知的限制**，
/// 完整模式搜不到正文就是缺陷。因此这里正面断言「只出现在正文里的词能被搜到」。
///
/// 更要紧的是第二组断言：全文检索**不得放宽任何一条权限过滤**。用检索绕过可见性
/// 是最隐蔽的越权——页面打不开，但搜索结果页把标题、版块和时间全列出来了。
///
/// 需要装有 `pg_jieba` 的实例（`supply-chain/postgres/Dockerfile` 构建的镜像）。
/// 扩展缺失时能力不会升级，用例明确跳过——跳过不是通过。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "需要装有 pg_jieba 的 PostgreSQL；见 docs/development/postgres.md"]
async fn postgres_full_text_search_finds_body_terms_and_keeps_permission_filters() {
    use content::search::{SearchBackend, SearchCapability, SearchQuery};

    let f = fixture(4).await;
    let forum = seeded_forum(&f).await;

    // 扩展探测：`pg_jieba` 未安装时不升级能力，也就没什么可测的。
    let capabilities = f.pool.capabilities().await;
    let Some(backend) = content::search::PostgresFullTextSearch::new(capabilities) else {
        eprintln!("跳过：该实例未安装 pg_jieba，检索能力未升级。跳过不等于通过。");
        return;
    };
    assert_eq!(backend.capability(), SearchCapability::FullText);
    let service = f
        .service
        .clone()
        .with_search_backend(std::sync::Arc::new(backend));
    assert_eq!(
        service.search_capability(),
        SearchCapability::FullText,
        "注入后服务必须报告完整模式"
    );

    let author = Credentials::new(f.users[0].to_string(), "csrf", new_operation_id());
    // 标题与正文各放一个独有词，且两者不重叠——这样命中来自哪一列是确定的。
    let title_term = "梧桐";
    let body_term = "曼陀罗";
    service
        .create_topic(
            &author,
            NewTopic {
                forum_id: forum,
                title: format!("{title_term}树下的公开主题"),
                markdown: format!("正文里只提到{body_term}这一个独有词，标题里没有它。"),
                tag_ids: Vec::new(),
                visibility: Visibility::Public,
            },
            at(),
        )
        .await
        .expect("建公开主题");

    let hits = |term: &str, session: Option<String>| {
        let service = service.clone();
        let term = term.to_owned();
        async move {
            let query = SearchQuery::build(&term, None, None, None, "", 50).expect("查询可构造");
            let (_, hits) = service
                .search(&query, session.as_deref(), at())
                .await
                .expect("检索不应失败");
            hits.len()
        }
    };

    // 1. 标题词与正文词都必须命中——正文可检索正是完整模式的定义。
    assert!(hits(title_term, None).await > 0, "标题词必须命中");
    assert!(
        hits(body_term, None).await > 0,
        "完整模式必须检索正文；搜不到说明分词或匹配没生效"
    );

    // 2. 权限过滤不得被检索绕过：仅成员可见的主题对匿名访客必须完全不出现，
    //    连标题都不能出现在结果里。
    let members_term = "紫檀";
    // 必须换一份凭据：幂等键随凭据走，复用同一份而载荷不同会被判为幂等冲突。
    // 这正是幂等保护在按预期工作，不是缺陷。
    let author = Credentials::new(f.users[0].to_string(), "csrf", new_operation_id());
    service
        .create_topic(
            &author,
            NewTopic {
                forum_id: forum,
                title: format!("{members_term}的仅成员主题"),
                markdown: "这条只对登录成员可见，匿名检索必须完全看不到它。".into(),
                tag_ids: Vec::new(),
                visibility: Visibility::Members,
            },
            at(),
        )
        .await
        .expect("建成员主题");
    assert_eq!(
        hits(members_term, None).await,
        0,
        "仅成员内容不得出现在匿名检索结果里——用搜索绕过可见性是最隐蔽的越权"
    );

    f.pool.close().await;
}
