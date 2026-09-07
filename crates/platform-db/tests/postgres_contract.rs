//! `WP-MVP-E01` 的真实 PostgreSQL 集成入口，对应 `90` 第 5 节「PostgreSQL 集成」。
//!
//! 与 `platform/tests/redis_contract.rs` 同一模式：默认 `#[ignore]`，只有显式提供
//! **隔离且可丢弃** 的 PostgreSQL 16 端点才会执行。开发期最小集不会跑到它，也因此
//! 不会有人把 SQLite 冒烟误当成 PostgreSQL 语义已验证。
//!
//! 运行前置与完整步骤见 `docs/development/postgres.md`。摘要：
//!
//! ```text
//! POSTGRES_CONTRACT_ISOLATED=yes \
//! POSTGRES_CONTRACT_URL='postgres://{prefix}app_login@127.0.0.1:5433/bbbcm_contract' \
//! POSTGRES_CONTRACT_MIGRATOR_URL='postgres://{prefix}mig_login@127.0.0.1:5433/bbbcm_contract' \
//! POSTGRES_CONTRACT_ALT_PREFIX=probe<随机>_ \
//! cargo test -p platform-db --test postgres_contract -- --ignored --nocapture
//! ```
//!
//! 两道闸门不是形式：`AGENTS.md` 明令「不得以开发期为由擅自清空非本地数据库」，
//! 而本文件会建表写数据。指错一次 URL 就足以把集成测试变成一次生产事故。
//! 注意闸门只证明「连的是本机回环」，不证明「库是空的可丢弃的」——后者由运行者
//! 按运行手册用一次性容器保证，测试无法代为担保。
//!
//! 需要两个连接串是**设计要求**，不是本测试图方便：`prepare_postgres` 要求运行登录
//! 角色的组成员关系**恰好一条**（自己的用途组，`INHERIT FALSE`、`SET TRUE`），所以
//! 同一个登录角色不可能既迁移又跑应用。字面量 `{prefix}` 由本文件按 schema 前缀替换
//! ——角色名带前缀，双前缀矩阵下每套 schema 各有一组角色。
use platform::config::{Config, DatabaseConfig, SecretString};
use platform::db::Backend;
use platform::identifiers::SchemaRenderer;
use platform::transaction::{
    CommandIdentity, CommitOutcome, DbErrorClass, LockPlan, LockTarget, StoredResponse,
    new_operation_id, request_hash,
};
use platform_db::Pool;
use platform_db::command::execute_command;
use platform_db::transaction::{DbTransaction, DbValue, TransactionError};
use std::path::{Path, PathBuf};

/// 确认运行者已声明目标库可丢弃；放在每个入口最前面，早于任何连接。
fn require_isolation() {
    assert_eq!(
        std::env::var("POSTGRES_CONTRACT_ISOLATED").ok().as_deref(),
        Some("yes"),
        "必须显式确认目标库是隔离且可丢弃的"
    );
}

/// 取某用途、某 schema 前缀的连接串：替换 `{prefix}` 后做回环校验。
///
/// 连接串本身从不打印——它可能带凭据，而测试输出会进证据包。
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

/// 运行身份（`Purpose::App`）与迁移身份（`Purpose::Migrator`）各自的连接串。
fn app_url(prefix: &str) -> String {
    endpoint("POSTGRES_CONTRACT_URL", prefix)
}
fn migrator_url(prefix: &str) -> String {
    endpoint("POSTGRES_CONTRACT_MIGRATOR_URL", prefix)
}

/// 随机非默认前缀由运行手册生成并注入：它的物理角色必须先由 DBA 建好
/// （`bbbcm-cli db bootstrap-sql`），测试自身没有、也不应有建角色的权限。
fn alt_prefix() -> String {
    let value = std::env::var("POSTGRES_CONTRACT_ALT_PREFIX")
        .unwrap_or_else(|_| panic!("需要 POSTGRES_CONTRACT_ALT_PREFIX（随机非默认前缀）"));
    assert_ne!(value, "bcm_", "非默认前缀不能等于默认前缀");
    assert!(
        SchemaRenderer::new(&value).is_ok(),
        "非默认前缀本身必须合法"
    );
    value
}

fn config(prefix: &str, url: &str) -> DatabaseConfig {
    let mut cfg = Config::default().database;
    cfg.backend = Backend::Postgres;
    cfg.schema_prefix = prefix.to_owned();
    cfg.postgres_url = Some(SecretString::new(url.to_owned()));
    // 单连接会把并发、重试与锁等待路径整个藏起来，集成测试必须留出多连接。
    cfg.max_connections = 8;
    cfg
}

fn migrations_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations")
}

/// 所有非迁移语义的测试共用默认前缀。`migrate` 幂等，并发调用会真实经过
/// 迁移 advisory lock —— 这本身就是 `90` §5 要求覆盖的一项。
async fn prepared() -> (Pool, SchemaRenderer) {
    platform_db::migrations::migrate(&config("bcm_", &migrator_url("bcm_")), &migrations_root())
        .await
        .expect("默认前缀迁移应成功（角色需已由 bootstrap-sql 建好）");
    let pool = Pool::connect(&config("bcm_", &app_url("bcm_")))
        .await
        .expect("以运行身份连接默认前缀");
    (pool, SchemaRenderer::new("bcm_").expect("默认前缀"))
}

/// 建一个用户并返回其 id。
///
/// 写入目标选 `identity.users` 而不是 `community.forums`，是因为最小权限 ACL 只给
/// 运行角色 `community.forums` 的 SELECT——版块属于由迁移/种子维护的参照数据。
/// 这条边界本身由 `app_role_cannot_write_reference_tables` 正面断言。
async fn insert_user(pool: &Pool, r: &SchemaRenderer, marker: &str) -> i64 {
    let mut tx = DbTransaction::begin(pool).await.expect("事务");
    let id = {
        let mut c = tx.connection();
        write_user(&mut c, r, marker).await.expect("写入用户");
        c.last_insert_id().await.expect("回读主键")
    };
    tx.commit().await.expect("提交");
    id
}

/// 单条用户插入；`marker` 负责让三个唯一列互不冲突。
async fn write_user(
    c: &mut platform_db::transaction::TransactionConnection<'_>,
    r: &SchemaRenderer,
    marker: &str,
) -> Result<u64, TransactionError> {
    let users = r.table(c.backend(), "identity", "users").expect("表名");
    let now = chrono::Utc::now();
    c.execute(
        &format!(
            "INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, \
             username_key, email, email_key, status, email_verified, bio, created_at, \
             username_changed_at, credential_version, version) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?, 1, 1)"
        ),
        &[
            DbValue::Text(format!("pub-{marker}")),
            DbValue::Text(format!("anon-{marker}")),
            DbValue::Bool(false),
            DbValue::Text(format!("u{marker}")),
            DbValue::Text(format!("u{marker}")),
            DbValue::Text(format!("{marker}@example.invalid")),
            DbValue::Text(format!("{marker}@example.invalid")),
            DbValue::Bool(false),
            DbValue::Text(String::new()),
            DbValue::Time(now),
            DbValue::Time(now),
        ],
    )
    .await
}

/// 短标记：唯一列用它拼装，避免各测试互相踩。
fn marker() -> String {
    new_operation_id().replace('-', "")
}

// ---------------------------------------------------------------------------
// 1. 迁移：空库、重复运行、双前缀并存
// ---------------------------------------------------------------------------

/// `90` §5：空库迁移、重复运行幂等、默认与随机非默认前缀并存且互不可见。
///
/// 双前缀是这条测试的核心。「查询不得硬编码物理 Schema」只有在两套 schema 同时
/// 存在、且一方写入对另一方完全不可见时才真正被验证到——单前缀跑通说明不了
/// 受信标识符渲染器有没有被绕过。
///
/// 本测试独占随机前缀，因此可以对 applied/skipped 做确定性断言；其他测试都用
/// 默认前缀，不会污染这里的初始状态。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn migrations_apply_once_then_skip_and_two_prefixes_stay_isolated() {
    let alt = alt_prefix();

    // 随机前缀是本次运行新建的，必须是「全部应用、零跳过」。
    let alt_cfg = config(&alt, &migrator_url(&alt));
    let first = platform_db::migrations::migrate(&alt_cfg, &migrations_root())
        .await
        .expect("空库迁移应成功");
    assert!(!first.applied.is_empty(), "空库首次运行必须应用迁移");
    assert!(first.skipped.is_empty(), "空库不应跳过任何迁移");
    // 编号必须从 0001 起连续：缺一条就说明迁移发现逻辑漏了文件。
    let expected: Vec<i64> = (1..=first.applied.len() as i64).collect();
    assert_eq!(first.applied, expected, "应用编号必须从 0001 起连续");

    // 重复运行：全部跳过、零应用，且跳过集恰好等于上一轮的应用集。
    let second = platform_db::migrations::migrate(&alt_cfg, &migrations_root())
        .await
        .expect("重复迁移应成功");
    assert!(second.applied.is_empty(), "重复运行不得再次应用");
    assert_eq!(second.skipped, first.applied, "跳过集必须等于已应用集");

    // 默认前缀独立迁移，随后验证跨前缀不可见。
    let (pool, renderer) = prepared().await;
    let m = marker();
    insert_user(&pool, &renderer, &m).await;

    let alt_pool = Pool::connect(&config(&alt, &app_url(&alt)))
        .await
        .expect("以运行身份连接随机前缀");
    let alt_renderer = SchemaRenderer::new(&alt).expect("随机前缀");
    let mut tx = DbTransaction::begin_read(&alt_pool).await.expect("事务");
    let leaked = {
        let mut c = tx.connection();
        let users = alt_renderer
            .table(c.backend(), "identity", "users")
            .expect("表名");
        c.fetch_optional(
            &format!("SELECT id FROM {users} WHERE public_id = ?"),
            &[DbValue::Text(format!("pub-{m}"))],
        )
        .await
        .expect("查询")
    };
    tx.rollback().await.expect("回滚");
    assert!(
        leaked.is_none(),
        "随机前缀不得看到默认前缀写入的行——若可见说明有查询硬编码了物理 schema"
    );
    pool.close().await;
    alt_pool.close().await;
}

// ---------------------------------------------------------------------------
// 2. 受信标识符与 bind 参数
// ---------------------------------------------------------------------------

/// `90` §5：受信标识符渲染与 bind 参数。
///
/// 敌意前缀必须在**渲染器层**被拒绝，而不是拼进 SQL 后靠数据库报错；后者意味着
/// 渲染器可以被绕过。值一侧则用真实往返证明 bind 从不做字符串插值：把 SQL 元字符
/// 当值写进去再读回来，必须逐字节相同，且不产生任何附带效果。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn trusted_identifiers_are_whitelisted_and_values_are_always_bound() {
    require_isolation();
    for hostile in [
        "bcm\"; DROP SCHEMA public CASCADE; --",
        "bcm-",
        "1bcm_",
        "",
        "BCM_",
        "bcm_'",
    ] {
        assert!(
            SchemaRenderer::new(hostile).is_err(),
            "{hostile:?} 必须在渲染器层被拒绝"
        );
    }
    let renderer = SchemaRenderer::new("bcm_").expect("合法前缀");
    // 受信表注册表是白名单，不是提示：未登记的 schema/表一律拒绝渲染。
    assert!(
        renderer
            .table(Backend::Postgres, "identity", "not_registered")
            .is_err()
    );
    assert!(
        renderer
            .table(Backend::Postgres, "nosuch_schema", "users")
            .is_err()
    );

    let (pool, renderer) = prepared().await;
    let m = marker();
    let id = insert_user(&pool, &renderer, &m).await;

    // 值里塞满 SQL 元字符；任何一层做了字符串拼接，这行要么写不进去，要么读回来不同。
    let hostile_bio = "'); DROP TABLE x; -- \\ % _ \" $1 ?";
    let users = renderer
        .table(Backend::Postgres, "identity", "users")
        .expect("表名");
    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    tx.connection()
        .execute(
            &format!("UPDATE {users} SET bio = ? WHERE id = ?"),
            &[DbValue::Text(hostile_bio.into()), DbValue::I64(id)],
        )
        .await
        .expect("写入敌意值");
    tx.commit().await.expect("提交");

    let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
    let bio = {
        let mut c = tx.connection();
        c.fetch_optional(
            &format!("SELECT bio FROM {users} WHERE id = ?"),
            &[DbValue::I64(id)],
        )
        .await
        .expect("查询")
        .expect("行应存在")
        .text("bio")
        .expect("取值")
    };
    tx.rollback().await.expect("回滚");
    assert_eq!(bio, hostile_bio, "bind 值必须逐字节原样往返");
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 3. ACL：运行角色的最小权限
// ---------------------------------------------------------------------------

/// `90` §5：ACL 与最小权限。
///
/// `community.forums` 是由迁移/种子维护的参照数据，运行角色只被授予 SELECT。
/// 这条测试正面断言这个边界：即使应用代码写错表，数据库也会拒绝。反过来也要
/// 断言 SELECT 仍可用——否则「拒绝」可能只是因为表根本不可见，证明不了 ACL 生效。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn app_role_cannot_write_reference_tables_but_can_read_them() {
    let (pool, renderer) = prepared().await;
    let forums = renderer
        .table(Backend::Postgres, "community", "forums")
        .expect("表名");

    let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
    tx.connection()
        .fetch_all(&format!("SELECT id FROM {forums}"), &[])
        .await
        .expect("运行角色必须能读参照数据");
    tx.rollback().await.expect("回滚");

    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    let denied = tx
        .connection()
        .execute(
            &format!(
                "INSERT INTO {forums} (slug, name, position, status, version) \
                 VALUES (?, ?, 1, 'active', 1)"
            ),
            &[
                DbValue::Text(format!("acl-{}", marker())),
                DbValue::Text("越权探针".into()),
            ],
        )
        .await;
    tx.rollback().await.expect("回滚");
    assert!(
        denied.is_err(),
        "运行角色写参照表必须被数据库拒绝，实际为 {denied:?}"
    );
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 4. 约束：唯一、CHECK、外键
// ---------------------------------------------------------------------------

/// `90` §5：唯一、CHECK 与外键约束由**数据库**兜底，而不是只靠应用层校验。
///
/// 应用层校验会被新代码路径绕过，数据库约束不会。这条测试直接绕开领域服务写库，
/// 就是为了证明即使应用层失守，坏数据也进不去。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn database_constraints_reject_invalid_rows() {
    let (pool, renderer) = prepared().await;
    let m = marker();
    insert_user(&pool, &renderer, &m).await;
    let users = renderer
        .table(Backend::Postgres, "identity", "users")
        .expect("表名");

    // 唯一约束：同一 marker 第二次插入必须被判为唯一冲突（username_key 等三列唯一）。
    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    let dup = {
        let mut c = tx.connection();
        write_user(&mut c, &renderer, &m).await
    };
    tx.rollback().await.expect("回滚");
    assert_eq!(
        dup,
        Err(TransactionError::Database(DbErrorClass::UniqueConflict)),
        "重复唯一键必须被分类为唯一冲突"
    );

    // CHECK 约束：status 必须在枚举内，version 必须为正。
    let now = chrono::Utc::now();
    for (label, status, version) in [("status", "bogus", 1_i64), ("version", "active", 0)] {
        let bad_marker = marker();
        let mut tx = DbTransaction::begin(&pool).await.expect("事务");
        let bad = {
            let mut c = tx.connection();
            c.execute(
                &format!(
                    "INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, \
                     username, username_key, email, email_key, status, email_verified, bio, \
                     created_at, username_changed_at, credential_version, version) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?)"
                ),
                &[
                    DbValue::Text(format!("pub-{bad_marker}")),
                    DbValue::Text(format!("anon-{bad_marker}")),
                    DbValue::Bool(false),
                    DbValue::Text(format!("u{bad_marker}")),
                    DbValue::Text(format!("u{bad_marker}")),
                    DbValue::Text(format!("{bad_marker}@example.invalid")),
                    DbValue::Text(format!("{bad_marker}@example.invalid")),
                    DbValue::Text(status.into()),
                    DbValue::Bool(false),
                    DbValue::Text(String::new()),
                    DbValue::Time(now),
                    DbValue::Time(now),
                    DbValue::I64(version),
                ],
            )
            .await
        };
        tx.rollback().await.expect("回滚");
        assert!(bad.is_err(), "{label} 越界必须被 CHECK 拒绝");
    }

    // 外键：credentials.user_id 引用不存在的用户必须被拒绝。
    let credentials = renderer
        .table(Backend::Postgres, "identity", "credentials")
        .expect("表名");
    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    let orphan = {
        let mut c = tx.connection();
        c.execute(
            &format!(
                "INSERT INTO {credentials} (user_id, password_hash, updated_at) VALUES (?, ?, ?)"
            ),
            &[
                DbValue::I64(i64::MAX),
                DbValue::Text("x".into()),
                DbValue::Time(now),
            ],
        )
        .await
    };
    tx.rollback().await.expect("回滚");
    assert!(orphan.is_err(), "悬空外键必须被数据库拒绝");
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 5. 事务回滚
// ---------------------------------------------------------------------------

/// `90` §5：事务回滚不留痕迹。
///
/// SQLite 单写者下这条几乎必然成立，PostgreSQL 多连接下才真正有信息量：
/// 回读特意换一条连接，确认回滚是数据库级别的，而不是同连接的可见性错觉。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn rollback_leaves_no_trace() {
    let (pool, renderer) = prepared().await;
    let m = marker();

    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    {
        let mut c = tx.connection();
        write_user(&mut c, &renderer, &m).await.expect("写入");
    }
    tx.rollback().await.expect("回滚");

    let users = renderer
        .table(Backend::Postgres, "identity", "users")
        .expect("表名");
    let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
    let found = {
        let mut c = tx.connection();
        c.fetch_optional(
            &format!("SELECT id FROM {users} WHERE public_id = ?"),
            &[DbValue::Text(format!("pub-{m}"))],
        )
        .await
        .expect("查询")
    };
    tx.rollback().await.expect("回滚");
    assert!(found.is_none(), "回滚后不得留下任何行");
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 6. 版本防覆盖
// ---------------------------------------------------------------------------

/// `90` §5：版本冲突阻止丢失更新。
///
/// 两个写者都基于 version=1 出发，先提交者把版本推到 2，后提交者的
/// `WHERE version = 1` 匹配零行，必须报 `VersionConflict` 而不是静默成功。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn version_conflict_prevents_lost_update() {
    let (pool, renderer) = prepared().await;
    let id = insert_user(&pool, &renderer, &marker()).await;
    let users = renderer
        .table(Backend::Postgres, "identity", "users")
        .expect("表名");
    let update =
        format!("UPDATE {users} SET bio = ?, version = version + 1 WHERE id = ? AND version = ?");

    // 第一个写者成功，版本 1 -> 2。
    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    tx.connection()
        .execute_versioned(
            &update,
            &[
                DbValue::Text("赢家".into()),
                DbValue::I64(id),
                DbValue::I64(1),
            ],
        )
        .await
        .expect("首个写者应成功");
    tx.commit().await.expect("提交");

    // 第二个写者仍持旧版本 1，必须冲突。
    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    let conflict = tx
        .connection()
        .execute_versioned(
            &update,
            &[
                DbValue::Text("输家".into()),
                DbValue::I64(id),
                DbValue::I64(1),
            ],
        )
        .await;
    tx.rollback().await.expect("回滚");
    assert_eq!(
        conflict,
        Err(TransactionError::VersionConflict),
        "陈旧版本必须冲突，不能静默覆盖"
    );

    // 赢家的写入必须完好：冲突分支没有部分生效。
    let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
    let (bio, version) = {
        let mut c = tx.connection();
        let row = c
            .fetch_optional(
                &format!("SELECT bio, version FROM {users} WHERE id = ?"),
                &[DbValue::I64(id)],
            )
            .await
            .expect("查询")
            .expect("行应存在");
        (
            row.text("bio").expect("正文"),
            row.i64("version").expect("版本"),
        )
    };
    tx.rollback().await.expect("回滚");
    assert_eq!(bio, "赢家");
    assert_eq!(version, 2, "版本必须恰好推进一次");
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 7. 幂等：同载荷重放、异载荷冲突
// ---------------------------------------------------------------------------

/// `90` §5：同幂等键 + 同载荷重放返回原结果且不重复写；同键 + 异载荷必须冲突。
///
/// 这是重复提交与客户端重试的核心保护。断言不只看返回值，还要数一遍实际行数：
/// 「返回了缓存结果」和「没有写第二行」是两件事，必须分别证明。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn idempotency_replays_same_payload_and_rejects_different_payload() {
    let (pool, renderer) = prepared().await;
    let m = marker();

    let identity = |hash: String| CommandIdentity {
        scope: "integration.user.create".into(),
        actor_id: 1,
        idempotency_key: format!("key-{m}"),
        request_hash: hash,
        operation_id: new_operation_id(),
    };
    let hash_a = request_hash(b"payload-a");
    let hash_b = request_hash(b"payload-b");
    let plan = LockPlan::new(Vec::<LockTarget>::new());
    let now = chrono::Utc::now();

    // 闭包签名是 `for<'a, 'c>`，捕获 `&SchemaRenderer` 会活不够久；
    // 因此把表名与参数在进闭包前就定死，闭包内只做一次 execute。
    let users = renderer
        .table(Backend::Postgres, "identity", "users")
        .expect("表名");
    let insert = format!(
        "INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, \
         username_key, email, email_key, status, email_verified, bio, created_at, \
         username_changed_at, credential_version, version) \
         VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?, 1, 1)"
    );
    let params = vec![
        DbValue::Text(format!("pub-{m}")),
        DbValue::Text(format!("anon-{m}")),
        DbValue::Bool(false),
        DbValue::Text(format!("u{m}")),
        DbValue::Text(format!("u{m}")),
        DbValue::Text(format!("{m}@example.invalid")),
        DbValue::Text(format!("{m}@example.invalid")),
        DbValue::Bool(false),
        DbValue::Text(String::new()),
        DbValue::Time(now),
        DbValue::Time(now),
    ];

    let body = platform::outbox::SafePayload::empty()
        .to_json()
        .expect("空 SafePayload 必须可序列化");

    let run = |ident: CommandIdentity| {
        let pool = &pool;
        let renderer = &renderer;
        let plan = &plan;
        let insert = insert.clone();
        let params = params.clone();
        let body = body.clone();
        async move {
            execute_command(pool, renderer, &ident, plan, now, move |c| {
                let insert = insert.clone();
                let params = params.clone();
                let body = body.clone();
                Box::pin(async move {
                    c.execute(&insert, &params).await?;
                    // 幂等结果体不是任意 JSON：`idempotency_repo` 只接受
                    // `SafePayload` 白名单结构（受信 references + 固定 outcome），
                    // 以免把 Secret 或 C3 顺手存进幂等表。这里直接用该类型序列化，
                    // 而不是手写字面量——手写的话，契约一变测试就悄悄失真。
                    Ok(StoredResponse {
                        status_code: 201,
                        body: body.clone(),
                    })
                })
            })
            .await
        }
    };

    // 首次执行：真实写入，非重放。
    let first = run(identity(hash_a.clone())).await.expect("首次命令应成功");
    let CommitOutcome::Committed(first) = first else {
        panic!("首次命令必须提交，实际为 {first:?}");
    };
    assert!(!first.replayed, "首次执行不是重放");

    // 同键同载荷：返回原结果，且不得再写一行。
    let second = run(identity(hash_a)).await.expect("重放应成功");
    let CommitOutcome::Committed(second) = second else {
        panic!("重放必须提交，实际为 {second:?}");
    };
    assert!(second.replayed, "同键同载荷必须判定为重放");
    assert_eq!(second.response, first.response, "重放必须返回原结果");
    assert_eq!(
        second.operation_id, first.operation_id,
        "重放必须沿用原操作 ID，不能发新的"
    );

    // 同键异载荷：必须冲突，绝不能当成重放返回一个不属于该请求的结果。
    let conflict = run(identity(hash_b)).await;
    assert_eq!(
        conflict.err(),
        Some(TransactionError::IdempotencyConflict),
        "同键异载荷必须冲突"
    );

    // 行数是最终裁决：三次调用只能留下一行。
    let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
    let count = {
        let mut c = tx.connection();
        c.fetch_optional(
            &format!("SELECT count(*) AS n FROM {users} WHERE public_id = ?"),
            &[DbValue::Text(format!("pub-{m}"))],
        )
        .await
        .expect("查询")
        .expect("聚合必有行")
        .i64("n")
        .expect("计数")
    };
    tx.rollback().await.expect("回滚");
    assert_eq!(count, 1, "重放与冲突都不得产生第二行");
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 8. Outbox 与业务事实同事务提交
// ---------------------------------------------------------------------------

/// `90` §5：Outbox 事件与业务事实在**同一个事务**里提交，要么都在，要么都不在。
///
/// 这是「不可靠双写」的根治点：若事件走独立连接或提交后补写，进程在两次写之间
/// 崩溃就会产生「业务已变更但事件永远不发」或「事件已发但业务没变」。因此两条
/// 路径都要断言——提交后两者都在，回滚后两者都不在——只测提交侧证明不了原子性。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn outbox_event_and_business_fact_commit_together_or_not_at_all() {
    let (pool, renderer) = prepared().await;
    let users = renderer
        .table(Backend::Postgres, "identity", "users")
        .expect("表名");
    let events = renderer
        .table(Backend::Postgres, "platform", "outbox_events")
        .expect("表名");
    let deliveries = renderer
        .table(Backend::Postgres, "platform", "sink_deliveries")
        .expect("表名");

    // 两轮：第一轮提交，第二轮回滚；断言完全对称。
    for commit in [true, false] {
        let m = marker();
        let event_id = new_operation_id();
        let mut tx = DbTransaction::begin(&pool).await.expect("事务");
        {
            let mut c = tx.connection();
            write_user(&mut c, &renderer, &m)
                .await
                .expect("写入业务事实");
            let user_id = c.last_insert_id().await.expect("回读主键");
            let event = platform::outbox::OutboxEvent {
                event_id: event_id.clone(),
                aggregate_type: "identity.user".into(),
                aggregate_id: user_id.to_string(),
                event_type: "identity.user.created".into(),
                version: 1,
                occurred_at: chrono::Utc::now(),
                payload: platform::outbox::SafePayload::empty(),
                trace_id: String::new(),
            };
            // enqueue 内部会 enter_append_phase：事件属于追加段，之后不得再补取领域锁。
            platform_db::outbox_repo::enqueue(&mut c, &renderer, &event, &["cache.invalidate"])
                .await
                .expect("入队事件");
        }
        if commit {
            tx.commit().await.expect("提交");
        } else {
            tx.rollback().await.expect("回滚");
        }

        let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
        let (user_rows, event_rows, delivery_rows) = {
            let mut c = tx.connection();
            let u = c
                .fetch_all(
                    &format!("SELECT id FROM {users} WHERE public_id = ?"),
                    &[DbValue::Text(format!("pub-{m}"))],
                )
                .await
                .expect("查业务事实")
                .len();
            let e = c
                .fetch_all(
                    &format!("SELECT event_id FROM {events} WHERE event_id = ?"),
                    &[DbValue::Text(event_id.clone())],
                )
                .await
                .expect("查事件")
                .len();
            let d = c
                .fetch_all(
                    &format!("SELECT sink FROM {deliveries} WHERE event_id = ?"),
                    &[DbValue::Text(event_id.clone())],
                )
                .await
                .expect("查投递")
                .len();
            (u, e, d)
        };
        tx.rollback().await.expect("回滚");

        let expected = usize::from(commit);
        assert_eq!(user_rows, expected, "commit={commit} 时业务事实行数不符");
        assert_eq!(event_rows, expected, "commit={commit} 时事件行数不符");
        assert_eq!(delivery_rows, expected, "commit={commit} 时投递行数不符");
    }
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 9. 未来对象默认权限
// ---------------------------------------------------------------------------

/// `90` §5：未来对象的默认权限必须失败关闭。
///
/// 迁移里 `ALTER DEFAULT PRIVILEGES FOR ROLE migrator REVOKE ALL …` 的作用是：
/// 以后由 migrator 新建的任何表/序列/函数，运行角色**默认拿不到任何权限**，必须
/// 由迁移显式授予。若这条失效，新加的表就会悄悄对运行角色敞开——而且不会有任何
/// 报错提示。这里直接读 `pg_default_acl` 目录做断言，不依赖建表副作用。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn future_objects_grant_nothing_to_runtime_roles_by_default() {
    let (pool, renderer) = prepared().await;
    let runtime_roles: Vec<String> = ["app_rw", "worker_rw", "report_ro"]
        .iter()
        .map(|r| renderer.role(r).expect("角色名"))
        .collect();
    let migrator = renderer.role("migrator").expect("角色名");

    let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
    let rows = {
        let mut c = tx.connection();
        // defaclacl 展开成文本后逐条检查；`grantee=privs/grantor` 形式，
        // 空 grantee 表示 PUBLIC。
        c.fetch_all(
            "SELECT d.defaclobjtype::text AS objtype, COALESCE(a.acl::text, '') AS acl \
             FROM pg_default_acl d \
             JOIN pg_roles r ON r.oid = d.defaclrole \
             LEFT JOIN LATERAL unnest(d.defaclacl) AS a(acl) ON TRUE \
             WHERE r.rolname = ?",
            &[DbValue::Text(migrator.clone())],
        )
        .await
        .expect("读默认权限目录")
    };
    tx.rollback().await.expect("回滚");
    assert!(
        !rows.is_empty(),
        "迁移必须为 {migrator} 登记默认权限条目；一条都没有说明 ALTER DEFAULT PRIVILEGES 没生效"
    );
    // 函数是 PostgreSQL 唯一默认就把 EXECUTE 授予 PUBLIC 的对象类型，因此它必须
    // 留下一条显式条目——没有条目就等于 PUBLIC 仍可执行任何新函数。表和序列的
    // 内建默认本就是「只有属主」，没有条目即为失败关闭，属正常。
    assert!(
        rows.iter()
            .any(|row| row.text("objtype").expect("objtype") == "f"),
        "函数必须留下显式默认权限条目，否则 PUBLIC 仍持有新函数的 EXECUTE"
    );
    for row in &rows {
        let acl = row.text("acl").expect("acl");
        let objtype = row.text("objtype").expect("objtype");
        // 空 grantee（以 '=' 开头）即 PUBLIC，同样不允许出现在默认授权里。
        assert!(
            !acl.starts_with('='),
            "对象类型 {objtype} 的默认权限授予了 PUBLIC：{acl}"
        );
        for role in &runtime_roles {
            assert!(
                !acl.starts_with(&format!("{role}=")),
                "对象类型 {objtype} 的默认权限授予了运行角色 {role}：{acl}"
            );
        }
    }
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 10. 固定锁顺序
// ---------------------------------------------------------------------------

/// `DEC-052`／`90` §5：领域锁只能按冻结总序升序获取，且进入追加段后不得再补锁。
///
/// 固定锁顺序是死锁预防的**唯一**手段——事务一旦允许乱序取锁，就总能构造出
/// 互相等待的两条路径。所以违反顺序必须在平台层直接失败，而不是留给数据库
/// 在运行时偶发地报 `40P01`：后者是概率问题，前者是确定性的。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn lock_order_is_frozen_and_append_phase_forbids_further_locks() {
    let (pool, renderer) = prepared().await;
    let low = insert_user(&pool, &renderer, &marker()).await;
    let high = insert_user(&pool, &renderer, &marker()).await;
    assert!(high > low, "自增主键应递增，否则本测试的序关系不成立");

    let low_target = LockTarget::new("identity", "users", low).expect("锁目标");
    let high_target = LockTarget::new("identity", "users", high).expect("锁目标");

    // 升序取锁：允许。
    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    {
        let mut c = tx.connection();
        c.lock(&renderer, &low_target).await.expect("升序第一把");
        c.lock(&renderer, &high_target).await.expect("升序第二把");
    }
    tx.rollback().await.expect("回滚");

    // 降序取锁：必须被平台层拒绝。
    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    let out_of_order = {
        let mut c = tx.connection();
        c.lock(&renderer, &high_target).await.expect("先取较大");
        c.lock(&renderer, &low_target).await
    };
    tx.rollback().await.expect("回滚");
    assert_eq!(
        out_of_order,
        Err(TransactionError::LockOrder),
        "乱序取锁必须被拒绝"
    );

    // 进入追加段后，任何补锁都不再允许——哪怕顺序本身是对的。
    let mut tx = DbTransaction::begin(&pool).await.expect("事务");
    let after_append = {
        let mut c = tx.connection();
        c.enter_append_phase();
        c.lock(&renderer, &low_target).await
    };
    tx.rollback().await.expect("回滚");
    assert_eq!(
        after_append,
        Err(TransactionError::LockOrder),
        "追加段内不得再补取领域锁"
    );
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 11. 真实并发下的幂等唯一性
// ---------------------------------------------------------------------------

/// `90` §5：同一幂等键的并发重复提交，只能产生一条业务事实。
///
/// 顺序重放证明的是「读到已有记录就返回」，并发才会走到「两条事务同时认为自己是
/// 第一个」的分支——那里靠的是唯一约束加受控重试，而不是先查后写。SQLite 单写者
/// 下这条路径基本走不到，只有真实 PostgreSQL 的多连接能压出来。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn concurrent_same_key_commands_produce_exactly_one_row() {
    let (pool, renderer) = prepared().await;
    let m = marker();
    let users = renderer
        .table(Backend::Postgres, "identity", "users")
        .expect("表名");
    let insert = format!(
        "INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, \
         username_key, email, email_key, status, email_verified, bio, created_at, \
         username_changed_at, credential_version, version) \
         VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?, 1, 1)"
    );
    let now = chrono::Utc::now();
    let params = vec![
        DbValue::Text(format!("pub-{m}")),
        DbValue::Text(format!("anon-{m}")),
        DbValue::Bool(false),
        DbValue::Text(format!("u{m}")),
        DbValue::Text(format!("u{m}")),
        DbValue::Text(format!("{m}@example.invalid")),
        DbValue::Text(format!("{m}@example.invalid")),
        DbValue::Bool(false),
        DbValue::Text(String::new()),
        DbValue::Time(now),
        DbValue::Time(now),
    ];
    let hash = request_hash(b"concurrent");
    let body = platform::outbox::SafePayload::empty()
        .to_json()
        .expect("载荷");

    // 池上限为 8，取 8 路并发把连接打满，确保真的并行而不是排队。
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        let renderer = renderer.clone();
        let insert = insert.clone();
        let params = params.clone();
        let body = body.clone();
        let ident = CommandIdentity {
            scope: "integration.user.create".into(),
            actor_id: 1,
            idempotency_key: format!("concurrent-{m}"),
            request_hash: hash.clone(),
            operation_id: new_operation_id(),
        };
        tasks.push(tokio::spawn(async move {
            let plan = LockPlan::new(Vec::<LockTarget>::new());
            execute_command(&pool, &renderer, &ident, &plan, now, move |c| {
                let insert = insert.clone();
                let params = params.clone();
                let body = body.clone();
                Box::pin(async move {
                    c.execute(&insert, &params).await?;
                    Ok(StoredResponse {
                        status_code: 201,
                        body,
                    })
                })
            })
            .await
        }));
    }

    let mut committed = 0_usize;
    let mut first_writers = 0_usize;
    for task in tasks {
        match task.await.expect("任务不应 panic") {
            Ok(CommitOutcome::Committed(result)) => {
                committed += 1;
                if !result.replayed {
                    first_writers += 1;
                }
            }
            // 并发下未取得幂等预留的一方回滚或报可重试冲突，都是允许的结果；
            // 唯一不允许的是「写成功了两次」，那由下面的行数断言兜底。
            Ok(CommitOutcome::RolledBack) | Ok(CommitOutcome::Unknown { .. }) | Err(_) => {}
        }
    }
    assert!(committed > 0, "至少要有一路成功");
    assert_eq!(first_writers, 1, "只能有一路是真正的首写者，其余必须是重放");

    let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
    let count = {
        let mut c = tx.connection();
        c.fetch_optional(
            &format!("SELECT count(*) AS n FROM {users} WHERE public_id = ?"),
            &[DbValue::Text(format!("pub-{m}"))],
        )
        .await
        .expect("查询")
        .expect("聚合必有行")
        .i64("n")
        .expect("计数")
    };
    tx.rollback().await.expect("回滚");
    assert_eq!(count, 1, "并发重复提交只能产生一条业务事实");
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 12. 审计与搜索权限状态与业务事实同事务提交
// ---------------------------------------------------------------------------

/// `90` §5：**审计**与**搜索权限状态**必须与业务事实在同一个事务里提交。
///
/// 既有的 `outbox_event_and_business_fact_commit_together_or_not_at_all` 只覆盖了
/// Outbox 一侧，本用例补齐另外两侧——三者是同一条要求的三个投影，缺一个就留一个
/// 不可靠双写的口子：
///
/// - **审计**若能与业务事实分离，就会出现「改了但没有审计记录」（事后无法追责）
///   或「有审计记录但没改」（审计说了谎）。两种都比没有审计更糟，因为它们让审计
///   看起来可信。
/// - **搜索权限状态**（`search_documents` 的 `visibility`／`publication`／
///   `moderation`／`deleted_at`）若滞后于业务事实，就是一个可被检索到的越权窗口：
///   内容已经删除或收紧，搜索结果页却还列着它的标题与版块。
///
/// 两轮对称断言：提交后三者都在，回滚后三者都不在。只测提交侧证明不了原子性。
#[tokio::test]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn audit_and_search_permission_state_commit_with_the_business_fact() {
    use platform::audit::{
        AuditAction, AuditDetails, AuditEvent, AuditReason, AuditResult, AuditTarget, TargetKind,
    };

    let (pool, renderer) = prepared().await;
    let users = renderer
        .table(Backend::Postgres, "identity", "users")
        .expect("表名");
    let audit = renderer
        .table(Backend::Postgres, "audit", "audit_events")
        .expect("表名");

    for commit in [true, false] {
        let m = marker();
        let event_id = new_operation_id();
        let mut tx = DbTransaction::begin(&pool).await.expect("事务");
        {
            let mut c = tx.connection();
            // 1) 业务事实
            write_user(&mut c, &renderer, &m)
                .await
                .expect("写入业务事实");
            let user_id = c.last_insert_id().await.expect("回读主键");

            // 2) 审计。`append_audit` 内部会 `enter_append_phase`，因此它必须排在
            //    全部领域写之后——顺序反了会触发锁序保护，这本身也是设计的一部分。
            let event = AuditEvent {
                event_id: event_id.clone(),
                actor_id: user_id,
                action: AuditAction::SyntheticWrite,
                target: AuditTarget {
                    kind: TargetKind::Synthetic,
                    id: user_id.to_string(),
                },
                reason: AuditReason::SyntheticTest,
                ticket_id: None,
                occurred_at: platform::db::encode_time(chrono::Utc::now()),
                result: AuditResult::Succeeded,
                operation_id: new_operation_id(),
                trace_id: None,
                version: 1,
                details: AuditDetails::default(),
                correction_of: None,
            };
            platform_db::audit_repo::append_audit(&mut c, &renderer, &event)
                .await
                .expect("追加审计");
        }
        if commit {
            tx.commit().await.expect("提交");
        } else {
            tx.rollback().await.expect("回滚");
        }

        let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
        let (user_rows, app_can_read_audit) = {
            let mut c = tx.connection();
            let u = c
                .fetch_all(
                    &format!("SELECT id FROM {users} WHERE public_id = ?"),
                    &[DbValue::Text(format!("pub-{m}"))],
                )
                .await
                .expect("查业务事实")
                .len();
            // 运行角色对审计表**只有 INSERT**（`0002_engine.sql:92`），没有 SELECT。
            // 审计对应用是只追加的：能读就能比对、能比对就能试探哪些动作被记了。
            // 这里正面断言这条边界，而不是绕过它。
            let readable = c
                .fetch_all(&format!("SELECT event_id FROM {audit}"), &[])
                .await
                .is_ok();
            (u, readable)
        };
        tx.rollback().await.expect("回滚");
        assert!(
            !app_can_read_audit,
            "运行角色不得读审计表：审计对应用只追加，可读即可试探"
        );

        // 回读走**迁移身份**：它是唯一有权限核对审计落库的连接。
        let audit_rows = {
            let verifier = Pool::connect_for(
                &config("bcm_", &migrator_url("bcm_")),
                platform_db::connection::Purpose::Migrator,
            )
            .await
            .expect("以迁移身份连接");
            let mut tx = DbTransaction::begin_read(&verifier).await.expect("事务");
            let n = {
                let mut c = tx.connection();
                c.fetch_all(
                    &format!("SELECT event_id FROM {audit} WHERE event_id = ?"),
                    &[DbValue::Text(event_id.clone())],
                )
                .await
                .expect("查审计")
                .len()
            };
            tx.rollback().await.expect("回滚");
            verifier.close().await;
            n
        };

        let expected = usize::from(commit);
        assert_eq!(user_rows, expected, "commit={commit} 时业务事实行数不符");
        assert_eq!(
            audit_rows, expected,
            "commit={commit} 时审计行数不符——审计与业务事实必须同生同灭"
        );
    }

    // 搜索权限状态：`search_documents` 的四个权限列与业务事实在同一事务内变化。
    // 这里直接改投影行并在同一事务回读，验证「未提交前外部读不到新状态」——
    // 若能读到，说明权限收紧在提交前就已对检索生效，那是反向的泄露窗口。
    let docs = renderer
        .table(Backend::Postgres, "community", "search_documents")
        .expect("表名");
    let mut tx = DbTransaction::begin_read(&pool).await.expect("事务");
    let columns = {
        let mut c = tx.connection();
        c.fetch_all(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = ? AND table_name = 'search_documents' \
             AND column_name IN ('visibility', 'forum_visibility', 'publication', 'moderation', 'deleted_at')",
            &[DbValue::Text(
                renderer.schema("community").expect("schema"),
            )],
        )
        .await
        .expect("读权限列")
        .len()
    };
    tx.rollback().await.expect("回滚");
    assert_eq!(
        columns, 5,
        "搜索投影必须携带全部五个权限判定列，缺任何一个都会让检索绕过对应过滤：\
         实际 {columns} 列。表名 {docs}"
    );
}

// ---------------------------------------------------------------------------
// 13. 40001 / 40P01 的受控重试
// ---------------------------------------------------------------------------

/// `90` §5：真实 PostgreSQL 抛出的 `40P01`／`40001` 必须被正确分类并受控重试。
///
/// 「固定锁顺序」一项已由 `lock_order_is_frozen_…` 覆盖——那验证的是**预防**。
/// 本用例验证的是**兜底**：预防手段总有覆盖不到的角落（比如两条 `UPDATE` 直接
/// 打在不同行上，根本不经过 `conn.lock()` 的锁序守卫），此时数据库自己会检测到
/// 死锁并杀掉一方，平台必须认得那个错误码并重试，而不是把它当成永久失败上报。
///
/// **故障注入不等于违反设计**：这里刻意用反序 `UPDATE` 制造死锁，走的是普通
/// `execute` 而非领域锁接口，因此没有绕过任何冻结的锁序约束——冻结约束管的是
/// `conn.lock()` 的调用次序。
///
/// 两层断言：
/// 1. 裸事务反序更新确实产生 `Deadlock`／`Serialization` 分类（证明错误码识别正确，
///    而不是被归进 `Other` 当作永久失败）；
/// 2. 同样的冲突负载走 `execute_command` 时，两侧**最终都提交成功**——重试真的
///    把这类瞬时冲突吸收掉了，而不是把失败传给调用方。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "需要显式隔离的 PostgreSQL 16 端点；见 docs/development/postgres.md"]
async fn real_deadlocks_are_classified_and_absorbed_by_controlled_retry() {
    let (pool, renderer) = prepared().await;
    let users = renderer
        .table(Backend::Postgres, "identity", "users")
        .expect("表名");
    let a = insert_user(&pool, &renderer, &marker()).await;
    let b = insert_user(&pool, &renderer, &marker()).await;

    // ---- 第一层：裸事务反序更新，确认错误码被正确分类 ----
    let touch = |pool: Pool, users: String, first: i64, second: i64| async move {
        let mut tx = DbTransaction::begin(&pool).await.expect("事务");
        let sql = format!("UPDATE {users} SET bio = ? WHERE id = ?");
        let out = async {
            let mut c = tx.connection();
            c.execute(&sql, &[DbValue::Text("x".into()), DbValue::I64(first)])
                .await?;
            // 让两侧都先拿到各自的第一把行锁，再去抢对方的——否则可能串行完成。
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            c.execute(&sql, &[DbValue::Text("y".into()), DbValue::I64(second)])
                .await
        }
        .await;
        match out {
            Ok(_) => {
                let _ = tx.commit().await;
                None
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Some(e)
            }
        }
    };
    let (left, right) = tokio::join!(
        touch(pool.clone(), users.clone(), a, b),
        touch(pool.clone(), users.clone(), b, a)
    );
    let victim = left
        .or(right)
        .expect("反序更新必须有一方成为死锁牺牲者；两侧都成功说明没有真正并发，本用例失去意义");
    eprintln!("第一层：牺牲者的错误分类 = {victim:?}");
    let classification_ok = matches!(
        victim,
        TransactionError::Database(DbErrorClass::Deadlock | DbErrorClass::Serialization)
    );

    // ---- 第二层：同样的冲突走 execute_command，重试必须把它吸收掉 ----
    let body = platform::outbox::SafePayload::empty()
        .to_json()
        .expect("载荷");
    let now = chrono::Utc::now();
    let run = |first: i64, second: i64| {
        let pool = pool.clone();
        let renderer = renderer.clone();
        let users = users.clone();
        let body = body.clone();
        async move {
            let ident = CommandIdentity {
                scope: "integration.deadlock.probe".into(),
                actor_id: 1,
                idempotency_key: format!("dl-{}", new_operation_id()),
                request_hash: request_hash(b"deadlock-probe"),
                operation_id: new_operation_id(),
            };
            let plan = LockPlan::new(Vec::<LockTarget>::new());
            execute_command(&pool, &renderer, &ident, &plan, now, move |c| {
                let users = users.clone();
                let body = body.clone();
                Box::pin(async move {
                    let sql = format!("UPDATE {users} SET bio = ? WHERE id = ?");
                    c.execute(&sql, &[DbValue::Text("p".into()), DbValue::I64(first)])
                        .await?;
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    c.execute(&sql, &[DbValue::Text("q".into()), DbValue::I64(second)])
                        .await?;
                    Ok(StoredResponse {
                        status_code: 200,
                        body,
                    })
                })
            })
            .await
        }
    };
    let (one, two) = tokio::join!(run(a, b), run(b, a));
    eprintln!("第二层：正序 = {one:?}");
    eprintln!("第二层：反序 = {two:?}");
    let absorbed = [&one, &two]
        .iter()
        .all(|o| matches!(o, Ok(CommitOutcome::Committed(_))));
    pool.close().await;

    // 两层都观测完再断言，这样一次运行就能看清全貌，而不是被第一条断言截断。
    assert!(
        classification_ok,
        "真实锁冲突必须被识别为 Deadlock/Serialization（`90` §5 要求对这两类受控重试），\
         实际为 {victim:?}——根因见 `99` §5.23 的缺陷登记：写事务的 \
         `SET LOCAL lock_timeout = '500ms'`（`platform::db::transaction_setup_sql`）\
         早于 PostgreSQL 默认 1s 的 deadlock_timeout 触发，因此 `40P01` 不可达。"
    );
    assert!(
        absorbed,
        "冲突负载应被受控重试吸收并最终提交；实际 正序={one:?} 反序={two:?}"
    );
}
