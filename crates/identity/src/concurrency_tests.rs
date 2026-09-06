//! `WP-MVP-E01` 并发矩阵的 identity 三项，对应 `90` 第 5 节「并发」：
//! 邮箱验证、密码恢复、邀请码消费/额度。
//!
//! 全部默认 `#[ignore]`，只在提供隔离 PostgreSQL 端点时运行（见
//! `docs/development/postgres.md`）。没有端点时 `Fixture::new_postgres()` 返回
//! `None`，用例**明确跳过并打印原因**——跳过不是通过。
//!
//! 为什么这三项必须在真实 PostgreSQL 上验证：它们的正确性全都依赖「两个请求同时
//! 认为自己是第一个」这条分支。SQLite 是单写者，会把这条分支整个串行化掉，
//! 于是在 SQLite 上无论实现对错都恒绿。
use crate::contract::MutationResult;
use crate::core_tests::Fixture;
use std::sync::Arc;

/// 复用夹具的固定时间基准。**不能用 `Utc::now()`**：夹具的邀请码签发于
/// `at()`、两天后过期，用真实当前时间会让它已经过期，注册直接被 `Rejected`，
/// 于是测的就成了「过期校验」而不是并发消费。
use crate::core_tests::at;

/// 汇总并发结果：成功次数 + 去重后的失败原因。
///
/// 把失败原因带进断言消息，是为了让「只成功了 0 次」这种结果自带解释——
/// 否则每次失败都要回去翻数据库日志，而并发失败往往不易复现。
async fn tally<E: std::fmt::Debug>(
    tasks: Vec<tokio::task::JoinHandle<Result<MutationResult, E>>>,
) -> (usize, Vec<String>) {
    let mut ok = 0;
    let mut errors = std::collections::BTreeSet::new();
    for t in tasks {
        match t.await.expect("任务不应 panic") {
            Ok(_) => ok += 1,
            Err(e) => {
                errors.insert(format!("{e:?}"));
            }
        }
    }
    (ok, errors.into_iter().collect())
}

/// 端点缺失时统一的跳过提示：让「没跑」在输出里看得见，不被误读成通过。
macro_rules! fixture_or_skip {
    () => {
        match Fixture::new_postgres().await {
            Some(f) => f,
            None => {
                eprintln!(
                    "跳过：未提供隔离 PostgreSQL 端点（需要 POSTGRES_CONTRACT_ISOLATED / \
                     _URL / _MIGRATOR_URL / _ALT_PREFIX）。跳过不等于通过。"
                );
                return;
            }
        }
    };
}

/// `90` §5 并发「邮箱验证」：同一枚验证令牌被并发消费，只能成功一次。
///
/// 一次性令牌若能被消费两次，重放攻击就成立：拿到一次链接即可反复触发授级、
/// 反复重置状态。断言分两层——恰好一路成功，且数据库里令牌确实已被标记消费，
/// 只看返回值不足以排除「成功两次但第二次是幂等重放」的情况。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn concurrent_email_verification_consumes_the_token_once() {
    let f = fixture_or_skip!();
    f.enable().await;
    let c = f.context().await;
    f.service
        .register(&c, f.registration(), at())
        .await
        .expect("注册");
    let (token_id, raw) = f.token("verify_email").await;
    let service = Arc::new(f.service.clone());
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let service = Arc::clone(&service);
        let raw = raw.clone();
        // 每路自带独立的预认证上下文：共用一个会让失败可能来自 CSRF 而非并发。
        let ctx = f.context().await;
        tasks.push(tokio::spawn(async move {
            service.consume_verification(&ctx, &raw, at()).await
        }));
    }
    let (ok, errors) = tally(tasks).await;
    assert_eq!(
        ok, 1,
        "同一验证令牌只能被成功消费一次，实际 {ok} 次；失败原因：{errors:?}"
    );

    // 只看返回值不足以排除「第二次其实也写了、只是返回了幂等重放」。
    // 令牌行保留但 `consumed` 必须已置真——这是一次性语义的落库事实。
    let mut tx = platform_db::transaction::DbTransaction::begin_read(&f.pool)
        .await
        .unwrap();
    let consumed = {
        let mut c = tx.connection();
        let t =
            f.r.table(c.backend(), "identity", "one_time_tokens")
                .unwrap();
        c.fetch_optional(
            &format!("SELECT consumed FROM {t} WHERE token_id = ?"),
            &[platform_db::transaction::DbValue::Text(token_id)],
        )
        .await
        .unwrap()
        .expect("令牌行应仍在")
        .boolean("consumed")
        .unwrap()
    };
    tx.rollback().await.unwrap();
    assert!(consumed, "并发消费后 consumed 必须为真");
}

/// `90` §5 并发「密码恢复」：同一枚重置令牌被并发消费，只能成功一次。
///
/// 与邮箱验证同理，但后果更重：重置令牌若可重放，攻击者能在受害者改回密码后
/// 再次改走。这里额外断言并发结束后该令牌彻底不可再用——把「一次性」验到底。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn concurrent_password_reset_consumes_the_token_once() {
    let f = fixture_or_skip!();
    f.enable().await;
    let c = f.context().await;
    f.service
        .register(&c, f.registration(), at())
        .await
        .expect("注册");
    let c = f.context().await;
    f.service
        .request_reset(&c, "member@example.invalid", at())
        .await
        .expect("申请重置");
    let (_, raw) = f.token("reset_password").await;

    let service = Arc::new(f.service.clone());
    let mut tasks = Vec::new();
    for i in 0..8 {
        let service = Arc::clone(&service);
        let raw = raw.clone();
        let ctx = f.context().await;
        // 各路用不同新密码：若出现两次成功，最终密码是哪一个将不可预测，
        // 这本身就是必须避免的状态。
        let password = format!("Synthetic password {i}00");
        tasks.push(tokio::spawn(async move {
            service.reset_password(&ctx, &raw, &password, at()).await
        }));
    }
    let (ok, errors) = tally(tasks).await;
    assert_eq!(
        ok, 1,
        "同一重置令牌只能被成功消费一次，实际 {ok} 次；失败原因：{errors:?}"
    );

    // 并发结束后再单独试一次：必须仍然失败，证明令牌是真的一次性。
    let ctx = f.context().await;
    assert!(
        f.service
            .reset_password(&ctx, &raw, "Synthetic password zzz", at())
            .await
            .is_err(),
        "已消费的重置令牌不得再次生效"
    );
}

/// `90` §5 并发「邀请码消费/额度」：同一枚邀请码被并发注册消费，只能成功一次。
///
/// 邀请额度是准入闸门。若一枚邀请码能被并发用出两个账号，邀请制就形同虚设——
/// 而这恰恰是「先查后写」实现最容易漏掉的分支：八路同时查到「还没被用」。
/// 因此除了成功次数，还要数实际建了几个账号：返回值可能骗人，行数不会。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn concurrent_registrations_consume_one_invitation_once() {
    let f = fixture_or_skip!();
    f.enable().await;
    let before = f.count("identity", "users").await;

    let service = Arc::new(f.service.clone());
    let mut tasks = Vec::new();
    for i in 0..8 {
        let service = Arc::clone(&service);
        let ctx = f.context().await;
        // 用户名与邮箱各不相同：否则唯一约束会先一步拦下，测的就成了唯一索引
        // 而不是邀请额度。
        let mut input = f.registration();
        input.username = format!("member_{i}");
        input.email = format!("member-{i}@example.invalid");
        tasks.push(tokio::spawn(async move {
            service.register(&ctx, input, at()).await
        }));
    }
    let (ok, errors) = tally(tasks).await;
    assert_eq!(
        ok, 1,
        "一枚邀请码只能成功消费一次，实际 {ok} 次；失败原因：{errors:?}"
    );

    let after = f.count("identity", "users").await;
    assert_eq!(
        after - before,
        1,
        "并发注册只应留下一个新账号：before={before} after={after}"
    );
}
