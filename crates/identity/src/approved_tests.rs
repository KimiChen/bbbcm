//! Real DEC-051 policy/risk implementation; only mail material and policy documents are synthetic.
use crate::{
    contract::*,
    core_tests::{Fixture, at},
    policy::ApprovedIdentityPolicy,
    repo,
};
use chrono::Duration;
use platform::transaction::new_operation_id;
use platform_db::transaction::{DbTransaction, DbValue};
use std::sync::Arc;

async fn verification(f: &Fixture, user: i64) -> String {
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let table = repo::table(&tx.connection(), &f.r, "one_time_tokens").unwrap();
    let row = tx.connection().fetch_optional(&format!("SELECT token_id, token_nonce, target_email_key, expires_at, issued_credential_version FROM {table} WHERE user_id = ? AND purpose = ? AND consumed = ?"), &[user.into(), "verify_email".into(), DbValue::Bool(false)]).await.unwrap().unwrap();
    let value = f.issuer.derive(
        &row.text("token_id").unwrap(),
        &row.text("token_nonce").unwrap(),
        user,
        "verify_email",
        &row.text("target_email_key").unwrap(),
        row.time("expires_at").unwrap(),
        row.i64("issued_credential_version").unwrap(),
    );
    tx.rollback().await.unwrap();
    value.expose().into()
}

#[tokio::test]
async fn approved_invites_verify_without_promoting_and_enforce_expiry_quota_and_replays() {
    let mut f = Fixture::new().await;
    f.service.policy = Arc::new(ApprovedIdentityPolicy);
    f.service.risk_policy = Arc::new(trust::risk::ApprovedIdentityRiskPolicy);
    f.enable().await;
    f.service
        .register(&f.context().await, f.registration(), at())
        .await
        .unwrap();
    let result = f
        .service
        .login(
            &f.context().await,
            LoginInput {
                identifier: "ORDINARY_MEMBER".into(),
                password: "Synthetic password 123".to_owned().into(),
                device_summary: "浏览器".into(),
            },
            at(),
        )
        .await
        .unwrap();
    let grant = result.grant.unwrap();
    let session = f
        .service
        .authenticate(grant.token.expose(), at())
        .await
        .unwrap()
        .unwrap();
    assert!(
        f.service
            .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), at())
            .await
            .is_err()
    );
    let token = verification(&f, session.user_id()).await;
    f.service
        .consume_verification(&f.context().await, &token, at())
        .await
        .unwrap();
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let snapshot =
        trust::core::load_identity_snapshot(&mut tx.connection(), &f.r, session.user_id())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(snapshot.level(), trust::contract::TrustLevel::Pending);
    assert!(snapshot.email_verified() && snapshot.invitation_allowed());
    assert_eq!(snapshot.policy_version(), 1);
    tx.rollback().await.unwrap();

    for days in [0, 8, u8::MAX] {
        assert!(
            f.service
                .generate_invitation_with_signals(
                    &session,
                    grant.csrf.expose(),
                    &new_operation_id(),
                    Some(days),
                    Default::default(),
                    at()
                )
                .await
                .is_err()
        );
    }
    let key = new_operation_id();
    let default = f
        .service
        .generate_invitation_with_signals(
            &session,
            grant.csrf.expose(),
            &key,
            None,
            Default::default(),
            at(),
        )
        .await
        .unwrap();
    // A bearer is displayed once. Replays return no new bearer and create no new invite.
    for days in [None, Some(3), Some(7)] {
        assert!(
            f.service
                .generate_invitation_with_signals(
                    &session,
                    grant.csrf.expose(),
                    &key,
                    days,
                    Default::default(),
                    at()
                )
                .await
                .is_err()
        );
    }
    assert_eq!(
        f.service
            .invitations(&session, at())
            .await
            .unwrap()
            .generated_today,
        1
    );
    let short = f
        .service
        .generate_invitation_with_signals(
            &session,
            grant.csrf.expose(),
            &new_operation_id(),
            Some(1),
            Default::default(),
            at(),
        )
        .await
        .unwrap();
    let long = f
        .service
        .generate_invitation_with_signals(
            &session,
            grant.csrf.expose(),
            &new_operation_id(),
            Some(7),
            Default::default(),
            at(),
        )
        .await
        .unwrap();
    let page = f.service.invitations(&session, at()).await.unwrap();
    for (reference, days) in [
        (&default.invitation_ref, 3),
        (&short.invitation_ref, 1),
        (&long.invitation_ref, 7),
    ] {
        assert_eq!(
            page.records
                .iter()
                .find(|row| &row.invitation_ref == reference)
                .unwrap()
                .expires_at,
            at() + Duration::days(days)
        );
    }
    f.service
        .revoke_invitation(
            &session,
            grant.csrf.expose(),
            &new_operation_id(),
            &default.invitation_ref,
            at(),
        )
        .await
        .unwrap();
    let page = f.service.invitations(&session, at()).await.unwrap();
    assert_eq!((page.generated_today, page.unused), (3, 2));
    for _ in 0..2 {
        f.service
            .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), at())
            .await
            .unwrap();
    }
    assert!(
        f.service
            .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), at())
            .await
            .is_err()
    );
    let next_day = at() + Duration::days(1);
    let page = f.service.invitations(&session, next_day).await.unwrap();
    assert_eq!((page.generated_today, page.unused), (0, 3));
    assert_eq!(
        page.records
            .iter()
            .find(|row| row.invitation_ref == short.invitation_ref)
            .unwrap()
            .state,
        "expired"
    );
    for _ in 0..2 {
        f.service
            .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), next_day)
            .await
            .unwrap();
    }
    assert!(
        f.service
            .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), next_day)
            .await
            .is_err()
    );
    let page = f.service.invitations(&session, next_day).await.unwrap();
    assert_eq!((page.generated_today, page.unused), (2, 5));
    f.service
        .revoke_invitation(
            &session,
            grant.csrf.expose(),
            &new_operation_id(),
            &long.invitation_ref,
            next_day,
        )
        .await
        .unwrap();
    let page = f.service.invitations(&session, next_day).await.unwrap();
    assert_eq!((page.generated_today, page.unused), (2, 4));
    f.service
        .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), next_day)
        .await
        .unwrap();
    assert!(!format!("{page:?}").contains(default.code.expose()));

    // A stale eligibility snapshot cannot override the locked/current account fact.
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let users = repo::table(&tx.connection(), &f.r, "users").unwrap();
    tx.connection()
        .execute(
            &format!("UPDATE {users} SET status = ? WHERE id = ?"),
            &["locked".into(), session.user_id().into()],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        f.service
            .generate_invitation(&session, grant.csrf.expose(), &new_operation_id(), next_day)
            .await
            .is_err()
    );
    assert!(
        f.service
            .authenticate(grant.token.expose(), next_day)
            .await
            .unwrap()
            .is_none()
    );
}
