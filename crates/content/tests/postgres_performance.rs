//! `WP-MVP-E01`／`E02` 的负载与延迟基线载体，对应 `90` 第 6 节固定性能指标表的
//! **延迟部分**。
//!
//! # 这份结果的效力边界
//!
//! `90` §6 表末写明：「测试报告必须说明地区、设备、缓存冷热、连接池、数据分布、
//! 测量窗口和资源规格。**规格未批准时只能形成实验结果，不能支持 Production
//! Approved**。」本载体把这些条件全部打印出来，但**是否采信为门禁基线，取决于
//! 运行环境的资源规格是否已获批准**——载体本身不能替用户做这个判断。
//!
//! 因此它断言的是「在本次运行的既定条件下，p95 是多少」，并在超出门槛时失败；
//! 通过不等于 `Production Approved`。
//!
//! 默认 `#[ignore]`。数据量由 `POSTGRES_PERF_TOPICS` 控制（默认 2000，
//! `90` §6 的 100 万基线需显式设置并预留时间），闸门与前置同
//! `docs/development/postgres.md`。
use chrono::DateTime;
use content::contract::*;
use content::core::ContentService;
use content::cursor::CursorKey;
use content::policy::ApprovedContentPolicy;
use platform::identifiers::SchemaRenderer;
use platform::transaction::new_operation_id;
use platform_db::Pool;
use platform_db::transaction::{DbTransaction, DbValue};
use std::sync::Arc;
use std::time::Instant;

mod support;
use support::{AllowAll, at, config, endpoint, migrations_root};

/// `90` §6 的延迟门槛。写成常量而不是散在断言里，方便与表格逐条对照。
const PUBLIC_READ_P95_MS: f64 = 400.0;
const SSR_FIRST_PAINT_P95_MS: f64 = 800.0;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// 分位数：对已排序样本取最近秩。样本量小时 p99 意义有限，因此一并打印样本数，
/// 让读报告的人自己判断这个 p99 值不值得信。
/// `search_documents` 的受信表名。分档回读要直接查这张表统计真实命中数，
/// 而 `ContentService::search` 受 `LIMIT 50` 限制，数不出总量。
fn r_docs(renderer: &SchemaRenderer, backend: platform::db::Backend) -> String {
    renderer
        .table(backend, "community", "search_documents")
        .expect("表名")
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

struct Stats {
    n: usize,
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

fn stats(mut samples: Vec<f64>) -> Stats {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("延迟样本不含 NaN"));
    Stats {
        n: samples.len(),
        p50: percentile(&samples, 0.50),
        p95: percentile(&samples, 0.95),
        p99: percentile(&samples, 0.99),
        max: samples.last().copied().unwrap_or(f64::NAN),
    }
}

/// 批量播种：以**迁移身份**直接写库，绕过领域服务。
///
/// 这是刻意的：基线要测的是读路径延迟，用领域服务逐条发帖会把播种时间拉到不可
/// 接受，而且顺带测的是写路径。播种数据仍严格满足读路径的全部过滤条件
/// （published / normal / 未删除 / public），否则测出来的是「查空表有多快」。
/// 播种并返回 `(连接池, 渲染器, 版块 id, search_documents 实际行数)`。
///
/// **返回实际行数而不是请求数**：本函数只做插入、不清空，同一前缀重复调用会累积。
/// 早先的运行正因如此把 21.2 万行的表报成「2,000 档」——档位标签一旦与真实行数
/// 脱节，整组延迟数字就失去意义。调用方必须用实际行数出报告。
async fn seed(prefix: &str, topics: usize) -> (Pool, SchemaRenderer, i64, i64) {
    let migrator = endpoint("POSTGRES_CONTRACT_MIGRATOR_URL", prefix);
    platform_db::migrations::migrate(&config(prefix, &migrator, 4), &migrations_root())
        .await
        .expect("迁移");
    let pool = Pool::connect_for(
        &config(prefix, &migrator, 4),
        platform_db::connection::Purpose::Migrator,
    )
    .await
    .expect("以迁移身份连接");
    let r = SchemaRenderer::new(prefix).expect("前缀");
    let now = at();

    // Feature Flag 由运维/迁移身份维护，运行角色只有 SELECT。读路径需要
    // `public_read`，不开的话列表一律 Forbidden，测出来的是拒绝路径的延迟。
    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    {
        let mut c = tx.connection();
        let flags = r
            .table(c.backend(), "platform", "feature_flags")
            .expect("表名");
        c.execute(
            &format!("UPDATE {flags} SET enabled = ?, version = version + 1 WHERE flag_key = ?"),
            &[DbValue::Bool(true), DbValue::Text("public_read".into())],
        )
        .await
        .expect("开 public_read");
    }
    tx.commit().await.expect("提交 Flag");

    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    let (author, forum) = {
        let mut c = tx.connection();
        let users = r.table(c.backend(), "identity", "users").expect("表名");
        let m = new_operation_id().replace('-', "");
        c.execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"),
            &[DbValue::Text(new_operation_id()), DbValue::Text(new_operation_id()), DbValue::Bool(true),
              DbValue::Text(format!("p{m}")), DbValue::Text(format!("p{m}")),
              DbValue::Text(format!("{m}@example.invalid")), DbValue::Text(format!("{m}@example.invalid")),
              DbValue::Bool(true), DbValue::Time(now), DbValue::Time(now)]).await.expect("建作者");
        let author = c.last_insert_id().await.expect("作者 ID");
        let forums = r.table(c.backend(), "community", "forums").expect("表名");
        c.execute(
            &format!(
                "INSERT INTO {forums} (slug, name, position, status, visibility, version) \
                 VALUES (?, '基线版块', 1, 'active', 'public', 1)"
            ),
            &[DbValue::Text(format!("perf-{}", new_operation_id()))],
        )
        .await
        .expect("建版块");
        let forum = c.last_insert_id().await.expect("版块 ID");
        (author, forum)
    };
    tx.commit().await.expect("提交");

    // 分批提交：单事务塞 100 万行会让 WAL 与内存都不好过，也让失败后无从续跑。
    const BATCH: usize = 500;
    let mut created = 0usize;
    while created < topics {
        let take = BATCH.min(topics - created);
        let mut tx = DbTransaction::begin(&pool).await.expect("事务");
        {
            let mut c = tx.connection();
            let t = r.table(c.backend(), "community", "topics").expect("表名");
            let p = r.table(c.backend(), "community", "posts").expect("表名");
            let docs = r
                .table(c.backend(), "community", "search_documents")
                .expect("表名");
            for i in 0..take {
                let n = created + i;
                let seq = (n + 1) as i64;
                let title = format!("基线主题 {n} 梧桐 与 曼陀罗");
                c.execute(&format!("INSERT INTO {t} (forum_id, author_id, title, publication, moderation, locked, visibility, deleted_at, archived_at, created_at, edited_at, create_seq, current_seq, last_reply_seq, next_floor, raw_reply_count, last_reply_author_id, last_reply_at, pinned, globally_pinned, featured, version) VALUES (?, ?, ?, 'published', 'normal', ?, 'public', ?, ?, ?, ?, ?, ?, ?, 2, 0, 0, ?, ?, ?, ?, 1)"),
                    &[DbValue::I64(forum), DbValue::I64(author), DbValue::Text(title.clone()),
                      DbValue::Bool(false), DbValue::Time(DateTime::UNIX_EPOCH), DbValue::Time(DateTime::UNIX_EPOCH),
                      DbValue::Time(now), DbValue::Time(DateTime::UNIX_EPOCH),
                      DbValue::I64(seq), DbValue::I64(seq), DbValue::I64(seq),
                      DbValue::Time(DateTime::UNIX_EPOCH), DbValue::Bool(false), DbValue::Bool(false), DbValue::Bool(false)]).await.expect("插主题");
                let topic_id = c.last_insert_id().await.expect("主题 ID");
                // 按可控命中率注入标记词。原来每篇都写「曼陀罗」，只能测出
                // 命中率 100% 的最坏边界——真实语料上一个词命中全部文档不会发生。
                // 这里额外按 10%／1%／0.1% 三档注入互不共字的词，让延迟能按命中率
                // 分档测量。选词避免共享汉字：jieba 会把「紫檀」「檀香」都切出
                // 「檀」，共字会让零命中那一档失真。
                let mut markers = String::new();
                if n.is_multiple_of(10) {
                    markers.push_str(" 蓝莓");
                }
                if n.is_multiple_of(100) {
                    markers.push_str(" 猕猴桃");
                }
                if n.is_multiple_of(1000) {
                    markers.push_str(" 山竹");
                }
                let body = format!("基线正文 {n}，含独有词 曼陀罗{markers}，用于检索路径测量。");
                c.execute(&format!("INSERT INTO {p} (topic_id, author_id, floor_no, body, quote_post_id, publication, moderation, deleted_at, created_at, edited_at, create_seq, version) VALUES (?, ?, 1, ?, 0, 'published', 'normal', ?, ?, ?, ?, 1)"),
                    &[DbValue::I64(topic_id), DbValue::I64(author), DbValue::Text(body.clone()),
                      DbValue::Time(DateTime::UNIX_EPOCH), DbValue::Time(now), DbValue::Time(DateTime::UNIX_EPOCH), DbValue::I64(seq)]).await.expect("插首帖");
                let post_id = c.last_insert_id().await.expect("帖子 ID");
                c.execute(&format!("INSERT INTO {docs} (post_id, topic_id, forum_id, title, title_normalized, body, visibility, forum_visibility, publication, moderation, deleted_at, entity_version, topic_version, create_seq) VALUES (?, ?, ?, ?, ?, ?, 'public', 'public', 'published', 'normal', ?, 1, 1, ?)"),
                    &[DbValue::I64(post_id), DbValue::I64(topic_id), DbValue::I64(forum),
                      DbValue::Text(title.clone()), DbValue::Text(title.to_lowercase()), DbValue::Text(body),
                      DbValue::Time(DateTime::UNIX_EPOCH), DbValue::I64(seq)]).await.expect("插检索文档");
            }
        }
        tx.commit().await.expect("提交批次");
        created += take;
    }
    pool.close().await;

    let app = Pool::connect(&config(
        prefix,
        &endpoint("POSTGRES_CONTRACT_URL", prefix),
        16,
    ))
    .await
    .expect("以运行身份连接");

    // 回读真实行数：请求数与表内实际行数只有在干净前缀上才相等。
    let mut tx = DbTransaction::begin(&app).await.expect("事务");
    let documents = {
        let mut c = tx.connection();
        let docs = r
            .table(c.backend(), "community", "search_documents")
            .expect("表名");
        c.fetch_all(&format!("SELECT COUNT(*) AS n FROM {docs}"), &[])
            .await
            .expect("回读行数")
            .first()
            .expect("计数应有一行")
            .i64("n")
            .expect("计数列")
    };
    tx.rollback().await.expect("回滚只读事务");

    (app, r, forum, documents)
}

/// 每个读路径单独测：并发 N 路，每路 rounds 次，汇总全部样本。
/// 不做通用测量闭包——async 闭包无法当函数指针传，硬凑只会让代码更难读。
async fn run<F, Fut>(concurrency: usize, rounds: usize, make: F) -> Stats
where
    F: Fn() -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let mut tasks = Vec::new();
    for _ in 0..concurrency {
        let make = make.clone();
        tasks.push(tokio::spawn(async move {
            let mut samples = Vec::with_capacity(rounds);
            for _ in 0..rounds {
                let t = Instant::now();
                make().await;
                samples.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            samples
        }));
    }
    let mut all = Vec::new();
    for t in tasks {
        all.extend(t.await.expect("任务不应 panic"));
    }
    stats(all)
}

/// `90` §6 延迟门槛：公开读 p95 ≤ 400ms、SSR 首屏 p95 ≤ 800ms。
///
/// 并发按 `90` §6 的「峰值在线 300 / 公开读 200 read/s 起点」取 16 路——本机
/// 不是生产规格，堆到 200 路只会测出连接池排队，那不是被测对象。报告里如实
/// 打印实际并发与数据量，读者据此判断这组数字的适用范围。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn public_read_latency_baseline() {
    let prefix = std::env::var("POSTGRES_CONTRACT_ALT_PREFIX")
        .unwrap_or_else(|_| panic!("需要 POSTGRES_CONTRACT_ALT_PREFIX"));
    let topics = env_usize("POSTGRES_PERF_TOPICS", 2_000);
    let concurrency = env_usize("POSTGRES_PERF_CONCURRENCY", 16);
    let rounds = env_usize("POSTGRES_PERF_ROUNDS", 20);

    let seed_started = Instant::now();
    let (pool, renderer, forum, documents) = seed(&prefix, topics).await;
    let seed_secs = seed_started.elapsed().as_secs_f64();

    let service = ContentService::new(
        pool.clone(),
        renderer.clone(),
        Arc::new(AllowAll::new(vec![1])),
        Arc::new(ApprovedContentPolicy::new().expect("已批准政策")),
        CursorKey::new(vec![7; 32]).expect("游标密钥"),
    );

    // 预热：第一次查询要建连接、填缓冲池，把它算进 p95 等于测冷启动而不是稳态。
    for _ in 0..concurrency {
        let _ = service
            .list(&ListFilter::default(), None, None, at())
            .await
            .expect("预热列表");
    }

    let s1 = service.clone();
    let list = (
        "公开列表（首页最新回复）",
        run(concurrency, rounds, move || {
            let s = s1.clone();
            async move {
                s.list(&ListFilter::default(), None, None, at())
                    .await
                    .expect("列表");
            }
        })
        .await,
    );
    let s2 = service.clone();
    let forum_list = (
        "版块列表",
        run(concurrency, rounds, move || {
            let s = s2.clone();
            async move {
                s.list(
                    &ListFilter {
                        forum_id: Some(forum),
                        ..ListFilter::default()
                    },
                    None,
                    None,
                    at(),
                )
                .await
                .expect("版块列表");
            }
        })
        .await,
    );

    println!("\n===== WP-MVP-E01/E02 公开读延迟基线 =====");
    // 打印**实际行数**而非请求数：seed 只插不清，同一前缀重复运行会累积。
    // 早先正因只打印请求数，把一张 21.2 万行的表报成了「2,000 档」，据此得出的
    // 「2,000 条就超门槛」结论也随之作废。
    println!("数据量：{documents} 篇检索文档（本次新增 {topics} 主题），播种耗时 {seed_secs:.1}s");
    println!("并发：{concurrency} 路 × 每路 {rounds} 次；连接池上限 16");
    println!("缓存：无应用层缓存（未注入 Redis）；数据库缓冲池已预热");
    println!("后端：PostgreSQL 16（运行身份 app_rw），本机回环");
    for (label, s) in [&list, &forum_list] {
        println!(
            "{label}: n={} p50={:.1}ms p95={:.1}ms p99={:.1}ms max={:.1}ms",
            s.n, s.p50, s.p95, s.p99, s.max
        );
    }
    println!(
        "门槛：公开读 p95 ≤ {PUBLIC_READ_P95_MS}ms、SSR 首屏 p95 ≤ {SSR_FIRST_PAINT_P95_MS}ms"
    );
    println!("效力：资源规格未获批准前，本结果只作实验记录，不支持 Production Approved。\n");

    for (label, s) in [&list, &forum_list] {
        assert!(
            s.p95 <= PUBLIC_READ_P95_MS,
            "{label} 的 p95 {:.1}ms 超出 90 §6 的 {PUBLIC_READ_P95_MS}ms 门槛",
            s.p95
        );
    }
    pool.close().await;
}

/// `90` §6 搜索读延迟基线。搜索读归「公开读 p95 ≤ 400ms」——`90` §6 没有为搜索
/// 单列门槛，因此这里对照同一条。
///
/// **测两个极端而不是一个中间值**：`seed` 给每篇文档都写了「梧桐」与「曼陀罗」，
/// 所以「曼陀罗」是**全量命中**（最坏召回：匹配集等于整个语料，排序与取前 N 的
/// 成本最高）；「紫檀」在语料里不存在，是**零命中**（只付索引查找的钱）。真实
/// 查询落在两者之间，报告同时给出两端，读者才能判断自己的场景靠近哪一侧。
///
/// 只在 `pg_jieba` 就位时有意义：缺扩展时检索能力不升级，测出来的是 `LIKE`
/// 降级路径而不是全文检索。因此缺扩展明确跳过——**跳过不是通过**。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要装有 pg_jieba 的隔离 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn search_latency_baseline() {
    use content::search::{SearchBackend, SearchCapability, SearchQuery};

    let prefix = std::env::var("POSTGRES_CONTRACT_ALT_PREFIX")
        .unwrap_or_else(|_| panic!("需要 POSTGRES_CONTRACT_ALT_PREFIX"));
    let topics = env_usize("POSTGRES_PERF_TOPICS", 2_000);
    let concurrency = env_usize("POSTGRES_PERF_CONCURRENCY", 16);
    let rounds = env_usize("POSTGRES_PERF_ROUNDS", 20);

    let seed_started = Instant::now();
    let (pool, renderer, _forum, documents) = seed(&prefix, topics).await;
    let seed_secs = seed_started.elapsed().as_secs_f64();

    let capabilities = pool.capabilities().await;
    let Some(backend) = content::search::PostgresFullTextSearch::new(capabilities) else {
        eprintln!(
            "跳过：该实例未安装 pg_jieba，检索能力未升级，测到的会是降级路径。跳过不等于通过。"
        );
        pool.close().await;
        return;
    };
    assert_eq!(backend.capability(), SearchCapability::FullText);

    let service = ContentService::new(
        pool.clone(),
        renderer.clone(),
        Arc::new(AllowAll::new(vec![1])),
        Arc::new(ApprovedContentPolicy::new().expect("已批准政策")),
        CursorKey::new(vec![7; 32]).expect("游标密钥"),
    )
    .with_search_backend(Arc::new(backend));
    assert_eq!(
        service.search_capability(),
        SearchCapability::FullText,
        "注入后服务必须报告完整模式，否则本用例测的不是全文检索"
    );

    let search = |service: ContentService, term: &str| {
        let term = term.to_owned();
        async move {
            let query = SearchQuery::build(&term, None, None, None, "", 50).expect("查询可构造");
            let (_, hits) = service.search(&query, None, at()).await.expect("检索");
            hits.len()
        }
    };

    // 预热：首次检索要建连接、载入词典并填缓冲池，算进 p95 等于测冷启动。
    // **每个词都要预热**——只热一个词时，其余各档的第一次查询仍要付冷启动的钱，
    // 那笔开销会落进某一档的尾部。实测零命中档曾因此出现 p99 4472ms 的孤立离群值。
    for term in ["紫檀", "山竹", "猕猴桃", "蓝莓", "曼陀罗"] {
        for _ in 0..concurrency {
            let _ = search(service.clone(), term).await;
        }
    }

    // 五档命中率，从 0% 到 100%。真实语料上一个词命中全部文档不会发生，
    // 只测 100% 得到的是最坏边界；只测 0% 得到的是最好边界。中间三档才是
    // 判断门槛能否达成的依据。
    //
    // 各档标记词在播种时按 `n % 10 / 100 / 1000` 注入，取词互不共字——
    // jieba 会把「紫檀」「檀香」都切出「檀」，共字会让零命中那一档失真。
    const RATES: &[(&str, &str, f64)] = &[
        ("零命中", "紫檀", 0.0),
        ("0.1% 命中", "山竹", 0.001),
        ("1% 命中", "猕猴桃", 0.01),
        ("10% 命中", "蓝莓", 0.10),
        ("全量命中", "曼陀罗", 1.0),
    ];

    // 防空：先回读每个标记词在库里的**真实**匹配数，与预期比例核对。播种若因
    // 分词、批次或累积而偏离，后面的延迟数字就是在测另一件事。
    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    let mut actual = Vec::new();
    for (label, term, expected) in RATES {
        let n = {
            let mut c = tx.connection();
            let docs = r_docs(&renderer, c.backend());
            c.fetch_all(
                &format!(
                    "SELECT COUNT(*) AS n FROM {docs} WHERE to_tsvector('public.jiebaqry', body) @@ plainto_tsquery('public.jiebacfg', ?)"
                ),
                &[DbValue::Text((*term).to_owned())],
            )
            .await
            .expect("回读命中数")
            .first()
            .expect("计数应有一行")
            .i64("n")
            .expect("计数列")
        };
        let rate = n as f64 / documents as f64;
        // 容差取预期的一半到两倍：批次边界与分词都可能让实际值略有出入，
        // 但数量级必须对得上，否则「1% 档」测的可能其实是 10%。
        if *expected > 0.0 {
            assert!(
                rate >= expected / 2.0 && rate <= expected * 2.0,
                "{label}（「{term}」）实际命中率 {:.4}（{n} / {documents}）偏离预期 {expected}，档位标签不成立",
                rate
            );
        } else {
            assert_eq!(
                n, 0,
                "{label}（「{term}」）应零命中，实际 {n} 条说明匹配过宽"
            );
        }
        actual.push((*label, *term, n, rate));
    }
    tx.rollback().await.expect("回滚只读事务");

    let mut results = Vec::new();
    for (label, term, _, _) in &actual {
        let s = service.clone();
        let term_owned = (*term).to_owned();
        results.push((
            *label,
            *term,
            run(concurrency, rounds, move || {
                let s = s.clone();
                let term = term_owned.clone();
                async move {
                    let query = SearchQuery::build(&term, None, None, None, "", 50).expect("查询");
                    s.search(&query, None, at()).await.expect("检索");
                }
            })
            .await,
        ));
    }

    println!("\n===== WP-MVP-E01/E02 搜索读延迟：按命中率分档 =====");
    println!("数据量：{documents} 篇检索文档（本次新增 {topics}），播种耗时 {seed_secs:.1}s");
    println!("并发：{concurrency} 路 × 每路 {rounds} 次；连接池上限 16");
    println!("检索：PostgreSQL 完整模式（pg_jieba，索引侧 jiebaqry / 查询侧 jiebacfg）");
    println!("缓存：无应用层缓存（未注入 Redis）；数据库缓冲池与词典已预热");
    println!("每页取前 50；命中率为回读实测值，非请求值");
    for ((label, term, n, rate), (_, _, s)) in actual.iter().zip(results.iter()) {
        println!(
            "{label}（「{term}」，实测 {n} 条 / {:.3}%）: p50={:.1}ms p95={:.1}ms p99={:.1}ms max={:.1}ms",
            rate * 100.0,
            s.p50,
            s.p95,
            s.p99,
            s.max
        );
    }
    println!("门槛：搜索读归公开读 p95 ≤ {PUBLIC_READ_P95_MS}ms");
    println!("效力：资源规格未获批准前，本结果只作实验记录，不支持 Production Approved。\n");

    // 逐档判定后一次性汇总：先断言第一档会让后面几档根本不输出，
    // 而「哪些档过了、哪些没过」正是本用例要回答的问题。
    let failed: Vec<String> = results
        .iter()
        .filter(|(_, _, s)| s.p95 > PUBLIC_READ_P95_MS)
        .map(|(label, term, s)| format!("{label}（「{term}」）p95 {:.1}ms", s.p95))
        .collect();
    assert!(
        failed.is_empty(),
        "以下档位超出 90 §6 的 {PUBLIC_READ_P95_MS}ms 门槛：{}",
        failed.join("；")
    );
    pool.close().await;
}
