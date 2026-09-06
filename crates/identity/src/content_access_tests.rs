//! Ordinary synthetic identity fixtures exercise the caller-owned transaction boundary.
use crate::{
    core::*,
    core_tests::{Fixture, at},
};
use chrono::Duration;
use platform::transaction::{LockTarget, new_operation_id};
use platform_db::transaction::{DbTransaction, DbValue};

#[tokio::test]
async fn content_access_rechecks_mail_csrf_status_revocation_and_preserves_pending() {
    let f = Fixture::new().await;
    f.enable().await;
    f.service
        .register(&f.context().await, f.registration(), at())
        .await
        .unwrap();
    let (grant, session) = f.login().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let read = read_content_identity(&mut tx.connection(), &f.r, grant.token.expose(), at())
        .await
        .unwrap()
        .unwrap();
    assert!(!read.email_verified());
    assert!(
        authorize_content_identity(
            &mut tx.connection(),
            &f.r,
            grant.token.expose(),
            grant.csrf.expose(),
            at()
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    f.service
        .consume_verification(&f.context().await, &f.token("verify_email").await.1, at())
        .await
        .unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let facts = authorize_content_identity(
        &mut tx.connection(),
        &f.r,
        grant.token.expose(),
        grant.csrf.expose(),
        at(),
    )
    .await
    .unwrap();
    assert_eq!(facts.user_id(), session.user_id());
    assert_eq!(
        facts.trust().unwrap().level(),
        trust::contract::TrustLevel::T0
    );
    assert!(facts.analytics_excluded());
    assert!(!format!("{facts:?}").contains("member@example.invalid"));
    // The helper has taken identity+trust locks: returning to an identity root is invalid.
    assert!(
        tx.connection()
            .lock(
                &f.r,
                &LockTarget::new("identity", "users", session.user_id()).unwrap()
            )
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    assert!(
        authorize_content_identity(
            &mut tx.connection(),
            &f.r,
            grant.token.expose(),
            "wrong-csrf",
            at()
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    f.service
        .logout(&session, grant.csrf.expose(), &new_operation_id(), at())
        .await
        .unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    assert!(
        authorize_content_identity(
            &mut tx.connection(),
            &f.r,
            grant.token.expose(),
            grant.csrf.expose(),
            at()
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    let (grant, _) = f.login().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let users =
        f.r.table(tx.connection().backend(), "identity", "users")
            .unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {users} SET status = ? WHERE id = ?"),
            &["locked".into(), session.user_id().into()],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    assert!(
        authorize_content_identity(
            &mut tx.connection(),
            &f.r,
            grant.token.expose(),
            grant.csrf.expose(),
            at()
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn content_menu_reads_never_extend_idle_session_lifetime() {
    let f = Fixture::new().await;
    f.enable().await;
    f.service
        .register(&f.context().await, f.registration(), at())
        .await
        .unwrap();
    let (grant, session) = f.login().await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let sessions =
        f.r.table(tx.connection().backend(), "identity", "sessions")
            .unwrap();
    let expiry = at() + Duration::hours(1);
    tx.connection()
        .execute(
            &format!("UPDATE {sessions} SET idle_expires_at = ? WHERE user_id = ?"),
            &[DbValue::Time(expiry), session.user_id().into()],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    assert!(
        read_content_identity(
            &mut tx.connection(),
            &f.r,
            grant.token.expose(),
            at() + Duration::minutes(59)
        )
        .await
        .unwrap()
        .is_some()
    );
    let row = tx
        .connection()
        .fetch_optional(
            &format!("SELECT idle_expires_at FROM {sessions} WHERE user_id = ?"),
            &[session.user_id().into()],
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.time("idle_expires_at").unwrap(), expiry);
    assert!(
        read_content_identity(&mut tx.connection(), &f.r, grant.token.expose(), expiry)
            .await
            .unwrap()
            .is_none()
    );
    tx.rollback().await.unwrap();
}
