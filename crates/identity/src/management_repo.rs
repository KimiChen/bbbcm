//! WP-MVP-B03 管理侧仓储：staff principal、MFA 凭据、恢复码、管理会话与 assertion。
//!
//! 所有函数都借用调用方的事务连接、绝不提交，也不做授权判定——「谁可以调用」由
//! `management` 服务层在同一事务内决定。凭据一律以哈希或密文形式落库。
use crate::contract::IdentityError;
use crate::repo::{table, text};
use chrono::{DateTime, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::{DbValue, TransactionConnection};

/// 未消费/未发生的时间戳统一用 Unix 纪元表示。
///
/// 三库交集子集下用固定哨兵值比 NULL 更好判等，也避免 `IS NULL` 在不同后端上的差异；
/// 与既有 `deleted_at`／`archived_at` 的处理保持一致。
fn never() -> DbValue {
    DbValue::Time(DateTime::UNIX_EPOCH)
}
fn is_never(value: DateTime<Utc>) -> bool {
    value <= DateTime::UNIX_EPOCH
}

#[derive(Clone)]
pub struct StaffPrincipal {
    pub id: i64,
    pub user_id: i64,
    pub active: bool,
    pub version: i64,
}
impl std::fmt::Debug for StaffPrincipal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaffPrincipal").finish_non_exhaustive()
    }
}

/// 系统当前是否已存在任何 staff principal。
///
/// `AUTH-014` 的一次性引导完全建立在这个判断上：只要已经有任何一个（哪怕是 suspended），
/// CLI 提权路径就必须拒绝，否则「一次性」形同虚设。
pub async fn any_staff_exists(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
) -> Result<bool, IdentityError> {
    let t = table(tx, r, "staff_principals")?;
    Ok(tx
        .fetch_optional(&format!("SELECT id FROM {t} ORDER BY id"), &[])
        .await?
        .is_some())
}

pub async fn staff_principal(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<Option<StaffPrincipal>, IdentityError> {
    let t = table(tx, r, "staff_principals")?;
    tx.fetch_optional(
        &format!("SELECT id, user_id, status, version FROM {t} WHERE user_id = ?"),
        &[user_id.into()],
    )
    .await?
    .map(|row| {
        Ok(StaffPrincipal {
            id: row.i64("id")?,
            user_id: row.i64("user_id")?,
            active: row.text("status")? == "active",
            version: row.i64("version")?,
        })
    })
    .transpose()
}

pub async fn insert_staff_principal(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    let t = table(tx, r, "staff_principals")?;
    tx.execute(
        &format!(
            "INSERT INTO {t} (user_id, status, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?)"
        ),
        &[
            user_id.into(),
            text("active"),
            DbValue::Time(now),
            DbValue::Time(now),
            DbValue::I64(1),
        ],
    )
    .await?;
    Ok(())
}

/// 按用户名唯一键查账号 ID。只取 ID，不带出任何资料字段。
pub async fn account_id_by_username_key(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    username_key: &str,
) -> Result<Option<i64>, IdentityError> {
    let t = table(tx, r, "users")?;
    Ok(tx
        .fetch_optional(
            &format!("SELECT id FROM {t} WHERE username_key = ? AND status = ?"),
            &[text(username_key), text("active")],
        )
        .await?
        .map(|row| row.i64("id"))
        .transpose()?)
}

#[derive(Clone)]
pub struct MfaCredential {
    pub id: i64,
    pub cipher: String,
    pub nonce: String,
    pub key_version: i64,
    pub confirmed: bool,
    pub last_step: i64,
    pub version: i64,
}
impl std::fmt::Debug for MfaCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MfaCredential").finish_non_exhaustive()
    }
}

pub async fn mfa_credential(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<Option<MfaCredential>, IdentityError> {
    let t = table(tx, r, "mfa_credentials")?;
    tx.fetch_optional(&format!("SELECT id, secret_cipher, secret_nonce, key_version, confirmed_at, last_step, version FROM {t} WHERE user_id = ?"), &[user_id.into()]).await?
        .map(|row| Ok(MfaCredential {
            id: row.i64("id")?,
            cipher: row.text("secret_cipher")?,
            nonce: row.text("secret_nonce")?,
            key_version: row.i64("key_version")?,
            confirmed: !is_never(row.time("confirmed_at")?),
            last_step: row.i64("last_step")?,
            version: row.i64("version")?,
        }))
        .transpose()
}

/// 写入（或替换）一份未确认的 TOTP 凭据。
///
/// 重新绑定会覆盖旧密钥并把 `confirmed_at` 清回未确认：绑定流程走完之前，旧密钥不再有效，
/// 新密钥也还不能用于登录，避免出现「两个密钥同时可用」的窗口。
pub async fn upsert_mfa_credential(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    cipher: &str,
    nonce: &str,
    key_version: i64,
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    let t = table(tx, r, "mfa_credentials")?;
    if let Some(existing) = mfa_credential(tx, r, user_id).await? {
        tx.execute_versioned(&format!("UPDATE {t} SET secret_cipher = ?, secret_nonce = ?, key_version = ?, confirmed_at = ?, last_step = 0, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?"),
            &[text(cipher), text(nonce), key_version.into(), never(), DbValue::Time(now), existing.id.into(), existing.version.into()]).await?;
        return Ok(());
    }
    tx.execute(&format!("INSERT INTO {t} (user_id, secret_cipher, secret_nonce, key_version, confirmed_at, last_step, created_at, updated_at, version) VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?)"),
        &[user_id.into(), text(cipher), text(nonce), key_version.into(), never(), DbValue::Time(now), DbValue::Time(now), DbValue::I64(1)]).await?;
    Ok(())
}

/// 确认绑定并记录本次使用的时间步。
pub async fn confirm_mfa_credential(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    credential: &MfaCredential,
    step: i64,
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    let t = table(tx, r, "mfa_credentials")?;
    tx.execute_versioned(&format!("UPDATE {t} SET confirmed_at = ?, last_step = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[DbValue::Time(now), step.into(), DbValue::Time(now), credential.id.into(), credential.version.into()]).await?;
    Ok(())
}

/// 只推进时间步。用于已确认凭据的每次成功验签，防止同一时间步被重放。
pub async fn advance_step(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    credential: &MfaCredential,
    step: i64,
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    let t = table(tx, r, "mfa_credentials")?;
    tx.execute_versioned(&format!("UPDATE {t} SET last_step = ?, updated_at = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[step.into(), DbValue::Time(now), credential.id.into(), credential.version.into()]).await?;
    Ok(())
}

/// 替换整组恢复码：先作废旧的未消费码，再写入新组。
///
/// 已消费的记录保留，作为「这个码用过了」的追加事实，不因重新生成而消失。
pub async fn replace_recovery_codes(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    hashes: &[String],
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    let t = table(tx, r, "mfa_recovery_codes")?;
    tx.execute(
        &format!("UPDATE {t} SET consumed_at = ?, version = version + 1 WHERE user_id = ? AND consumed_at = ?"),
        &[DbValue::Time(now), user_id.into(), never()],
    )
    .await?;
    for hash in hashes {
        tx.execute(
            &format!("INSERT INTO {t} (user_id, code_hash, consumed_at, created_at, version) VALUES (?, ?, ?, ?, ?)"),
            &[user_id.into(), text(hash), never(), DbValue::Time(now), DbValue::I64(1)],
        )
        .await?;
    }
    Ok(())
}

/// 消费一个恢复码，成功返回真。已消费或不存在都返回假，调用方对外给统一响应。
pub async fn consume_recovery_code(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    hash: &str,
    now: DateTime<Utc>,
) -> Result<bool, IdentityError> {
    let t = table(tx, r, "mfa_recovery_codes")?;
    let Some(row) = tx
        .fetch_optional(
            &format!(
                "SELECT id, consumed_at, version FROM {t} WHERE user_id = ? AND code_hash = ?"
            ),
            &[user_id.into(), text(hash)],
        )
        .await?
    else {
        return Ok(false);
    };
    if !is_never(row.time("consumed_at")?) {
        return Ok(false);
    }
    tx.execute_versioned(
        &format!(
            "UPDATE {t} SET consumed_at = ?, version = version + 1 WHERE id = ? AND version = ?"
        ),
        &[
            DbValue::Time(now),
            row.i64("id")?.into(),
            row.i64("version")?.into(),
        ],
    )
    .await?;
    Ok(true)
}

#[derive(Clone)]
pub struct ManagementSession {
    pub id: i64,
    pub user_id: i64,
    pub csrf_hash: String,
    pub last_seen_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
    pub mfa_bound: bool,
    pub version: i64,
}
impl std::fmt::Debug for ManagementSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagementSession").finish_non_exhaustive()
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_management_session(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    session_hash: &str,
    csrf_hash: &str,
    mfa_bound: bool,
    absolute_expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    let t = table(tx, r, "management_sessions")?;
    tx.execute(&format!("INSERT INTO {t} (session_hash, csrf_hash, user_id, created_at, last_seen_at, absolute_expires_at, mfa_bound, revoked, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"),
        &[text(session_hash), text(csrf_hash), user_id.into(), DbValue::Time(now), DbValue::Time(now), DbValue::Time(absolute_expires_at), DbValue::Bool(mfa_bound), DbValue::Bool(false), DbValue::I64(1)]).await?;
    Ok(())
}

pub async fn read_management_session(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    session_hash: &str,
) -> Result<Option<ManagementSession>, IdentityError> {
    let t = table(tx, r, "management_sessions")?;
    tx.fetch_optional(&format!("SELECT id, user_id, csrf_hash, last_seen_at, absolute_expires_at, mfa_bound, version FROM {t} WHERE session_hash = ? AND revoked = ?"), &[text(session_hash), DbValue::Bool(false)]).await?
        .map(|row| Ok(ManagementSession {
            id: row.i64("id")?,
            user_id: row.i64("user_id")?,
            csrf_hash: row.text("csrf_hash")?,
            last_seen_at: row.time("last_seen_at")?,
            absolute_expires_at: row.time("absolute_expires_at")?,
            mfa_bound: row.boolean("mfa_bound")?,
            version: row.i64("version")?,
        }))
        .transpose()
}

/// 按主键重读一条未撤销的管理会话。
///
/// 绑定确认后需要用**当前**版本号去更新会话，不能沿用请求入口时读到的旧版本，
/// 否则中间那次空闲刷新会让版本 CAS 失败。
pub async fn read_management_session_by_id(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
) -> Result<Option<ManagementSession>, IdentityError> {
    let t = table(tx, r, "management_sessions")?;
    tx.fetch_optional(&format!("SELECT id, user_id, csrf_hash, last_seen_at, absolute_expires_at, mfa_bound, version FROM {t} WHERE id = ? AND revoked = ?"), &[id.into(), DbValue::Bool(false)]).await?
        .map(|row| Ok(ManagementSession {
            id: row.i64("id")?,
            user_id: row.i64("user_id")?,
            csrf_hash: row.text("csrf_hash")?,
            last_seen_at: row.time("last_seen_at")?,
            absolute_expires_at: row.time("absolute_expires_at")?,
            mfa_bound: row.boolean("mfa_bound")?,
            version: row.i64("version")?,
        }))
        .transpose()
}

/// 刷新空闲计时，并在需要时标记 MFA 已绑定。
pub async fn touch_management_session(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    session: &ManagementSession,
    mfa_bound: bool,
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    let t = table(tx, r, "management_sessions")?;
    tx.execute_versioned(&format!("UPDATE {t} SET last_seen_at = ?, mfa_bound = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[DbValue::Time(now), DbValue::Bool(mfa_bound), session.id.into(), session.version.into()]).await?;
    Ok(())
}

/// 撤销某账号的全部管理会话。改密码、改邮箱、MFA 变更或账号状态变更时调用。
pub async fn revoke_management_sessions(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<(), IdentityError> {
    let t = table(tx, r, "management_sessions")?;
    tx.execute(
        &format!(
            "UPDATE {t} SET revoked = ?, version = version + 1 WHERE user_id = ? AND revoked = ?"
        ),
        &[DbValue::Bool(true), user_id.into(), DbValue::Bool(false)],
    )
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_assertion(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    hash: &str,
    user_id: i64,
    session_id: i64,
    purpose: &str,
    expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), IdentityError> {
    let t = table(tx, r, "mfa_assertions")?;
    tx.execute(&format!("INSERT INTO {t} (assertion_hash, user_id, management_session_id, purpose, expires_at, consumed_at, created_at, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"),
        &[text(hash), user_id.into(), session_id.into(), text(purpose), DbValue::Time(expires_at), never(), DbValue::Time(now), DbValue::I64(1)]).await?;
    Ok(())
}

/// 消费一条 assertion。用户、会话、用途、过期与是否已消费全部匹配才算成功。
///
/// 四项绑定缺一不可：只校验用户会让「改密码时做的验证」被拿去授予特权角色；
/// 只校验用途会让另一台设备上的会话复用这次验证。
pub async fn consume_assertion(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    hash: &str,
    user_id: i64,
    session_id: i64,
    purpose: &str,
    now: DateTime<Utc>,
) -> Result<bool, IdentityError> {
    let t = table(tx, r, "mfa_assertions")?;
    let Some(row) = tx.fetch_optional(&format!("SELECT id, user_id, management_session_id, purpose, expires_at, consumed_at, version FROM {t} WHERE assertion_hash = ?"), &[text(hash)]).await? else {
        return Ok(false);
    };
    if row.i64("user_id")? != user_id
        || row.i64("management_session_id")? != session_id
        || row.text("purpose")? != purpose
        || !is_never(row.time("consumed_at")?)
        || row.time("expires_at")? <= now
    {
        return Ok(false);
    }
    tx.execute_versioned(
        &format!(
            "UPDATE {t} SET consumed_at = ?, version = version + 1 WHERE id = ? AND version = ?"
        ),
        &[
            DbValue::Time(now),
            row.i64("id")?.into(),
            row.i64("version")?.into(),
        ],
    )
    .await?;
    Ok(true)
}
