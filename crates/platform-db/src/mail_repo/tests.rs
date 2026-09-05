use super::*;
use crate::transaction::tests::Fixture;
use platform::config::Environment;
use platform::integrations::local::LocalRoot;
use platform::mail::local::{LocalFeedbackSource, LocalMailProvider};
use platform::outbox::Sink;
use std::sync::Arc;

fn message(class: MailClass) -> MailMessage {
    MailMessage {
        message_id: platform::transaction::new_operation_id(),
        recipient_ref: platform::transaction::new_operation_id(),
        class,
        from: "noreply@example.test".into(),
        to: "synthetic@example.test".into(),
        subject: "合成通知".into(),
        text: "only synthetic local mail".into(),
        unsubscribe_url: (class == MailClass::Marketing)
            .then(|| "https://example.test/unsubscribe/synthetic".into()),
        queued_at: platform::db::decode_time("2026-09-05T00:00:00Z").unwrap(),
    }
}
fn feedback(mail: &MailMessage, kind: FeedbackKind) -> TrustedFeedback {
    TrustedFeedback {
        provider: "local_test".into(),
        feedback_id: platform::transaction::new_operation_id(),
        message_id: mail.message_id.clone(),
        kind,
        occurred_at: mail.queued_at + chrono::Duration::seconds(10),
        received_at: mail.queued_at + chrono::Duration::seconds(11),
        simulated: true,
    }
}
async fn enqueue(f: &Fixture, mail: &MailMessage) -> OutboxEvent {
    let event = OutboxEvent {
        event_id: platform::transaction::new_operation_id(),
        aggregate_type: "mail".into(),
        aggregate_id: mail.message_id.clone(),
        event_type: "mail.requested".into(),
        version: 1,
        occurred_at: mail.queued_at,
        payload: SafePayload::empty()
            .with_reference("resource_id", &mail.message_id)
            .unwrap(),
        trace_id: String::new(),
    };
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    crate::outbox_repo::enqueue(
        &mut tx.connection(),
        &f.renderer,
        &event,
        &["mail.delivery"],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    event
}
async fn submitted(f: &Fixture, mail: &MailMessage) -> (DbMailStore, OutboxEvent) {
    let store = DbMailStore::new(f.pool.clone(), f.renderer.clone());
    let event = enqueue(f, mail).await;
    assert_eq!(
        store.prepare(&event, mail, true).await.unwrap(),
        DispatchDecision::Submit
    );
    store
        .record(
            &event.event_id,
            DeliveryReceipt {
                state: DeliveryState::LocalRecorded,
                simulated: true,
            },
            mail.queued_at + chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
    (store, event)
}
struct Source(MailMessage);
impl MailMessageSource for Source {
    fn load<'a>(&'a self, message_id: &'a str) -> BoxFuture<'a, Result<MailMessage, AdapterError>> {
        Box::pin(async move {
            if self.0.message_id == message_id {
                Ok(self.0.clone())
            } else {
                Err(AdapterError::NotFound)
            }
        })
    }
}

#[tokio::test]
async fn local_mail_writes_once_restarts_and_preserves_error_states() {
    let f = Fixture::new().await;
    let mail = message(MailClass::Security);
    let event = enqueue(&f, &mail).await;
    let store = Arc::new(DbMailStore::new(f.pool.clone(), f.renderer.clone()));
    let root = LocalRoot::new(
        &f.dir.canonicalize().unwrap().join("spool"),
        Environment::Test,
    )
    .unwrap();
    let sink = MailSink {
        source: Arc::new(Source(mail.clone())),
        store: store.clone(),
        provider: Arc::new(LocalMailProvider { root: root.clone() }),
    };
    sink.deliver(&event).await.unwrap();
    sink.deliver(&event).await.unwrap();
    let recorded = root
        .read("mail", &event.event_id, "eml", 1_000_000)
        .await
        .unwrap();
    assert!(
        String::from_utf8(recorded)
            .unwrap()
            .contains("X-BBBcm-Delivery: local-test-only")
    );
    assert_eq!(
        std::fs::read_dir(f.dir.join("spool/mail")).unwrap().count(),
        1
    );
    let restarted = DbMailStore::new(f.pool.clone(), f.renderer.clone());
    assert_eq!(
        restarted.prepare(&event, &mail, true).await.unwrap(),
        DispatchDecision::AlreadyRecorded
    );
    let mut changed = mail.clone();
    changed.text = "different source body".into();
    assert_eq!(
        restarted.prepare(&event, &changed, true).await,
        Err(AdapterError::Conflict)
    );
    let other = message(MailClass::Security);
    let other_event = enqueue(&f, &other).await;
    let closed = MailSink {
        source: Arc::new(Source(other.clone())),
        store: store.clone(),
        provider: Arc::new(DisabledMailProvider),
    };
    assert_eq!(
        closed.deliver(&other_event).await,
        Err(platform::outbox::SinkError::Disabled)
    );
    let name = f
        .renderer
        .table(f.pool.backend(), "platform", "mail_dispatches")
        .unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let state = tx
        .connection()
        .fetch_optional(
            &format!("SELECT state FROM {name} WHERE message_id = ?"),
            &[text(&other.message_id)],
        )
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(
        state.is_none(),
        "disabled provider leaves Outbox queued without binding a provider mode"
    );
    let enabled = MailSink {
        source: Arc::new(Source(other)),
        store,
        provider: Arc::new(LocalMailProvider { root }),
    };
    enabled.deliver(&other_event).await.unwrap();
    let unavailable_mail = message(MailClass::Security);
    let unavailable_event = enqueue(&f, &unavailable_mail).await;
    let unavailable_root_path = f.dir.canonicalize().unwrap().join("unavailable_spool");
    let unavailable_root = LocalRoot::new(&unavailable_root_path, Environment::Test).unwrap();
    std::fs::remove_dir(&unavailable_root_path).unwrap();
    let unavailable_sink = MailSink {
        source: Arc::new(Source(unavailable_mail.clone())),
        store: Arc::new(restarted.clone()),
        provider: Arc::new(LocalMailProvider {
            root: unavailable_root,
        }),
    };
    assert_eq!(
        unavailable_sink.deliver(&unavailable_event).await,
        Err(platform::outbox::SinkError::Retryable)
    );
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let failed = tx
        .connection()
        .fetch_optional(
            &format!("SELECT state FROM {name} WHERE message_id = ?"),
            &[text(&unavailable_mail.message_id)],
        )
        .await
        .unwrap()
        .unwrap()
        .text("state")
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(failed, "failed");
    LocalRoot::new(&unavailable_root_path, Environment::Test).unwrap();
    unavailable_sink.deliver(&unavailable_event).await.unwrap();
}

#[tokio::test]
async fn marketing_unsubscribe_does_not_suppress_security_or_transactions() {
    for prefix in ["bcm_", "mail_alt_"] {
        let f = Fixture::with_prefix(prefix).await;
        let marketing = message(MailClass::Marketing);
        let (store, _) = submitted(&f, &marketing).await;
        let mut failed_marketing = message(MailClass::Marketing);
        failed_marketing.recipient_ref = marketing.recipient_ref.clone();
        let failed_event = enqueue(&f, &failed_marketing).await;
        assert_eq!(
            store
                .prepare(&failed_event, &failed_marketing, true)
                .await
                .unwrap(),
            DispatchDecision::Submit
        );
        store
            .record(
                &failed_event.event_id,
                DeliveryReceipt {
                    state: DeliveryState::Failed,
                    simulated: true,
                },
                failed_marketing.queued_at,
            )
            .await
            .unwrap();
        let report = feedback(&marketing, FeedbackKind::Unsubscribe);
        let outcome = store.apply(&report).await.unwrap();
        assert!(outcome.suppressed);
        assert!(outcome.alert_id.is_none());
        assert!(store.apply(&report).await.unwrap().duplicate);
        assert_eq!(
            store
                .prepare(&failed_event, &failed_marketing, true)
                .await
                .unwrap(),
            DispatchDecision::Suppressed
        );
        let name = f
            .renderer
            .table(f.pool.backend(), "platform", "mail_dispatches")
            .unwrap();
        let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
        let stopped = tx
            .connection()
            .fetch_optional(
                &format!("SELECT state FROM {name} WHERE message_id = ?"),
                &[text(&failed_marketing.message_id)],
            )
            .await
            .unwrap()
            .unwrap()
            .text("state")
            .unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(stopped, "suppressed");
        for class in [
            MailClass::Marketing,
            MailClass::Security,
            MailClass::Transactional,
        ] {
            let mut next = message(class);
            next.recipient_ref = marketing.recipient_ref.clone();
            let event = enqueue(&f, &next).await;
            assert_eq!(
                store.prepare(&event, &next, true).await.unwrap(),
                if class == MailClass::Marketing {
                    DispatchDecision::Suppressed
                } else {
                    DispatchDecision::Submit
                }
            );
        }
        assert_eq!(f.count("platform", "mail_feedback").await, 1);
    }
}

#[tokio::test]
async fn hard_bounces_are_deduplicated_persistent_and_stop_retry_with_outbox_alerts() {
    let f = Fixture::new().await;
    let mail = message(MailClass::Security);
    let (store, event) = submitted(&f, &mail).await;
    let report = feedback(&mail, FeedbackKind::HardBounce);
    let source =
        LocalFeedbackSource::new(vec![report.clone(), report.clone()], Environment::Test).unwrap();
    let first = consume_feedback(&source, &store).await.unwrap().unwrap();
    assert!(first.suppressed);
    assert!(first.alert_id.is_some());
    assert!(
        consume_feedback(&source, &store)
            .await
            .unwrap()
            .unwrap()
            .duplicate
    );
    assert!(consume_feedback(&source, &store).await.unwrap().is_none());
    let mut changed = report.clone();
    changed.kind = FeedbackKind::Complaint;
    assert_eq!(store.apply(&changed).await, Err(AdapterError::Conflict));
    // Distinct authenticated hard-bounce observations remain stopped and alert again;
    // duplicate transport receipts never inflate the count or generate extra alerts.
    store
        .apply(&feedback(&mail, FeedbackKind::HardBounce))
        .await
        .unwrap();
    let root = LocalRoot::new(
        &f.dir.canonicalize().unwrap().join("spool"),
        Environment::Test,
    )
    .unwrap();
    let sink = MailSink {
        source: Arc::new(Source(mail.clone())),
        store: Arc::new(store.clone()),
        provider: Arc::new(LocalMailProvider { root: root.clone() }),
    };
    assert_eq!(sink.deliver(&event).await, Ok(()));
    assert!(
        !f.dir.join("spool/mail").exists(),
        "a recorded replay must never reach provider, even after suppression"
    );
    let mut later = message(MailClass::Security);
    later.recipient_ref = mail.recipient_ref.clone();
    let later_event = enqueue(&f, &later).await;
    let restarted = DbMailStore::new(f.pool.clone(), f.renderer.clone());
    assert_eq!(
        restarted.prepare(&later_event, &later, true).await.unwrap(),
        DispatchDecision::Suppressed
    );
    let name = f
        .renderer
        .table(f.pool.backend(), "platform", "mail_suppressions")
        .unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let count = tx
        .connection()
        .fetch_optional(
            &format!("SELECT hard_bounces FROM {name} WHERE recipient_ref = ?"),
            &[text(&mail.recipient_ref)],
        )
        .await
        .unwrap()
        .unwrap()
        .i64("hard_bounces")
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(count, 2);
    assert_eq!(f.count("platform", "mail_alerts").await, 2);
    assert_eq!(f.count("platform", "mail_feedback").await, 2);
    let tracker = Arc::new(platform::integrations::local::LocalAdapters { root });
    let alert_sink = MailAlertSink {
        source: Arc::new(store.clone()),
        tracker,
    };
    let events_name = f
        .renderer
        .table(f.pool.backend(), "platform", "outbox_events")
        .unwrap();
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let rows = tx
        .connection()
        .fetch_all(
            &format!(
                "SELECT event_id, aggregate_id, payload FROM {events_name} WHERE event_type = ?"
            ),
            &[text("mail.alert")],
        )
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        let payload = row.text("payload").unwrap();
        assert!(!payload.contains(&mail.recipient_ref));
        assert!(!payload.contains("synthetic@example.test"));
        alert_sink
            .deliver(&OutboxEvent {
                event_id: row.text("event_id").unwrap(),
                aggregate_id: row.text("aggregate_id").unwrap(),
                aggregate_type: "mail_alert".into(),
                event_type: "mail.alert".into(),
                version: 1,
                occurred_at: report.received_at,
                payload: SafePayload::from_json(&payload).unwrap(),
                trace_id: String::new(),
            })
            .await
            .unwrap();
    }
    assert_eq!(
        std::fs::read_dir(f.dir.join("spool/errors"))
            .unwrap()
            .count(),
        2
    );
}

#[tokio::test]
async fn complaints_soft_bounces_and_rollback_never_loosen_suppression() {
    let f = Fixture::new().await;
    for kind in [FeedbackKind::Complaint, FeedbackKind::SoftBounce] {
        let mail = message(MailClass::Security);
        let (store, event) = submitted(&f, &mail).await;
        store.apply(&feedback(&mail, kind)).await.unwrap();
        store
            .apply(&feedback(&mail, FeedbackKind::Delivered))
            .await
            .unwrap();
        assert_eq!(
            store.prepare(&event, &mail, true).await.unwrap(),
            DispatchDecision::AlreadyRecorded
        );
        let mut later = message(MailClass::Security);
        later.recipient_ref = mail.recipient_ref.clone();
        let later_event = enqueue(&f, &later).await;
        assert_eq!(
            store.prepare(&later_event, &later, true).await.unwrap(),
            DispatchDecision::Suppressed
        );
    }
    let mail = message(MailClass::Security);
    let (store, event) = submitted(&f, &mail).await;
    let before = f.count("platform", "outbox_events").await;
    let mut tx = DbTransaction::begin(&f.pool).await.unwrap();
    let report = feedback(&mail, FeedbackKind::HardBounce);
    apply_feedback(&mut tx.connection(), &f.renderer, &report)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(f.count("platform", "outbox_events").await, before);
    assert_eq!(
        store.prepare(&event, &mail, true).await.unwrap(),
        DispatchDecision::AlreadyRecorded
    );
    let mut fake = report.clone();
    fake.simulated = false;
    assert_eq!(store.apply(&fake).await, Err(AdapterError::InvalidInput));
    fake = feedback(&mail, FeedbackKind::Unsubscribe);
    assert_eq!(store.apply(&fake).await, Err(AdapterError::InvalidInput));
    let mut pending = feedback(&mail, FeedbackKind::Delivered);
    pending.message_id = platform::transaction::new_operation_id();
    let source = LocalFeedbackSource::new(vec![pending.clone()], Environment::Test).unwrap();
    assert_eq!(
        consume_feedback(&source, &store).await,
        Err(AdapterError::NotFound)
    );
    assert_eq!(
        source.next().await.unwrap().unwrap().feedback_id,
        pending.feedback_id,
        "storage failure cannot acknowledge or discard trusted feedback"
    );
}

#[tokio::test]
async fn metric_cohorts_separate_simulation_deduplicate_messages_and_bound_time() {
    let f = Fixture::new().await;
    let mail = message(MailClass::Security);
    let (store, _) = submitted(&f, &mail).await;
    for kind in [
        FeedbackKind::Delivered,
        FeedbackKind::Delivered,
        FeedbackKind::Delayed,
        FeedbackKind::HardBounce,
        FeedbackKind::HardBounce,
        FeedbackKind::Complaint,
    ] {
        store.apply(&feedback(&mail, kind)).await.unwrap();
    }
    let end = mail.queued_at + chrono::Duration::seconds(60);
    let sample = store
        .metric_snapshot(mail.queued_at, end, true)
        .await
        .unwrap();
    assert_eq!(
        sample,
        MailMetricSample {
            simulated: true,
            submitted: 1,
            delivered: 1,
            delayed: 1,
            bounced: 1,
            complained: 1,
            max_delay_ms: Some(10_000)
        }
    );
    let real = store
        .metric_snapshot(mail.queued_at, end, false)
        .await
        .unwrap();
    assert_eq!(real, MailMetricSample::default());
    let before = store
        .metric_snapshot(
            mail.queued_at - chrono::Duration::seconds(1),
            mail.queued_at,
            true,
        )
        .await
        .unwrap();
    assert_eq!(before.submitted, 0);
    assert_eq!(before.max_delay_ms, None);
    assert!(
        store
            .metric_snapshot(end, mail.queued_at, true)
            .await
            .is_err()
    );
    assert!(
        store
            .metric_snapshot(
                mail.queued_at,
                mail.queued_at + chrono::Duration::nanoseconds(500),
                true
            )
            .await
            .is_err()
    );
}
