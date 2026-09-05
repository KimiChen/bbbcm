use super::*;
#[tokio::test]
async fn actual_sqlite_announcements_empty_boundaries_and_failure() {
    for prefix in ["bcm_", "status_test_"] {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-status-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = platform::config::Config::default().database;
        config.sqlite_path = dir.join("status.db").to_string_lossy().into();
        config.schema_prefix = prefix.into();
        let source = DbStatusSource::new(config.clone());
        let now = platform::db::decode_time("2026-09-05T00:00:00Z").unwrap();
        assert!(source.read(now).await.is_err());
        assert!(
            !std::path::Path::new(&config.sqlite_path).exists(),
            "status must not create an absent DB"
        );
        crate::migrations::migrate(
            &config,
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db/migrations"),
        )
        .await
        .unwrap();
        let snapshot = source.read(now).await.unwrap();
        assert!(snapshot.announcements.is_empty());
        assert!(!snapshot.public_read_enabled);
        assert_eq!(snapshot.observations.outbox_pending, 0);
        assert_eq!(snapshot.observations.lock_waits, Measurement::Unsupported);
        let pool = Pool::connect_existing(&config).await.unwrap();
        let r = SchemaRenderer::new(prefix).unwrap();
        let table = r
            .table(pool.backend(), "operations", "status_announcements")
            .unwrap();
        let mut tx = DbTransaction::begin(&pool).await.unwrap();
        for (id, published, published_at, ends_at) in [
            ("current", true, now, now + chrono::Duration::hours(1)),
            ("draft", false, now, now + chrono::Duration::hours(1)),
            (
                "future_publication",
                true,
                now + chrono::Duration::seconds(1),
                now + chrono::Duration::hours(1),
            ),
            ("ended", true, now - chrono::Duration::hours(1), now),
        ] {
            tx.connection().execute(&format!("INSERT INTO {table} (announcement_id,kind,title,body,published,published_at,starts_at,ends_at,version) VALUES (?,?,?,?,?,?,?,?,?)"),&[
                id.into(),"maintenance".into(),"合成维护".into(),"<script>synthetic</script>".into(),DbValue::Bool(published),DbValue::Time(published_at),DbValue::Time(now-chrono::Duration::hours(1)),DbValue::Time(ends_at),1i64.into()
            ]).await.unwrap();
        }
        tx.commit().await.unwrap();
        let snapshot = source.read(now).await.unwrap();
        assert_eq!(snapshot.announcements.len(), 1);
        assert_eq!(snapshot.announcements[0].title, "合成维护");
        let events = r
            .table(pool.backend(), "platform", "outbox_events")
            .unwrap();
        let deliveries = r
            .table(pool.backend(), "platform", "sink_deliveries")
            .unwrap();
        let mut tx = DbTransaction::begin(&pool).await.unwrap();
        for (index, (state, error)) in [
            ("ready", ""),
            ("leased", ""),
            ("ready", "retryable"),
            ("ready", "external_disabled"),
            ("dlq", "permanent"),
            ("delivered", ""),
        ]
        .into_iter()
        .enumerate()
        {
            let event = platform::outbox::OutboxEvent {
                event_id: platform::transaction::new_operation_id(),
                aggregate_type: "synthetic".into(),
                aggregate_id: "1".into(),
                event_type: "synthetic.status".into(),
                version: 1,
                occurred_at: now - chrono::Duration::seconds(15),
                payload: platform::outbox::SafePayload::empty(),
                trace_id: String::new(),
            };
            crate::outbox_repo::enqueue(
                &mut tx.connection(),
                &r,
                &event,
                &["local.probe", &format!("synthetic.private_{index}")],
            )
            .await
            .unwrap();
            tx.connection()
                .execute(
                    &format!("UPDATE {events} SET state = ?, error_code = ? WHERE event_id = ?"),
                    &[state.into(), error.into(), event.event_id.clone().into()],
                )
                .await
                .unwrap();
            if state == "delivered" {
                tx.connection()
                    .execute(
                        &format!(
                            "UPDATE {deliveries} SET state = ?, delivered_at = ? WHERE event_id = ?"
                        ),
                        &[
                            "delivered".into(),
                            DbValue::Time(now),
                            event.event_id.into(),
                        ],
                    )
                    .await
                    .unwrap();
            }
        }
        tx.commit().await.unwrap();
        let snapshot = source.read(now).await.unwrap();
        let expected = OutboxStateCounts {
            ready: 1,
            leased: 1,
            retry: 1,
            paused: 1,
            dlq: 1,
            completed: 1,
        };
        assert_eq!(snapshot.observations.outbox_events, expected);
        assert_eq!(snapshot.observations.outbox_pending, 4);
        assert_eq!(
            snapshot.observations.outbox_oldest_pending_ms,
            Measurement::Collected(15_000)
        );
        assert_eq!(
            snapshot.observations.outbox_commit_to_sink_ms,
            Measurement::NotConnected
        );
        let groups = &snapshot.observations.outbox_deliveries;
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].sink, SinkGroup::Local);
        assert_eq!(groups[1].sink, SinkGroup::Other);
        assert!(groups.iter().all(|g| g.counts == expected));
        pool.close().await;
        // Closed pool is an unavailable observation, never an empty/healthy snapshot.
        let closed = DbStatusSource::with_pool(config, pool);
        assert_eq!(
            closed.read(now).await.unwrap_err(),
            StatusError::Unavailable
        );
        drop(source);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
