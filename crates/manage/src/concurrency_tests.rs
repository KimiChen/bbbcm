//! `WP-MVP-E01` 并发矩阵的「治理命令」一项，对应 `90` 第 5 节「并发」。
//!
//! 默认 `#[ignore]`，只在提供隔离 PostgreSQL 端点时运行（见
//! `docs/development/postgres.md`）。端点缺失时**打印原因并跳过——跳过不是通过**。
//!
//! 高危操作的双人审批（`03` §7）是治理链路上最后一道闸。它的正确性完全取决于
//! 「同一条审批只能被批准一次」和「操作者不能给自己开绿灯」这两条在**并发下**
//! 依然成立。SQLite 单写者会把并发批准串行化，永远走不到竞态分支。
//!
//! # 当前状态：两个用例都卡在同一个产品缺陷上，不是测试的问题
//!
//! 2026-09-07 首次执行，两个用例都在**建审批**这一步就失败（`IamError::Unavailable`），
//! 数据库报 `permission denied for schema <prefix>iam`。根因是
//! `0016_iam.sql` 建了 `iam` schema 并逐表授了 `SELECT, INSERT, UPDATE`，
//! 却**没有授 `GRANT USAGE ON SCHEMA iam`**——而 0001／0002／0004／0005／0007
//! 每一个建 schema 的迁移都授了。PostgreSQL 里没有 schema USAGE，表级授权一律无效。
//!
//! 影响面：**完整模式下整个 `/manage` 不可用**——角色分配、撤销、审批申请与批准、
//! 以及十步授权的 `collect_facts` 事实读取，全部落在 `iam` schema 上。
//! SQLite 侧看不到：SQLite 没有 schema 级权限概念。
//!
//! 修复需要一条新的前向迁移（迁移不可改历史文件），而 N 轨编号必须跨 postgres／
//! sqlite 对齐，纯 ACL 修复在 SQLite 轨没有对应语句可写。待用户裁决，详见 `99` §5.23。
use crate::iam::{commands, repo};
use chrono::{DateTime, Duration, Utc};
use platform::config::{Config, DatabaseConfig, SecretString};
use platform::identifiers::SchemaRenderer;
use platform::transaction::new_operation_id;
use platform_db::Pool;
use platform_db::transaction::{DbTransaction, DbValue};
use std::path::Path;

/// 高危权限：账号级处置。选它是因为 registry 里它标了 `high_risk = true`，
/// 只有高危权限才允许（也才需要）走审批。
const HIGH_RISK: &str = "moderation.enforcement.account";

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
    /// `users[0]` 是操作者，其余是候选批准人。
    users: Vec<i64>,
}

async fn fixture(approvers: u32) -> Option<Fixture> {
    let prefix = std::env::var("POSTGRES_CONTRACT_ALT_PREFIX").ok()?;
    let migrator = endpoint("POSTGRES_CONTRACT_MIGRATOR_URL", &prefix)?;
    let app = endpoint("POSTGRES_CONTRACT_URL", &prefix)?;

    // 同一前缀可能被别的用例用过；先以迁移身份整套重建，保证初始状态确定。
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

    let pool = Pool::connect(&config(&prefix, &app, approvers + 2))
        .await
        .unwrap();
    let renderer = SchemaRenderer::new(&prefix).unwrap();
    let mut users = Vec::new();
    let mut tx = DbTransaction::begin(&pool).await.unwrap();
    {
        let mut c = tx.connection();
        let t = renderer.table(c.backend(), "identity", "users").unwrap();
        for _ in 0..=approvers {
            let m = new_operation_id().replace('-', "");
            c.execute(&format!("INSERT INTO {t} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"),
                &[DbValue::Text(new_operation_id()), DbValue::Text(new_operation_id()), DbValue::Bool(true),
                  DbValue::Text(format!("s{m}")), DbValue::Text(format!("s{m}")),
                  DbValue::Text(format!("{m}@example.invalid")), DbValue::Text(format!("{m}@example.invalid")),
                  DbValue::Bool(true), DbValue::Time(at()), DbValue::Time(at())]).await.unwrap();
            users.push(c.last_insert_id().await.unwrap());
        }
    }
    tx.commit().await.unwrap();
    Some(Fixture {
        pool,
        renderer,
        users,
    })
}

/// 新建一条待批准的高危审批，返回其 ID。
async fn request(f: &Fixture, operator: i64) -> i64 {
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let id = {
        let mut c = tx.connection();
        commands::request_approval(
            &mut c,
            &f.renderer,
            HIGH_RISK,
            operator,
            operator,
            "account",
            9001,
            Duration::hours(1),
            at(),
        )
        .await
        .expect("申请审批")
    };
    tx.commit().await.unwrap();
    id
}

/// `90` §7 并发「治理命令」：同一条高危审批被并发批准，只能生效一次。
///
/// 若两个批准人都「成功」，审计里记录的批准人就成了不确定值——事后无法回答
/// 「到底是谁放行的」，而这恰恰是双人审批存在的全部意义。
///
/// 断言三层：
/// 1. 恰好一路成功；
/// 2. 落库的 `approver_id` 正是成功那一路的人，不是随便某个候选；
/// 3. 状态确为 `approved` 且只被推进过一次（版本恰好 +1）。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn concurrent_approvals_take_effect_exactly_once() {
    const APPROVERS: u32 = 8;
    let Some(f) = fixture(APPROVERS).await else {
        eprintln!(
            "跳过：未提供隔离 PostgreSQL 端点（需要 POSTGRES_CONTRACT_ISOLATED / _URL / \
             _MIGRATOR_URL / _ALT_PREFIX）。跳过不等于通过。"
        );
        return;
    };
    let operator = f.users[0];
    let approval_id = request(&f, operator).await;

    let mut tasks = Vec::new();
    for approver in f.users[1..].iter().copied() {
        let pool = f.pool.clone();
        let renderer = f.renderer.clone();
        tasks.push(tokio::spawn(async move {
            let mut tx = DbTransaction::begin(&pool).await.expect("事务");
            let result = {
                let mut c = tx.connection();
                commands::approve(&mut c, &renderer, approval_id, approver, at()).await
            };
            match result {
                // 领域调用成功后仍要提交成功才算数：并发下 `execute_versioned`
                // 的乐观并发会让落后的一路在提交前就已失败。
                Ok(()) => tx.commit().await.ok().map(|()| approver),
                Err(_) => {
                    let _ = tx.rollback().await;
                    None
                }
            }
        }));
    }
    let mut winners = Vec::new();
    for t in tasks {
        if let Some(approver) = t.await.expect("任务不应 panic") {
            winners.push(approver);
        }
    }
    assert_eq!(
        winners.len(),
        1,
        "同一条审批只能被成功批准一次，实际 {} 次：{winners:?}",
        winners.len()
    );

    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let record = {
        let mut c = tx.connection();
        repo::approval(&mut c, &f.renderer, approval_id)
            .await
            .unwrap()
            .expect("审批应存在")
    };
    tx.rollback().await.unwrap();
    assert_eq!(record.state, "approved", "状态必须是 approved");
    assert_eq!(
        record.approver_id, winners[0],
        "落库的批准人必须正是成功那一路"
    );
    assert_eq!(record.version, 2, "版本必须恰好推进一次（1 -> 2）");
}

/// `03` §7 职责分离：操作者本人不得批准自己的申请，并发下也不行。
///
/// 单请求下这条由 `approve` 的显式判断保证。这里要验证的是并发不会打开缺口——
/// 例如「先通过版本检查再判身份」之类的顺序问题。因此让操作者本人 8 路同时批，
/// 必须**全部失败**，且审批状态一动不动。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn operator_cannot_self_approve_even_concurrently() {
    const ATTEMPTS: u32 = 8;
    let Some(f) = fixture(ATTEMPTS).await else {
        eprintln!("跳过：未提供隔离 PostgreSQL 端点。跳过不等于通过。");
        return;
    };
    let operator = f.users[0];
    let approval_id = request(&f, operator).await;

    let mut tasks = Vec::new();
    for _ in 0..ATTEMPTS {
        let pool = f.pool.clone();
        let renderer = f.renderer.clone();
        tasks.push(tokio::spawn(async move {
            let mut tx = DbTransaction::begin(&pool).await.expect("事务");
            let result = {
                let mut c = tx.connection();
                commands::approve(&mut c, &renderer, approval_id, operator, at()).await
            };
            match result {
                Ok(()) => tx.commit().await.is_ok(),
                Err(_) => {
                    // 失败路径回滚干净即可；这里只关心「有没有成功」。
                    let _ = tx.rollback().await;
                    false
                }
            }
        }));
    }
    let mut succeeded = 0_usize;
    for t in tasks {
        if t.await.expect("任务不应 panic") {
            succeeded += 1;
        }
    }
    assert_eq!(
        succeeded, 0,
        "操作者自批必须全部失败，实际成功 {succeeded} 次"
    );

    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let (record, satisfied) = {
        let mut c = tx.connection();
        let record = repo::approval(&mut c, &f.renderer, approval_id)
            .await
            .unwrap()
            .expect("审批应存在");
        // 更强的一层：即便有人绕过 `approve` 直接改了状态，这条查询也把
        // 「批准人 == 操作者」排除在 SQL 里，授权求值仍然不满足。
        let satisfied = repo::approval_satisfied(
            &mut c,
            &f.renderer,
            HIGH_RISK,
            operator,
            "account",
            9001,
            at(),
        )
        .await
        .unwrap();
        (record, satisfied)
    };
    tx.rollback().await.unwrap();
    assert_eq!(record.state, "pending", "自批失败后状态必须保持 pending");
    assert_eq!(record.version, 1, "自批失败不得推进版本");
    assert!(!satisfied, "自批不得让高危操作的审批条件被满足");
}
