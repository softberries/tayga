//! JSON API (spec §10).

use crate::params::{
    alert_filter, group_filter, parse_fingerprint, parse_hex_id, parse_since, since_or,
    template_filter,
};
use crate::repo::Repo;
use axum::Json;
use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::registry::Registry;
use serde::Deserialize;
use std::sync::Arc;

/// Label for counters split by scrape job.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct JobLabel {
    pub job: String,
}

#[derive(Clone, Default)]
pub struct ApiMetrics {
    pub repo_errors: Counter,
    pub scrape_failures: Family<JobLabel, Counter>,
    pub lag_errors: Counter,
}

impl ApiMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let m = Self::default();
        registry.register(
            "tayga_api_repo_errors",
            "Requests answered 503 because ClickHouse failed",
            m.repo_errors.clone(),
        );
        registry.register(
            "tayga_api_scrape_failures",
            "Metric scrapes of a recorder target that failed",
            m.scrape_failures.clone(),
        );
        registry.register(
            "tayga_api_lag_errors",
            "Consumer-lag reads from Kafka that failed",
            m.lag_errors.clone(),
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
pub struct AlertsQuery {
    pub since: Option<String>,
    pub kind: Option<String>,
    pub service: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct TemplatesQuery {
    pub since: Option<String>,
    pub service: Option<String>,
    pub q: Option<String>,
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
        .route("/api/v1/log-alerts", get(log_alerts::<R>))
        .route("/api/v1/log-templates", get(log_templates::<R>))
        .route("/api/v1/log-templates/{id}", get(log_template::<R>))
        .route(
            "/api/v1/traces/{trace_id}/log-templates",
            get(trace_log_templates::<R>),
        )
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
    let since = parse_since(since_or(q.since.as_deref(), "24h")).map_err(ApiError::BadRequest)?;
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
    let since = parse_since(since_or(q.since.as_deref(), "1h")).map_err(ApiError::BadRequest)?;
    let graph = s
        .repo
        .service_graph(since)
        .await
        .map_err(|e| s.unavailable(e))?;
    Ok(Json(graph).into_response())
}

async fn log_alerts<R: Repo>(
    State(s): State<AppState<R>>,
    q: Result<Query<AlertsQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(q) = q.map_err(|r| ApiError::BadRequest(r.body_text()))?;
    let f = alert_filter(q.since.as_deref(), q.kind.as_deref(), q.service.as_deref())
        .map_err(ApiError::BadRequest)?;
    let alerts = s.repo.log_alerts(&f).await.map_err(|e| s.unavailable(e))?;
    Ok(Json(alerts).into_response())
}

async fn log_templates<R: Repo>(
    State(s): State<AppState<R>>,
    q: Result<Query<TemplatesQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(q) = q.map_err(|r| ApiError::BadRequest(r.body_text()))?;
    let f = template_filter(q.since.as_deref(), q.service.as_deref(), q.q.as_deref())
        .map_err(ApiError::BadRequest)?;
    let templates = s
        .repo
        .log_templates(&f)
        .await
        .map_err(|e| s.unavailable(e))?;
    Ok(Json(templates).into_response())
}

async fn log_template<R: Repo>(
    State(s): State<AppState<R>>,
    Path(id): Path<String>,
    q: Result<Query<SinceQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(q) = q.map_err(|r| ApiError::BadRequest(r.body_text()))?;
    let id = parse_fingerprint(&id).map_err(ApiError::BadRequest)?;
    let since = parse_since(since_or(q.since.as_deref(), "24h")).map_err(ApiError::BadRequest)?;
    match s
        .repo
        .log_template(&id, since)
        .await
        .map_err(|e| s.unavailable(e))?
    {
        Some(d) => Ok(Json(d).into_response()),
        None => Err(ApiError::NotFound),
    }
}

async fn trace_log_templates<R: Repo>(
    State(s): State<AppState<R>>,
    Path(trace_id): Path<String>,
) -> Result<Response, ApiError> {
    let id = parse_hex_id(&trace_id).map_err(ApiError::BadRequest)?;
    let rows = s
        .repo
        .trace_log_templates(&id)
        .await
        .map_err(|e| s.unavailable(e))?;
    Ok(Json(rows).into_response())
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
            bucket_secs: 60,
            buckets: vec![(60, 4)],
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
        assert_eq!(json[0]["buckets"][0][1], 4);
        assert_eq!(json[0]["bucket_secs"], 60);
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
    async fn empty_since_uses_the_default() {
        let repo = Arc::new(FakeRepo::default());
        let (status, _) = get_with(
            repo.clone(),
            ApiMetrics::default(),
            "/api/v1/story-groups?since=&kind=&service=",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            repo.last_filter
                .lock()
                .unwrap()
                .as_ref()
                .map(|f| f.since_secs),
            Some(3600)
        );
        // No such group in the fake: 404 proves `since=` was accepted rather than a 400.
        assert_eq!(
            get(FakeRepo::default(), "/api/v1/story-groups/42?since=")
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get(FakeRepo::default(), "/api/v1/service-map?since=%20")
                .await
                .0,
            StatusCode::OK
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

    fn template_view() -> LogTemplateView {
        LogTemplateView {
            template_id: "17393964261140422938".into(),
            service: "payment".into(),
            template: "Payment request failed <*>".into(),
            count: 9,
            first_seen_ns: 1,
            last_seen_ns: 2,
            max_severity: 17,
            alerting: true,
        }
    }

    #[tokio::test]
    async fn log_alerts_json_and_filter() {
        let alert = LogAlertView {
            alert_id: "a1".into(),
            kind: "spike".into(),
            template_id: "17393964261140422938".into(),
            service: "payment".into(),
            template: "Payment request failed <*>".into(),
            started_at_ns: 1,
            last_at_ns: 2,
            window_count: 30,
            peak_count: 31,
            baseline_per_window: 0.5,
            active: true,
            example_traces: vec![
                ExampleTrace {
                    trace_id: "ab".repeat(16),
                    story_id: Some("ab".repeat(16)),
                },
                ExampleTrace {
                    trace_id: "cd".repeat(16),
                    story_id: None,
                },
            ],
        };
        let repo = Arc::new(FakeRepo {
            alerts: vec![alert],
            ..Default::default()
        });
        let (status, json) = get_with(
            repo.clone(),
            ApiMetrics::default(),
            "/api/v1/log-alerts?kind=spike&service=payment&since=",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["template_id"], "17393964261140422938");
        assert_eq!(json[0]["active"], true);
        assert_eq!(json[0]["example_traces"][0]["story_id"], "ab".repeat(16));
        assert!(json[0]["example_traces"][1]["story_id"].is_null());
        assert_eq!(
            repo.last_alert_filter.lock().unwrap().clone(),
            Some(crate::params::AlertFilter {
                since_secs: 86_400,
                kind: Some("spike".into()),
                service: Some("payment".into()),
            })
        );
    }

    #[tokio::test]
    async fn log_templates_json_filter_and_validation() {
        let repo = Arc::new(FakeRepo {
            templates: vec![template_view()],
            ..Default::default()
        });
        let (status, json) = get_with(
            repo.clone(),
            ApiMetrics::default(),
            "/api/v1/log-templates?q=%20failed%20&service=payment",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["template_id"], "17393964261140422938");
        assert_eq!(json[0]["alerting"], true);
        assert_eq!(json[0]["count"], 9);
        let f = repo.last_template_filter.lock().unwrap().clone().unwrap();
        assert_eq!(
            (f.since_secs, f.q.as_deref(), f.service.as_deref()),
            (3600, Some("failed"), Some("payment"))
        );
        let long = format!("/api/v1/log-templates?q={}", "a".repeat(201));
        assert_eq!(
            get(FakeRepo::default(), &long).await.0,
            StatusCode::BAD_REQUEST
        );
        let ok = format!("/api/v1/log-templates?q={}", "a".repeat(200));
        assert_eq!(get(FakeRepo::default(), &ok).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn bad_log_params_are_400() {
        for uri in [
            "/api/v1/log-alerts?kind=bogus",
            "/api/v1/log-alerts?since=9d",
            "/api/v1/log-templates?since=0s",
            "/api/v1/log-templates/notanumber",
            "/api/v1/log-templates/1?since=nope",
            "/api/v1/traces/xyz/log-templates",
        ] {
            assert_eq!(
                get(FakeRepo::default(), uri).await.0,
                StatusCode::BAD_REQUEST,
                "{uri}"
            );
        }
    }

    #[tokio::test]
    async fn log_template_detail_404_and_found() {
        assert_eq!(
            get(FakeRepo::default(), "/api/v1/log-templates/42").await.0,
            StatusCode::NOT_FOUND
        );
        let repo = FakeRepo {
            template_detail: Some(LogTemplateDetail {
                template: template_view(),
                sample: "Payment request failed 500".into(),
                bucket_secs: 60,
                buckets: vec![(60, 3)],
                recent: vec![TemplateHitView {
                    ts_ns: 5,
                    trace_id: "ab".repeat(16),
                    span_id: "01".repeat(8),
                    severity_number: 17,
                    story_id: None,
                }],
                alerts: vec![],
            }),
            ..Default::default()
        };
        let (status, json) = get(repo, "/api/v1/log-templates/17393964261140422938?since=").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["template"]["template_id"], "17393964261140422938");
        assert_eq!(json["sample"], "Payment request failed 500");
        assert_eq!(json["buckets"][0][1], 3);
        assert_eq!(json["recent"][0]["severity_number"], 17);
    }

    #[tokio::test]
    async fn trace_log_templates_json() {
        let uri = format!("/api/v1/traces/{}/log-templates", "ab".repeat(16));
        let (status, json) = get(FakeRepo::default(), &uri).await;
        assert_eq!((status, json), (StatusCode::OK, serde_json::json!([])));
        let repo = FakeRepo {
            trace_templates: vec![TraceLogTemplate {
                log_id: "99".into(),
                template_id: "17393964261140422938".into(),
                template: "t".into(),
                alert: Some("new".into()),
            }],
            ..Default::default()
        };
        let (status, json) = get(repo, &uri).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["log_id"], "99");
        assert_eq!(json[0]["alert"], "new");
    }

    #[tokio::test]
    async fn log_routes_repo_failure_is_503() {
        for uri in [
            "/api/v1/log-alerts".to_string(),
            "/api/v1/log-templates".to_string(),
            "/api/v1/log-templates/42".to_string(),
            format!("/api/v1/traces/{}/log-templates", "ab".repeat(16)),
        ] {
            let metrics = ApiMetrics::default();
            let repo = FakeRepo {
                fail: true,
                ..Default::default()
            };
            let (status, json) = get_with(Arc::new(repo), metrics.clone(), &uri).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
            assert_eq!(json["error"], "storage unavailable");
            assert_eq!(metrics.repo_errors.get(), 1);
        }
    }

    #[tokio::test]
    async fn trace_carries_attrs_events_self_time_and_story() {
        let id = "ab".repeat(16);
        let repo = FakeRepo {
            trace: Some(TraceView {
                trace_id: id.clone(),
                spans: vec![TraceSpanRow {
                    span_id: "01".repeat(8),
                    parent_span_id: String::new(),
                    service_name: "payment".into(),
                    span_name: "charge".into(),
                    kind: "server".into(),
                    start_ns: 1,
                    duration_ns: 10,
                    status: "error".into(),
                    status_message: "boom".into(),
                    attrs: vec![("http.method".into(), "POST".into())],
                    resource: vec![("service.name".into(), "payment".into())],
                    events: vec![SpanEvent {
                        ts_ns: 3,
                        name: "exception".into(),
                        attrs: vec![("exception.stacktrace".into(), "at x".into())],
                    }],
                    self_ns: 4,
                }],
                logs: vec![],
                story_id: Some(id.clone()),
            }),
            ..Default::default()
        };
        let (status, json) = get(repo, &format!("/api/v1/traces/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        let span = &json["spans"][0];
        assert_eq!(span["attrs"], serde_json::json!([["http.method", "POST"]]));
        assert_eq!(span["resource"][0][1], "payment");
        assert_eq!(span["events"][0]["name"], "exception");
        assert_eq!(span["events"][0]["attrs"][0][0], "exception.stacktrace");
        assert_eq!(span["self_ns"], 4);
        assert_eq!(json["story_id"], id);
    }

    #[tokio::test]
    async fn service_map_has_edges_and_nodes() {
        let repo = Arc::new(FakeRepo {
            edges: vec![EdgeView::from_row(EdgeRow {
                parent_service: "frontend".into(),
                child_service: "payment".into(),
                calls: 4,
                errors: 1,
                duration_ns_sum: 40,
            })],
            nodes: vec![NodeView::from_row(
                NodeRow {
                    service: "payment".into(),
                    calls: 100,
                    errors: 10,
                    p99_ns: 5.0,
                    baseline_p99_ns: 1.0,
                },
                100,
            )],
            ..Default::default()
        });
        let (status, json) = get_with(
            repo.clone(),
            ApiMetrics::default(),
            "/api/v1/service-map?since=15m",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(*repo.last_since.lock().unwrap(), Some(900));
        assert_eq!(json["edges"][0]["parent"], "frontend");
        assert_eq!(json["edges"][0]["error_rate"], 0.25);
        let node = &json["nodes"][0];
        assert_eq!(node["service"], "payment");
        assert_eq!(node["health"], "error");
        assert_eq!(node["rate"], 1.0);
        assert_eq!(node["error_ratio"], 0.1);
        assert_eq!(node["p99_ns"], 5.0);
    }
}
