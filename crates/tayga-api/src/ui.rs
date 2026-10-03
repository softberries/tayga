//! Server-rendered pages (spec §10). No JavaScript.

use crate::model::{GroupView, StoryView, TraceLogRow};
use crate::params::{bucket_secs, group_filter, parse_fingerprint, parse_hex_id, parse_since};
use crate::repo::Repo;
use crate::routes::{ApiMetrics, AppState, GroupsQuery, SinceQuery};
use crate::svg::{WaterfallRow, sparkline, waterfall};
use askama::Template;
use askama_web::WebTemplate;
use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone)]
pub struct UiLinks {
    pub jaeger_url: String,
    pub grafana_url: String,
}

struct UiState<R> {
    app: AppState<R>,
    links: UiLinks,
}

// Manual impl: a derive would require `R: Clone`, which repositories need not be.
impl<R> Clone for UiState<R> {
    fn clone(&self) -> Self {
        Self {
            app: self.app.clone(),
            links: self.links.clone(),
        }
    }
}

pub fn ui_router<R: Repo>(repo: Arc<R>, metrics: ApiMetrics, links: UiLinks) -> Router {
    Router::new()
        .route("/", get(groups_page::<R>))
        .route("/groups/{fingerprint}", get(group_page::<R>))
        .route("/stories/{story_id}", get(story_page::<R>))
        .route("/service-map", get(map_page::<R>))
        .with_state(UiState {
            app: AppState { repo, metrics },
            links,
        })
}

#[derive(Template, WebTemplate)]
#[template(path = "error.html")]
struct ErrorPage {
    status: u16,
    message: String,
}

fn error_page(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        ErrorPage {
            status: status.as_u16(),
            message: message.into(),
        },
    )
        .into_response()
}

/// UTC `YYYY-MM-DD HH:MM:SS UTC` from unix nanoseconds (Hinnant's `civil_from_days`).
fn fmt_time(ns: i64) -> String {
    let secs = ns.div_euclid(1_000_000_000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        sod / 3600,
        sod / 60 % 60,
        sod % 60
    )
}

fn ms(ns: u64) -> String {
    format!("{:.1}", ns as f64 / 1e6)
}

struct GroupRowView {
    kind: String,
    fingerprint: String,
    summary: String,
    rc_service: String,
    endpoint: String,
    stories: u64,
    spark: String,
    last_seen: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "groups.html")]
struct GroupsPage {
    since: String,
    kind: String,
    service: String,
    rows: Vec<GroupRowView>,
}

/// Sparkline window `(from, to, step)`: bucket starts aligned like ClickHouse's
/// `toStartOfInterval` (epoch multiples of the step), so stored buckets land on points.
#[derive(Clone, Copy)]
struct SparkWindow {
    from: u32,
    to: u32,
    step: u32,
}

impl SparkWindow {
    fn new(since_secs: u32) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        Self::at(now, since_secs)
    }

    fn at(now: u32, since_secs: u32) -> Self {
        let step = bucket_secs(since_secs);
        Self {
            from: now.saturating_sub(since_secs) / step * step,
            to: now / step * step,
            step,
        }
    }

    fn render(self, points: &[(u32, u64)], width: u32, height: u32) -> String {
        sparkline(points, self.from, self.to, self.step, width, height)
    }
}

fn row_view(g: &GroupView, w: SparkWindow) -> GroupRowView {
    GroupRowView {
        kind: g.group.kind.clone(),
        fingerprint: g.group.fingerprint.clone(),
        summary: g.group.summary.clone(),
        rc_service: g.group.rc_service.clone(),
        endpoint: format!("{} {}", g.group.endpoint_service, g.group.endpoint_name),
        stories: g.group.stories,
        spark: w.render(&g.buckets, 120, 22),
        last_seen: fmt_time(g.group.last_seen_ns),
    }
}

async fn groups_page<R: Repo>(
    State(s): State<UiState<R>>,
    q: Result<Query<GroupsQuery>, QueryRejection>,
) -> Response {
    let Query(q) = match q {
        Ok(q) => q,
        Err(r) => return error_page(StatusCode::BAD_REQUEST, r.body_text()),
    };
    let f = match group_filter(q.since.as_deref(), q.kind.as_deref(), q.service.as_deref()) {
        Ok(f) => f,
        Err(e) => return error_page(StatusCode::BAD_REQUEST, e),
    };
    let window = SparkWindow::new(f.since_secs);
    match s.app.repo.story_groups(&f).await {
        Ok(groups) => GroupsPage {
            since: q.since.as_deref().map_or("1h", str::trim).to_string(),
            kind: f.kind.clone().unwrap_or_default(),
            service: f.service.clone().unwrap_or_default(),
            rows: groups.iter().map(|g| row_view(g, window)).collect(),
        }
        .into_response(),
        Err(e) => {
            s.app.unavailable(e);
            error_page(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable")
        }
    }
}

struct ExampleView {
    story_id: String,
    time: String,
    duration_ms: String,
    summary: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "group.html")]
struct GroupPage {
    kind: String,
    summary: String,
    fingerprint: String,
    stories: u64,
    rc_service: String,
    endpoint: String,
    spark: String,
    examples: Vec<ExampleView>,
}

async fn group_page<R: Repo>(
    State(s): State<UiState<R>>,
    Path(fingerprint): Path<String>,
    q: Result<Query<SinceQuery>, QueryRejection>,
) -> Response {
    let Query(q) = match q {
        Ok(q) => q,
        Err(r) => return error_page(StatusCode::BAD_REQUEST, r.body_text()),
    };
    let (Ok(fp), Ok(since)) = (
        parse_fingerprint(&fingerprint),
        parse_since(q.since.as_deref().unwrap_or("24h")),
    ) else {
        return error_page(StatusCode::BAD_REQUEST, "invalid fingerprint or since");
    };
    match s.app.repo.story_group(&fp, since).await {
        Ok(Some(d)) => {
            let window = SparkWindow::new(since);
            let row = row_view(&d.group, window);
            GroupPage {
                kind: row.kind,
                summary: row.summary,
                fingerprint: row.fingerprint,
                stories: row.stories,
                rc_service: row.rc_service,
                endpoint: row.endpoint,
                spark: window.render(&d.group.buckets, 600, 60),
                examples: d
                    .examples
                    .into_iter()
                    .map(|e| ExampleView {
                        story_id: e.story_id,
                        time: fmt_time(e.ts_ns),
                        duration_ms: ms(e.duration_ns),
                        summary: e.summary,
                    })
                    .collect(),
            }
            .into_response()
        }
        Ok(None) => error_page(StatusCode::NOT_FOUND, "no such story group in this window"),
        Err(e) => {
            s.app.unavailable(e);
            error_page(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable")
        }
    }
}

struct LogView {
    time: String,
    service: String,
    severity: String,
    body: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "story.html")]
struct StoryPage {
    kind: String,
    summary: String,
    path: String,
    time: String,
    duration_ms: String,
    span_count: u32,
    flags: Vec<String>,
    jaeger_link: String,
    fingerprint: String,
    rows: Vec<WaterfallRow>,
    diff_lines: Vec<String>,
    logs: Vec<LogView>,
}

fn diff_lines(story: &StoryView) -> Vec<String> {
    let Some(d) = &story.baseline_diff else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for op in d["new_ops"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
    {
        out.push(format!("new operation: {op}"));
    }
    for op in d["missing_ops"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
    {
        out.push(format!("usually present, missing here: {op}"));
    }
    for s in d["slower_ops"].as_array().into_iter().flatten() {
        out.push(format!(
            "slower than usual: {} ({} ms vs p95 {} ms)",
            s["op"].as_str().unwrap_or("?"),
            ms(s["duration_ns"].as_u64().unwrap_or(0)),
            ms(s["baseline_p95_ns"].as_u64().unwrap_or(0))
        ));
    }
    out
}

fn log_view(l: &TraceLogRow) -> LogView {
    LogView {
        time: fmt_time(l.ts_ns),
        service: l.service_name.clone(),
        severity: if l.severity_text.is_empty() {
            l.severity_number.to_string()
        } else {
            l.severity_text.clone()
        },
        body: l.body.clone(),
    }
}

async fn story_page<R: Repo>(
    State(s): State<UiState<R>>,
    Path(story_id): Path<String>,
) -> Response {
    let Ok(id) = parse_hex_id(&story_id) else {
        return error_page(StatusCode::BAD_REQUEST, "invalid story id");
    };
    let story = match s.app.repo.story(&id).await {
        Ok(Some(v)) => v,
        Ok(None) => return error_page(StatusCode::NOT_FOUND, "no such story"),
        Err(e) => {
            s.app.unavailable(e);
            return error_page(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable");
        }
    };
    let trace = match s.app.repo.trace(&story.trace_id).await {
        Ok(t) => t,
        Err(e) => {
            s.app.unavailable(e);
            return error_page(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable");
        }
    };
    let critical: HashSet<String> = story.critical_span_ids().into_iter().collect();
    StoryPage {
        kind: story.kind.clone(),
        summary: story.summary.clone(),
        path: story.path_services.join(" → "),
        time: fmt_time(story.ts_ns),
        duration_ms: ms(story.duration_ns),
        span_count: story.span_count,
        flags: story.flags.clone(),
        jaeger_link: format!(
            "{}/trace/{}",
            s.links.jaeger_url.trim_end_matches('/'),
            story.trace_id
        ),
        fingerprint: story.fingerprint.clone(),
        rows: waterfall(&trace.spans, &critical, &story.root_cause.span_id),
        diff_lines: diff_lines(&story),
        logs: trace.logs.iter().map(log_view).collect(),
    }
    .into_response()
}

struct EdgeRowView {
    parent: String,
    child: String,
    calls: u64,
    errors: u64,
    error_rate: String,
    avg_ms: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "service_map.html")]
struct MapPage {
    since: String,
    grafana_link: String,
    edges: Vec<EdgeRowView>,
}

async fn map_page<R: Repo>(
    State(s): State<UiState<R>>,
    q: Result<Query<SinceQuery>, QueryRejection>,
) -> Response {
    let Query(q) = match q {
        Ok(q) => q,
        Err(r) => return error_page(StatusCode::BAD_REQUEST, r.body_text()),
    };
    let since_raw = q.since.as_deref().map_or("1h", str::trim).to_string();
    let Ok(since) = parse_since(&since_raw) else {
        return error_page(StatusCode::BAD_REQUEST, "invalid since");
    };
    match s.app.repo.service_map(since).await {
        Ok(edges) => MapPage {
            since: since_raw,
            grafana_link: format!(
                "{}/d/tayga-service-map",
                s.links.grafana_url.trim_end_matches('/')
            ),
            edges: edges
                .into_iter()
                .map(|e| EdgeRowView {
                    error_rate: format!("{:.1}%", e.error_rate * 100.0),
                    avg_ms: ms(e.avg_duration_ns),
                    parent: e.parent,
                    child: e.child,
                    calls: e.calls,
                    errors: e.errors,
                })
                .collect(),
        }
        .into_response(),
        Err(e) => {
            s.app.unavailable(e);
            error_page(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable")
        }
    }
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

    fn links() -> UiLinks {
        UiLinks {
            jaeger_url: "http://localhost:8080/jaeger/ui".into(),
            grafana_url: "http://localhost:3001".into(),
        }
    }

    async fn html(repo: FakeRepo, uri: &str) -> (StatusCode, String) {
        let app = ui_router(Arc::new(repo), ApiMetrics::default(), links());
        let res = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[test]
    fn fmt_time_formats_utc() {
        assert_eq!(fmt_time(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(
            fmt_time(1_700_000_000_000_000_000),
            "2023-11-14 22:13:20 UTC"
        );
        assert_eq!(fmt_time(951_782_400_000_000_000), "2000-02-29 00:00:00 UTC");
        assert_eq!(fmt_time(-1), "1969-12-31 23:59:59 UTC");
        let _ = fmt_time(i64::MIN);
        let _ = fmt_time(i64::MAX);
    }

    #[tokio::test]
    async fn malformed_query_is_html_400() {
        let (status, body) = html(FakeRepo::default(), "/service-map?since=1h&since=2h").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("<html"));
    }

    #[tokio::test]
    async fn groups_page_renders_empty_state() {
        let (status, body) = html(FakeRepo::default(), "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("No stories in this window."));
    }

    #[tokio::test]
    async fn groups_page_lists_groups_with_sparkline() {
        let g = GroupView {
            group: StoryGroupRow {
                fingerprint: "42".into(),
                kind: "error".into(),
                summary: "payment charge failed".into(),
                rc_service: "payment".into(),
                rc_span_name: "charge".into(),
                endpoint_service: "load-generator".into(),
                endpoint_name: "user_checkout_single".into(),
                stories: 7,
                first_seen_ns: 0,
                last_seen_ns: 0,
                sample_story_id: "ab".repeat(16),
            },
            bucket_secs: 60,
            buckets: vec![],
        };
        let (status, body) = html(
            FakeRepo {
                groups: vec![g],
                ..Default::default()
            },
            "/?since=15m",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("payment charge failed")
                && body.contains("<svg")
                && body.contains("/groups/42?since=15m")
        );
    }

    #[tokio::test]
    async fn groups_page_sparkline_is_bounded_at_7d() {
        // Worst case: the repo hands back one point per minute for the whole week.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;
        let start = now - 7 * 86_400;
        let g = GroupView {
            group: StoryGroupRow {
                fingerprint: "42".into(),
                kind: "slow".into(),
                summary: "shipping slow".into(),
                rc_service: "shipping".into(),
                rc_span_name: "quote".into(),
                endpoint_service: "frontend".into(),
                endpoint_name: "POST /api/checkout".into(),
                stories: 10_080,
                first_seen_ns: 0,
                last_seen_ns: 0,
                sample_story_id: "ab".repeat(16),
            },
            bucket_secs: 60,
            buckets: (0..10_080).map(|i| (start + i * 60, 1)).collect(),
        };
        let (status, body) = html(
            FakeRepo {
                groups: vec![g],
                ..Default::default()
            },
            "/?since=7d",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let svg_start = body.find("<svg").expect("sparkline rendered");
        let svg_len = body[svg_start..].find("</svg>").unwrap() + "</svg>".len();
        assert!(svg_len < 10 * 1024, "7d sparkline is {svg_len} bytes");
    }

    #[test]
    fn spark_window_aligns_to_bucket_width() {
        let w = SparkWindow::at(1_791_029_999, 7 * 86_400);
        assert_eq!(w.step, 5040);
        assert_eq!(w.to, 1_791_029_520);
        assert_eq!(w.from % 5040, 0);
        assert!((w.to - w.from) / w.step <= 121);
    }

    #[tokio::test]
    async fn story_page_escapes_log_bodies_and_marks_root_cause() {
        let mut story = StoryView::from_record(record());
        story.root_cause.span_id = "a".into();
        let trace = TraceView {
            trace_id: "ab".repeat(16),
            spans: vec![TraceSpanRow {
                span_id: "a".into(),
                parent_span_id: String::new(),
                service_name: "payment".into(),
                span_name: "<script>x</script>".into(),
                kind: "server".into(),
                start_ns: 0,
                duration_ns: 10,
                status: "error".into(),
                status_message: String::new(),
            }],
            logs: vec![TraceLogRow {
                ts_ns: 0,
                span_id: "a".into(),
                service_name: "payment".into(),
                severity_number: 17,
                severity_text: "ERROR".into(),
                body: "<img src=x onerror=alert(1)>".into(),
            }],
        };
        let repo = FakeRepo {
            story: Some(story),
            trace: Some(trace),
            ..Default::default()
        };
        let (status, body) = html(repo, &format!("/stories/{}", "ab".repeat(16))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body.contains("<script>x</script>") && !body.contains("<img src=x"));
        assert!(
            body.contains("&#60;img src=x onerror=alert(1)&#62;")
                || body.contains("&lt;img src=x onerror=alert(1)&gt;")
        );
        assert!(body.contains("class=\"rc\""));
        assert!(body.contains("http://localhost:8080/jaeger/ui/trace/"));
        assert!(body.contains("load-generator → checkout → payment"));
    }

    #[tokio::test]
    async fn unknown_story_and_bad_id() {
        assert_eq!(
            html(
                FakeRepo::default(),
                &format!("/stories/{}", "ab".repeat(16))
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            html(FakeRepo::default(), "/stories/zz").await.0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn service_map_page_links_grafana() {
        let edges = vec![EdgeView {
            parent: "frontend".into(),
            child: "checkout".into(),
            calls: 10,
            errors: 1,
            error_rate: 0.1,
            avg_duration_ns: 2_000_000,
        }];
        let (status, body) = html(
            FakeRepo {
                edges,
                ..Default::default()
            },
            "/service-map",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("frontend")
                && body.contains("10.0%")
                && body.contains("http://localhost:3001/d/tayga-service-map")
        );
    }

    #[tokio::test]
    async fn storage_failure_renders_503_and_counts() {
        let metrics = ApiMetrics::default();
        let repo = FakeRepo {
            fail: true,
            ..Default::default()
        };
        let app = ui_router(Arc::new(repo), metrics.clone(), links());
        let res = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(metrics.repo_errors.get(), 1);
    }

    #[tokio::test]
    async fn group_page_renders_examples_and_404s() {
        let id = "cd".repeat(16);
        let detail = GroupDetail {
            group: GroupView {
                group: StoryGroupRow {
                    fingerprint: "42".into(),
                    kind: "error".into(),
                    summary: "payment charge failed".into(),
                    rc_service: "payment".into(),
                    rc_span_name: "charge".into(),
                    endpoint_service: "load-generator".into(),
                    endpoint_name: "checkout".into(),
                    stories: 1,
                    first_seen_ns: 0,
                    last_seen_ns: 0,
                    sample_story_id: id.clone(),
                },
                bucket_secs: 60,
                buckets: vec![],
            },
            examples: vec![StorySummaryRow {
                story_id: id.clone(),
                ts_ns: 0,
                trace_id: "ab".repeat(16),
                duration_ns: 1_000_000,
                summary: "payment charge failed".into(),
            }],
        };
        let repo = FakeRepo {
            detail: Some(detail),
            ..Default::default()
        };
        let (status, body) = html(repo, "/groups/42").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(&format!("/stories/{id}")));
        let (status, _) = html(FakeRepo::default(), "/groups/42").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn filter_values_are_attribute_escaped() {
        let (status, body) = html(FakeRepo::default(), "/?service=%22%3E%3Cscript%3E").await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body.contains("\"><script>"));
    }
}
