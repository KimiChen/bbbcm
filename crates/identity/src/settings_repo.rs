//! 资料与账号设置的受限存储访问；事务与授权由设置 core 持有。
use crate::contract::{Account, NormalizedIdentity, USERNAME_RESERVATION_DAYS};
use crate::repo::{table, text};
use crate::settings::{DeviceSummary, PublicProfile};
use chrono::{DateTime, Duration, Utc};
use platform::identifiers::SchemaRenderer;
use platform_db::transaction::{DbValue, TransactionConnection, TransactionError};

pub(crate) async fn public_profile(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    public_id: &str,
) -> Result<Option<(i64, PublicProfile)>, TransactionError> {
    let users = table(tx, renderer, "users")?;
    tx.fetch_optional(
        &format!("SELECT id, public_id, username, bio, created_at FROM {users} WHERE public_id = ? AND status = ?"),
        &[text(public_id), text("active")],
    ).await?.map(|row| Ok((row.i64("id")?, PublicProfile {
        public_ref: row.text("public_id")?,
        username: row.text("username")?,
        bio: row.text("bio")?,
        joined_at: row.time("created_at")?,
        avatar_preset: "default",
        content_data_available: false,
        trust_level: None,
        role_data_available: false,
    }))).transpose()
}

pub(crate) async fn update_bio(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    account: &Account,
    bio: &str,
) -> Result<(), TransactionError> {
    let users = table(tx, renderer, "users")?;
    tx.execute_versioned(
        &format!("UPDATE {users} SET bio = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[text(bio), account.id.into(), account.version.into()],
    )
    .await
}

/// 当前名和历史保留名共用唯一键；新增占用的唯一冲突及CAS失效均回滚整次变更。
/// 历史记录只追加，过期只解除占用，不清除审计事实。
pub(crate) async fn change_username(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    account: &Account,
    username: &NormalizedIdentity,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    let claims = table(tx, renderer, "username_claims")?;
    let history = table(tx, renderer, "username_history")?;
    let users = table(tx, renderer, "users")?;
    let reserved_until = now + Duration::days(USERNAME_RESERVATION_DAYS);
    if username.key == account.username_key {
        return Err(TransactionError::InvalidInput);
    }
    let existing = tx
        .fetch_optional(
            &format!("SELECT kind, reserved_until, version FROM {claims} WHERE username_key = ?"),
            &[text(&username.key)],
        )
        .await?;
    if let Some(existing) = existing {
        // A current claim is never expired. Historical claims retain their owner for 90 days.
        if existing.text("kind")? != "history" || existing.time("reserved_until")? > now {
            return Err(TransactionError::InvalidInput);
        }
        tx.execute_versioned(
            &format!("UPDATE {claims} SET user_id = ?, kind = ?, reserved_until = ?, version = version + 1 WHERE username_key = ? AND version = ?"),
            &[account.id.into(), text("current"), DbValue::Time(now), text(&username.key), existing.i64("version")?.into()],
        ).await?;
    } else {
        tx.execute(
            &format!("INSERT INTO {claims} (username_key, user_id, kind, reserved_until, version) VALUES (?, ?, ?, ?, ?)"),
            &[text(&username.key), account.id.into(), text("current"), DbValue::Time(now), 1_i64.into()],
        ).await?;
    }
    tx.execute_versioned(
        &format!("UPDATE {claims} SET kind = ?, reserved_until = ?, version = version + 1 WHERE username_key = ? AND user_id = ? AND kind = ?"),
        &[text("history"), DbValue::Time(reserved_until), text(&account.username_key), account.id.into(), text("current")],
    ).await?;
    tx.execute(
        &format!("INSERT INTO {history} (username_key, user_id, released_at, reserved_until) VALUES (?, ?, ?, ?)"),
        &[text(&account.username_key), account.id.into(), DbValue::Time(now), DbValue::Time(reserved_until)],
    ).await?;
    tx.execute_versioned(
        &format!("UPDATE {users} SET username = ?, username_key = ?, username_changed_at = ?, version = version + 1 WHERE id = ? AND version = ?"),
        &[text(&username.display), text(&username.key), DbValue::Time(now), account.id.into(), account.version.into()],
    ).await
}

pub(crate) async fn change_password(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    account: &Account,
    password_hash: &str,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    let users = table(tx, renderer, "users")?;
    let credentials = table(tx, renderer, "credentials")?;
    tx.execute_versioned(
        &format!("UPDATE {users} SET credential_version = credential_version + 1, version = version + 1 WHERE id = ? AND version = ? AND credential_version = ?"),
        &[account.id.into(), account.version.into(), account.credential_version.into()],
    ).await?;
    tx.execute_versioned(
        &format!("UPDATE {credentials} SET password_hash = ?, updated_at = ? WHERE user_id = ?"),
        &[text(password_hash), DbValue::Time(now), account.id.into()],
    )
    .await
}

pub(crate) async fn change_email(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    account: &Account,
    email: &str,
    email_key: &str,
) -> Result<(), TransactionError> {
    let users = table(tx, renderer, "users")?;
    tx.execute_versioned(
        &format!("UPDATE {users} SET email = ?, email_key = ?, email_verified = ?, credential_version = credential_version + 1, version = version + 1 WHERE id = ? AND version = ? AND credential_version = ?"),
        &[text(email), text(email_key), DbValue::Bool(true), account.id.into(), account.version.into(), account.credential_version.into()],
    ).await
}

pub(crate) async fn touch_version(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    account: &Account,
) -> Result<(), TransactionError> {
    let users = table(tx, renderer, "users")?;
    tx.execute_versioned(
        &format!("UPDATE {users} SET version = version + 1 WHERE id = ? AND version = ?"),
        &[account.id.into(), account.version.into()],
    )
    .await
}

pub(crate) async fn devices(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
    current_session_id: i64,
    now: DateTime<Utc>,
) -> Result<Vec<DeviceSummary>, TransactionError> {
    let sessions = table(tx, renderer, "sessions")?;
    let rows = tx.fetch_all(
        &format!("SELECT id, session_ref, device_summary, created_at, last_seen_at FROM {sessions} WHERE user_id = ? AND revoked = ? AND absolute_expires_at > ? AND idle_expires_at > ? ORDER BY last_seen_at DESC, id DESC"),
        &[user_id.into(), DbValue::Bool(false), DbValue::Time(now), DbValue::Time(now)],
    ).await?;
    rows.into_iter()
        .map(|row| {
            Ok(DeviceSummary {
                session_ref: row.text("session_ref")?,
                summary: row.text("device_summary")?,
                created_at: row.time("created_at")?,
                last_seen_at: row.time("last_seen_at")?,
                current: row.i64("id")? == current_session_id,
            })
        })
        .collect()
}

/// 新请求替换待确认的新邮箱，不改变当前邮箱或撤销当前会话。
pub(crate) async fn invalidate_email_requests(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    user_id: i64,
) -> Result<(), TransactionError> {
    let tokens = table(tx, renderer, "one_time_tokens")?;
    tx.execute(
        &format!(
            "UPDATE {tokens} SET consumed = ? WHERE user_id = ? AND purpose = ? AND consumed = ?"
        ),
        &[
            DbValue::Bool(true),
            user_id.into(),
            text("change_email"),
            DbValue::Bool(false),
        ],
    )
    .await?;
    Ok(())
}

/// 查归属只用于先形成完整账号锁计划。消费状态在取得账号锁后再次检查。
pub(crate) async fn email_change_owner(
    tx: &mut TransactionConnection<'_>,
    renderer: &SchemaRenderer,
    token_hash: &str,
) -> Result<Option<i64>, TransactionError> {
    let tokens = table(tx, renderer, "one_time_tokens")?;
    tx.fetch_optional(
        &format!("SELECT user_id FROM {tokens} WHERE token_hash = ? AND purpose = ?"),
        &[text(token_hash), text("change_email")],
    )
    .await?
    .map(|row| row.i64("user_id"))
    .transpose()
}
