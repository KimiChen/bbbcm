//! `SOCIAL-002`／`SOCIAL-003` 正向反馈的隔离测试。
//!
//! 常规语义在 SQLite 上验；**互动唯一性的并发**必须在真实 PostgreSQL 上验
//! （`90` §5 并发清单第 5 项）——SQLite 单写者会把「两路同时插入」串行化，
//! 唯一约束那条分支永远走不到。
use crate::contract::*;
use crate::core::{self, FeedbackSubject};
use chrono::{DateTime, Utc};
use platform::config::Config;
use platform::identifiers::SchemaRenderer;
use platform::transaction::new_operation_id;
use platform_db::Pool;
use platform_db::transaction::{DbTransaction, DbValue};
use std::path::{Path, PathBuf};

fn at() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-05T00:00:00.000000Z").unwrap()
}

struct Fixture {
    dir: PathBuf,
    pool: Pool,
    r: SchemaRenderer,
    /// `users[0]` 是内容作者，其余是反馈者。
    users: Vec<i64>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    async fn new(count: usize) -> Self {
        let dir = std::env::temp_dir().join(format!("bbbcm-feedback-{}", new_operation_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default().database;
        cfg.sqlite_path = dir.join("feedback.sqlite3").to_string_lossy().into();
        cfg.schema_prefix = "fbtest_".into();
        platform_db::migrations::migrate(
            &cfg,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&cfg).await.unwrap();
        let r = SchemaRenderer::new(&cfg.schema_prefix).unwrap();
        let mut users = Vec::new();
        let mut tx = DbTransaction::begin(&pool).await.unwrap();
        {
            let mut c = tx.connection();
            let t = r.table(c.backend(), "identity", "users").unwrap();
            for _ in 0..count {
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
        Self {
            dir,
            pool,
            r,
            users,
        }
    }

    fn subject(&self) -> FeedbackSubject {
        FeedbackSubject {
            target: FeedbackTarget::Post,
            target_id: 4242,
            author_id: self.users[0],
        }
    }

    /// 在独立事务里跑一次操作并提交。
    async fn run<F, T>(&self, op: F) -> Result<T, InteractionError>
    where
        F: for<'a, 'c> FnOnce(
            &'a mut platform_db::transaction::TransactionConnection<'c>,
            &'a SchemaRenderer,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, InteractionError>> + Send + 'a>,
        >,
    {
        let mut tx = DbTransaction::begin(&self.pool).await.unwrap();
        let result = {
            let mut c = tx.connection();
            op(&mut c, &self.r).await
        };
        match result {
            Ok(v) => {
                tx.commit().await.unwrap();
                Ok(v)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }
}

/// `SOCIAL-003`：同一用户对同一对象只能产生一次有效反馈，且不能给自己反馈。
///
/// 幂等这一层单独断言：重复点击返回 `changed = false` 而不是错误，计数不涨。
/// 若重复点击报错，前端只能靠禁用按钮来回避，而按钮状态在多标签页下并不可靠。
#[tokio::test]
async fn feedback_is_once_per_user_idempotent_and_never_self() {
    let f = Fixture::new(2).await;
    let author = f.users[0];
    let reader = f.users[1];
    let s = f.subject();

    // 自我反馈：领域层给出明确错误，不是笼统冲突。
    assert_eq!(
        f.run(|c, r| Box::pin(core::give(c, r, author, s, at())))
            .await
            .unwrap_err(),
        InteractionError::SelfFeedback
    );

    let first = f
        .run(|c, r| Box::pin(core::give(c, r, reader, s, at())))
        .await
        .unwrap();
    assert_eq!((first.count, first.active, first.changed), (1, true, true));

    // 重复点击：幂等，不报错、不重复计数。
    let again = f
        .run(|c, r| Box::pin(core::give(c, r, reader, s, at())))
        .await
        .unwrap();
    assert_eq!((again.count, again.active, again.changed), (1, true, false));

    // 撤销后计数归零，但行仍在——重新反馈时唯一约束才挡得住第二行。
    let off = f
        .run(|c, r| Box::pin(core::withdraw(c, r, reader, s, at())))
        .await
        .unwrap();
    assert_eq!((off.count, off.active, off.changed), (0, false, true));

    // 重复撤销同样幂等。
    let off_again = f
        .run(|c, r| Box::pin(core::withdraw(c, r, reader, s, at())))
        .await
        .unwrap();
    assert!(!off_again.changed);

    // 重做：复用原行，不新增。
    let redo = f
        .run(|c, r| Box::pin(core::give(c, r, reader, s, at())))
        .await
        .unwrap();
    assert_eq!((redo.count, redo.active, redo.changed), (1, true, true));

    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let rows = {
        let mut c = tx.connection();
        let t = f.r.table(c.backend(), "interaction", "feedback").unwrap();
        c.fetch_all(&format!("SELECT id FROM {t}"), &[])
            .await
            .unwrap()
            .len()
    };
    tx.rollback().await.unwrap();
    assert_eq!(rows, 1, "撤销与重做必须复用同一行，不得产生第二行");
}

/// 数据库约束必须独立成立：即使领域层被绕过，坏数据也进不去。
///
/// 领域层的自我反馈校验会被新代码路径绕过，CHECK 不会。这条测试直接写库，
/// 就是为了证明这一点。
#[tokio::test]
async fn database_constraints_reject_self_feedback_and_duplicates() {
    let f = Fixture::new(2).await;
    let author = f.users[0];
    let reader = f.users[1];

    let insert = |user: i64, author: i64| {
        let pool = f.pool.clone();
        let r = f.r.clone();
        async move {
            let mut tx = DbTransaction::begin(&pool).await.unwrap();
            let out = {
                let mut c = tx.connection();
                let t = r.table(c.backend(), "interaction", "feedback").unwrap();
                c.execute(
                    &format!(
                        "INSERT INTO {t} (user_id, target_kind, target_id, author_id, withdrawn, \
                         created_at, updated_at, version) VALUES (?, 'post', 4242, ?, ?, ?, ?, 1)"
                    ),
                    &[
                        DbValue::I64(user),
                        DbValue::I64(author),
                        DbValue::Bool(false),
                        DbValue::Time(at()),
                        DbValue::Time(at()),
                    ],
                )
                .await
            };
            let ok = out.is_ok();
            if ok {
                tx.commit().await.unwrap();
            } else {
                let _ = tx.rollback().await;
            }
            ok
        }
    };

    assert!(!insert(author, author).await, "自我反馈必须被 CHECK 拒绝");
    assert!(insert(reader, author).await, "正常反馈应写入");
    assert!(
        !insert(reader, author).await,
        "同一用户对同一对象的第二行必须被 UNIQUE 拒绝"
    );
}

// ---------------------------------------------------------------------------
// NOTIFY-001／002／003 站内通知
// ---------------------------------------------------------------------------

/// `NOTIFY-003` 幂等 + `NOTIFY-002` 未读计数与已读。
///
/// 重复投递**不报错**这一点单独断言：重试是正常路径，报错会让 Worker 把成功的
/// 重试当成失败继续重试下去，形成一个永远消不掉的队列项。
#[tokio::test]
async fn notifications_are_idempotent_and_read_state_is_per_recipient() {
    use crate::notification::{self, Delivery, NotificationKind, SourceKind};

    let f = Fixture::new(3).await;
    let recipient = f.users[0];
    let other = f.users[1];
    let actor = f.users[2];

    let delivery = |key: &str, to: i64| Delivery {
        recipient_id: to,
        kind: NotificationKind::Reply,
        source: SourceKind::Post,
        source_id: 77,
        actor_id: actor,
        idempotency_key: key.to_owned(),
    };

    let first = f
        .run({
            let d = delivery("evt-1", recipient);
            move |c, r| Box::pin(async move { notification::deliver(c, r, &d, at()).await })
        })
        .await
        .unwrap();
    assert!(first.created);

    // 同键重投：不新建，也不报错。
    let retry = f
        .run({
            let d = delivery("evt-1", recipient);
            move |c, r| Box::pin(async move { notification::deliver(c, r, &d, at()).await })
        })
        .await
        .unwrap();
    assert!(!retry.created, "同幂等键不得产生第二条通知");

    // 不同键：正常新建。
    f.run({
        let d = delivery("evt-2", recipient);
        move |c, r| Box::pin(async move { notification::deliver(c, r, &d, at()).await })
    })
    .await
    .unwrap();

    let count = f
        .run(|c, r| Box::pin(notification::unread_count(c, r, recipient)))
        .await
        .unwrap();
    assert_eq!(count, 2, "两条不同事件应各计一次未读");

    // 取第一条的 ID，验证「单条已读」与越权边界。
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let id = {
        let mut c = tx.connection();
        let t =
            f.r.table(c.backend(), "interaction", "notifications")
                .unwrap();
        c.fetch_optional(
            &format!("SELECT id FROM {t} WHERE idempotency_key = ?"),
            &[DbValue::Text("evt-1".into())],
        )
        .await
        .unwrap()
        .unwrap()
        .i64("id")
        .unwrap()
    };
    tx.rollback().await.unwrap();

    // ID 替换越权：拿到别人的通知 ID 也不能把它标为已读。
    let stolen = f
        .run(|c, r| Box::pin(notification::mark_read(c, r, other, id, at())))
        .await
        .unwrap();
    assert!(!stolen, "他人不得标记不属于自己的通知");

    let ok = f
        .run(|c, r| Box::pin(notification::mark_read(c, r, recipient, id, at())))
        .await
        .unwrap();
    assert!(ok);
    // 重复标记：不报错、不重复计数。
    let again = f
        .run(|c, r| Box::pin(notification::mark_read(c, r, recipient, id, at())))
        .await
        .unwrap();
    assert!(!again, "重复标记应为无操作");

    assert_eq!(
        f.run(|c, r| Box::pin(notification::unread_count(c, r, recipient)))
            .await
            .unwrap(),
        1
    );

    let all = f
        .run(|c, r| Box::pin(notification::mark_all_read(c, r, recipient, at())))
        .await
        .unwrap();
    assert_eq!(all, 1, "全部已读只应影响剩下的那条");
    assert_eq!(
        f.run(|c, r| Box::pin(notification::unread_count(c, r, recipient)))
            .await
            .unwrap(),
        0
    );
}

/// `NOTIFY-001`：`adopted` 只预留类型，MVP 不得产生该类通知。
///
/// 单独一条而不是靠调用方自觉：一旦被误用，用户会收到一条指向尚不存在功能的
/// 通知，点进去是空的。另断言不给自己发通知。
#[tokio::test]
async fn reserved_kind_and_self_notification_are_rejected() {
    use crate::notification::{self, Delivery, NotificationKind, SourceKind};

    let f = Fixture::new(2).await;
    let recipient = f.users[0];
    let actor = f.users[1];

    let reserved = Delivery {
        recipient_id: recipient,
        kind: NotificationKind::Adopted,
        source: SourceKind::Post,
        source_id: 1,
        actor_id: actor,
        idempotency_key: "reserved".into(),
    };
    assert_eq!(
        f.run(move |c, r| Box::pin(
            async move { notification::deliver(c, r, &reserved, at()).await }
        ))
        .await
        .unwrap_err(),
        InteractionError::InvalidInput
    );

    let to_self = Delivery {
        recipient_id: recipient,
        kind: NotificationKind::Reply,
        source: SourceKind::Post,
        source_id: 1,
        actor_id: recipient,
        idempotency_key: "self".into(),
    };
    assert_eq!(
        f.run(move |c, r| Box::pin(
            async move { notification::deliver(c, r, &to_self, at()).await }
        ))
        .await
        .unwrap_err(),
        InteractionError::InvalidInput,
        "自己回复自己的主题不该产生「有人回复了你」"
    );
}
