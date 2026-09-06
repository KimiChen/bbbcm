//! SQL只借用当前事务；不拥有连接池或提交能力。
use crate::{
    contract::*,
    core::{constant_equal, secret_hash},
};
use chrono::{DateTime, Duration, Utc};
use platform::{db::Backend, identifiers::SchemaRenderer};
use platform_db::transaction::{DbRow, DbValue, TransactionConnection, TransactionError};
pub(crate) fn table(
    tx: &TransactionConnection<'_>,
    r: &SchemaRenderer,
    name: &str,
) -> Result<String, TransactionError> {
    Ok(r.table(tx.backend(), "identity", name)?)
}
pub(crate) fn text(v: &str) -> DbValue {
    DbValue::Text(v.into())
}
pub(crate) fn decode_account(row: &DbRow) -> Result<Account, TransactionError> {
    Ok(Account {
        id: row.i64("id")?,
        public_id: row.text("public_id")?,
        username: row.text("username")?,
        username_key: row.text("username_key")?,
        email: row.text("email")?,
        email_key: row.text("email_key")?,
        status: row.text("status")?,
        email_verified: row.boolean("email_verified")?,
        bio: row.text("bio")?,
        created_at: row.time("created_at")?,
        username_changed_at: row.time("username_changed_at")?,
        credential_version: row.i64("credential_version")?,
        // PRIV-001/002 的逐用户开关。列有非空默认值，缺失即视为数据损坏而不是静默放行。
        profile_lists_public: row.boolean("profile_lists_public")?,
        topic_body_public: row.boolean("topic_body_public")?,
        version: row.i64("version")?,
    })
}
pub(crate) async fn account_by_id(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
) -> Result<Option<Account>, TransactionError> {
    let t = table(tx, r, "users")?;
    tx.fetch_optional(&format!("SELECT id, public_id, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, profile_lists_public, topic_body_public, version FROM {t} WHERE id = ?"),&[id.into()]).await?.as_ref().map(decode_account).transpose()
}
pub(crate) async fn account_by_identifier(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    key: &str,
    email: bool,
) -> Result<Option<Account>, TransactionError> {
    let t = table(tx, r, "users")?;
    let sql = if email {
        format!(
            "SELECT id, public_id, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, profile_lists_public, topic_body_public, version FROM {t} WHERE email_key = ?"
        )
    } else {
        format!(
            "SELECT id, public_id, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, profile_lists_public, topic_body_public, version FROM {t} WHERE username_key = ?"
        )
    };
    tx.fetch_optional(&sql, &[text(key)])
        .await?
        .as_ref()
        .map(decode_account)
        .transpose()
}
pub(crate) async fn password_hash(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
) -> Result<Option<String>, TransactionError> {
    let t = table(tx, r, "credentials")?;
    tx.fetch_optional(
        &format!("SELECT password_hash FROM {t} WHERE user_id = ?"),
        &[id.into()],
    )
    .await?
    .map(|row| row.text("password_hash"))
    .transpose()
}
pub(crate) async fn create_principal(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    hash: &str,
    csrf: &str,
    now: DateTime<Utc>,
    expires: DateTime<Utc>,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "request_principals")?;
    tx.execute(
        &format!(
            "INSERT INTO {t} (token_hash, csrf_hash, created_at, expires_at) VALUES (?, ?, ?, ?)"
        ),
        &[
            text(hash),
            text(csrf),
            DbValue::Time(now),
            DbValue::Time(expires),
        ],
    )
    .await?;
    Ok(())
}
pub(crate) async fn principal(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    context: &RequestContext,
    now: DateTime<Utc>,
) -> Result<i64, TransactionError> {
    let t = table(tx, r, "request_principals")?;
    let row = tx
        .fetch_optional(
            &format!("SELECT id, csrf_hash, expires_at FROM {t} WHERE token_hash = ?"),
            &[text(&secret_hash(context.preauth.expose()))],
        )
        .await?
        .ok_or(TransactionError::InvalidInput)?;
    if row.time("expires_at")? <= now
        || !constant_equal(&row.text("csrf_hash")?, &secret_hash(context.csrf.expose()))
    {
        return Err(TransactionError::InvalidInput);
    }
    row.i64("id")
}
pub(crate) async fn require_session(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    session: &SessionIdentity,
    now: DateTime<Utc>,
    reauth: bool,
) -> Result<Account, TransactionError> {
    let t = table(tx, r, "sessions")?;
    let row=tx.fetch_optional(&format!("SELECT user_id, credential_version, csrf_hash, absolute_expires_at, idle_expires_at, reauthenticated_at, revoked FROM {t} WHERE id = ? AND session_ref = ?"),&[session.session_id.into(),text(&session.session_ref)]).await?.ok_or(TransactionError::InvalidInput)?;
    let account = account_by_id(tx, r, session.user_id)
        .await?
        .ok_or(TransactionError::InvalidInput)?;
    if row.i64("user_id")? != session.user_id
        || row.boolean("revoked")?
        || account.status != "active"
        || account.credential_version != row.i64("credential_version")?
        || session.credential_version != account.credential_version
        || row.time("absolute_expires_at")? <= now
        || row.time("idle_expires_at")? <= now
        || !constant_equal(&row.text("csrf_hash")?, &session.csrf_hash)
        || (reauth
            && (row.time("reauthenticated_at")? > now
                || now - row.time("reauthenticated_at")? > Duration::seconds(REAUTH_SECONDS)))
    {
        return Err(TransactionError::InvalidInput);
    }
    Ok(account)
}
/// Read-only authentication for a caller-owned transaction; never extends a session.
pub(crate) async fn read_session(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    hash: &str,
    now: DateTime<Utc>,
) -> Result<Option<SessionIdentity>, TransactionError> {
    let t = table(tx, r, "sessions")?;
    let row=tx.fetch_optional(&format!("SELECT id, user_id, session_ref, credential_version, csrf_hash, reauthenticated_at FROM {t} WHERE token_hash = ?"),&[text(hash)]).await?;
    let Some(row) = row else { return Ok(None) };
    let session = SessionIdentity {
        user_id: row.i64("user_id")?,
        session_id: row.i64("id")?,
        session_ref: row.text("session_ref")?,
        credential_version: row.i64("credential_version")?,
        csrf_hash: row.text("csrf_hash")?,
        reauthenticated_at: row.time("reauthenticated_at")?,
    };
    match require_session(tx, r, &session, now, false).await {
        Ok(_) => {}
        Err(TransactionError::InvalidInput) => return Ok(None),
        Err(e) => return Err(e),
    }
    Ok(Some(session))
}

pub(crate) async fn authenticate(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    hash: &str,
    now: DateTime<Utc>,
) -> Result<Option<SessionIdentity>, TransactionError> {
    let Some(session) = read_session(tx, r, hash, now).await? else {
        return Ok(None);
    };
    let t = table(tx, r, "sessions")?;
    let changed = tx
        .execute(
            &format!(
                "UPDATE {t} SET last_seen_at = ?, idle_expires_at = ? WHERE id = ? AND revoked = ?"
            ),
            &[
                DbValue::Time(now),
                DbValue::Time(now + Duration::days(SESSION_IDLE_DAYS)),
                session.session_id.into(),
                DbValue::Bool(false),
            ],
        )
        .await?;
    Ok((changed == 1).then_some(session))
}
pub(crate) async fn revoke_sessions(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    keep: Option<i64>,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "sessions")?;
    match keep {
        Some(id) => {
            tx.execute(
                &format!("UPDATE {t} SET revoked = ? WHERE user_id = ? AND id <> ?"),
                &[DbValue::Bool(true), user_id.into(), id.into()],
            )
            .await?;
        }
        None => {
            tx.execute(
                &format!("UPDATE {t} SET revoked = ? WHERE user_id = ?"),
                &[DbValue::Bool(true), user_id.into()],
            )
            .await?;
        }
    }
    Ok(())
}
pub(crate) async fn revoke_one(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    reference: &str,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "sessions")?;
    let count = tx
        .execute(
            &format!("UPDATE {t} SET revoked = ? WHERE user_id = ? AND session_ref = ?"),
            &[DbValue::Bool(true), user_id.into(), text(reference)],
        )
        .await?;
    if count != 1 {
        return Err(TransactionError::InvalidInput);
    }
    Ok(())
}
pub(crate) async fn set_reauthenticated(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "sessions")?;
    tx.execute(
        &format!("UPDATE {t} SET reauthenticated_at = ? WHERE id = ?"),
        &[DbValue::Time(now), id.into()],
    )
    .await?;
    Ok(())
}
pub(crate) async fn new_id(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    name: &str,
    key: &str,
    value: &str,
) -> Result<i64, TransactionError> {
    match tx.backend() {
        Backend::Sqlite => tx.last_insert_id().await,
        _ => {
            let t = table(tx, r, name)?;
            let sql = match key {
                "public_id" => format!("SELECT id FROM {t} WHERE public_id = ?"),
                "session_ref" => format!("SELECT id FROM {t} WHERE session_ref = ?"),
                "invite_ref" => format!("SELECT id FROM {t} WHERE invite_ref = ?"),
                _ => return Err(TransactionError::InvalidInput),
            };
            tx.fetch_optional(&sql, &[text(value)])
                .await?
                .ok_or(TransactionError::CorruptData)?
                .i64("id")
        }
    }
}
#[derive(Clone)]
pub(crate) struct TokenRecord {
    pub token_id: String,
    pub user_id: i64,
    pub target_email: String,
    pub target_email_key: String,
    pub expires_at: DateTime<Utc>,
    pub issued_credential_version: i64,
}
pub(crate) async fn token_by_hash(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    hash: &str,
    purpose: &str,
    now: DateTime<Utc>,
) -> Result<Option<TokenRecord>, TransactionError> {
    let t = table(tx, r, "one_time_tokens")?;
    let row=tx.fetch_optional(&format!("SELECT token_id, user_id, target_email, target_email_key, expires_at, issued_credential_version FROM {t} WHERE token_hash = ? AND purpose = ? AND consumed = ? AND expires_at > ?"),&[text(hash),text(purpose),DbValue::Bool(false),DbValue::Time(now)]).await?;
    row.map(|row| {
        Ok(TokenRecord {
            token_id: row.text("token_id")?,
            user_id: row.i64("user_id")?,
            target_email: row.text("target_email")?,
            target_email_key: row.text("target_email_key")?,
            expires_at: row.time("expires_at")?,
            issued_credential_version: row.i64("issued_credential_version")?,
        })
    })
    .transpose()
}
pub(crate) async fn consume_token(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: &str,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "one_time_tokens")?;
    if tx
        .execute(
            &format!(
                "UPDATE {t} SET consumed = ? WHERE token_id = ? AND consumed = ? AND expires_at > ?"
            ),
            &[
                DbValue::Bool(true),
                text(id),
                DbValue::Bool(false),
                DbValue::Time(now),
            ],
        )
        .await?
        != 1
    {
        return Err(TransactionError::InvalidInput);
    }
    Ok(())
}
pub(crate) async fn invalidate_tokens(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "one_time_tokens")?;
    tx.execute(
        &format!("UPDATE {t} SET consumed = ? WHERE user_id = ?"),
        &[DbValue::Bool(true), user_id.into()],
    )
    .await?;
    Ok(())
}
pub(crate) async fn invalidate_token_purpose(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    purpose: &str,
) -> Result<(), TransactionError> {
    if !matches!(purpose, "verify_email" | "reset_password" | "change_email") {
        return Err(TransactionError::InvalidInput);
    }
    let t = table(tx, r, "one_time_tokens")?;
    tx.execute(
        &format!("UPDATE {t} SET consumed = ? WHERE user_id = ? AND purpose = ? AND consumed = ?"),
        &[
            DbValue::Bool(true),
            user_id.into(),
            text(purpose),
            DbValue::Bool(false),
        ],
    )
    .await?;
    Ok(())
}
#[derive(Clone)]
pub(crate) struct Invite {
    pub id: i64,
    pub creator_id: i64,
    pub state: String,
    pub expires_at: DateTime<Utc>,
}
pub(crate) async fn invite_by_hash(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    hash: &str,
) -> Result<Option<Invite>, TransactionError> {
    let t = table(tx, r, "invitations")?;
    tx.fetch_optional(
        &format!("SELECT id, creator_id, state, expires_at FROM {t} WHERE code_hash = ?"),
        &[text(hash)],
    )
    .await?
    .map(|row| {
        Ok(Invite {
            id: row.i64("id")?,
            creator_id: row.i64("creator_id")?,
            state: row.text("state")?,
            expires_at: row.time("expires_at")?,
        })
    })
    .transpose()
}
pub(crate) async fn consume_invitation(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
    user_id: i64,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "invitations")?;
    if tx.execute(&format!("UPDATE {t} SET state = ?, consumer_id = ?, consumed_at = ? WHERE id = ? AND state = ? AND expires_at > ?"),&[text("used"),user_id.into(),DbValue::Time(now),id.into(),text("available"),DbValue::Time(now)]).await?!=1{return Err(TransactionError::InvalidInput)}
    Ok(())
}
pub(crate) async fn assert_unlocked(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    flag: platform::flags::Flag,
) -> Result<(), TransactionError> {
    let t = r.table(tx.backend(), "platform", "feature_flags")?;
    if let Some(row) = tx
        .fetch_optional(
            &format!("SELECT enabled FROM {t} WHERE flag_key = ?"),
            &[text(flag.key())],
        )
        .await?
        && row.boolean("enabled")?
    {
        return Err(TransactionError::InvalidInput);
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_account(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    public_id: &str,
    pseudonym: &str,
    username: &NormalizedIdentity,
    email: &NormalizedIdentity,
    password_hash: &str,
    analytics_excluded: bool,
    now: DateTime<Utc>,
) -> Result<i64, TransactionError> {
    let users = table(tx, r, "users")?;
    tx.execute(&format!("INSERT INTO {users} (public_id, analytics_pseudonym, analytics_excluded, username, username_key, email, email_key, status, email_verified, bio, created_at, username_changed_at, credential_version, version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),&[text(public_id),text(pseudonym),DbValue::Bool(analytics_excluded),text(&username.display),text(&username.key),text(&email.display),text(&email.key),text("active"),DbValue::Bool(false),text(""),DbValue::Time(now),DbValue::Time(now),1.into(),1.into()]).await?;
    let id = new_id(tx, r, "users", "public_id", public_id).await?;
    let claims = table(tx, r, "username_claims")?;
    let created=tx.insert_if_absent(&format!("INSERT INTO {claims} (username_key, user_id, kind, reserved_until, version) VALUES (?, ?, ?, ?, ?)"),&[text(&username.key),id.into(),text("current"),DbValue::Time(now),1.into()]).await?;
    if !created {
        let row=tx.fetch_optional(&format!("SELECT user_id, kind, reserved_until, version FROM {claims} WHERE username_key = ?"),&[text(&username.key)]).await?.ok_or(TransactionError::CorruptData)?;
        if row.text("kind")? != "history" || row.time("reserved_until")? > now {
            return Err(TransactionError::InvalidInput);
        }
        if tx.execute(&format!("UPDATE {claims} SET user_id = ?, kind = ?, reserved_until = ?, version = version + 1 WHERE username_key = ? AND kind = ? AND version = ? AND reserved_until <= ?"),&[id.into(),text("current"),DbValue::Time(now),text(&username.key),text("history"),row.i64("version")?.into(),DbValue::Time(now)]).await?!=1{return Err(TransactionError::VersionConflict)}
    }
    let credentials = table(tx, r, "credentials")?;
    tx.execute(
        &format!("INSERT INTO {credentials} (user_id, password_hash, updated_at) VALUES (?, ?, ?)"),
        &[id.into(), text(password_hash), DbValue::Time(now)],
    )
    .await?;
    Ok(id)
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_session(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    account: &Account,
    reference: &str,
    hash: &str,
    csrf: &str,
    device: &str,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "sessions")?;
    tx.execute(&format!("INSERT INTO {t} (session_ref, user_id, token_hash, csrf_hash, created_at, last_seen_at, absolute_expires_at, idle_expires_at, reauthenticated_at, credential_version, device_summary, revoked) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),&[text(reference),account.id.into(),text(hash),text(csrf),DbValue::Time(now),DbValue::Time(now),DbValue::Time(now+Duration::days(SESSION_ABSOLUTE_DAYS)),DbValue::Time(now+Duration::days(SESSION_IDLE_DAYS)),DbValue::Time(now),account.credential_version.into(),text(device),DbValue::Bool(false)]).await?;
    Ok(())
}
pub(crate) async fn update_password(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    hash: &str,
    now: DateTime<Utc>,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "credentials")?;
    tx.execute(
        &format!("UPDATE {t} SET password_hash = ?, updated_at = ? WHERE user_id = ?"),
        &[text(hash), DbValue::Time(now), user_id.into()],
    )
    .await?;
    let u = table(tx, r, "users")?;
    tx.execute(&format!("UPDATE {u} SET credential_version = credential_version + 1, version = version + 1 WHERE id = ?"),&[user_id.into()]).await?;
    Ok(())
}
pub(crate) async fn mark_verified(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "users")?;
    tx.execute(
        &format!("UPDATE {t} SET email_verified = ?, version = version + 1 WHERE id = ?"),
        &[DbValue::Bool(true), user_id.into()],
    )
    .await?;
    Ok(())
}
pub(crate) async fn invitation_page(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    window: &InvitationWindow,
    now: DateTime<Utc>,
) -> Result<InvitationPage, TransactionError> {
    let t = table(tx, r, "invitations")?;
    let generated=tx.fetch_optional(&format!("SELECT COUNT(*) AS total FROM {t} WHERE creator_id = ? AND created_at >= ? AND created_at < ?"),&[user_id.into(),DbValue::Time(window.start),DbValue::Time(window.end)]).await?.ok_or(TransactionError::CorruptData)?.i64("total")?;
    let unused=tx.fetch_optional(&format!("SELECT COUNT(*) AS total FROM {t} WHERE creator_id = ? AND state = ? AND expires_at > ?"),&[user_id.into(),text("available"),DbValue::Time(now)]).await?.ok_or(TransactionError::CorruptData)?.i64("total")?;
    let rows=tx.fetch_all(&format!("SELECT invite_ref, state, created_at, expires_at FROM {t} WHERE creator_id = ? ORDER BY created_at DESC, id DESC"),&[user_id.into()]).await?;
    let records = rows
        .into_iter()
        .map(|row| {
            let state = row.text("state")?;
            let expiry = row.time("expires_at")?;
            Ok(InvitationRecord {
                invitation_ref: row.text("invite_ref")?,
                used: state == "used",
                state: if state == "available" && expiry <= now {
                    "expired".into()
                } else {
                    state
                },
                created_at: row.time("created_at")?,
                expires_at: expiry,
            })
        })
        .collect::<Result<Vec<_>, TransactionError>>()?;
    Ok(InvitationPage {
        generated_today: generated,
        unused,
        daily_limit: window.daily_limit,
        unused_limit: window.unused_limit,
        records,
    })
}
pub(crate) async fn create_invitation(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
    reference: &str,
    hash: &str,
    now: DateTime<Utc>,
    expires: DateTime<Utc>,
) -> Result<(), TransactionError> {
    let t = table(tx, r, "invitations")?;
    tx.execute(&format!("INSERT INTO {t} (invite_ref, creator_id, code_hash, source, state, created_at, expires_at, consumed_at, consumer_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"),&[text(reference),user_id.into(),text(hash),text("user"),text("available"),DbValue::Time(now),DbValue::Time(expires),DbValue::Time(now),0.into()]).await?;
    Ok(())
}

pub(crate) async fn token_owner(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    hash: &str,
    purpose: &str,
) -> Result<Option<TokenRecord>, TransactionError> {
    let t = table(tx, r, "one_time_tokens")?;
    tx.fetch_optional(&format!("SELECT token_id, user_id, target_email, target_email_key, expires_at, issued_credential_version FROM {t} WHERE token_hash = ? AND purpose = ?"),&[text(hash),text(purpose)]).await?.map(|row|Ok(TokenRecord{token_id:row.text("token_id")?,user_id:row.i64("user_id")?,target_email:row.text("target_email")?,target_email_key:row.text("target_email_key")?,expires_at:row.time("expires_at")?,issued_credential_version:row.i64("issued_credential_version")?})).transpose()
}

/// Minimal stored analytics authority; only a core projection may release it.
pub(crate) async fn analytics_identity(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    user_id: i64,
) -> Result<(String, bool), TransactionError> {
    let users = table(tx, r, "users")?;
    let row = tx
        .fetch_optional(
            &format!("SELECT analytics_pseudonym, analytics_excluded FROM {users} WHERE id = ?"),
            &[user_id.into()],
        )
        .await?
        .ok_or(TransactionError::InvalidInput)?;
    Ok((
        row.text("analytics_pseudonym")?,
        row.boolean("analytics_excluded")?,
    ))
}
pub(crate) async fn content_author(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    id: i64,
) -> Result<Option<crate::core::ContentAuthor>, TransactionError> {
    let users = table(tx, r, "users")?;
    tx.fetch_optional(
        &format!(
            "SELECT public_id, username, status, profile_lists_public, topic_body_public FROM {users} WHERE id = ?"
        ),
        &[id.into()],
    )
    .await?
    .map(|row| {
        let active = row.text("status")? == "active";
        Ok(crate::core::ContentAuthor {
            user_id: id,
            public_ref: if active {
                row.text("public_id")?
            } else {
                String::new()
            },
            username: if active {
                row.text("username")?
            } else {
                "不可用账号".into()
            },
            // 非活跃账号一律按最严格处理：不暴露列表，也不暴露正文。
            profile_lists_public: active && row.boolean("profile_lists_public")?,
            topic_body_public: active && row.boolean("topic_body_public")?,
        })
    })
    .transpose()
}
