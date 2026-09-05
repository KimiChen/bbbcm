use crate::{app_state::AppState, assets::filters, contract::PageFrame, fixtures::FixtureQuery};
use askama::Template;
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::Utc;
use platform::status::{Announcement, DependencyState, Health};
use std::{net::SocketAddr, sync::Arc};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(page))
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/__diagnostics/metrics", get(diagnostics))
}
#[derive(Template)]
#[template(path = "status/index.html")]
struct StatusTemplate {
    frame: PageFrame,
    theme: &'static str,
    service: &'static str,
    community: &'static str,
    degradation: Option<&'static str>,
    announcements: Vec<Announcement>,
}
async fn redis_state(state: &AppState) -> DependencyState {
    match state.runtime.as_ref().and_then(|r| r.redis.as_ref()) {
        Some(redis) => match redis.health().await {
            Ok(()) => DependencyState::Available,
            Err(_) => DependencyState::Unavailable,
        },
        None if state.config.features.redis => DependencyState::NotConnected,
        None => DependencyState::Disabled,
    }
}
async fn live() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({"live":true})),
    )
}
async fn ready(State(state): State<Arc<AppState>>) -> Response {
    let (snapshot, redis) = tokio::join!(state.status.read(Utc::now()), redis_state(&state));
    let health = Health::from_dependencies(
        if snapshot.is_ok() {
            DependencyState::Available
        } else {
            DependencyState::Unavailable
        },
        redis,
    );
    (
        if health.ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        [(header::CACHE_CONTROL, "no-store")],
        Json(health),
    )
        .into_response()
}
async fn page(State(state): State<Arc<AppState>>, Query(query): Query<FixtureQuery>) -> Response {
    let (snapshot, redis) = tokio::join!(state.status.read(Utc::now()), redis_state(&state));
    let (code, frame, service, community, announcements) = match snapshot {
        Ok(s) => (
            StatusCode::OK,
            PageFrame {
                empty: if s.announcements.is_empty() {
                    Some("暂无已发布的计划维护或事故公告。")
                } else {
                    None
                },
                ..Default::default()
            },
            "可响应",
            if s.public_read_enabled {
                "公开阅读已开放"
            } else {
                "社区功能尚未开放"
            },
            s.announcements,
        ),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            PageFrame {
                error: Some("暂时无法读取最新状态和公告，请稍后刷新。"),
                ..Default::default()
            },
            "暂不可用",
            "状态未知",
            Vec::new(),
        ),
    };
    let template = StatusTemplate {
        frame,
        theme: query.theme.as_str(),
        service,
        community,
        degradation: if redis == DependencyState::Unavailable {
            Some("部分服务降级。")
        } else {
            None
        },
        announcements,
    };
    match crate::router::render(&template, &state.assets) {
        Ok(html) => (
            code,
            [
                (header::CACHE_CONTROL, "no-store"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            html,
        )
            .into_response(),
        Err(error) => error.into_response(),
    }
}
/// No production/staging diagnostic route or forwarded-header trust. Only the local
/// development peer may inspect the process, and even this projection has no secrets.
async fn diagnostics(
    State(state): State<Arc<AppState>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
) -> Response {
    if state.config.server.environment.is_production_like()
        || !peer.is_some_and(|p| p.0.0.ip().is_loopback())
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let snapshot = state.status.read(Utc::now()).await;
    let db = match snapshot {
        Ok(s) => platform::observability::Measurement::Collected(s.observations),
        Err(_) => platform::observability::Measurement::Unavailable,
    };
    let redis = state
        .runtime
        .as_ref()
        .and_then(|r| r.redis.as_ref())
        .map(|r| r.metrics());
    ([(header::CACHE_CONTROL,"no-store")],Json(serde_json::json!({
        "observed_at":platform::db::encode_time(Utc::now()),
        "process_instance":platform::observability::metrics().instance_id(),
        "process":platform::observability::metrics().snapshot(),
        "database":db,"database_pools":platform::observability::metrics().pools.snapshot(),
        "series":platform::observability::metrics().grouped.snapshot(),
        "worker_active_jobs":platform::observability::metrics().grouped.active_jobs_observation(),
        "series_histogram_bounds_seconds":platform::observability::LATENCY_BOUNDS_SECONDS,
        "redis":redis,"redis_enabled":state.config.features.redis,
        "mail":"not_connected","media":"not_connected","search":"not_connected",
        "duration_boundary":"origin_response_headers_ready","histogram_bounds_ms":platform::observability::LATENCY_BOUNDS_MS
    }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use platform::{observability::Measurement, outbox::BoxFuture, status::*};
    use tower::ServiceExt;
    struct Source(bool);
    impl StatusSource for Source {
        fn read(
            &self,
            _: chrono::DateTime<Utc>,
        ) -> BoxFuture<'_, Result<StatusSnapshot, StatusError>> {
            Box::pin(async move {
                if self.0 {
                    return Err(StatusError::Unavailable);
                }
                Ok(StatusSnapshot {
                    public_read_enabled: false,
                    announcements: Vec::new(),
                    observations: DatabaseObservations {
                        pool_size: 1,
                        pool_idle: 1,
                        outbox_pending: 0,
                        outbox_dlq: 0,
                        outbox_leased: 0,
                        outbox_oldest_pending_ms: Measurement::Collected(0),
                        outbox_events: OutboxStateCounts::default(),
                        outbox_deliveries: Vec::new(),
                        outbox_commit_to_sink_ms: Measurement::NotConnected,
                        lock_waits: Measurement::Unsupported,
                        long_transactions: Measurement::Unsupported,
                        deadlocks: Measurement::Unsupported,
                        autovacuum: Measurement::Unsupported,
                        dead_tuples: Measurement::Unsupported,
                        wal_bytes: Measurement::Unsupported,
                        relation_bytes: Measurement::Unsupported,
                    },
                })
            })
        }
    }
    async fn request(failed: bool, path: &str) -> Response {
        let mut state = AppState::new(
            platform::config::Config::default(),
            crate::assets::tests::local(),
        );
        state.status = Arc::new(Source(failed));
        crate::router::router(Arc::new(state))
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn status_empty_fault_theme_and_health_are_real_states() {
        for theme in ["light", "dark", "system"] {
            let response = request(false, &format!("/status?theme={theme}")).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let body = String::from_utf8(
                to_bytes(response.into_body(), 1_000_000)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap();
            assert!(body.contains("暂无已发布的计划维护或事故公告"));
            assert!(body.contains("社区功能尚未开放"));
            assert!(body.contains("草莓论坛"));
            assert!(!body.contains("全部正常"));
            assert!(!body.contains("postgres") && !body.contains("pool_size"));
        }
        let response = request(true, "/status").await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = String::from_utf8(
            to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains("暂时无法读取最新状态和公告"));
        assert!(!body.contains("暂无已发布"));
        assert_eq!(request(true, "/health/live").await.status(), StatusCode::OK);
        assert_eq!(
            request(true, "/health/ready").await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            request(false, "/health/ready").await.status(),
            StatusCode::OK
        );
        assert_eq!(
            request(false, "/__diagnostics/metrics").await.status(),
            StatusCode::NOT_FOUND
        );
    }
    #[test]
    fn announcement_content_is_escaped_and_uses_frozen_local_assets() {
        let template = StatusTemplate {
            frame: PageFrame::default(),
            theme: "light",
            service: "可响应",
            community: "社区功能尚未开放",
            degradation: None,
            announcements: vec![Announcement {
                kind: AnnouncementKind::Incident,
                title: "<script>private</script>".into(),
                body: "<img src=evil>".into(),
                starts_at: Utc::now(),
                ends_at: Utc::now(),
            }],
        };
        let html = crate::router::render(&template, &crate::assets::tests::local())
            .unwrap_or_else(|_| panic!("status renders"))
            .0;
        assert!(!html.contains("<script>private"));
        assert!(!html.contains("<img src=evil>"));
        assert!(html.contains("&lt;script") || html.contains("&#60;script"));
        assert!(!html.contains("https://") && !html.contains("src=\"//"));
    }
}
