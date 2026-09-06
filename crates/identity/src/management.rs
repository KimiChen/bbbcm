//! WP-MVP-B03 管理侧服务：MFA 绑定与验签、管理会话、重新认证与首账号引导。
//!
//! 边界：本模块只负责「是不是本人、MFA 过没过、管理会话有没有效」。**具体能做什么**
//! 由 `WP-MVP-D04` 的权限 registry 判定，本模块不持有任何角色或权限枚举，
//! 也不因为「是 staff」就放行任何业务动作。
use crate::{
    contract::IdentityError,
    core::{normalized_now, secret_hash},
    management_repo as repo, mfa,
};
use chrono::{DateTime, Duration, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::TransactionConnection;

/// 03 §6：管理会话最长 8 小时。
pub const MANAGEMENT_ABSOLUTE_HOURS: i64 = 8;
/// 03 §6：管理会话空闲 15 分钟即失效。
pub const MANAGEMENT_IDLE_MINUTES: i64 = 15;
/// 03 §6：高危操作要求 5 分钟内重新认证。
pub const ASSERTION_TTL_SECONDS: i64 = 300;

/// 管理会话在本次请求中的可用状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagementAccess {
    /// 会话有效且已完成 MFA 绑定，可进入正常管理流程。
    Ready,
    /// 会话有效但尚未完成 TOTP 绑定。
    ///
    /// `AUTH-014` 要求此时只能访问绑定流程：调用方必须据此把其余管理路由一律拒绝，
    /// 而不是仅在前端隐藏入口。
    BindingRequired,
}

/// 一次通过校验的管理会话。
#[derive(Clone)]
pub struct ManagementIdentity {
    pub user_id: i64,
    pub session_id: i64,
    pub access: ManagementAccess,
    csrf_hash: String,
}
impl std::fmt::Debug for ManagementIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagementIdentity([REDACTED])")
    }
}
impl ManagementIdentity {
    /// 校验管理侧 CSRF。与普通会话各自独立，两边的令牌不可互换。
    pub fn check_csrf(&self, submitted: &str) -> Result<(), IdentityError> {
        if crate::core::constant_equal(&secret_hash(submitted), &self.csrf_hash) {
            Ok(())
        } else {
            Err(IdentityError::Csrf)
        }
    }
}

/// 校验一次管理会话，并在通过时刷新空闲计时。
///
/// 每次请求都重新读库校验（`03` §4），不接受任何缓存的判定结果：
///   * 会话必须存在且未撤销；
///   * 未超过 8 小时绝对上限；
///   * 距上次活动未超过 15 分钟；
///   * 对应账号必须仍是 active 的 staff principal——普通账号的会话永远进不来。
///
/// 任何一项不满足都返回 `Unauthenticated`，不区分具体原因，避免枚举探测。
pub async fn authorize_management_session(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    raw_session: &str,
    now: DateTime<Utc>,
) -> Result<ManagementIdentity, IdentityError> {
    let now = normalized_now(now);
    let session = repo::read_management_session(tx, r, &secret_hash(raw_session))
        .await?
        .ok_or(IdentityError::Unauthenticated)?;
    let idle_deadline = session
        .last_seen_at
        .checked_add_signed(
            Duration::try_minutes(MANAGEMENT_IDLE_MINUTES).ok_or(IdentityError::Unavailable)?,
        )
        .ok_or(IdentityError::Unavailable)?;
    if now >= session.absolute_expires_at || now >= idle_deadline {
        return Err(IdentityError::Unauthenticated);
    }
    let principal = repo::staff_principal(tx, r, session.user_id)
        .await?
        .ok_or(IdentityError::Unauthenticated)?;
    if !principal.active {
        return Err(IdentityError::Unauthenticated);
    }
    repo::touch_management_session(tx, r, &session, session.mfa_bound, now).await?;
    Ok(ManagementIdentity {
        user_id: session.user_id,
        session_id: session.id,
        access: if session.mfa_bound {
            ManagementAccess::Ready
        } else {
            ManagementAccess::BindingRequired
        },
        csrf_hash: session.csrf_hash,
    })
}

/// 开启一个管理会话。
///
/// 调用方必须在同一事务内先完成密码校验；本函数只负责建立会话本身。
/// 尚未确认 TOTP 的账号也能建立会话，但会落在 `BindingRequired` 状态，
/// 只能走绑定流程——这正是 `AUTH-014` 对首账号的要求。
pub async fn open_management_session(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    raw_session: &str,
    raw_csrf: &str,
    now: DateTime<Utc>,
) -> Result<ManagementAccess, IdentityError> {
    let now = normalized_now(now);
    let principal = repo::staff_principal(tx, r, user_id)
        .await?
        .ok_or(IdentityError::Unauthenticated)?;
    if !principal.active {
        return Err(IdentityError::Unauthenticated);
    }
    let bound = repo::mfa_credential(tx, r, user_id)
        .await?
        .is_some_and(|c| c.confirmed);
    let expires = now
        .checked_add_signed(
            Duration::try_hours(MANAGEMENT_ABSOLUTE_HOURS).ok_or(IdentityError::Unavailable)?,
        )
        .ok_or(IdentityError::Unavailable)?;
    repo::create_management_session(
        tx,
        r,
        user_id,
        &secret_hash(raw_session),
        &secret_hash(raw_csrf),
        bound,
        expires,
        now,
    )
    .await?;
    Ok(if bound {
        ManagementAccess::Ready
    } else {
        ManagementAccess::BindingRequired
    })
}

/// 开始一次 TOTP 绑定，返回给用户扫码的 Base32 密钥与 otpauth URI。
///
/// 明文密钥只在本函数返回值里出现一次，供绑定界面展示；库中只落密文（`DEC-062`）。
/// 调用方不得把它写进日志、审计或任何持久化结构。
pub async fn begin_mfa_binding(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    wrapper: &mfa::SecretWrapper,
    user_id: i64,
    account_label: &str,
    now: DateTime<Utc>,
) -> Result<(String, String), IdentityError> {
    let now = normalized_now(now);
    let secret = mfa::generate_secret();
    let (cipher, nonce) = wrapper.seal(&secret)?;
    repo::upsert_mfa_credential(tx, r, user_id, &cipher, &nonce, wrapper.version(), now).await?;
    let encoded = mfa::base32(&secret);
    // otpauth URI 只放 Base32 密钥与标签，不含邮箱等可识别信息之外的任何账号数据。
    let uri = format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
        "草莓论坛",
        account_label,
        encoded,
        "草莓论坛",
        mfa::TOTP_DIGITS,
        mfa::TOTP_STEP_SECONDS
    );
    Ok((encoded, uri))
}

/// 用一枚验证码确认绑定，成功时生成并返回一次性恢复码。
///
/// 恢复码明文只在这里返回一次（`03` §6「只显示一次」），库中只存哈希。
/// 确认成功会把当前管理会话标记为已绑定，使它从 `BindingRequired` 转为 `Ready`。
pub async fn confirm_mfa_binding(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    wrapper: &mfa::SecretWrapper,
    identity: &ManagementIdentity,
    code: &str,
    now: DateTime<Utc>,
) -> Result<Vec<String>, IdentityError> {
    let now = normalized_now(now);
    let credential = repo::mfa_credential(tx, r, identity.user_id)
        .await?
        .ok_or(IdentityError::Rejected)?;
    let secret = wrapper.open(&credential.cipher, &credential.nonce)?;
    let step = mfa::verify(&secret, code, mfa::step_for(now.timestamp())?)?;
    repo::confirm_mfa_credential(tx, r, &credential, step, now).await?;
    let codes = mfa::generate_recovery_codes();
    let hashes: Vec<String> = codes.iter().map(|c| secret_hash(c)).collect();
    repo::replace_recovery_codes(tx, r, identity.user_id, &hashes, now).await?;
    let session = repo::read_management_session_by_id(tx, r, identity.session_id)
        .await?
        .ok_or(IdentityError::Unauthenticated)?;
    repo::touch_management_session(tx, r, &session, true, now).await?;
    Ok(codes)
}

/// 用 TOTP 或恢复码完成一次重新认证，签发一枚绑定用途的 assertion。
///
/// 返回的原始 assertion 只在内存中传递给调用方，库中只存哈希；5 分钟过期、单次消费，
/// 并同时绑定用户、管理会话与用途（`03` §6）。
#[allow(clippy::too_many_arguments)]
pub async fn issue_assertion(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    wrapper: &mfa::SecretWrapper,
    identity: &ManagementIdentity,
    code: &str,
    purpose: &str,
    raw_assertion: &str,
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    let now = normalized_now(now);
    if purpose.is_empty() || purpose.len() > 64 || raw_assertion.len() < 16 {
        return Err(IdentityError::Rejected);
    }
    let credential = repo::mfa_credential(tx, r, identity.user_id)
        .await?
        .filter(|c| c.confirmed)
        .ok_or(IdentityError::Rejected)?;
    let secret = wrapper.open(&credential.cipher, &credential.nonce)?;
    match mfa::verify(&secret, code, mfa::step_for(now.timestamp())?) {
        Ok(step) => {
            // 同一时间步只允许成功一次：旁观者看到的验证码不能在 30 秒内被重放。
            if step <= credential.last_step {
                return Err(IdentityError::Rejected);
            }
            repo::advance_step(tx, r, &credential, step, now).await?;
        }
        Err(_) => {
            // TOTP 不通过时退到恢复码。两条路径的失败对外返回同一个错误，
            // 避免从响应差异推断某个码是否存在。
            if !repo::consume_recovery_code(tx, r, identity.user_id, &secret_hash(code), now)
                .await?
            {
                return Err(IdentityError::Rejected);
            }
        }
    }
    let expires = now
        .checked_add_signed(
            Duration::try_seconds(ASSERTION_TTL_SECONDS).ok_or(IdentityError::Unavailable)?,
        )
        .ok_or(IdentityError::Unavailable)?;
    repo::insert_assertion(
        tx,
        r,
        &secret_hash(raw_assertion),
        identity.user_id,
        identity.session_id,
        purpose,
        expires,
        now,
    )
    .await
}

/// 消费一枚 assertion，用于放行一次高危操作。
pub async fn consume_assertion(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    identity: &ManagementIdentity,
    raw_assertion: &str,
    purpose: &str,
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    let now = normalized_now(now);
    if repo::consume_assertion(
        tx,
        r,
        &secret_hash(raw_assertion),
        identity.user_id,
        identity.session_id,
        purpose,
        now,
    )
    .await?
    {
        Ok(())
    } else {
        Err(IdentityError::Rejected)
    }
}

/// 按用户名查出账号 ID，供 `AUTH-014` 的 CLI 引导定位目标账号。
///
/// 只返回 ID，不返回任何资料字段：CLI 引导只需要知道「提权给哪个账号」。
/// 用户名按既有规范化规则统一小写比较，与注册时的唯一键一致。
pub async fn account_id_by_username(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    username: &str,
) -> Result<Option<i64>, IdentityError> {
    repo::account_id_by_username_key(tx, r, &username.to_lowercase()).await
}

/// `AUTH-014`：一次性创建首个站点所有者。
///
/// 只有在系统尚无任何 staff principal 时才允许执行；已存在时必须拒绝，
/// 否则「一次性引导」就退化成一条常驻提权后门。调用方负责在同一事务内追加审计与告警。
pub async fn bootstrap_first_owner(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    if repo::any_staff_exists(tx, r).await? {
        return Err(IdentityError::Rejected);
    }
    repo::insert_staff_principal(tx, r, user_id, normalized_now(now)).await
}

#[cfg(test)]
mod tests;
