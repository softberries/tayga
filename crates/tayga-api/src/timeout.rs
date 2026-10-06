//! Bounds every `/api/` request. ClickHouse stops a read at `max_execution_time` (the route then
//! answers 504); this layer answers 504 when a request is still running `SLACK_SECS` later, e.g.
//! against a ClickHouse that accepts the connection and never answers. The app, `/metrics` and
//! `/healthz` are not bounded.

use axum::Json;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::time::Duration;

/// Seconds past `query_timeout_secs` before a request is answered 504.
pub const SLACK_SECS: u64 = 5;

pub async fn api_timeout(State(limit): State<Duration>, req: Request, next: Next) -> Response {
    if !req.uri().path().starts_with("/api/") {
        return next.run(req).await;
    }
    match tokio::time::timeout(limit, next.run(req)).await {
        Ok(res) => res,
        Err(_) => {
            tracing::warn!(limit_ms = limit.as_millis() as u64, "api request timed out");
            storage_timeout()
        }
    }
}

/// The one answer for "storage too slow": a JSON 504.
pub fn storage_timeout() -> Response {
    (
        StatusCode::GATEWAY_TIMEOUT,
        Json(serde_json::json!({ "error": "storage timeout" })),
    )
        .into_response()
}

/// True when ClickHouse stopped the query at `max_execution_time` (error 159, TIMEOUT_EXCEEDED).
pub fn is_clickhouse_timeout(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        matches!(
            c.downcast_ref::<clickhouse::error::Error>(),
            Some(clickhouse::error::Error::BadResponse(m)) if m.contains("Code: 159")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::middleware;
    use axum::routing::get;
    use tower::ServiceExt;

    fn app(limit: Duration) -> Router {
        Router::new()
            .route(
                "/api/v1/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    "late"
                }),
            )
            .route("/api/v1/fast", get(|| async { "ok" }))
            .route(
                "/slow-page",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    "page"
                }),
            )
            .layer(middleware::from_fn_with_state(limit, api_timeout))
    }

    async fn call(app: Router, uri: &str) -> (StatusCode, String) {
        let res = app
            .oneshot(HttpRequest::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), 1 << 16)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn a_slow_api_request_is_a_json_504() {
        let started = std::time::Instant::now();
        let (status, body) = call(app(Duration::from_millis(50)), "/api/v1/slow").await;
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(body, r#"{"error":"storage timeout"}"#);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn fast_api_requests_and_other_paths_are_not_bounded() {
        let limit = Duration::from_millis(50);
        assert_eq!(
            call(app(limit), "/api/v1/fast").await,
            (StatusCode::OK, "ok".into())
        );
        assert_eq!(
            call(app(limit), "/slow-page").await,
            (StatusCode::OK, "page".into())
        );
    }
}
