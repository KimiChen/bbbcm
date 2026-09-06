//! `WP-MVP-E01` 并发矩阵的「举报合并」一项，对应 `90` 第 5 节「并发」。
//!
//! 默认 `#[ignore]`，只在提供隔离 PostgreSQL 端点时运行（见
//! `docs/development/postgres.md`）。端点缺失时**打印原因并跳过——跳过不是通过**。
//!
//! 为什么必须用真实 PostgreSQL：`submit_report` 的合并逻辑是先查「有没有未关闭
//! 的案件」再决定建案还是挂靠，典型的 read-modify-write。SQLite 单写者会把这条
//! 分支整个串行化，于是无论实现对错都恒绿。
use crate::cases::{ReportCategory, TargetKind, submit_report};
use chrono::{DateTime, Utc};
use platform::config::{Config, DatabaseConfig, SecretString};
use platform::identifiers::SchemaRenderer;
use platform::transaction::new_operation_id;
use platform_db::Pool;
use platform_db::transaction::{DbTransaction, DbValue};
use std::collections::BTreeSet;
use std::path::Path;

fn at() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-05T00:00:00.000000Z").unwrap()
}

fn endpoint(var: &str, prefix: &str) -> Option<String> {
    if std::env::var("POSTGRES_CONTRACT_ISOLATED").ok().as_deref() != Some("yes") {
        return None;
    }
    let raw = std::env::var(var).ok()?.replace("{prefix}", prefix);
    let parsed = url::Url::parse(&raw).expect("端点 URL 非法");
    assert!(
        parsed.host_str().is_some_and(|h| h
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())),
        "{var} 只允许字面回环 IP"
    );
    Some(raw)
}

fn config(prefix: &str, url: &str, max_connections: u32) -> DatabaseConfig {
    let mut cfg = Config::default().database;
    cfg.backend = platform::db::Backend::Postgres;
    cfg.schema_prefix = prefix.to_owned();
    cfg.postgres_url = Some(SecretString::new(url.to_owned()));
    cfg.max_connections = max_connections;
    cfg
}

struct Fixture {
    pool: Pool,
    renderer: SchemaRenderer,
    reporters: Vec<i64>,
}

/// 返回 `None` 表示没有端点，调用方应跳过。
async fn fixture(reporters: u32) -> Option<Fixture> {
    let prefix = std::env::var("POSTGRES_CONTRACT_ALT_PREFIX").ok()?;
    let migrator = endpoint("POSTGRES_CONTRACT_MIGRATOR_URL", &prefix)?;
    let app = endpoint("POSTGRES_CONTRACT_URL", &prefix)?;

    // 同一前缀可能被别的用例用过；先以迁移身份整套重建，保证初始状态确定。
    // 用裸 sqlx 是因为 `DbTransaction` 只允许 DML；`SET ROLE` 必需——登录角色
    // 是 NOINHERIT，不会自动获得 migrator 的属主权限。
    {
        use sqlx::Connection as _;
        let renderer = SchemaRenderer::new(&prefix).unwrap();
        let mut raw = sqlx::PgConnection::connect(&migrator).await.unwrap();
        sqlx::query(&format!("SET ROLE {}", renderer.role("migrator").unwrap()))
            .execute(&mut raw)
            .await
            .unwrap();
        for schema in platform::identifiers::LOGICAL_SCHEMAS {
            sqlx::query(&format!(
                "DROP SCHEMA IF EXISTS {} CASCADE",
                renderer.schema(schema).unwrap()
            ))
            .execute(&mut raw)
            .await
            .unwrap();
        }
        raw.close().await.unwrap();
    }
    platform_db::migrations::migrate(
        &config(&prefix, &migrator, 4),
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
    )
    .await
    .unwrap();

    // 每个举报人一条连接，再留一条给校验读；连接不够会让并发退化成排队。
    let pool = Pool::connect(&config(&prefix, &app, reporters + 1))
        .await
        .unwrap();
    let renderer = SchemaRenderer::new(&prefix).unwrap();
    let mut ids = Vec::new();
    let mut tx = DbTransaction::begin(&pool).await.unwrap();
    {
        let mut c = tx.connection();
        let users = renderer.table(c.backend(), "identity", "users").unwrap();
        for _ in 0..reporters {
            let m = new_operation_id().replace('-', "");
            c.execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"),
                &[DbValue::Text(new_operation_id()), DbValue::Text(new_operation_id()), DbValue::Bool(true),
                  DbValue::Text(format!("r{m}")), DbValue::Text(format!("r{m}")),
                  DbValue::Text(format!("{m}@example.invalid")), DbValue::Text(format!("{m}@example.invalid")),
                  DbValue::Bool(true), DbValue::Time(at()), DbValue::Time(at())]).await.unwrap();
            ids.push(c.last_insert_id().await.unwrap());
        }
    }
    tx.commit().await.unwrap();
    Some(Fixture {
        pool,
        renderer,
        reporters: ids,
    })
}

/// `90` §5 并发「举报合并」：同一目标的并发举报只能合并到一个案件。
///
/// 合并是审核工作量的关键：若 8 个人同时举报同一条内容却开出 8 个案件，
/// 审核队列会被同一件事刷屏，而且每个案件各自独立处置，结论可能互相矛盾。
///
/// 三条断言缺一不可：
/// 1. **只建一个案件**——`created_case` 为真的次数恰好一次；
/// 2. **全部挂到同一案件**——返回的 `case_id` 去重后只有一个；
/// 3. **数据库侧复核**——该目标在库里确实只有一行案件，返回值可能骗人，行数不会。
///
/// # 当前状态：因产品缺陷而失败，不是本测试的问题
///
/// 2026-09-07 首次执行：8 路并发在同一目标上开出了 **8 个案件**。根因是合并逻辑
/// 既没有序列化点，也没有唯一约束兜底：
///
/// - `case_repo::open_case` 是不带 `FOR UPDATE` 的普通 `SELECT`；
/// - `submit_report` 读到 `None` 就 `insert_case`，8 路同时读到 `None`；
/// - `moderation.case_target_idx`（`0014_reports_and_cases.sql:29`）是**普通索引**，
///   不是唯一索引，数据库不会挡下重复建案。
///
/// 影响面：同一条内容被多人同时举报时，审核队列会被同一件事刷屏，且每个案件
/// 各自独立处置，结论可能互相矛盾——这正是 `MOD-002` 合并语义要防的。
/// SQLite 侧看不到：单写者会把两次「读到 None」串行成「读到 None」+「读到已有」。
///
/// 修复触及事实源，须先取得用户批准：唯一偏索引受 `DEC-022` 三库交集子集限制
/// （MySQL 8.0 不支持偏索引），加锁则会改动 `02` 第 386 节冻结的锁序。
/// 因此本测试暂时保持失败并如实登记，不通过降低断言把它「修绿」。详见 `99` §5.23。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn concurrent_reports_on_one_target_merge_into_a_single_case() {
    const REPORTERS: u32 = 8;
    let Some(f) = fixture(REPORTERS).await else {
        eprintln!(
            "跳过：未提供隔离 PostgreSQL 端点（需要 POSTGRES_CONTRACT_ISOLATED / _URL / \
             _MIGRATOR_URL / _ALT_PREFIX）。跳过不等于通过。"
        );
        return;
    };
    // 目标 ID 只需为正：举报不校验目标是否存在，那属处置阶段。
    let target_id = 4242_i64;

    let mut tasks = Vec::new();
    for (i, reporter) in f.reporters.iter().copied().enumerate() {
        let pool = f.pool.clone();
        let renderer = f.renderer.clone();
        // 交替使用高危与普通类型：高危会触发 escalate 分支，让并发同时压到
        // 「建案」与「已有案件提优先级」两条路径上。
        let category = if i % 2 == 0 {
            ReportCategory::Illegal
        } else {
            ReportCategory::Spam
        };
        tasks.push(tokio::spawn(async move {
            for _attempt in 0..64 {
                let mut tx = DbTransaction::begin(&pool).await.expect("事务");
                let outcome = {
                    let mut c = tx.connection();
                    submit_report(
                        &mut c,
                        &renderer,
                        reporter,
                        TargetKind::Post,
                        target_id,
                        category,
                        "并发举报探针",
                        Some("合成证据正文"),
                        at(),
                    )
                    .await
                };
                match outcome {
                    Ok(outcome) => {
                        if tx.commit().await.is_ok() {
                            return Some(outcome);
                        }
                        // 提交失败按可重试处理：并发下两路同时建案，唯一约束会挡下一路。
                        tokio::task::yield_now().await;
                    }
                    Err(_) => {
                        let _ = tx.rollback().await;
                        tokio::task::yield_now().await;
                    }
                }
            }
            None
        }));
    }

    let mut cases = BTreeSet::new();
    let mut created = 0_usize;
    let mut done = 0_usize;
    for t in tasks {
        if let Some(outcome) = t.await.expect("任务不应 panic") {
            done += 1;
            created += usize::from(outcome.created_case);
            cases.insert(outcome.case_id);
        }
    }
    assert_eq!(done, REPORTERS as usize, "所有举报最终都应被受理");
    assert_eq!(created, 1, "同一目标只能建一个案件，实际建了 {created} 个");
    assert_eq!(cases.len(), 1, "所有举报必须挂到同一案件，实际 {cases:?}");

    // 数据库侧复核：返回值可能骗人，行数不会。
    let mut tx = DbTransaction::begin_read(&f.pool).await.expect("事务");
    let (case_rows, report_rows) = {
        let mut c = tx.connection();
        let cases_t = f
            .renderer
            .table(c.backend(), "moderation", "cases")
            .unwrap();
        let reports_t = f
            .renderer
            .table(c.backend(), "moderation", "reports")
            .unwrap();
        let n = c
            .fetch_optional(
                &format!("SELECT count(*) AS n FROM {cases_t} WHERE target_id = ?"),
                &[DbValue::I64(target_id)],
            )
            .await
            .unwrap()
            .unwrap()
            .i64("n")
            .unwrap();
        let m = c
            .fetch_optional(&format!("SELECT count(*) AS n FROM {reports_t}"), &[])
            .await
            .unwrap()
            .unwrap()
            .i64("n")
            .unwrap();
        (n, m)
    };
    tx.rollback().await.unwrap();
    assert_eq!(case_rows, 1, "库里该目标只应有一行案件");
    assert_eq!(
        report_rows, REPORTERS as i64,
        "每位举报人应各留下一条举报工单"
    );
}
