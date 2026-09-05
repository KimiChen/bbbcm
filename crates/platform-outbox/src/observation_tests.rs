use super::*;
use platform::outbox::SafePayload;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Store {
    pending: Mutex<Vec<LeasedEvent>>,
    failures: Mutex<Vec<FailureCode>>,
    claims: AtomicUsize,
    renewals: AtomicUsize,
    claim_started: tokio::sync::Notify,
    claim_delay: Duration,
    claim_error: bool,
    renew_error_at: usize,
    acknowledgement_error: bool,
}
impl Store {
    fn new(names: &[&str]) -> Self {
        let id = platform::transaction::new_operation_id();
        Self {
            pending: Mutex::new(if names.is_empty() {
                Vec::new()
            } else {
                vec![LeasedEvent {
                    event: OutboxEvent {
                        event_id: id.clone(),
                        aggregate_type: "synthetic".into(),
                        aggregate_id: "1".into(),
                        event_type: "synthetic.observation".into(),
                        version: 1,
                        occurred_at: Utc::now(),
                        payload: SafePayload::empty(),
                        trace_id: id,
                    },
                    owner: "synthetic".into(),
                    batch_id: platform::transaction::new_operation_id(),
                    lease_until: Utc::now() + chrono::Duration::seconds(2),
                    attempts: 1,
                    pending_sinks: names.iter().map(|s| (*s).into()).collect(),
                }]
            }),
            failures: Mutex::new(Vec::new()),
            claims: AtomicUsize::new(0),
            renewals: AtomicUsize::new(0),
            claim_started: tokio::sync::Notify::new(),
            claim_delay: Duration::ZERO,
            claim_error: false,
            renew_error_at: usize::MAX,
            acknowledgement_error: false,
        }
    }
}
impl OutboxStore for Store {
    fn claim<'a>(&'a self, _: Claim) -> BoxFuture<'a, Result<Vec<LeasedEvent>, OutboxError>> {
        Box::pin(async move {
            self.claims.fetch_add(1, Ordering::SeqCst);
            self.claim_started.notify_one();
            if !self.claim_delay.is_zero() {
                tokio::time::sleep(self.claim_delay).await;
            }
            if self.claim_error {
                return Err(OutboxError::Unavailable);
            }
            Ok(std::mem::take(&mut *self.pending.lock().unwrap()))
        })
    }
    fn renew<'a>(
        &'a self,
        _: &'a LeasedEvent,
        _: chrono::DateTime<Utc>,
        _: chrono::DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), OutboxError>> {
        Box::pin(async move {
            if self.renewals.fetch_add(1, Ordering::SeqCst) + 1 >= self.renew_error_at {
                Err(OutboxError::LeaseLost)
            } else {
                Ok(())
            }
        })
    }
    fn delivery_done<'a>(
        &'a self,
        _: &'a LeasedEvent,
        _: &'a str,
        _: chrono::DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), OutboxError>> {
        Box::pin(async move {
            if self.acknowledgement_error {
                Err(OutboxError::Unavailable)
            } else {
                Ok(())
            }
        })
    }
    fn complete<'a>(
        &'a self,
        _: &'a LeasedEvent,
        _: chrono::DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), OutboxError>> {
        Box::pin(async { Ok(()) })
    }
    fn fail<'a>(
        &'a self,
        _: &'a LeasedEvent,
        failure: Failure,
    ) -> BoxFuture<'a, Result<(), OutboxError>> {
        Box::pin(async move {
            self.failures.lock().unwrap().push(failure.code);
            Ok(())
        })
    }
}
struct ProbeSink {
    name: &'static str,
    result: Result<(), SinkError>,
    pending: bool,
    started: tokio::sync::Notify,
}
impl ProbeSink {
    fn new(name: &'static str, result: Result<(), SinkError>, pending: bool) -> Arc<Self> {
        Arc::new(Self {
            name,
            result,
            pending,
            started: tokio::sync::Notify::new(),
        })
    }
}
impl Sink for ProbeSink {
    fn name(&self) -> &str {
        self.name
    }
    fn is_external(&self) -> bool {
        false
    }
    fn deliver<'a>(&'a self, _: &'a OutboxEvent) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async move {
            self.started.notify_one();
            if self.pending {
                std::future::pending::<()>().await;
            }
            self.result
        })
    }
}
fn scheduler(store: Arc<Store>, sink: Option<Arc<ProbeSink>>) -> Arc<Scheduler> {
    let mut registry = SinkRegistry::default();
    if let Some(sink) = sink {
        registry.register(sink).unwrap();
    }
    let mut scheduler = Scheduler::new(
        store,
        registry,
        Arc::new(ClosedExternalEffects),
        Settings {
            poll_interval: Duration::from_millis(1),
            lease_duration: Duration::from_secs(2),
            renew_interval: Duration::from_millis(10),
            ..Settings::default()
        },
    )
    .unwrap()
    .with_backend(Backend::Sqlite);
    scheduler.observations = Arc::new(Metrics::default());
    Arc::new(scheduler)
}
fn count(scheduler: &Scheduler, key: SeriesKey) -> u64 {
    scheduler
        .observations
        .grouped
        .snapshot()
        .iter()
        .find(|series| series.dimensions == key)
        .map_or(0, |series| series.sample.count)
}
fn outbox(sink: SinkGroup, outcome: OutboxOutcome) -> SeriesKey {
    SeriesKey::OutboxTotal {
        backend: DatabaseBackend::Sqlite,
        sink,
        outcome,
    }
}
fn sink(sink: SinkGroup, outcome: SinkOutcome) -> SeriesKey {
    SeriesKey::SinkSeconds {
        backend: DatabaseBackend::Sqlite,
        sink,
        outcome,
    }
}
fn exited(outcome: WorkerOutcome) -> SeriesKey {
    SeriesKey::WorkerExitTotal {
        backend: DatabaseBackend::Sqlite,
        outcome,
    }
}

#[tokio::test]
async fn sink_outcomes_have_finite_dimensions_and_confirmed_outbox_transitions() {
    for (result, observed, transition) in [
        (Ok(()), SinkOutcome::Success, OutboxOutcome::Completed),
        (
            Err(SinkError::Retryable),
            SinkOutcome::Retryable,
            OutboxOutcome::Retryable,
        ),
        (
            Err(SinkError::Retryable),
            SinkOutcome::Retryable,
            OutboxOutcome::RetryExhausted,
        ),
        (
            Err(SinkError::Permanent),
            SinkOutcome::Permanent,
            OutboxOutcome::Permanent,
        ),
        (
            Err(SinkError::Disabled),
            SinkOutcome::Disabled,
            OutboxOutcome::ExternalDisabled,
        ),
    ] {
        let store = Arc::new(Store::new(&["mail.delivery"]));
        if transition == OutboxOutcome::RetryExhausted {
            store.pending.lock().unwrap()[0].attempts = Settings::default().max_attempts;
        }
        let scheduler = scheduler(store, Some(ProbeSink::new("mail.delivery", result, false)));
        assert_eq!(scheduler.tick(&CancellationToken::new()).await.unwrap(), 1);
        assert_eq!(
            count(&scheduler, outbox(SinkGroup::All, OutboxOutcome::Claimed)),
            1
        );
        assert_eq!(
            count(&scheduler, outbox(SinkGroup::All, OutboxOutcome::Renewed)),
            1
        );
        assert_eq!(count(&scheduler, sink(SinkGroup::Mail, observed)), 1);
        let group = if result.is_ok() {
            SinkGroup::All
        } else {
            SinkGroup::Mail
        };
        assert_eq!(count(&scheduler, outbox(group, transition)), 1);
        assert_eq!(scheduler.observations.grouped.active_jobs(), 0);
        assert_eq!(
            count(&scheduler, outbox(SinkGroup::All, OutboxOutcome::Completed)),
            u64::from(result.is_ok())
        );
    }
    let scheduler = scheduler(Arc::new(Store::new(&["private.caller_label"])), None);
    scheduler.tick(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        count(
            &scheduler,
            outbox(SinkGroup::Other, OutboxOutcome::Unregistered)
        ),
        1
    );
    assert!(
        !format!("{:?}", scheduler.observations.grouped.snapshot())
            .contains("private.caller_label")
    );
}

#[tokio::test]
async fn provider_success_with_failed_acknowledgement_is_never_completed() {
    let mut store = Store::new(&["cache.invalidate"]);
    store.acknowledgement_error = true;
    let scheduler = scheduler(
        Arc::new(store),
        Some(ProbeSink::new("cache.invalidate", Ok(()), false)),
    );
    assert_eq!(
        scheduler.tick(&CancellationToken::new()).await,
        Err(OutboxError::Unavailable)
    );
    assert_eq!(
        count(
            &scheduler,
            sink(SinkGroup::CacheInvalidation, SinkOutcome::Success)
        ),
        1
    );
    assert_eq!(
        count(
            &scheduler,
            outbox(SinkGroup::CacheInvalidation, OutboxOutcome::StoreError)
        ),
        1
    );
    assert_eq!(
        count(&scheduler, outbox(SinkGroup::All, OutboxOutcome::Completed)),
        0
    );
    assert_eq!(scheduler.observations.grouped.active_jobs(), 0);
}

#[tokio::test]
async fn graceful_cancellation_and_task_abort_release_active_job_gauges() {
    for abort in [false, true] {
        let store = Arc::new(Store::new(&["analytics.project"]));
        let probe = ProbeSink::new("analytics.project", Ok(()), true);
        let scheduler = scheduler(store.clone(), Some(probe.clone()));
        let cancel = CancellationToken::new();
        let task_scheduler = scheduler.clone();
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move { task_scheduler.run(task_cancel).await });
        tokio::time::timeout(Duration::from_secs(1), probe.started.notified())
            .await
            .unwrap();
        assert_eq!(scheduler.observations.grouped.active_jobs(), 1);
        if abort {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            cancel.cancel();
            task.await.unwrap().unwrap();
        }
        assert_eq!(scheduler.observations.grouped.active_jobs(), 0);
        assert_eq!(
            count(
                &scheduler,
                sink(SinkGroup::Analytics, SinkOutcome::Cancelled)
            ),
            1
        );
        assert_eq!(
            count(
                &scheduler,
                exited(if abort {
                    WorkerOutcome::Aborted
                } else {
                    WorkerOutcome::Cancelled
                })
            ),
            1
        );
        assert_eq!(store.failures.lock().unwrap().len(), usize::from(!abort));
    }
}

#[tokio::test]
async fn lease_loss_during_delivery_records_lost_lease_without_completion() {
    let mut store = Store::new(&["local.probe"]);
    store.renew_error_at = 2;
    let scheduler = scheduler(
        Arc::new(store),
        Some(ProbeSink::new("local.probe", Ok(()), true)),
    );
    tokio::time::timeout(
        Duration::from_secs(1),
        scheduler.tick(&CancellationToken::new()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        count(&scheduler, sink(SinkGroup::Local, SinkOutcome::LeaseLost)),
        1
    );
    assert_eq!(
        count(&scheduler, outbox(SinkGroup::All, OutboxOutcome::LeaseLost)),
        1
    );
    assert_eq!(
        count(&scheduler, outbox(SinkGroup::All, OutboxOutcome::Completed)),
        0
    );
    assert_eq!(scheduler.observations.grouped.active_jobs(), 0);
}

#[tokio::test]
async fn actual_poll_intervals_include_work_and_failed_claims_have_explicit_exit() {
    let mut store = Store::new(&[]);
    store.claim_delay = Duration::from_millis(10);
    let store = Arc::new(store);
    let scheduler = scheduler(store.clone(), None);
    let cancel = CancellationToken::new();
    let task_scheduler = scheduler.clone();
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move { task_scheduler.run(task_cancel).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while store.claims.load(Ordering::SeqCst) < 3 {
            store.claim_started.notified().await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    task.await.unwrap().unwrap();
    let polls = scheduler
        .observations
        .grouped
        .snapshot()
        .into_iter()
        .find(|s| {
            s.dimensions
                == SeriesKey::WorkerPollSeconds {
                    backend: DatabaseBackend::Sqlite,
                }
        })
        .unwrap();
    assert!(polls.sample.count >= 2);
    assert!(polls.sample.sum / polls.sample.count as f64 >= 0.010);
    assert_eq!(count(&scheduler, exited(WorkerOutcome::Cancelled)), 1);
    let mut store = Store::new(&[]);
    store.claim_error = true;
    let failed = self::scheduler(Arc::new(store), None);
    assert_eq!(
        failed.run(CancellationToken::new()).await,
        Err(OutboxError::Unavailable)
    );
    assert_eq!(count(&failed, exited(WorkerOutcome::Failed)), 1);
    assert_eq!(
        count(&failed, outbox(SinkGroup::All, OutboxOutcome::StoreError)),
        1
    );
    assert_eq!(
        count(&failed, outbox(SinkGroup::All, OutboxOutcome::Claimed)),
        0
    );
}
