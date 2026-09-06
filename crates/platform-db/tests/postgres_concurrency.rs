//! `WP-MVP-E01` 并发矩阵中**平台层**可覆盖的部分，对应 `90` 第 5 节「并发」。
//!
//! 与 `postgres_contract.rs` 同一套闸门与运行前置（见 `docs/development/postgres.md`），
//! 默认 `#[ignore]`。分成独立文件是因为 `90` 把并发列为独立矩阵，且这里的测试按设计
//! 会长时间持锁、开多连接，不该与合同测试抢同一批连接。
//!
//! **范围限定**：`90` §5 的并发清单共九项，本文件只覆盖 Outbox lease 一项——
//! 它是纯平台层的。邀请码消费、邮箱验证、密码恢复、主题楼层、举报合并、治理命令
//! 属领域 crate，而 `platform-db` 在依赖图下游，无法反向依赖它们；互动唯一性与
//! 通知去重所属的 `WP-MVP-C04` 尚未开始，功能不存在。这些缺口登记在 `99` §5.23，
//! 不因本文件存在而视作已覆盖。
use platform::config::{Config, DatabaseConfig, SecretString};
use platform::db::Backend;
use platform::identifiers::SchemaRenderer;
use platform::outbox::{Claim, OutboxEvent, SafePayload};
use platform::transaction::new_operation_id;
use platform_db::Pool;
use platform_db::transaction::{DbTransaction, DbValue};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn require_isolation() {
    assert_eq!(
        std::env::var("POSTGRES_CONTRACT_ISOLATED").ok().as_deref(),
        Some("yes"),
        "必须显式确认目标库是隔离且可丢弃的"
    );
}

fn endpoint(var: &str, prefix: &str) -> String {
    require_isolation();
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

async fn prepared(workers: u32) -> (Pool, SchemaRenderer) {
    let migrator = endpoint("POSTGRES_CONTRACT_MIGRATOR_URL", "bcm_");
    platform_db::migrations::migrate(&config("bcm_", &migrator, 4), &migrations_root())
        .await
        .expect("默认前缀迁移应成功");
    let app = endpoint("POSTGRES_CONTRACT_URL", "bcm_");
    // 每个 worker 一条连接，再留一条给校验读；连接不够会把并发退化成排队，
    // 那样测试就变成了顺序执行，什么也证明不了。
    let pool = Pool::connect(&config("bcm_", &app, workers + 1))
        .await
        .expect("以运行身份连接");
    (pool, SchemaRenderer::new("bcm_").expect("默认前缀"))
}

/// `90` §5 并发：Outbox lease 在多 Worker 争用下不得把同一事件租给两个持有者。
///
/// 这是「至少一次投递」能否收敛为「有效一次」的关键。若两个 Worker 同时拿到同一
/// 事件，下游就会收到重复投递；而 `claim` 依赖的是 `SELECT … FOR UPDATE SKIP LOCKED`
/// 语义——这在 SQLite 单写者下永远不会被真正检验，只有真实 PostgreSQL 的多连接
/// 才能压出来。
///
/// 断言分三层，缺一层都不足以证明正确：
/// 1. 每个事件**至多**被租一次（无重复投递）；
/// 2. 所有事件**都**被租到（`SKIP LOCKED` 不能把事件永久跳过而饿死）；
/// 3. 各 Worker 拿到的集合两两不相交（不是靠总数凑巧相等）。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn concurrent_outbox_claims_never_lease_one_event_twice() {
    const WORKERS: u32 = 8;
    const EVENTS: usize = 120;
    let (pool, renderer) = prepared(WORKERS).await;

    // 用一个本轮独有的 aggregate_id 打标，避免与其他测试或残留数据互相干扰。
    let batch_marker = new_operation_id();
    let mut expected = BTreeSet::new();
    let now = chrono::Utc::now();
    for index in 0..EVENTS {
        let event_id = new_operation_id();
        expected.insert(event_id.clone());
        let mut tx = DbTransaction::begin(&pool).await.expect("事务");
        {
            let mut c = tx.connection();
            let event = OutboxEvent {
                event_id: event_id.clone(),
                aggregate_type: "integration.concurrency".into(),
                // aggregate_id 必须是正整数或 UUID；这里用序号，marker 走 trace_id。
                aggregate_id: (index + 1).to_string(),
                event_type: "integration.concurrency.probe".into(),
                version: 1,
                occurred_at: now,
                payload: SafePayload::empty(),
                trace_id: batch_marker.clone(),
            };
            platform_db::outbox_repo::enqueue(&mut c, &renderer, &event, &["cache.invalidate"])
                .await
                .expect("入队");
        }
        tx.commit().await.expect("提交");
    }

    // 八路 Worker 同时反复 claim，直到再也拿不到本轮的事件为止。
    let empty_rounds = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for worker in 0..WORKERS {
        let pool = pool.clone();
        let renderer = renderer.clone();
        let marker = batch_marker.clone();
        let empty_rounds = Arc::clone(&empty_rounds);
        tasks.push(tokio::spawn(async move {
            let owner = format!("worker-{worker}");
            let mut mine: Vec<String> = Vec::new();
            loop {
                let mut tx = DbTransaction::begin(&pool).await.expect("事务");
                let claimed = {
                    let mut c = tx.connection();
                    platform_db::outbox_repo::claim(
                        &mut c,
                        &renderer,
                        &Claim {
                            owner: owner.clone(),
                            batch_id: new_operation_id(),
                            now: chrono::Utc::now(),
                            // 租约足够长：本测试要验证「不重复租」，不是验证过期续租。
                            lease_until: chrono::Utc::now() + chrono::Duration::minutes(10),
                            limit: 5,
                        },
                    )
                    .await
                    .expect("claim 不应失败")
                };
                tx.commit().await.expect("提交租约");
                let batch: Vec<String> = claimed
                    .into_iter()
                    .filter(|e| e.event.trace_id == marker)
                    .map(|e| e.event.event_id)
                    .collect();
                if batch.is_empty() {
                    // 连续空轮才退出：单次空可能只是恰好被别人抢光了这一批。
                    if empty_rounds.fetch_add(1, Ordering::SeqCst) >= WORKERS as usize * 3 {
                        break;
                    }
                    tokio::task::yield_now().await;
                    continue;
                }
                empty_rounds.store(0, Ordering::SeqCst);
                mine.extend(batch);
            }
            (owner, mine)
        }));
    }

    let mut all: Vec<String> = Vec::new();
    let mut per_worker: Vec<(String, BTreeSet<String>)> = Vec::new();
    for task in tasks {
        let (owner, mine) = task.await.expect("Worker 不应 panic");
        all.extend(mine.iter().cloned());
        per_worker.push((owner, mine.into_iter().collect()));
    }

    // 0. 反空转：必须真的发生了争用。若某个 Worker 独吞全部事件，上面三条断言
    //    照样成立——不相交、无重复、无饿死——但测试其实退化成了单线程，什么并发
    //    性质都没验证。所以先要求至少两个 Worker 各自领到过事件。
    let active: Vec<&(String, BTreeSet<String>)> = per_worker
        .iter()
        .filter(|(_, set)| !set.is_empty())
        .collect();
    let distribution: Vec<(&str, usize)> = per_worker
        .iter()
        .map(|(owner, set)| (owner.as_str(), set.len()))
        .collect();
    assert!(
        active.len() >= 2,
        "只有 {} 个 Worker 领到事件，未构成争用，本测试无效；分布：{distribution:?}",
        active.len()
    );
    println!("领取分布：{distribution:?}");

    // 1. 至多一次：去重后数量必须与原始数量相同，否则存在重复租约。
    let unique: BTreeSet<String> = all.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        all.len(),
        "同一事件被租了多次：总领取 {} 条、去重后 {} 条",
        all.len(),
        unique.len()
    );

    // 2. 不饿死：本轮入队的事件必须全部被领走。
    assert_eq!(
        unique, expected,
        "有事件从未被领取，SKIP LOCKED 不得让事件永久饿死"
    );

    // 3. 两两不相交：逐对求交集，避免只靠总数相等蒙混过关。
    for i in 0..per_worker.len() {
        for j in (i + 1)..per_worker.len() {
            let overlap: Vec<&String> = per_worker[i].1.intersection(&per_worker[j].1).collect();
            assert!(
                overlap.is_empty(),
                "{} 与 {} 领到了相同事件：{:?}",
                per_worker[i].0,
                per_worker[j].0,
                overlap
            );
        }
    }

    // 数据库侧复核：每个事件的持有者唯一，且状态确实是 leased。
    let events = renderer
        .table(Backend::Postgres, "platform", "outbox_events")
        .expect("表名");
    let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
    let rows = {
        let mut c = tx.connection();
        c.fetch_all(
            &format!("SELECT state, count(*) AS n FROM {events} WHERE trace_id = ? GROUP BY state"),
            &[DbValue::Text(batch_marker.clone())],
        )
        .await
        .expect("查询状态分布")
    };
    tx.rollback().await.expect("回滚");
    let mut leased = 0_i64;
    for row in &rows {
        let state = row.text("state").expect("state");
        let n = row.i64("n").expect("n");
        assert_eq!(
            state, "leased",
            "本轮事件只应处于 leased 状态，实际有 {state}"
        );
        leased += n;
    }
    assert_eq!(leased, EVENTS as i64, "leased 事件数必须等于入队数");
    pool.close().await;
}
