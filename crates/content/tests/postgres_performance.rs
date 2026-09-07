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
async fn seed(prefix: &str, topics: usize) -> (Pool, SchemaRenderer, i64) {
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
                let body = format!("基线正文 {n}，含独有词 曼陀罗，用于检索路径测量。");
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
    (app, r, forum)
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
    let (pool, renderer, forum) = seed(&prefix, topics).await;
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

    // 每个读路径单独测：并发 N 路，每路 rounds 次，汇总全部样本。
    // 不做通用测量闭包——async 闭包无法当函数指针传，硬凑只会让代码更难读。
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
    println!("数据量：{topics} 主题（每主题 1 首帖 + 1 检索文档），播种耗时 {seed_secs:.1}s");
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
