use super::*;
use crate::idempotency_repo::{query_result, request_hash};
use crate::transaction::DbValue;
use crate::transaction::tests::Fixture;
use platform::db::Backend;
use platform::transaction::{OperationQuery, OperationStatus, new_operation_id};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn identity(key: &str) -> CommandIdentity {
    CommandIdentity {
        scope: "synthetic.change".into(),
        actor_id: 42,
        idempotency_key: key.into(),
        request_hash: request_hash(b"synthetic canonical request"),
        operation_id: new_operation_id(),
    }
}
fn response() -> StoredResponse {
    StoredResponse {
        status_code: 200,
        body: platform::outbox::SafePayload::empty()
            .with_reference("version", "1")
            .unwrap()
            .to_json()
            .unwrap(),
    }
}

#[tokio::test]
async fn same_payload_replays_original_result_different_payload_conflicts() {
    let f = Fixture::new().await;
    let command = identity("synthetic-key");
    let table = f.forums();
    let calls = Arc::new(AtomicUsize::new(0));
    let now = Utc::now();
    let result = execute_command(&f.pool, &f.renderer, &command, &LockPlan::default(), now, |conn| {
        let table = table.clone();
        let calls = calls.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            conn.execute(&format!("INSERT INTO {table} (slug, name, position, status, version) VALUES (?, ?, ?, ?, ?)"), &["synthetic".into(), "fixture".into(), 1_i64.into(), "draft".into(), 1_i64.into()]).await?;
            Ok(response())
        })
    }).await.unwrap();
    assert!(matches!(
        result,
        CommitOutcome::Committed(CommandResult {
            replayed: false,
            ..
        })
    ));
    let mut same = command.clone();
    same.operation_id = new_operation_id();
    let replay = execute_command(
        &f.pool,
        &f.renderer,
        &same,
        &LockPlan::default(),
        now,
        |_| Box::pin(async { panic!("不能再次执行业务") }),
    )
    .await
    .unwrap();
    assert!(
        matches!(replay, CommitOutcome::Committed(CommandResult { operation_id, replayed: true, .. }) if operation_id == command.operation_id)
    );
    let mut different = same;
    different.request_hash = request_hash(b"different");
    assert_eq!(
        execute_command(
            &f.pool,
            &f.renderer,
            &different,
            &LockPlan::default(),
            now,
            |_| Box::pin(async { panic!("异载荷不能执行业务") })
        )
        .await,
        Err(TransactionError::IdempotencyConflict)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.count("community", "forums").await, 1);
    assert_eq!(f.count("platform", "idempotency_records").await, 1);
}

#[tokio::test]
async fn business_failure_and_invalid_response_roll_back_reserved_key() {
    let f = Fixture::new().await;
    for invalid_body in [
        None,
        Some("invalid-json"),
        Some(r#"{"password":"secret"}"#),
        Some(r#"{"references":{"resource_id":"C3 text"},"outcome":null}"#),
    ] {
        let table = f.forums();
        let command = identity(if invalid_body.is_some() {
            "invalid"
        } else {
            "failure"
        });
        let result = execute_command(&f.pool, &f.renderer, &command, &LockPlan::default(), Utc::now(), |conn| {
            let table = table.clone();
            Box::pin(async move {
                conn.execute(&format!("INSERT INTO {table} (slug, name, position, status, version) VALUES (?, ?, ?, ?, ?)"), &["synthetic".into(), "fixture".into(), 1_i64.into(), "draft".into(), 1_i64.into()]).await?;
                if let Some(body) = invalid_body { Ok(StoredResponse { status_code: 200, body: body.into() }) }
                else { Err(TransactionError::VersionConflict) }
            })
        }).await;
        assert_eq!(
            result,
            Err(if invalid_body.is_some() {
                TransactionError::InvalidInput
            } else {
                TransactionError::VersionConflict
            })
        );
        assert_eq!(f.count("community", "forums").await, 0);
        assert_eq!(f.count("platform", "idempotency_records").await, 0);
    }
}

#[tokio::test]
async fn retry_success_commits_only_last_attempt_and_expiry_never_reexecutes() {
    let f = Fixture::new().await;
    let table = f.forums();
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let command = identity("recover-after-busy");
    let now = Utc::now();
    let result = execute_command(&f.pool, &f.renderer, &command, &LockPlan::default(), now, move |conn| {
        let table = table.clone();
        let called = called.clone();
        Box::pin(async move {
            conn.execute(&format!("INSERT INTO {table} (slug, name, position, status, version) VALUES (?, ?, ?, ?, ?)"), &["synthetic".into(), "fixture".into(), 1_i64.into(), "draft".into(), 1_i64.into()]).await?;
            if called.fetch_add(1, Ordering::SeqCst) < 2 { Err(TransactionError::Database(DbErrorClass::Busy)) }
            else { Ok(response()) }
        })
    }).await.unwrap();
    assert!(matches!(result, CommitOutcome::Committed(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(f.count("community", "forums").await, 1);
    assert_eq!(f.count("platform", "idempotency_records").await, 1);
    assert_eq!(
        execute_command(
            &f.pool,
            &f.renderer,
            &command,
            &LockPlan::default(),
            now + chrono::Duration::days(30),
            |_| Box::pin(async { panic!("响应过期不重放业务") })
        )
        .await,
        Err(TransactionError::ResponseExpired)
    );
    assert_eq!(f.count("community", "forums").await, 1);
}

#[tokio::test]
async fn command_applies_complete_lock_plan_and_rejects_stale_version_atomically() {
    let f = Fixture::new().await;
    let table = f.forums();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    tx.connection()
        .execute(
            &format!(
                "INSERT INTO {table} (slug, name, position, status, version) VALUES (?, ?, ?, ?, ?)"
            ),
            &[
                "synthetic".into(),
                "fixture".into(),
                1_i64.into(),
                "draft".into(),
                1_i64.into(),
            ],
        )
        .await
        .unwrap();
    let id = tx.connection().last_insert_id().await.unwrap();
    tx.commit().await.unwrap();
    let locks = LockPlan::new(vec![
        platform::transaction::LockTarget::new("community", "forums", id).unwrap(),
    ]);
    for stale in [false, true] {
        let command = identity(if stale { "stale" } else { "versioned" });
        let result = execute_command(&f.pool, &f.renderer, &command, &locks, Utc::now(), |conn| {
            let table = table.clone();
            Box::pin(async move {
                conn.execute_versioned(&format!("UPDATE {table} SET name = ?, version = version + 1 WHERE id = ? AND version = ?"), &["new-value".into(), id.into(), 1_i64.into()]).await?;
                Ok(response())
            })
        }).await;
        if stale {
            assert_eq!(result, Err(TransactionError::VersionConflict));
        } else {
            assert!(matches!(result, Ok(CommitOutcome::Committed(_))));
        }
    }
    assert_eq!(f.count("platform", "idempotency_records").await, 1);
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut conn = tx.connection();
    conn.lock(
        &f.renderer,
        &platform::transaction::LockTarget::new("community", "forums", id).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        idempotency_repo::reserve(
            &mut conn,
            &f.renderer,
            &identity("bad-lock-order"),
            Utc::now()
        )
        .await,
        Err(TransactionError::LockOrder)
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn unknown_commit_simulation_requires_original_context_and_never_replays() {
    let f = Fixture::new().await;
    for (mode, persisted) in [
        (CommitTestMode::LoseAcknowledgementAfterCommit, true),
        (CommitTestMode::LoseAcknowledgementAfterRollback, false),
    ] {
        let Pool::Sqlite(sqlite, _) = &f.pool else {
            panic!("SQLite fixture");
        };
        sqlx::query("CREATE TEMP TABLE connection_marker (id INTEGER)")
            .execute(sqlite)
            .await
            .unwrap();
        let command = identity(if persisted {
            "committed"
        } else {
            "rolled-back"
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let now = Utc::now();
        let result = execute_inner(
            &f.pool,
            &f.renderer,
            &command,
            &LockPlan::default(),
            now,
            move |_| {
                called.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(response()) })
            },
            mode,
        )
        .await
        .unwrap();
        assert_eq!(
            result,
            CommitOutcome::Unknown {
                operation_id: command.operation_id.clone()
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let marker_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_temp_master WHERE name = 'connection_marker'",
        )
        .fetch_one(sqlite)
        .await
        .unwrap();
        assert_eq!(
            marker_count, 0,
            "提交未知即使连接仍健康也必须丢弃，不能归池"
        );
        let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
        let mut conn = tx.connection();
        let query = OperationQuery::from(&command);
        assert_eq!(
            query_result(&mut conn, &f.renderer, &query, now)
                .await
                .unwrap(),
            if persisted {
                OperationStatus::Completed(response())
            } else {
                OperationStatus::NotObserved
            }
        );
        for altered in 0..4 {
            let mut wrong = query.clone();
            match altered {
                0 => wrong.actor_id += 1,
                1 => wrong.scope = "other.scope".into(),
                2 => wrong.idempotency_key = "other-key".into(),
                _ => wrong.operation_id = new_operation_id(),
            }
            assert_eq!(
                query_result(&mut conn, &f.renderer, &wrong, now)
                    .await
                    .unwrap(),
                OperationStatus::NotObserved
            );
        }
        if persisted {
            assert_eq!(
                query_result(
                    &mut conn,
                    &f.renderer,
                    &query,
                    now + chrono::Duration::days(30)
                )
                .await
                .unwrap(),
                OperationStatus::ResponseExpired
            );
        }
        tx.rollback().await.unwrap();
    }
}

#[tokio::test]
async fn retry_template_rolls_back_each_attempt_and_never_retries_unique_conflict() {
    let f = Fixture::new().await;
    for (class, expected_calls) in [
        (DbErrorClass::Busy, 4),
        (DbErrorClass::BusySnapshot, 4),
        (DbErrorClass::UniqueConflict, 1),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let command = identity("retry-budget");
        let result = execute_command(
            &f.pool,
            &f.renderer,
            &command,
            &LockPlan::default(),
            Utc::now(),
            move |_| {
                called.fetch_add(1, Ordering::SeqCst);
                // 合成错误验证重试控制；真实 SQLite BUSY 另由双连接 BEGIN 测试产生。
                Box::pin(async move { Err(TransactionError::Database(class)) })
            },
        )
        .await;
        assert_eq!(result, Err(TransactionError::Database(class)));
        assert_eq!(calls.load(Ordering::SeqCst), expected_calls);
        assert_eq!(f.count("platform", "idempotency_records").await, 0);
    }
}

/// `90` §5 PostgreSQL 集成「未知提交」：回执丢失后不得重放，且连接必须被丢弃。
///
/// 既有的 `unknown_commit_simulation_requires_original_context_and_never_replays`
/// 跑在 SQLite 上，用 `sqlite_temp_master` 间接验证连接被丢弃。那套手段在
/// PostgreSQL 上不存在，因此本用例用 `pg_backend_pid()`——**后端进程号变了，
/// 才证明旧连接真的没被放回池里复用**。
///
/// 为什么这条必须验：回执丢失时业务可能已经提交。若连接被放回池中复用，后续请求
/// 可能落在一个状态不明的会话上；若框架此时"贴心"地重放，就会把一次已提交的写
/// 变成两次。两者都比直接报 `Unknown` 更糟——`Unknown` 至少诚实。
///
/// 断言四层：结果为 `Unknown` 且带原操作 ID；业务闭包恰好执行一次（绝不重放）；
/// 提交模式下业务事实**在库里**、回滚模式下**不在**；两种模式下连接均被丢弃。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn postgres_unknown_commit_never_replays_and_discards_the_connection() {
    // 池上限取 1：这是让「连接被丢弃」可断言的前提，理由见 `new_postgres_with`。
    let Some(f) = Fixture::new_postgres_with(1).await else {
        eprintln!(
            "跳过：未提供隔离 PostgreSQL 端点（需要 POSTGRES_CONTRACT_ISOLATED / _URL / \
             _MIGRATOR_URL / _ALT_PREFIX）。跳过不等于通过。"
        );
        return;
    };
    let Pool::Postgres(pg, _) = &f.pool else {
        panic!("本用例只对 PostgreSQL 有意义");
    };
    // 反空转前置：连续取两次 PID，必须相同。
    //
    // 若连接池本来就每次换连接，那么「丢回执后 PID 变了」就什么都证明不了——
    // 断言会因为错误的原因通过。先确立「稳态下 PID 稳定」这个基线，后面的
    // 变化才具有含义。
    let pid_before: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(pg)
        .await
        .unwrap();
    let pid_again: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(pg)
        .await
        .unwrap();
    assert_eq!(
        pid_before, pid_again,
        "稳态下连接池应复用同一条连接；若这里就已不同，则后面的 PID 断言无意义"
    );

    let users = f
        .renderer
        .table(Backend::Postgres, "identity", "users")
        .unwrap();
    for (mode, persisted) in [
        (CommitTestMode::LoseAcknowledgementAfterCommit, true),
        (CommitTestMode::LoseAcknowledgementAfterRollback, false),
    ] {
        let marker = new_operation_id().replace('-', "");
        let command = identity(&format!("pg-{marker}"));
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let now = Utc::now();
        let insert = format!(
            "INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, \
             username_key, email, email_key, status, email_verified, bio, created_at, \
             username_changed_at, credential_version, version) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"
        );
        let seed = marker.clone();
        let result = execute_inner(
            &f.pool,
            &f.renderer,
            &command,
            &LockPlan::default(),
            now,
            move |tx| {
                called.fetch_add(1, Ordering::SeqCst);
                let insert = insert.clone();
                let m = seed.clone();
                Box::pin(async move {
                    tx.execute(
                        &insert,
                        &[
                            DbValue::Text(format!("pub-{m}")),
                            DbValue::Text(format!("anon-{m}")),
                            DbValue::Bool(true),
                            DbValue::Text(format!("u{m}")),
                            DbValue::Text(format!("u{m}")),
                            DbValue::Text(format!("{m}@example.invalid")),
                            DbValue::Text(format!("{m}@example.invalid")),
                            DbValue::Bool(true),
                            DbValue::Time(now),
                            DbValue::Time(now),
                        ],
                    )
                    .await?;
                    Ok(response())
                })
            },
            mode,
        )
        .await
        .unwrap();

        assert_eq!(
            result,
            CommitOutcome::Unknown {
                operation_id: command.operation_id.clone()
            },
            "丢回执必须诚实报 Unknown 并带回原操作 ID"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "业务闭包只能执行一次——重放会把一次已提交的写变成两次"
        );

        // 业务事实是否落库，取决于回执是在提交后还是回滚后丢的。
        let rows: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {users} WHERE public_id = $1"
        ))
        .bind(format!("pub-{marker}"))
        .fetch_one(pg)
        .await
        .unwrap();
        assert_eq!(
            rows,
            i64::from(persisted),
            "persisted={persisted} 时业务事实的落库状态不符"
        );
    }

    // 连接丢弃：两轮之后再取 PID，必然不是最初那条。
    let pid_after: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(pg)
        .await
        .unwrap();
    assert_ne!(
        pid_before, pid_after,
        "丢回执后的连接必须被丢弃，不能放回池中复用"
    );
    f.pool.close().await;
}
