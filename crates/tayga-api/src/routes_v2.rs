//! JSON routes for the web app (spec §8): overview, story series, trace search, services,
//! ⌘K search, pipeline series and lag, and the client config.

use crate::lag::{self, Lag};
use crate::model::SeriesView;
use crate::params::{
    SeriesKind, TraceParams, group_filter, parse_q, parse_service, series_query, trace_filter,
};
use crate::repo::Repo;
use crate::routes::{ApiError, ApiMetrics, AppState, GroupsQuery, WindowQuery, window_of};
use crate::series;
use axum::Json;
use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One consumer-lag read from the broker.
pub type LagFetch = Arc<dyn Fn() -> BoxFuture<'static, anyhow::Result<Vec<Lag>>> + Send + Sync>;

/// How long a lag result, success or failure, is served before the broker is asked again.
pub const LAG_TTL: Duration = Duration::from_secs(5);

type LagSlot = Option<(Instant, Result<Vec<Lag>, String>)>;

/// Single-flight cache in front of the lag read. `lag::fetch` runs on a blocking thread that
/// outlives its 5 s timeout while the broker is down, so requests must not each start one:
/// the lock is held across the read, so concurrent requests wait for the one in flight and then
/// share its result, and any result younger than the TTL is served without a read. The read runs
/// in its own task that owns the lock, so a request dropped mid-read (client gone) neither
/// cancels it nor lets the next request start a second one.
pub struct LagCache {
    fetch: LagFetch,
    ttl: Duration,
    slot: Arc<tokio::sync::Mutex<LagSlot>>,
}

impl LagCache {
    pub fn new(fetch: LagFetch, ttl: Duration) -> Self {
        Self {
            fetch,
            ttl,
            slot: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Lag of the Tayga consumer groups on `topic`, read through `brokers`.
    pub fn kafka(brokers: String, topic: String) -> Self {
        let fetch: LagFetch = Arc::new(move || {
            let (brokers, topic) = (brokers.clone(), topic.clone());
            Box::pin(async move { lag::fetch(&brokers, &topic, &lag::GROUPS).await })
        });
        Self::new(fetch, LAG_TTL)
    }

    /// The cached result, or a fresh read when it is older than the TTL. The flag is true when
    /// this call performed the read.
    pub async fn get(&self) -> (Result<Vec<Lag>, String>, bool) {
        let mut slot = self.slot.clone().lock_owned().await;
        if let Some((at, result)) = slot.as_ref()
            && at.elapsed() < self.ttl
        {
            return (result.clone(), false);
        }
        let read = (self.fetch)();
        let task = tokio::spawn(async move {
            let result = read.await.map_err(|e| format!("{e:#}"));
            *slot = Some((Instant::now(), result.clone()));
            result
        });
        let result = task
            .await
            .unwrap_or_else(|e| Err(format!("lag read task failed: {e}")));
        (result, true)
    }
}

/// External links the web app may show; empty settings are `null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClientConfig {
    pub jaeger_url: Option<String>,
    pub grafana_url: Option<String>,
}

impl ClientConfig {
    pub fn new(jaeger_url: &str, grafana_url: &str) -> Self {
        let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        Self {
            jaeger_url: opt(jaeger_url),
            grafana_url: opt(grafana_url),
        }
    }
}

struct V2State<R> {
    app: AppState<R>,
    lag: Arc<LagCache>,
    config: Arc<ClientConfig>,
}

impl<R> Clone for V2State<R> {
    fn clone(&self) -> Self {
        Self {
            app: self.app.clone(),
            lag: self.lag.clone(),
            config: self.config.clone(),
        }
    }
}

#[derive(Deserialize, Default)]
pub struct TracesQuery {
    pub since: Option<String>,
    pub until: Option<String>,
    pub service: Option<String>,
    pub touched: Option<String>,
    pub endpoint: Option<String>,
    pub min_ms: Option<String>,
    pub max_ms: Option<String>,
    pub errors: Option<String>,
    pub limit: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct SearchQuery {
    pub q: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct SeriesParams {
    pub since: Option<String>,
    pub until: Option<String>,
    pub metric: Option<String>,
    pub job: Option<String>,
    pub kind: Option<String>,
    pub labels: Option<String>,
}

pub fn router<R: Repo>(
    repo: Arc<R>,
    metrics: ApiMetrics,
    lag: LagCache,
    config: ClientConfig,
) -> Router {
    Router::new()
        .route("/api/v1/overview", get(overview::<R>))
        .route("/api/v1/stories/series", get(stories_series::<R>))
        .route("/api/v1/traces/search", get(traces_search::<R>))
        .route("/api/v1/services", get(services::<R>))
        .route("/api/v1/services/{name}", get(service::<R>))
        .route("/api/v1/search", get(search::<R>))
        .route("/api/v1/pipeline/series", get(pipeline_series::<R>))
        .route("/api/v1/pipeline/lag", get(pipeline_lag::<R>))
        .route("/api/v1/config", get(client_config::<R>))
        .with_state(V2State {
            app: AppState { repo, metrics },
            lag: Arc::new(lag),
            config: Arc::new(config),
        })
}

fn query<T>(q: Result<Query<T>, QueryRejection>) -> Result<T, ApiError> {
    q.map(|Query(q)| q)
        .map_err(|r| ApiError::BadRequest(r.body_text()))
}

async fn overview<R: Repo>(
    State(s): State<V2State<R>>,
    q: Result<Query<WindowQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let q = query(q)?;
    let w = window_of(&q.since, &q.until, "1h")?;
    let v = s
        .app
        .repo
        .overview(w)
        .await
        .map_err(|e| s.app.unavailable(e))?;
    Ok(Json(v).into_response())
}

async fn stories_series<R: Repo>(
    State(s): State<V2State<R>>,
    q: Result<Query<GroupsQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let q = query(q)?;
    let w = window_of(&q.since, &q.until, "1h")?;
    let f =
        group_filter(w, q.kind.as_deref(), q.service.as_deref()).map_err(ApiError::BadRequest)?;
    let v = s
        .app
        .repo
        .stories_series(&f)
        .await
        .map_err(|e| s.app.unavailable(e))?;
    Ok(Json(v).into_response())
}

async fn traces_search<R: Repo>(
    State(s): State<V2State<R>>,
    q: Result<Query<TracesQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let q = query(q)?;
    let w = window_of(&q.since, &q.until, "1h")?;
    let f = trace_filter(
        w,
        &TraceParams {
            service: q.service.as_deref(),
            touched: q.touched.as_deref(),
            endpoint: q.endpoint.as_deref(),
            min_ms: q.min_ms.as_deref(),
            max_ms: q.max_ms.as_deref(),
            errors: q.errors.as_deref(),
            limit: q.limit.as_deref(),
        },
    )
    .map_err(ApiError::BadRequest)?;
    let v = s
        .app
        .repo
        .traces_search(&f)
        .await
        .map_err(|e| s.app.unavailable(e))?;
    Ok(Json(v).into_response())
}

async fn services<R: Repo>(State(s): State<V2State<R>>) -> Result<Response, ApiError> {
    let v = s
        .app
        .repo
        .services()
        .await
        .map_err(|e| s.app.unavailable(e))?;
    Ok(Json(v).into_response())
}

async fn service<R: Repo>(
    State(s): State<V2State<R>>,
    Path(name): Path<String>,
    q: Result<Query<WindowQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let q = query(q)?;
    let name = parse_service(&name).map_err(ApiError::BadRequest)?;
    let w = window_of(&q.since, &q.until, "1h")?;
    match s
        .app
        .repo
        .service(&name, w)
        .await
        .map_err(|e| s.app.unavailable(e))?
    {
        Some(v) => Ok(Json(v).into_response()),
        None => Err(ApiError::NotFound),
    }
}

async fn search<R: Repo>(
    State(s): State<V2State<R>>,
    q: Result<Query<SearchQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let q = parse_q(query(q)?.q.as_deref()).map_err(ApiError::BadRequest)?;
    let v = s
        .app
        .repo
        .search(&q)
        .await
        .map_err(|e| s.app.unavailable(e))?;
    Ok(Json(v).into_response())
}

async fn pipeline_series<R: Repo>(
    State(s): State<V2State<R>>,
    q: Result<Query<SeriesParams>, QueryRejection>,
) -> Result<Response, ApiError> {
    let q = query(q)?;
    let w = window_of(&q.since, &q.until, "1h")?;
    let sq = series_query(
        w,
        q.metric.as_deref(),
        q.job.as_deref(),
        q.kind.as_deref(),
        q.labels.as_deref(),
    )
    .map_err(ApiError::BadRequest)?;
    let step = w.step();
    let pts = s
        .app
        .repo
        .metric_buckets(&sq)
        .await
        .map_err(|e| s.app.unavailable(e))?;
    let some = |v: Vec<(i64, f64)>| v.into_iter().map(|(t, v)| (t, Some(v))).collect();
    let points = if let Some(quantile) = sq.kind.quantile() {
        series::quantile(&pts, quantile, step)
    } else if sq.kind == SeriesKind::Rate {
        some(series::rate(&pts, step))
    } else {
        some(series::gauge(&pts, step))
    };
    Ok(Json(SeriesView {
        metric: sq.metric,
        kind: sq.kind.as_str().to_string(),
        bucket_secs: step,
        points,
    })
    .into_response())
}

async fn pipeline_lag<R: Repo>(State(s): State<V2State<R>>) -> Response {
    match s.lag.get().await {
        (Ok(lags), _) => Json(lags).into_response(),
        (Err(e), fetched) => {
            if fetched {
                tracing::warn!(error = %e, "consumer lag read failed");
                s.app.metrics.lag_errors.inc();
            }
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "kafka unavailable" })),
            )
                .into_response()
        }
    }
}

async fn client_config<R: Repo>(State(s): State<V2State<R>>) -> Response {
    Json(s.config.as_ref().clone()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::params::{SeriesQuery, TraceFilter};
    use crate::routes::api_router;
    use crate::testrepo::FakeRepo;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tayga_store::metrics_store::MetricPointRow;
    use tower::ServiceExt;

    fn lags() -> Vec<Lag> {
        vec![Lag {
            group: "tayga-writer".into(),
            committed: 5,
            end: 7,
            lag: 2,
        }]
    }

    /// A lag source that counts its reads, answering `result` after `delay`.
    fn counting(result: Result<Vec<Lag>, String>, delay: Duration) -> (LagFetch, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let fetch: LagFetch = Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
            let result = result.clone();
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                result.map_err(anyhow::Error::msg)
            })
        });
        (fetch, calls)
    }

    struct App {
        repo: Arc<FakeRepo>,
        metrics: ApiMetrics,
        lag: Option<LagCache>,
    }

    impl App {
        fn new(repo: FakeRepo) -> Self {
            Self {
                repo: Arc::new(repo),
                metrics: ApiMetrics::default(),
                lag: None,
            }
        }

        fn router(self) -> Router {
            let lag = self
                .lag
                .unwrap_or_else(|| LagCache::new(counting(Ok(lags()), Duration::ZERO).0, LAG_TTL));
            // Mounted beside the v1 routes, as in main, so path overlaps would panic here.
            api_router(self.repo.clone(), self.metrics.clone()).merge(router(
                self.repo,
                self.metrics,
                lag,
                ClientConfig::new("http://jaeger", " "),
            ))
        }
    }

    async fn call(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let res = app
            .clone()
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

    async fn get(repo: FakeRepo, uri: &str) -> (StatusCode, serde_json::Value) {
        call(&App::new(repo).router(), uri).await
    }

    #[tokio::test]
    async fn overview_shape_and_default_window() {
        let repo = Arc::new(FakeRepo {
            overview: OverviewView {
                bucket_secs: 60,
                error_stories: 3,
                slow_stories: 1,
                active_alerts: 2,
                spans_per_sec: 12.5,
                data_lag_secs: None,
                stories: StoriesSeries {
                    bucket_secs: 60,
                    error: vec![(60, 3)],
                    slow: vec![(120, 1)],
                },
                spans: vec![(60, 12.5)],
            },
            ..Default::default()
        });
        let app = App {
            repo: repo.clone(),
            metrics: ApiMetrics::default(),
            lag: None,
        }
        .router();
        let (status, json) = call(&app, "/api/v1/overview?since=").await;
        assert_eq!(status, StatusCode::OK);
        let secs = || repo.last_window.lock().unwrap().map(|w| w.secs());
        assert_eq!(secs(), Some(3600));
        assert_eq!(json["error_stories"], 3);
        assert_eq!(json["active_alerts"], 2);
        assert_eq!(json["spans_per_sec"], 12.5);
        assert!(json["data_lag_secs"].is_null());
        assert_eq!(json["stories"]["error"][0], serde_json::json!([60, 3]));
        assert_eq!(json["stories"]["slow"][0][0], 120);
        assert_eq!(json["spans"][0], serde_json::json!([60, 12.5]));
        call(&app, "/api/v1/overview?since=7d").await;
        assert_eq!(secs(), Some(604_800));
    }

    #[tokio::test]
    async fn stories_series_parses_the_group_filter() {
        let repo = Arc::new(FakeRepo {
            stories_series: StoriesSeries {
                bucket_secs: 720,
                error: vec![(720, 4)],
                slow: vec![],
            },
            ..Default::default()
        });
        let app = App {
            repo: repo.clone(),
            metrics: ApiMetrics::default(),
            lag: None,
        }
        .router();
        let (status, json) = call(
            &app,
            "/api/v1/stories/series?since=24h&kind=slow&service=payment",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json,
            serde_json::json!({"bucket_secs": 720, "error": [[720, 4]], "slow": []})
        );
        let f = repo.last_filter.lock().unwrap().clone().unwrap();
        assert_eq!(
            (f.window.secs(), f.kind.as_deref(), f.service.as_deref()),
            (86_400, Some("slow"), Some("payment"))
        );
    }

    #[tokio::test]
    async fn traces_search_shape_and_filter() {
        let hit = TraceHitView {
            trace_id: "ab".repeat(16),
            ts_ns: 5,
            endpoint_service: "frontend".into(),
            endpoint_name: "POST /api/checkout".into(),
            duration_ns: 9,
            is_error: true,
            span_count: 4,
            story_id: Some("ab".repeat(16)),
            story_kind: Some("error".into()),
        };
        let unstoried = TraceHitView {
            trace_id: "cd".repeat(16),
            is_error: false,
            story_id: None,
            story_kind: None,
            ..hit.clone()
        };
        let repo = Arc::new(FakeRepo {
            trace_hits: vec![hit, unstoried],
            ..Default::default()
        });
        let app = App {
            repo: repo.clone(),
            metrics: ApiMetrics::default(),
            lag: None,
        }
        .router();
        let (status, json) = call(
            &app,
            "/api/v1/traces/search?since=15m&service=payment&touched=1&endpoint=POST%20%2Fapi%2Fcheckout&min_ms=5&max_ms=100&errors=1&limit=500",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["trace_id"], "ab".repeat(16));
        assert_eq!(json[0]["is_error"], true);
        assert_eq!(json[0]["story_id"], "ab".repeat(16));
        assert_eq!(json[0]["story_kind"], "error");
        // No story: both fields are present as null.
        assert_eq!(json[1]["story_id"], serde_json::Value::Null);
        assert_eq!(json[1]["story_kind"], serde_json::Value::Null);
        assert!(json[1].as_object().unwrap().contains_key("story_kind"));
        let f = repo.last_trace_filter.lock().unwrap().clone().unwrap();
        assert_eq!(f.window.secs(), 900);
        assert_eq!(
            f,
            TraceFilter {
                window: f.window,
                service: Some("payment".into()),
                touched: true,
                endpoint: Some("POST /api/checkout".into()),
                min_ns: 5_000_000,
                max_ns: 100_000_000,
                errors_only: true,
                limit: 500,
            }
        );
    }

    #[tokio::test]
    async fn services_and_service_detail() {
        let (status, json) = get(
            FakeRepo {
                services: vec!["cart".into(), "payment".into()],
                ..Default::default()
            },
            "/api/v1/services",
        )
        .await;
        assert_eq!(
            (status, json),
            (StatusCode::OK, serde_json::json!(["cart", "payment"]))
        );

        assert_eq!(
            get(FakeRepo::default(), "/api/v1/services/payment").await.0,
            StatusCode::NOT_FOUND
        );
        let repo = Arc::new(FakeRepo {
            service: Some(ServiceView {
                service: "payment".into(),
                bucket_secs: 60,
                calls: 6,
                errors: 3,
                buckets: vec![RedPoint {
                    bucket: 60,
                    rate: 0.1,
                    error_ratio: 0.5,
                    p50_ns: 1.0,
                    p95_ns: 2.0,
                    p99_ns: 3.0,
                }],
            }),
            ..Default::default()
        });
        let app = App {
            repo: repo.clone(),
            metrics: ApiMetrics::default(),
            lag: None,
        }
        .router();
        let (status, json) = call(&app, "/api/v1/services/payment?since=15m").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["service"], "payment");
        assert_eq!(json["buckets"][0]["p99_ns"], 3.0);
        assert_eq!(json["buckets"][0]["error_ratio"], 0.5);
        assert_eq!(
            repo.last_window.lock().unwrap().map(|w| w.secs()),
            Some(900)
        );
    }

    #[tokio::test]
    async fn search_shape_and_q_trimmed() {
        let repo = Arc::new(FakeRepo {
            search: SearchView {
                services: vec!["payment".into()],
                templates: vec![SearchTemplate {
                    template_id: "17393964261140422938".into(),
                    service: "payment".into(),
                    template: "Payment failed <*>".into(),
                }],
                groups: vec![SearchGroup {
                    fingerprint: "42".into(),
                    kind: "error".into(),
                    summary: "payment charge failed".into(),
                    stories: 3,
                }],
                trace_id: None,
            },
            ..Default::default()
        });
        let app = App {
            repo: repo.clone(),
            metrics: ApiMetrics::default(),
            lag: None,
        }
        .router();
        let (status, json) = call(&app, "/api/v1/search?q=%20pay%20").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(repo.last_q.lock().unwrap().as_deref(), Some("pay"));
        assert_eq!(json["services"][0], "payment");
        assert_eq!(json["templates"][0]["template_id"], "17393964261140422938");
        assert_eq!(json["groups"][0]["fingerprint"], "42");
        assert!(json["trace_id"].is_null());
    }

    fn p(ts_s: i64, labels: &[(&str, &str)], value: f64) -> MetricPointRow {
        MetricPointRow {
            ts_ms: ts_s * 1000,
            job: "tayga-writer".into(),
            labels: labels
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            value,
        }
    }

    /// The latest moment that is a multiple of `step`: a fixed `until` keeps the fixtures below
    /// clear of the window's edges.
    fn aligned_until(step: i64) -> i64 {
        crate::params::now_ms() / 1000 / step * step
    }

    async fn series(points: Vec<MetricPointRow>, uri: &str) -> (serde_json::Value, SeriesQuery) {
        let repo = Arc::new(FakeRepo {
            metric_points: points,
            ..Default::default()
        });
        let app = App {
            repo: repo.clone(),
            metrics: ApiMetrics::default(),
            lag: None,
        }
        .router();
        let (status, json) = call(&app, uri).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let seen = repo.last_series_query.lock().unwrap().clone().unwrap();
        (json, seen)
    }

    #[tokio::test]
    async fn pipeline_series_applies_the_series_math() {
        let counter = vec![p(0, &[], 0.0), p(60, &[], 120.0), p(120, &[], 180.0)];
        let until = aligned_until(60);
        let (json, q) = series(
            counter.clone(),
            &format!("/api/v1/pipeline/series?metric=tayga_writer_rows_inserted_total&kind=rate&job=tayga-writer&labels=kind%3Dspans&until={until}"),
        )
        .await;
        assert_eq!(q.window.step(), 60, "bucket_secs of the default 1h window");
        assert_eq!((q.window.start, q.window.end), (until - 3600, until));
        assert_eq!(q.job.as_deref(), Some("tayga-writer"));
        assert_eq!(q.labels, vec![("kind".to_string(), "spans".to_string())]);
        assert_eq!(
            json,
            serde_json::json!({
                "metric": "tayga_writer_rows_inserted_total",
                "kind": "rate",
                "bucket_secs": 60,
                "points": [[60_000, 2.0], [120_000, 1.0]],
            })
        );

        let until = aligned_until(4320);
        let (json, q) = series(
            counter,
            &format!("/api/v1/pipeline/series?metric=g&kind=gauge&since=6d&until={until}"),
        )
        .await;
        assert_eq!(q.window.step(), 4320);
        assert_eq!(json["bucket_secs"], 4320);
        assert_eq!(
            json["points"],
            serde_json::json!([[0, 180.0]]),
            "one 4320 s bucket, last value"
        );

        let mut hist: Vec<MetricPointRow> = ["1", "2", "+Inf"]
            .iter()
            .map(|le| p(0, &[("le", le)], 0.0))
            .collect();
        hist.extend([("1", 4.0), ("2", 8.0), ("+Inf", 8.0)].map(|(le, v)| p(60, &[("le", le)], v)));
        // Unchanged counters: no observations in the last step.
        hist.extend(
            [("1", 4.0), ("2", 8.0), ("+Inf", 8.0)].map(|(le, v)| p(120, &[("le", le)], v)),
        );
        let until = aligned_until(60);
        let (json, q) = series(
            hist,
            &format!("/api/v1/pipeline/series?metric=h_seconds&kind=q50&until={until}"),
        )
        .await;
        assert_eq!(q.metric, "h_seconds_bucket");
        assert_eq!(json["kind"], "q50");
        assert_eq!(json["points"][0], serde_json::json!([60_000, 1.0]));
        assert!(
            json["points"][1][1].is_null(),
            "no observations in the last step"
        );
    }

    #[tokio::test]
    async fn pipeline_series_buckets_lie_on_the_epoch_grid() {
        // Two windows 10 s apart, neither starting on a minute: the same bucket edges.
        let base = aligned_until(60) - 53;
        let mut seen = Vec::new();
        for until in [base, base + 10] {
            let start = until - 900;
            let pts = vec![
                p(start + 20, &[], 4.0),
                p(start + 40, &[], 6.0),
                p(start + 80, &[], 9.0),
            ];
            let (json, q) = series(
                pts,
                &format!("/api/v1/pipeline/series?metric=g&kind=gauge&since=15m&until={until}"),
            )
            .await;
            assert_eq!((q.window.start, q.window.end), (start, until));
            let edges: Vec<i64> = json["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|pt| pt[0].as_i64().unwrap())
                .collect();
            assert!(edges.iter().all(|t| t % 60_000 == 0), "{edges:?}");
            seen.push(edges);
        }
        assert_eq!(seen[0], seen[1], "a moving window keeps its bucket edges");
    }

    #[tokio::test]
    async fn until_moves_the_window_and_bad_until_is_400() {
        let now = crate::params::now_ms() / 1000;
        let end = now - 86_400;
        let repo = Arc::new(FakeRepo {
            service: Some(ServiceView {
                service: "payment".into(),
                bucket_secs: 60,
                calls: 0,
                errors: 0,
                buckets: vec![],
            }),
            ..Default::default()
        });
        let app = App {
            repo: repo.clone(),
            metrics: ApiMetrics::default(),
            lag: None,
        }
        .router();
        let w = |secs: i64| crate::params::Window {
            start: end - secs,
            end,
            live: false,
        };
        let ok = |uri: String| {
            let app = app.clone();
            async move {
                let (status, json) = call(&app, &uri).await;
                assert_eq!(status, StatusCode::OK, "{uri}: {json}");
            }
        };
        ok(format!("/api/v1/overview?since=2h&until={end}")).await;
        assert_eq!(repo.last_window.lock().unwrap().take(), Some(w(7200)));
        ok(format!("/api/v1/services/payment?until={end}")).await;
        assert_eq!(repo.last_window.lock().unwrap().take(), Some(w(3600)));
        ok(format!("/api/v1/stories/series?since=15m&until={end}")).await;
        let f = repo.last_filter.lock().unwrap().take().unwrap();
        assert_eq!(f.window, w(900));
        ok(format!("/api/v1/traces/search?since=15m&until={end}")).await;
        let f = repo.last_trace_filter.lock().unwrap().take().unwrap();
        assert_eq!(f.window, w(900));
        ok(format!(
            "/api/v1/pipeline/series?metric=x&kind=rate&until={end}"
        ))
        .await;
        let q = repo.last_series_query.lock().unwrap().take().unwrap();
        assert_eq!(q.window, w(3600));
        // Lag is live only: `until` is ignored, not rejected.
        ok("/api/v1/pipeline/lag?until=nonsense".to_string()).await;

        let ahead = now + 3600;
        let old = now - 7 * 86_400 + 60;
        for (uri, needle) in [
            (format!("/api/v1/overview?until={ahead}"), "future"),
            (format!("/api/v1/stories/series?until={ahead}"), "future"),
            (format!("/api/v1/traces/search?until={ahead}"), "future"),
            (format!("/api/v1/services/payment?until={ahead}"), "future"),
            (
                format!("/api/v1/pipeline/series?metric=x&kind=rate&until={ahead}"),
                "future",
            ),
            (
                format!("/api/v1/overview?since=1h&until={old}"),
                "retention",
            ),
            (
                format!("/api/v1/traces/search?since=1h&until={old}"),
                "retention",
            ),
            (
                format!("/api/v1/pipeline/series?metric=x&kind=rate&until={old}"),
                "retention",
            ),
            ("/api/v1/overview?until=noon".to_string(), "RFC 3339"),
            (
                "/api/v1/services/payment?until=12:00".to_string(),
                "RFC 3339",
            ),
        ] {
            let (status, json) = call(&app, &uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            let message = json["error"].as_str().unwrap_or_default();
            assert!(message.contains(needle), "{uri}: {message}");
        }
    }

    #[tokio::test]
    async fn bad_params_are_400() {
        for uri in [
            "/api/v1/overview?since=9d",
            "/api/v1/overview?since=1h&since=2h",
            "/api/v1/stories/series?kind=bogus",
            "/api/v1/traces/search?limit=501",
            "/api/v1/traces/search?min_ms=abc",
            "/api/v1/traces/search?min_ms=10&max_ms=5",
            "/api/v1/traces/search?errors=maybe",
            "/api/v1/services/payment?since=0s",
            "/api/v1/search",
            "/api/v1/search?q=%20%20",
            "/api/v1/pipeline/series?kind=rate",
            "/api/v1/pipeline/series?metric=Bad-Name&kind=rate",
            "/api/v1/pipeline/series?metric=x%27%3B&kind=rate",
            "/api/v1/pipeline/series?metric=x&kind=q95",
            "/api/v1/pipeline/series?metric=x&kind=rate&labels=nokey",
        ] {
            let (status, json) = get(FakeRepo::default(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert!(json["error"].as_str().is_some(), "{uri}");
        }
        let long = format!("/api/v1/search?q={}", "a".repeat(201));
        assert_eq!(
            get(FakeRepo::default(), &long).await.0,
            StatusCode::BAD_REQUEST
        );
        let long = format!("/api/v1/services/{}", "a".repeat(201));
        assert_eq!(
            get(FakeRepo::default(), &long).await.0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn repo_failure_is_503() {
        for uri in [
            "/api/v1/overview",
            "/api/v1/stories/series",
            "/api/v1/traces/search",
            "/api/v1/services",
            "/api/v1/services/payment",
            "/api/v1/search?q=pay",
            "/api/v1/pipeline/series?metric=x&kind=rate",
        ] {
            let app = App {
                repo: Arc::new(FakeRepo {
                    fail: true,
                    ..Default::default()
                }),
                metrics: ApiMetrics::default(),
                lag: None,
            };
            let metrics = app.metrics.clone();
            let (status, json) = call(&app.router(), uri).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
            assert_eq!(json["error"], "storage unavailable");
            assert_eq!(metrics.repo_errors.get(), 1, "{uri}");
        }
    }

    #[tokio::test]
    async fn config_maps_empty_links_to_null() {
        let (status, json) = get(FakeRepo::default(), "/api/v1/config").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json,
            serde_json::json!({"jaeger_url": "http://jaeger", "grafana_url": null})
        );
        assert_eq!(
            ClientConfig::new("", " http://g "),
            ClientConfig {
                jaeger_url: None,
                grafana_url: Some("http://g".into())
            }
        );
    }

    #[tokio::test]
    async fn lag_is_served_and_cached() {
        let (fetch, calls) = counting(Ok(lags()), Duration::ZERO);
        let app = App {
            repo: Arc::new(FakeRepo::default()),
            metrics: ApiMetrics::default(),
            lag: Some(LagCache::new(fetch, LAG_TTL)),
        }
        .router();
        let (status, json) = call(&app, "/api/v1/pipeline/lag").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json,
            serde_json::json!([{"group": "tayga-writer", "committed": 5, "end": 7, "lag": 2}])
        );
        call(&app, "/api/v1/pipeline/lag").await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second call within the TTL is cached"
        );
    }

    #[tokio::test]
    async fn lag_error_is_opaque_503_counted_once_and_cached() {
        let (fetch, calls) = counting(Err("broker down: secret-host:9092".into()), Duration::ZERO);
        let metrics = ApiMetrics::default();
        let app = App {
            repo: Arc::new(FakeRepo::default()),
            metrics: metrics.clone(),
            lag: Some(LagCache::new(fetch, LAG_TTL)),
        }
        .router();
        for _ in 0..3 {
            let (status, json) = call(&app, "/api/v1/pipeline/lag").await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(json, serde_json::json!({"error": "kafka unavailable"}));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the error is cached");
        assert_eq!(metrics.lag_errors.get(), 1);
        assert_eq!(metrics.repo_errors.get(), 0);
    }

    #[tokio::test]
    async fn concurrent_lag_requests_share_one_read() {
        let (fetch, calls) = counting(Ok(lags()), Duration::from_millis(50));
        let cache = Arc::new(LagCache::new(fetch, LAG_TTL));
        let results = futures::future::join_all((0..8).map(|_| {
            let cache = cache.clone();
            async move { cache.get().await }
        }))
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(results.iter().filter(|(_, fetched)| *fetched).count(), 1);
        assert!(
            results
                .iter()
                .all(|(r, _)| r.as_ref().is_ok_and(|l| l.len() == 1))
        );
    }

    #[tokio::test]
    async fn a_dropped_request_does_not_start_a_second_read() {
        let (fetch, calls) = counting(Ok(lags()), Duration::from_millis(50));
        let cache = LagCache::new(fetch, LAG_TTL);
        // The first caller gives up (its future is dropped) while the read is in flight.
        assert!(
            tokio::time::timeout(Duration::from_millis(5), cache.get())
                .await
                .is_err()
        );
        let (result, fetched) = cache.get().await;
        assert!(
            result.is_ok() && !fetched,
            "the abandoned read finished and was shared"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn lag_is_read_again_after_the_ttl() {
        let (fetch, calls) = counting(Ok(lags()), Duration::ZERO);
        let cache = LagCache::new(fetch, Duration::ZERO);
        assert!(cache.get().await.1);
        assert!(cache.get().await.1, "an expired result is read again");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
