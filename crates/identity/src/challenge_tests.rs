use crate::{
    challenge::{ChallengeSubmission, LocalChallengeProvider},
    contract::*,
    core_tests::{Fixture, at},
};
use chrono::Duration;
use platform::{
    config::Environment, integrations::local::LocalRoot, transaction::new_operation_id,
};
use platform_db::transaction::{DbTransaction, DbValue};
use std::sync::Arc;

async fn provider(f: &Fixture) -> (crate::core::IdentityService, LocalRoot) {
    let root = LocalRoot::new(&f.dir.join("challenge-provider"), Environment::Test).unwrap();
    (
        f.service
            .clone()
            .with_challenge_provider(Arc::new(LocalChallengeProvider::new(
                root.clone(),
                f.issuer.clone(),
            ))),
        root,
    )
}
async fn proof(root: &LocalRoot, error: IdentityError) -> ChallengeSubmission {
    let IdentityError::ChallengeRequired { challenge_id } = error else {
        panic!("expected challenge")
    };
    let bytes = root
        .read("processed", &challenge_id, "json", 16384)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["provider"].as_str().unwrap().contains("LOCAL TEST"));
    ChallengeSubmission {
        challenge_id,
        proof: json["proof"].as_str().unwrap().to_owned().into(),
    }
}
#[tokio::test]
async fn challenge_private_output_binding_single_use_expiry_and_receipt_replay() {
    let f = Fixture::new().await;
    let (service, root) = provider(&f).await;
    let context = f.context().await.with_challenge(true, None);
    assert!(
        service
            .request_reset(&context, "missing_member", at())
            .await
            .is_err()
    );
    assert_eq!(f.count("identity", "challenges").await, 0);
    f.enable().await;
    assert_eq!(
        f.service
            .request_reset(&context, "missing_member", at())
            .await
            .unwrap_err(),
        IdentityError::Unavailable
    );
    let submission = proof(
        &root,
        service
            .request_reset(&context, "missing_member", at())
            .await
            .unwrap_err(),
    )
    .await;
    let again = proof(
        &root,
        service
            .request_reset(&context, "missing_member", at())
            .await
            .unwrap_err(),
    )
    .await;
    assert_eq!(submission.challenge_id, again.challenge_id);
    assert_eq!(submission.proof.expose(), again.proof.expose());
    assert_eq!(f.count("identity", "challenges").await, 1);
    let signed = context
        .clone()
        .with_challenge(false, Some(submission.clone()));
    assert_eq!(
        service
            .request_reset(&signed, "other_member", at())
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    let mut other_key = signed.clone();
    other_key.idempotency_key = new_operation_id();
    assert_eq!(
        service
            .request_reset(&other_key, "missing_member", at())
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    let other_principal = f
        .context()
        .await
        .with_challenge(true, Some(submission.clone()));
    assert_eq!(
        service
            .request_reset(&other_principal, "missing_member", at())
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    let wrong = context.clone().with_challenge(
        true,
        Some(ChallengeSubmission {
            challenge_id: submission.challenge_id.clone(),
            proof: "incorrect-local-proof".to_owned().into(),
        }),
    );
    assert_eq!(
        service
            .request_reset(&wrong, "missing_member", at())
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    assert_eq!(
        service
            .resend_verification(&signed, "missing_member", at())
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    assert!(
        !service
            .request_reset(&signed, "missing_member", at())
            .await
            .unwrap()
            .replayed
    );
    assert!(
        service
            .request_reset(&signed, "missing_member", at() + Duration::minutes(6))
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(f.count("identity", "mail_requests").await, 0);
    let expiring = f.context().await.with_challenge(true, None);
    let expired_proof = proof(
        &root,
        service
            .request_reset(&expiring, "missing_member", at())
            .await
            .unwrap_err(),
    )
    .await;
    let expired = expiring.with_challenge(true, Some(expired_proof));
    assert_eq!(
        service
            .request_reset(&expired, "missing_member", at() + Duration::minutes(5))
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    assert_eq!(
        service
            .request_reset(
                &expired.with_hard_rate_limited(true),
                "missing_member",
                at() + Duration::minutes(5)
            )
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let t = crate::repo::table(&tx.connection(), &f.r, "challenges").unwrap();
    let row = tx
        .connection()
        .fetch_optional(
            &format!("SELECT proof_hash, request_hash, consumed FROM {t} WHERE challenge_id = ?"),
            &[submission.challenge_id.into()],
        )
        .await
        .unwrap()
        .unwrap();
    assert!(row.boolean("consumed").unwrap());
    assert!(
        !row.text("proof_hash")
            .unwrap()
            .contains(submission.proof.expose())
    );
    assert_eq!(row.text("request_hash").unwrap().len(), 64);
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn challenge_consumption_rolls_back_with_registration_failure() {
    let f = Fixture::new().await;
    f.enable().await;
    let (service, root) = provider(&f).await;
    let context = f.context().await.with_challenge(true, None);
    let input = f.registration();
    let submission = proof(
        &root,
        service
            .register(&context, input.clone(), at())
            .await
            .unwrap_err(),
    )
    .await;
    assert_eq!(f.count("trust", "risk_events").await, 1);
    let replayed = proof(
        &root,
        service
            .register(&context, input.clone(), at())
            .await
            .unwrap_err(),
    )
    .await;
    assert_eq!(replayed.challenge_id, submission.challenge_id);
    assert_eq!(f.count("trust", "risk_events").await, 1);
    let signed = context.with_challenge(true, Some(submission.clone()));
    let mut changed_display = input.clone();
    changed_display.username = changed_display.username.to_ascii_uppercase();
    assert_eq!(
        service
            .register(&signed, changed_display, at())
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    for state in ["revoked", "available"] {
        let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
        let t = crate::repo::table(&tx.connection(), &f.r, "invitations").unwrap();
        tx.connection()
            .execute(
                &format!("UPDATE {t} SET state = ?"),
                &[DbValue::Text(state.into())],
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let result = service.register(&signed, input.clone(), at()).await;
        if state == "revoked" {
            assert!(result.is_err());
        } else {
            assert!(result.is_ok());
        }
    }
    assert!(
        service
            .register(&signed, input, at())
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(f.count("identity", "users").await, 2);
    assert_eq!(f.count("trust", "invite_uses").await, 1);
    assert_eq!(f.count("trust", "risk_events").await, 2);
}

#[tokio::test]
async fn challenge_failed_local_publication_restart_rotation_and_risk_dedup() {
    let f = Fixture::new().await;
    f.enable().await;
    let path = f.dir.join("challenge-restart");
    let root = LocalRoot::new(&path, Environment::Test).unwrap();
    let service = f
        .service
        .clone()
        .with_challenge_provider(Arc::new(LocalChallengeProvider::new(
            root,
            f.issuer.clone(),
        )));
    // Lose the private provider directory after construction. Database issuance
    // commits, publication fails closed, and a retry must recover that same fact.
    std::fs::remove_dir(&path).unwrap();
    let context = f.context().await.with_challenge(true, None);
    let input = LoginInput {
        identifier: "missing_member".into(),
        password: "synthetic challenge password".to_owned().into(),
        device_summary: "unknown".into(),
    };
    assert_eq!(
        service
            .login(&context, input.clone(), at())
            .await
            .unwrap_err(),
        IdentityError::Unavailable
    );
    assert_eq!(f.count("identity", "challenges").await, 1);
    assert_eq!(f.count("trust", "risk_events").await, 1);
    let root = LocalRoot::new(&path, Environment::Test).unwrap();
    let new_issuer = Arc::new(
        crate::core::TokenIssuer::new(crate::core::random_secret(), "synthetic_v2".into()).unwrap(),
    );
    let rotated = f.service.clone().with_challenge_provider(Arc::new(
        LocalChallengeProvider::new(root.clone(), new_issuer.clone())
            .with_retained_issuers(vec![new_issuer.clone(), f.issuer.clone()])
            .unwrap(),
    ));
    let submission = proof(
        &root,
        rotated
            .login(&context, input.clone(), at() + Duration::seconds(1))
            .await
            .unwrap_err(),
    )
    .await;
    assert!(!format!("{submission:?}").contains(submission.proof.expose()));
    assert_eq!(f.count("identity", "challenges").await, 1);
    assert_eq!(f.count("trust", "risk_events").await, 1);
    let signed = context.with_challenge(false, Some(submission.clone()));
    let active_only = f
        .service
        .clone()
        .with_challenge_provider(Arc::new(LocalChallengeProvider::new(root, new_issuer)));
    // Already issued proof verification needs its stored hash, never an old key.
    assert_eq!(
        active_only
            .login(&signed, input.clone(), at() + Duration::seconds(2))
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    assert_eq!(
        active_only
            .login(&signed, input, at() + Duration::seconds(3))
            .await
            .unwrap_err(),
        IdentityError::Rejected
    );
    assert_eq!(f.count("trust", "risk_events").await, 2);
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let t =
        f.r.table(tx.connection().backend(), "trust", "risk_events")
            .unwrap();
    let row = tx.connection().fetch_optional(&format!("SELECT rule_version, outcome, reason_code, signals_json FROM {t} WHERE event_id = ?"), &[submission.challenge_id.into()]).await.unwrap().unwrap();
    assert_eq!(row.i64("rule_version").unwrap(), 1);
    assert_eq!(row.text("outcome").unwrap(), "challenge");
    assert_eq!(row.text("reason_code").unwrap(), "explicit_challenge");
    assert!(
        !row.text("signals_json")
            .unwrap()
            .contains("synthetic challenge password")
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn hard_budget_keeps_challenge_shape_but_never_consumes_proof_or_writes_business() {
    let f = Fixture::new().await;
    f.enable().await;
    f.service
        .register(&f.context().await, f.registration(), at())
        .await
        .unwrap();
    let (service, root) = provider(&f).await;
    let context = f
        .context()
        .await
        .with_challenge(true, None)
        .with_hard_rate_limited(true);
    let submission = proof(
        &root,
        service
            .request_reset(&context, "ordinary_member", at())
            .await
            .unwrap_err(),
    )
    .await;
    let allowed_shape = context.clone().with_hard_rate_limited(false);
    let same = proof(
        &root,
        service
            .request_reset(&allowed_shape, "ordinary_member", at())
            .await
            .unwrap_err(),
    )
    .await;
    assert_eq!(submission.challenge_id, same.challenge_id);
    let signed = context.with_challenge(false, Some(submission.clone()));
    let before = service.passwords.verification_count();
    assert!(
        service
            .request_reset(&signed, "ordinary_member", at())
            .await
            .unwrap()
            .resource_ref
            .is_none()
    );
    assert_eq!(service.passwords.verification_count(), before + 1);
    assert_eq!(f.count("identity", "one_time_tokens").await, 1);
    assert_eq!(f.count("identity", "mail_requests").await, 1);
    let mut tx = DbTransaction::begin_read(&f.pool).await.unwrap();
    let t = crate::repo::table(&tx.connection(), &f.r, "challenges").unwrap();
    assert!(
        !tx.connection()
            .fetch_optional(
                &format!("SELECT consumed FROM {t} WHERE challenge_id = ?"),
                &[submission.challenge_id.into()]
            )
            .await
            .unwrap()
            .unwrap()
            .boolean("consumed")
            .unwrap()
    );
    tx.rollback().await.unwrap();
    service
        .request_reset(
            &signed.with_hard_rate_limited(false),
            "ordinary_member",
            at(),
        )
        .await
        .unwrap();
    assert_eq!(f.count("identity", "one_time_tokens").await, 2);
    let login_context = f.context().await.with_challenge(true, None);
    let input = LoginInput {
        identifier: "ordinary_member".into(),
        password: "Synthetic password 123".to_owned().into(),
        device_summary: "unknown".into(),
    };
    let login_proof = proof(
        &root,
        service
            .login(&login_context, input.clone(), at())
            .await
            .unwrap_err(),
    )
    .await;
    let login_signed = login_context.with_challenge(true, Some(login_proof));
    assert!(
        service
            .login(&login_signed, input.clone(), at())
            .await
            .unwrap()
            .grant
            .is_some()
    );
    let limited_replay = service
        .login(
            &login_signed.with_hard_rate_limited(true),
            input,
            at() + Duration::minutes(6),
        )
        .await
        .unwrap();
    assert!(limited_replay.replayed && limited_replay.grant.is_none());
    assert_eq!(f.count("identity", "sessions").await, 1);
}
