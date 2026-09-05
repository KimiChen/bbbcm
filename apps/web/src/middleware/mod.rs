//! Observations never read headers, body, query text, raw path or client trace IDs.
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use platform::db::Backend;
use platform::observability::{
    CacheState, HttpMethod, HttpRoute, HttpStatusClass, Metric, REQUEST_ID, SeriesKey, metrics,
    new_request_id,
};
use tracing::Instrument;

pub async fn observe(State(backend): State<Backend>, request: Request, next: Next) -> Response {
    let request_id = new_request_id();
    let method = HttpMethod::from_method(request.method().as_str());
    let route = HttpRoute::from_path(request.uri().path());
    let span = tracing::info_span!("http", request_id = %request_id, trace_id = %request_id, method = ?method, route = ?route);
    REQUEST_ID
        .scope(
            request_id.clone(),
            async move {
                let started = std::time::Instant::now();
                metrics().increment(if matches!(method, HttpMethod::Get | HttpMethod::Head) {
                    Metric::HttpRead
                } else {
                    Metric::HttpWrite
                });
                let mut response = next.run(request).await;
                // Header-ready duration; complete wire/body timing remains the client/E02 boundary.
                metrics().duration(Metric::HttpLatencyMs, started.elapsed());
                metrics().grouped.duration(
                    SeriesKey::HttpHeadersSeconds {
                        backend: backend.into(),
                        route,
                        method,
                        status_class: HttpStatusClass::from_status(response.status().as_u16()),
                        cache_state: CacheState::NotConnected,
                    },
                    started.elapsed(),
                );
                if response.status().is_client_error() || response.status().is_server_error() {
                    metrics().increment(Metric::HttpError);
                }
                tracing::info!(
                    status = response.status().as_u16(),
                    duration_ms = started.elapsed().as_secs_f64() * 1000.,
                    "request completed"
                );
                if let Ok(value) = request_id.parse() {
                    response.headers_mut().insert("x-request-id", value);
                }
                response
            }
            .instrument(span),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::Request as HttpRequest,
        routing::get,
    };
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;
    #[derive(Clone)]
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[tokio::test(flavor = "current_thread")]
    async fn log_and_response_keep_generated_correlation_without_sensitive_input() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(move || Writer(writer.clone()))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let router = Router::new()
            .route(
                "/probe",
                get(|| async {
                    crate::error::ApiError(platform::error::AppError::Internal(
                        "PRIVATE_SENTINEL cookie=secret postgres://secret".into(),
                    ))
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                Backend::Sqlite,
                observe,
            ));
        let response = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/probe?token=PRIVATE_SENTINEL")
                    .header("cookie", "PRIVATE_SENTINEL")
                    .header("x-request-id", "PRIVATE_SENTINEL")
                    .header("traceparent", "PRIVATE_SENTINEL")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let request_id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(platform::transaction::validate_operation_id(&request_id));
        let body = String::from_utf8(
            to_bytes(response.into_body(), 10000)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        let log = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(log.contains(&request_id) && log.contains("request completed"));
        assert!(!log.contains("PRIVATE_SENTINEL") && !log.contains("postgres://"));
        assert!(!body.contains("PRIVATE_SENTINEL"));
        assert!(metrics().grouped.snapshot().iter().any(|series| matches!(
            series.dimensions,
            SeriesKey::HttpHeadersSeconds {
                backend: platform::observability::DatabaseBackend::Sqlite,
                route: HttpRoute::Other,
                method: HttpMethod::Get,
                status_class: HttpStatusClass::ServerError,
                cache_state: CacheState::NotConnected,
            }
        ) && series.sample.count > 0));
        assert!(
            !serde_json::to_string(&metrics().grouped.snapshot())
                .unwrap()
                .contains("PRIVATE_SENTINEL")
        );
        for line in log.lines() {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
    }
}
