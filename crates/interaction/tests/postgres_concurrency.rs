//! `WP-MVP-E01` 并发矩阵的「互动唯一性」一项，对应 `90` 第 5 节「并发」。
//!
//! 默认 `#[ignore]`，端点闸门与前置同 `docs/development/postgres.md`。
//! 端点缺失时**打印原因并跳过——跳过不是通过**。
//!
//! 为什么必须用真实 PostgreSQL：`SOCIAL-003` 的唯一性靠数据库 UNIQUE 兜底，
//! 而 SQLite 是单写者，「两路同时插入同一个 (user, target)」这条分支会被
//! 串行化掉，永远走不到——在那上面跑绿证明不了唯一约束真的挡住了并发。
use chrono::{DateTime, Utc};
use interaction::contract::{FeedbackTarget, InteractionError};
use interaction::core::{self, FeedbackSubject};
use platform::config::{Config, DatabaseConfig, SecretString};
use platform::identifiers::SchemaRenderer;
use platform::transaction::new_operation_id;
use platform_db::Pool;
use platform_db::transaction::{DbTransaction, DbValue};
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
    r: SchemaRenderer,
    users: Vec<i64>,
}

async fn fixture(readers: u32) -> Option<Fixture> {
    let prefix = std::env::var("POSTGRES_CONTRACT_ALT_PREFIX").ok()?;
    let migrator = endpoint("POSTGRES_CONTRACT_MIGRATOR_URL", &prefix)?;
    let app = endpoint("POSTGRES_CONTRACT_URL", &prefix)?;

    // 同一前缀可能被别的用例用过；以迁移身份整套重建，保证初始状态确定。
    {
        use sqlx::Connection as _;
        let r = SchemaRenderer::new(&prefix).unwrap();
        let mut raw = sqlx::PgConnection::connect(&migrator).await.unwrap();
        sqlx::query(&format!("SET ROLE {}", r.role("migrator").unwrap()))
            .execute(&mut raw)
            .await
            .unwrap();
        for schema in platform::identifiers::LOGICAL_SCHEMAS {
            sqlx::query(&format!(
                "DROP SCHEMA IF EXISTS {} CASCADE",
                r.schema(schema).unwrap()
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

    // 每个反馈者一条连接，另留一条给校验读；连接不够会把并发退化成排队。
    let pool = Pool::connect(&config(&prefix, &app, readers + 2))
        .await
        .unwrap();
    let r = SchemaRenderer::new(&prefix).unwrap();
    let mut users = Vec::new();
    let mut tx = DbTransaction::begin(&pool).await.unwrap();
    {
        let mut c = tx.connection();
        let t = r.table(c.backend(), "identity", "users").unwrap();
        for _ in 0..=readers {
            let m = new_operation_id().replace('-', "");
            c.execute(&format!("INSERT INTO {t} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"),
                &[DbValue::Text(new_operation_id()), DbValue::Text(new_operation_id()), DbValue::Bool(true),
                  DbValue::Text(format!("f{m}")), DbValue::Text(format!("f{m}")),
                  DbValue::Text(format!("{m}@example.invalid")), DbValue::Text(format!("{m}@example.invalid")),
                  DbValue::Bool(true), DbValue::Time(at()), DbValue::Time(at())]).await.unwrap();
            users.push(c.last_insert_id().await.unwrap());
        }
    }
    tx.commit().await.unwrap();
    Some(Fixture { pool, r, users })
}

/// `90` §5 并发「互动唯一性」：同一用户并发反馈同一对象，只能留下一行、只计一次。
///
/// 这是 `SOCIAL-003` 在并发下的真正考验。八路同时提交时，`core::give` 的
/// 「先查再插」都会读到「还没有」，唯一约束是唯一的拦截点。
///
/// 四层断言，缺一层都不足以证明正确：
/// 1. **每路都有确定结果**——要么成功、要么明确冲突，不允许静默失败；
/// 2. **恰好一路是真正的首写**（`changed = true`）；
/// 3. **库里只有一行**，返回值可能骗人，行数不会；
/// 4. **有效计数为 1**，即读路径看到的也是一次。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn concurrent_feedback_from_one_user_counts_once() {
    const READERS: u32 = 8;
    let Some(f) = fixture(READERS).await else {
        eprintln!(
            "跳过：未提供隔离 PostgreSQL 端点（需要 POSTGRES_CONTRACT_ISOLATED / _URL / \
             _MIGRATOR_URL / _ALT_PREFIX）。跳过不等于通过。"
        );
        return;
    };
    let author = f.users[0];
    let reader = f.users[1];
    let subject = FeedbackSubject {
        target: FeedbackTarget::Post,
        target_id: 4242,
        author_id: author,
    };

    let mut tasks = Vec::new();
    for _ in 0..READERS {
        let pool = f.pool.clone();
        let r = f.r.clone();
        tasks.push(tokio::spawn(async move {
            // 冲突后重试：并发下拿到 Conflict 是正常结果，客户端应重读后重试。
            // 不重试的话，测的就成了「有没有冲突」而不是「最终状态对不对」。
            for _ in 0..32 {
                let mut tx = DbTransaction::begin(&pool).await.expect("事务");
                let out = {
                    let mut c = tx.connection();
                    core::give(&mut c, &r, reader, subject, at()).await
                };
                match out {
                    Ok(o) => {
                        if tx.commit().await.is_ok() {
                            return Some(o.changed);
                        }
                        tokio::task::yield_now().await;
                    }
                    Err(InteractionError::Conflict) | Err(InteractionError::Unavailable) => {
                        let _ = tx.rollback().await;
                        tokio::task::yield_now().await;
                    }
                    Err(other) => panic!("非预期错误：{other:?}"),
                }
            }
            None
        }));
    }

    let mut done = 0usize;
    let mut first_writers = 0usize;
    for t in tasks {
        match t.await.expect("任务不应 panic") {
            Some(changed) => {
                done += 1;
                if changed {
                    first_writers += 1;
                }
            }
            None => panic!("有一路始终没能得到确定结果"),
        }
    }
    assert_eq!(done, READERS as usize, "每一路都应得到确定结果");
    assert_eq!(
        first_writers, 1,
        "只能有一路是真正的首写，其余必须观察到既有状态，实际 {first_writers} 路"
    );

    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let (rows, count) = {
        let mut c = tx.connection();
        let t = f.r.table(c.backend(), "interaction", "feedback").unwrap();
        let rows = c
            .fetch_all(
                &format!("SELECT id FROM {t} WHERE user_id = ? AND target_id = ?"),
                &[DbValue::I64(reader), DbValue::I64(subject.target_id)],
            )
            .await
            .unwrap()
            .len();
        let count = core::count(&mut c, &f.r, subject.target, subject.target_id)
            .await
            .unwrap();
        (rows, count)
    };
    tx.rollback().await.unwrap();
    assert_eq!(rows, 1, "并发反馈只能留下一行");
    assert_eq!(count, 1, "有效计数必须为 1");
    f.pool.close().await;
}
