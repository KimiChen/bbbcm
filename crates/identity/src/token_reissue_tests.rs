use crate::{
    contract::*,
    core_tests::{Fixture, at},
};
use chrono::Duration;

#[tokio::test]
async fn verification_resend_replaces_old_token_and_preserves_same_key_and_uniform_response() {
    let f = Fixture::new().await;
    f.enable().await;
    f.service
        .register(&f.context().await, f.registration(), at())
        .await
        .unwrap();
    let (old_id, old) = f.token("verify_email").await;
    let context = f.context().await;
    let first = f
        .service
        .resend_verification(&context, "ordinary_member", at() + Duration::seconds(1))
        .await
        .unwrap();
    assert!(first.resource_ref.is_none());
    let (new_id, new) = f.token("verify_email").await;
    assert_ne!(old_id, new_id);
    assert!(
        f.service
            .resend_verification(&context, "ordinary_member", at() + Duration::seconds(2))
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(f.count("identity", "one_time_tokens").await, 2);
    assert_eq!(
        f.service
            .consume_verification(&f.context().await, &old, at() + Duration::seconds(3))
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    f.service
        .consume_verification(&f.context().await, &new, at() + Duration::seconds(3))
        .await
        .unwrap();
    for identifier in ["ordinary_member", "missing_member", "invalid input"] {
        let count = f.service.passwords.verification_count();
        let result = f
            .service
            .resend_verification(&f.context().await, identifier, at() + Duration::seconds(4))
            .await
            .unwrap();
        assert!(result.resource_ref.is_none());
        assert_eq!(f.service.passwords.verification_count(), count + 1);
    }
    assert_eq!(f.count("identity", "one_time_tokens").await, 2);
    assert_eq!(f.count("identity", "mail_requests").await, 2);
}

#[tokio::test]
async fn new_recovery_request_invalidates_previous_purpose_only_and_replay_adds_nothing() {
    let f = Fixture::new().await;
    f.enable().await;
    f.service
        .register(&f.context().await, f.registration(), at())
        .await
        .unwrap();
    let (_, verification) = f.token("verify_email").await;
    let first = f.context().await;
    f.service
        .request_reset(&first, "ordinary_member", at())
        .await
        .unwrap();
    let (old_id, old) = f.token("reset_password").await;
    let second = f.context().await;
    f.service
        .request_reset(&second, "ordinary_member", at() + Duration::seconds(1))
        .await
        .unwrap();
    let (new_id, new) = f.token("reset_password").await;
    assert_ne!(old_id, new_id);
    assert!(
        f.service
            .request_reset(&second, "ordinary_member", at() + Duration::seconds(2))
            .await
            .unwrap()
            .replayed
    );
    assert!(
        f.service
            .request_reset(&first, "ordinary_member", at() + Duration::seconds(2))
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(f.count("identity", "one_time_tokens").await, 3);
    assert_eq!(f.count("identity", "mail_requests").await, 3);
    assert_eq!(
        f.service
            .reset_password(
                &f.context().await,
                &old,
                "replacement synthetic password",
                at() + Duration::seconds(3)
            )
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    // Reset issuance does not invalidate the independent email-verification purpose.
    f.service
        .consume_verification(
            &f.context().await,
            &verification,
            at() + Duration::seconds(3),
        )
        .await
        .unwrap();
    f.service
        .reset_password(
            &f.context().await,
            &new,
            "replacement synthetic password",
            at() + Duration::seconds(4),
        )
        .await
        .unwrap();
}
