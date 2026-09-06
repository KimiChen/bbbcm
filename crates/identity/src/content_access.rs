//! Current-transaction identity projection for application-injected content authorization.
//! This module exposes no credential, grant or publishing policy and has no content dependency.
use super::{check_csrf, secret_hash};
use crate::{contract::*, repo, settings_repo};
use chrono::{DateTime, Utc};
use platform::{identifiers::SchemaRenderer, transaction::LockTarget};
use platform_db::transaction::TransactionConnection;

#[derive(Clone)]
pub struct ContentIdentity {
    user_id: i64,
    public_ref: String,
    username: String,
    email_verified: bool,
    created_at: DateTime<Utc>,
    analytics_pseudonym: String,
    analytics_excluded: bool,
    trust: Option<trust::contract::TrustSnapshot>,
}
impl std::fmt::Debug for ContentIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ContentIdentity([REDACTED])")
    }
}
impl ContentIdentity {
    pub fn user_id(&self) -> i64 {
        self.user_id
    }
    pub fn public_ref(&self) -> &str {
        &self.public_ref
    }
    pub fn username(&self) -> &str {
        &self.username
    }
    pub fn email_verified(&self) -> bool {
        self.email_verified
    }
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
    pub fn analytics_pseudonym(&self) -> &str {
        &self.analytics_pseudonym
    }
    pub fn analytics_excluded(&self) -> bool {
        self.analytics_excluded
    }
    pub fn trust(&self) -> Option<&trust::contract::TrustSnapshot> {
        self.trust.as_ref()
    }
}
async fn projection(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    account: Account,
    trust: Option<trust::contract::TrustSnapshot>,
) -> Result<ContentIdentity, IdentityError> {
    let (analytics_pseudonym, analytics_excluded) =
        repo::analytics_identity(tx, r, account.id).await?;
    Ok(ContentIdentity {
        user_id: account.id,
        public_ref: account.public_id,
        username: account.username,
        email_verified: account.email_verified,
        created_at: account.created_at,
        analytics_pseudonym,
        analytics_excluded,
        trust,
    })
}

/// Optional session read for menus and current viewer permissions. No database writes,
/// no session extension, no trust promotion and no implicit publishing authorization.
pub async fn read_content_identity(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    raw_session: &str,
    now: DateTime<Utc>,
) -> Result<Option<ContentIdentity>, IdentityError> {
    let Some(session) = repo::read_session(tx, r, &secret_hash(raw_session), now).await? else {
        return Ok(None);
    };
    let account = repo::require_session(tx, r, &session, now, false).await?;
    projection(tx, r, account, None).await.map(Some)
}

/// Caller owns the transaction and must invoke this before taking any community lock.
/// Every account/session revocation command uses the same identity.users root lock.
/// No transaction-outside snapshot is accepted as a substitute for this reread.
pub async fn authorize_content_identity(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    raw_session: &str,
    csrf: &str,
    now: DateTime<Utc>,
) -> Result<ContentIdentity, IdentityError> {
    let session = repo::read_session(tx, r, &secret_hash(raw_session), now)
        .await?
        .ok_or(IdentityError::Unauthenticated)?;
    tx.lock(
        r,
        &LockTarget::new("identity", "users", session.user_id).ok_or(IdentityError::Unavailable)?,
    )
    .await?;
    tx.lock(
        r,
        &LockTarget::new("identity", "sessions", session.session_id)
            .ok_or(IdentityError::Unavailable)?,
    )
    .await?;
    let account = repo::require_session(tx, r, &session, now, false).await?;
    check_csrf(&session, csrf)?;
    if !account.email_verified {
        return Err(IdentityError::Rejected);
    }
    let snapshot = trust::core::lock_content_snapshot(tx, r, account.id)
        .await
        .map_err(|_| IdentityError::Unavailable)?;
    if !snapshot.email_verified() {
        return Err(IdentityError::Rejected);
    }
    projection(tx, r, account, Some(snapshot)).await
}

#[derive(Clone)]
pub struct ContentAuthor {
    pub user_id: i64,
    pub public_ref: String,
    pub username: String,
}
impl std::fmt::Debug for ContentAuthor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ContentAuthor([REDACTED])")
    }
}
/// Safe public author projection. No cross-domain repository access is needed by callers.
pub async fn content_authors(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    ids: &[i64],
) -> Result<Vec<ContentAuthor>, IdentityError> {
    if ids.len() > 256 || ids.iter().any(|id| *id <= 0) {
        return Err(IdentityError::Rejected);
    }
    let mut authors = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(author) = repo::content_author(tx, r, *id).await? {
            authors.push(author);
        }
    }
    Ok(authors)
}
/// Public UUID reference resolution stays within identity's core/repository boundary.
pub async fn content_profile_id(
    tx: &mut TransactionConnection<'_>,
    r: &SchemaRenderer,
    public_ref: &str,
) -> Result<Option<i64>, IdentityError> {
    settings_repo::public_profile(tx, r, public_ref)
        .await
        .map(|profile| profile.map(|(id, _)| id))
        .map_err(Into::into)
}
