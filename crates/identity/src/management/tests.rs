//! WP-MVP-B03 管理侧的隔离测试夹具。
//!
//! 这里创建的 staff principal 与管理会话都是显式合成的测试事实，不构成任何运行时
//! 引导旁路：生产路径唯一入口是 `bootstrap_first_owner`，且只在系统无 staff 时可用。
use super::*;
use crate::mfa;
use platform::config::Config;
use platform_db::{
    Pool,
    transaction::{DbTransaction, DbValue},
};
use std::path::{Path, PathBuf};

struct Fixture {
    dir: PathBuf,
    pool: Pool,
    renderer: SchemaRenderer,
    a: i64,
    b: i64,
}
impl Fixture {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-manage-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = Config::default().database;
        config.sqlite_path = dir.join("manage.sqlite3").to_string_lossy().into();
        config.schema_prefix = "manage_probe_".into();
        platform_db::migrations::migrate(
            &config,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let pool = Pool::connect(&config).await.unwrap();
        let renderer = SchemaRenderer::new(&config.schema_prefix).unwrap();
        let mut tx = DbTransaction::begin(&pool).await.unwrap();
        let users = renderer
            .table(tx.connection().backend(), "identity", "users")
            .unwrap();
        let mut ids = Vec::new();
        for name in ["owner-candidate", "second-candidate"] {
            tx.connection().execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, '', ?, ?, 1, 1)"),
                &[DbValue::Text(platform::transaction::new_operation_id()), DbValue::Text(platform::transaction::new_operation_id()), DbValue::Bool(false), DbValue::Text(name.into()), DbValue::Text(name.into()), DbValue::Text(format!("{name}@example.invalid")), DbValue::Text(format!("{name}@example.invalid")), DbValue::Bool(true), DbValue::Time(at()), DbValue::Time(at())]).await.unwrap();
            ids.push(tx.connection().last_insert_id().await.unwrap());
        }
        tx.commit().await.unwrap();
        Self {
            dir,
            pool,
            renderer,
            a: ids[0],
            b: ids[1],
        }
    }
    fn wrapper(&self) -> mfa::SecretWrapper {
        mfa::SecretWrapper::new(&[5u8; 32], 1).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn at() -> DateTime<Utc> {
    platform::db::decode_time("2026-09-06T00:00:00.000000Z").unwrap()
}

/// AUTH-014：首账号提权只能执行一次，之后 CLI 路径必须拒绝。
#[tokio::test]
async fn first_owner_bootstrap_is_single_use_and_refuses_once_any_staff_exists() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    bootstrap_first_owner(&mut tx.connection(), &f.renderer, f.a, at())
        .await
        .unwrap();
    // 系统已有 staff principal，任何后续提权（包括对另一个账号）一律拒绝。
    assert!(
        bootstrap_first_owner(&mut tx.connection(), &f.renderer, f.b, at())
            .await
            .is_err()
    );
    assert!(
        bootstrap_first_owner(&mut tx.connection(), &f.renderer, f.a, at())
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
}

/// 管理会话：与普通会话隔离、8 小时绝对上限、15 分钟空闲上限、撤销即失效。
#[tokio::test]
async fn management_session_enforces_absolute_and_idle_limits_and_revocation() {
    let f = Fixture::new().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    bootstrap_first_owner(&mut c, &f.renderer, f.a, at())
        .await
        .unwrap();
    // 尚未绑定 TOTP：会话建立得起来，但只能走绑定流程（AUTH-014）。
    let access = open_management_session(&mut c, &f.renderer, f.a, "raw-session", "raw-csrf", at())
        .await
        .unwrap();
    assert_eq!(access, ManagementAccess::BindingRequired);

    // 非 staff 账号无法开启管理会话——普通账号的凭据进不了管理体系。
    assert!(
        open_management_session(&mut c, &f.renderer, f.b, "other", "csrf", at())
            .await
            .is_err()
    );

    let identity = authorize_management_session(&mut c, &f.renderer, "raw-session", at())
        .await
        .unwrap();
    assert_eq!(identity.user_id, f.a);
    assert_eq!(identity.access, ManagementAccess::BindingRequired);
    // 管理侧 CSRF 独立校验。
    identity.check_csrf("raw-csrf").unwrap();
    assert!(identity.check_csrf("raw-session").is_err());

    // 空闲上限：14 分钟仍有效并刷新计时，16 分钟后失效。
    let t14 = at() + Duration::minutes(14);
    assert!(
        authorize_management_session(&mut c, &f.renderer, "raw-session", t14)
            .await
            .is_ok()
    );
    assert!(
        authorize_management_session(
            &mut c,
            &f.renderer,
            "raw-session",
            t14 + Duration::minutes(16)
        )
        .await
        .is_err()
    );

    // 绝对上限：即便一直活跃，超过 8 小时也必须失效。
    let mut now = at();
    for _ in 0..40 {
        now += Duration::minutes(14);
        let _ = authorize_management_session(&mut c, &f.renderer, "raw-session", now).await;
    }
    assert!(
        authorize_management_session(
            &mut c,
            &f.renderer,
            "raw-session",
            at() + Duration::hours(9)
        )
        .await
        .is_err()
    );

    // 撤销后立即失效。
    crate::management_repo::revoke_management_sessions(&mut c, &f.renderer, f.a)
        .await
        .unwrap();
    assert!(
        authorize_management_session(&mut c, &f.renderer, "raw-session", at())
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
}

/// TOTP 绑定、恢复码只显示一次、assertion 的四项绑定与单次消费。
#[tokio::test]
async fn binding_recovery_codes_and_assertions_enforce_single_use_and_binding() {
    let f = Fixture::new().await;
    let wrapper = f.wrapper();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let mut c = tx.connection();
    bootstrap_first_owner(&mut c, &f.renderer, f.a, at())
        .await
        .unwrap();
    open_management_session(&mut c, &f.renderer, f.a, "s1", "c1", at())
        .await
        .unwrap();
    let identity = authorize_management_session(&mut c, &f.renderer, "s1", at())
        .await
        .unwrap();

    let (encoded, uri) = begin_mfa_binding(
        &mut c,
        &f.renderer,
        &wrapper,
        f.a,
        "owner@example.invalid",
        at(),
    )
    .await
    .unwrap();
    assert!(uri.starts_with("otpauth://totp/"));
    assert!(uri.contains(&encoded), "URI 必须带上给认证器扫码的密钥");

    // 用错误验证码确认失败；正确验证码确认成功并返回一组只显示一次的恢复码。
    assert!(
        confirm_mfa_binding(&mut c, &f.renderer, &wrapper, &identity, "000000", at())
            .await
            .is_err()
    );
    let secret = decode_base32(&encoded);
    let step = mfa::step_for(at().timestamp()).unwrap();
    let code = totp_code(&secret, step);
    let codes = confirm_mfa_binding(&mut c, &f.renderer, &wrapper, &identity, &code, at())
        .await
        .unwrap();
    assert_eq!(codes.len(), mfa::RECOVERY_CODE_COUNT);

    // 绑定完成后会话转为 Ready。
    let identity = authorize_management_session(&mut c, &f.renderer, "s1", at())
        .await
        .unwrap();
    assert_eq!(identity.access, ManagementAccess::Ready);

    // 同一时间步不可重放：确认时已用掉这一步，再签发 assertion 必须被拒。
    assert!(
        issue_assertion(
            &mut c,
            &f.renderer,
            &wrapper,
            &identity,
            &code,
            "grant_role",
            "assertion-token-0001",
            at(),
        )
        .await
        .is_err()
    );

    // 下一个时间步的验证码可以签发 assertion。
    let later = at() + Duration::seconds(mfa::TOTP_STEP_SECONDS);
    let next = totp_code(&secret, mfa::step_for(later.timestamp()).unwrap());
    issue_assertion(
        &mut c,
        &f.renderer,
        &wrapper,
        &identity,
        &next,
        "grant_role",
        "assertion-token-0001",
        later,
    )
    .await
    .unwrap();

    // 用途不匹配、令牌不匹配一律拒绝。
    assert!(
        consume_assertion(
            &mut c,
            &f.renderer,
            &identity,
            "assertion-token-0001",
            "other",
            later
        )
        .await
        .is_err()
    );
    assert!(
        consume_assertion(
            &mut c,
            &f.renderer,
            &identity,
            "wrong-token-000000",
            "grant_role",
            later
        )
        .await
        .is_err()
    );
    // 正确消费一次成功，第二次必须失败（单次消费）。
    consume_assertion(
        &mut c,
        &f.renderer,
        &identity,
        "assertion-token-0001",
        "grant_role",
        later,
    )
    .await
    .unwrap();
    assert!(
        consume_assertion(
            &mut c,
            &f.renderer,
            &identity,
            "assertion-token-0001",
            "grant_role",
            later,
        )
        .await
        .is_err()
    );
    // 超过 5 分钟的 assertion 不可再消费。
    issue_assertion(
        &mut c,
        &f.renderer,
        &wrapper,
        &identity,
        &totp_code(
            &secret,
            mfa::step_for((later + Duration::seconds(60)).timestamp()).unwrap(),
        ),
        "grant_role",
        "assertion-token-0002",
        later + Duration::seconds(60),
    )
    .await
    .unwrap();
    assert!(
        consume_assertion(
            &mut c,
            &f.renderer,
            &identity,
            "assertion-token-0002",
            "grant_role",
            later + Duration::seconds(400),
        )
        .await
        .is_err()
    );

    // 恢复码可以代替 TOTP 完成重新认证，且只能用一次。
    let recovery = codes[0].clone();
    issue_assertion(
        &mut c,
        &f.renderer,
        &wrapper,
        &identity,
        &recovery,
        "recover",
        "assertion-token-0003",
        later,
    )
    .await
    .unwrap();
    assert!(
        issue_assertion(
            &mut c,
            &f.renderer,
            &wrapper,
            &identity,
            &recovery,
            "recover",
            "assertion-token-0004",
            later,
        )
        .await
        .is_err()
    );
    tx.commit().await.unwrap();
}

/// 测试辅助：把 Base32 解回字节，用于在测试里算出正确的 TOTP 码。
fn decode_base32(input: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::new();
    for ch in input.bytes() {
        let value = ALPHABET.iter().position(|c| *c == ch).unwrap() as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    out
}
/// 测试辅助：直接算出某个时间步的 TOTP 码。
///
/// 走与实现同一条 `code_at`，因此断言的是「绑定流程能接受合法码」，
/// 而算法本身的正确性由 `mfa::tests` 对 RFC 6238 官方向量的断言保证。
fn totp_code(secret: &[u8], step: i64) -> String {
    format!("{:06}", mfa::code_at(secret, step).unwrap())
}
