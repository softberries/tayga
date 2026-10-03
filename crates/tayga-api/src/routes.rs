//! JSON API (spec §10).

use crate::params::{group_filter, parse_fingerprint, parse_hex_id, parse_since};
use crate::repo::Repo;
use axum::Json;
use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::registry::Registry;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct ApiMetrics {
    pub repo_errors: Counter,
}

impl ApiMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let m = Self::default();
        registry.register(
            "tayga_api_repo_errors",
            "Requests answered 503 because ClickHouse failed",
            m.repo_errors.clone(),
        );
        m
    }
}

pub enum ApiError {
    BadRequest(String),
    NotFound,
    Unavailable(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::Unavailable(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "storage unavailable".to_string(),
            ),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

pub struct AppState<R> {
    pub repo: Arc<R>,
    pub metrics: ApiMetrics,
}

impl<R> Clone for AppState<R> {
    fn clone(&self) -> Self {
        Self {
            repo: self.repo.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

impl<R> AppState<R> {
    pub fn unavailable(&self, e: anyhow::Error) -> ApiError {
        tracing::warn!(error = %e, "repository query failed");
        self.metrics.repo_errors.inc();
        ApiError::Unavailable(e)
    }
}

#[derive(Deserialize, Default)]
pub struct GroupsQuery {
    pub since: Option<String>,
    pub kind: Option<String>,
    pub service: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct SinceQuery {
    pub since: Option<String>,
}

pub fn api_router<R: Repo>(repo: Arc<R>, metrics: ApiMetrics) -> Router {
    Router::new()
        .route("/api/v1/story-groups", get(groups::<R>))
        .route("/api/v1/story-groups/{fingerprint}", get(group::<R>))
        .route("/api/v1/stories/{story_id}", get(story::<R>))
        .route("/api/v1/traces/{trace_id}", get(trace::<R>))
        .route("/api/v1/service-map", get(service_map::<R>))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(AppState { repo, metrics })
}

async fn groups<R: Repo>(
    State(s): State<AppState<R>>,
    q: Result<Query<GroupsQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(q) = q.map_err(|r| ApiError::BadRequest(r.body_text()))?;
    let f = group_filter(q.since.as_deref(), q.kind.as_deref(), q.service.as_deref())
        .map_err(ApiError::BadRequest)?;
    let groups = s
        .repo
        .story_groups(&f)
        .await
        .map_err(|e| s.unavailable(e))?;
    Ok(Json(groups).into_response())
}

async fn group<R: Repo>(
    State(s): State<AppState<R>>,
    Path(fingerprint): Path<String>,
    q: Result<Query<SinceQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(q) = q.map_err(|r| ApiError::BadRequest(r.body_text()))?;
    let fp = parse_fingerprint(&fingerprint).map_err(ApiError::BadRequest)?;
    let since = parse_since(q.since.as_deref().unwrap_or("24h")).map_err(ApiError::BadRequest)?;
    match s
        .repo
        .story_group(&fp, since)
        .await
        .map_err(|e| s.unavailable(e))?
    {
        Some(d) => Ok(Json(d).into_response()),
        None => Err(ApiError::NotFound),
    }
}

async fn story<R: Repo>(
    State(s): State<AppState<R>>,
    Path(story_id): Path<String>,
) -> Result<Response, ApiError> {
    let id = parse_hex_id(&story_id).map_err(ApiError::BadRequest)?;
    match s.repo.story(&id).await.map_err(|e| s.unavailable(e))? {
        Some(v) => Ok(Json(v).into_response()),
        None => Err(ApiError::NotFound),
    }
}

async fn trace<R: Repo>(
    State(s): State<AppState<R>>,
    Path(trace_id): Path<String>,
) -> Result<Response, ApiError> {
    let id = parse_hex_id(&trace_id).map_err(ApiError::BadRequest)?;
    let t = s.repo.trace(&id).await.map_err(|e| s.unavailable(e))?;
    if t.spans.is_empty() && t.logs.is_empty() {
        return Err(ApiError::NotFound);
    }
    Ok(Json(t).into_response())
}

async fn service_map<R: Repo>(
    State(s): State<AppState<R>>,
    q: Result<Query<SinceQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(q) = q.map_err(|r| ApiError::BadRequest(r.body_text()))?;
    let since = parse_since(q.since.as_deref().unwrap_or("1h")).map_err(ApiError::BadRequest)?;
    let edges = s
        .repo
        .service_map(since)
        .await
        .map_err(|e| s.unavailable(e))?;
    Ok(Json(edges).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::record;
    use crate::model::*;
    use crate::testrepo::FakeRepo;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn group_view() -> GroupView {
        GroupView {
            group: StoryGroupRow {
                fingerprint: "17393964261140422938".into(),
                kind: "error".into(),
                summary: "payment charge failed".into(),
                rc_service: "payment".into(),
                rc_span_name: "charge".into(),
                endpoint_service: "load-generator".into(),
                endpoint_name: "user_checkout_single".into(),
                stories: 4,
                first_seen_ns: 1,
                last_seen_ns: 2,
                sample_story_id: "ab".repeat(16),
            },
            per_minute: vec![(60, 4)],
        }
    }

    async fn get(repo: FakeRepo, uri: &str) -> (StatusCode, serde_json::Value) {
        get_with(Arc::new(repo), ApiMetrics::default(), uri).await
    }

    async fn get_with(
        repo: Arc<FakeRepo>,
        metrics: ApiMetrics,
        uri: &str,
    ) -> (StatusCode, serde_json::Value) {
        let app = api_router(repo, metrics);
        let res = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn groups_json_has_string_fingerprint_and_filter_is_parsed() {
        let repo = FakeRepo {
            groups: vec![group_view()],
            ..Default::default()
        };
        let repo = Arc::new(repo);
        let (status, json) = get_with(
            repo.clone(),
            ApiMetrics::default(),
            "/api/v1/story-groups?since=15m&kind=error&service=payment",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["fingerprint"], "17393964261140422938");
        assert_eq!(json[0]["per_minute"][0][1], 4);
        assert_eq!(json[0]["rc_service"], "payment");
        assert_eq!(
            repo.last_filter.lock().unwrap().clone(),
            Some(crate::params::GroupFilter {
                since_secs: 900,
                kind: Some("error".into()),
                service: Some("payment".into()),
            })
        );
    }

    #[tokio::test]
    async fn bad_params_are_400() {
        assert_eq!(
            get(FakeRepo::default(), "/api/v1/story-groups?since=9d")
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            get(FakeRepo::default(), "/api/v1/story-groups/notanumber")
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        let long = format!("/api/v1/stories/{}", "a".repeat(5000));
        assert_eq!(
            get(FakeRepo::default(), &long).await.0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn missing_story_is_404_and_found_story_is_json() {
        let id = "ab".repeat(16);
        assert_eq!(
            get(FakeRepo::default(), &format!("/api/v1/stories/{id}"))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        let repo = FakeRepo {
            story: Some(StoryView::from_record(record())),
            ..Default::default()
        };
        let (status, json) = get(repo, &format!("/api/v1/stories/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["root_cause"]["service"], "payment");
    }

    #[tokio::test]
    async fn repo_failure_is_503() {
        let metrics = ApiMetrics::default();
        let repo = FakeRepo {
            fail: true,
            ..Default::default()
        };
        let (status, json) = get_with(Arc::new(repo), metrics.clone(), "/api/v1/service-map").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"], "storage unavailable");
        assert_eq!(metrics.repo_errors.get(), 1);
    }

    #[tokio::test]
    async fn malformed_query_is_json_400() {
        let (status, json) =
            get(FakeRepo::default(), "/api/v1/service-map?since=1h&since=2h").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().is_some());
    }

    #[tokio::test]
    async fn empty_trace_is_404_and_healthz_ok() {
        assert_eq!(
            get(
                FakeRepo::default(),
                &format!("/api/v1/traces/{}", "cd".repeat(16))
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        let app = api_router(Arc::new(FakeRepo::default()), ApiMetrics::default());
        let res = app
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
