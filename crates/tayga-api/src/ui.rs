//! Server-rendered pages (spec §10). No JavaScript.

use crate::model::{
    ExampleTrace, GroupView, LogAlertView, StoryView, TraceLogRow, TraceLogTemplate, TraceSpanRow,
};
use crate::params::{
    alert_filter, bucket_secs, group_filter, parse_fingerprint, parse_hex_id, parse_since,
    since_or, template_filter,
};
use crate::repo::Repo;
use crate::routes::{AlertsQuery, ApiMetrics, AppState, GroupsQuery, SinceQuery, TemplatesQuery};
use crate::svg::{WaterfallRow, sparkline, waterfall};
use askama::Template;
use askama_web::WebTemplate;
use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
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
        .route("/alerts", get(alerts_page::<R>))
        .route("/templates", get(templates_page::<R>))
        .route("/templates/{id}", get(template_page::<R>))
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

/// UTC wall clock with milliseconds, `HH:MM:SS.mmm`, for log lines within one story.
fn fmt_clock_ms(ns: i64) -> String {
    let sod = ns.div_euclid(1_000_000_000).rem_euclid(86_400);
    let millis = ns.rem_euclid(1_000_000_000) / 1_000_000;
    format!(
        "{:02}:{:02}:{:02}.{millis:03}",
        sod / 3600,
        sod / 60 % 60,
        sod % 60
    )
}

/// Percent-encodes everything except unreserved URL characters, for query values in links.
fn url_component(raw: &str) -> String {
    raw.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn jaeger_trace_url(links: &UiLinks, trace_id: &str) -> String {
    format!(
        "{}/trace/{trace_id}",
        links.jaeger_url.trim_end_matches('/')
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
            since: since_or(q.since.as_deref(), "1h").to_string(),
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
        parse_since(since_or(q.since.as_deref(), "24h")),
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

/// Template cell of one log row; `text` is truncated, `full` is the whole template.
struct TemplateCell {
    id: String,
    text: String,
    full: String,
    alert: Option<String>,
}

struct LogView {
    time: String,
    service: String,
    span: String,
    severity: String,
    body: String,
    template: Option<TemplateCell>,
}

const TEMPLATE_CELL_CHARS: usize = 80;

/// At most `max` characters, with `…` replacing the cut-off tail.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// One service filter link above the log table; `href` is a relative link to this story.
struct LogChip {
    label: String,
    count: usize,
    href: String,
    active: bool,
}

#[derive(Deserialize, Default)]
struct StoryQuery {
    log_service: Option<String>,
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
    log_total: usize,
    log_chips: Vec<LogChip>,
    log_filter: String,
    trace_span_count: usize,
    trace_service_count: usize,
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

fn log_view(
    l: &TraceLogRow,
    span_names: &HashMap<&str, &str>,
    templates: &HashMap<&str, &TraceLogTemplate>,
) -> LogView {
    LogView {
        time: fmt_clock_ms(l.ts_ns),
        service: l.service_name.clone(),
        span: span_names
            .get(l.span_id.as_str())
            .map_or_else(|| "—".to_string(), |n| n.to_string()),
        severity: match (l.severity_text.is_empty(), l.severity_number) {
            (false, _) => l.severity_text.clone(),
            // OTLP severity 0 means "unspecified", e.g. Envoy access logs.
            (true, 0) => "—".to_string(),
            (true, n) => n.to_string(),
        },
        body: l.body.clone(),
        template: templates.get(l.log_id.as_str()).map(|t| TemplateCell {
            id: t.template_id.clone(),
            text: truncate_chars(&t.template, TEMPLATE_CELL_CHARS),
            full: t.template.clone(),
            alert: t.alert.clone(),
        }),
    }
}

/// Logs linked to the trace (time order), the per-service filter chips and the filtered rows.
fn story_logs(
    story_id: &str,
    spans: &[TraceSpanRow],
    logs: &[TraceLogRow],
    templates: &[TraceLogTemplate],
    filter: &str,
) -> (Vec<LogView>, Vec<LogChip>) {
    let by_log: HashMap<&str, &TraceLogTemplate> =
        templates.iter().map(|t| (t.log_id.as_str(), t)).collect();
    let span_names: HashMap<&str, &str> = spans
        .iter()
        .map(|s| (s.span_id.as_str(), s.span_name.as_str()))
        .collect();
    let mut sorted: Vec<&TraceLogRow> = logs.iter().collect();
    sorted.sort_by_key(|l| l.ts_ns);
    let mut per_service: BTreeMap<&str, usize> = BTreeMap::new();
    for l in &sorted {
        *per_service.entry(l.service_name.as_str()).or_default() += 1;
    }
    let base = format!("/stories/{story_id}");
    let mut chips = vec![LogChip {
        label: "all".to_string(),
        count: sorted.len(),
        href: format!("{base}#logs"),
        active: filter.is_empty(),
    }];
    chips.extend(per_service.iter().map(|(svc, count)| LogChip {
        label: svc.to_string(),
        count: *count,
        href: format!("{base}?log_service={}#logs", url_component(svc)),
        active: *svc == filter,
    }));
    let rows = sorted
        .into_iter()
        .filter(|l| filter.is_empty() || l.service_name == filter)
        .map(|l| log_view(l, &span_names, &by_log))
        .collect();
    (rows, chips)
}

async fn story_page<R: Repo>(
    State(s): State<UiState<R>>,
    Path(story_id): Path<String>,
    q: Result<Query<StoryQuery>, QueryRejection>,
) -> Response {
    let Ok(id) = parse_hex_id(&story_id) else {
        return error_page(StatusCode::BAD_REQUEST, "invalid story id");
    };
    let q = match q {
        Ok(Query(q)) => q,
        Err(rejection) => return error_page(StatusCode::BAD_REQUEST, rejection.body_text()),
    };
    let log_filter = q.log_service.unwrap_or_default().trim().to_string();
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
    // Templates are an enhancement: when the lookup fails the column shows dashes.
    let templates = if trace.logs.is_empty() {
        Vec::new()
    } else {
        match s.app.repo.trace_log_templates(&story.trace_id).await {
            Ok(t) => t,
            Err(e) => {
                s.app.unavailable(e);
                Vec::new()
            }
        }
    };
    let (logs, log_chips) = story_logs(&id, &trace.spans, &trace.logs, &templates, &log_filter);
    let trace_service_count = trace
        .spans
        .iter()
        .map(|sp| sp.service_name.as_str())
        .collect::<HashSet<_>>()
        .len();
    StoryPage {
        kind: story.kind.clone(),
        summary: story.summary.clone(),
        path: story.path_services.join(" → "),
        time: fmt_time(story.ts_ns),
        duration_ms: ms(story.duration_ns),
        span_count: story.span_count,
        flags: story.flags.clone(),
        jaeger_link: jaeger_trace_url(&s.links, &story.trace_id),
        fingerprint: story.fingerprint.clone(),
        rows: waterfall(&trace.spans, &critical, &story.root_cause.span_id),
        diff_lines: diff_lines(&story),
        logs,
        log_total: trace.logs.len(),
        log_chips,
        log_filter,
        trace_span_count: trace.spans.len(),
        trace_service_count,
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
    let since_raw = since_or(q.since.as_deref(), "1h").to_string();
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

/// A trace reference: the story page when a story exists, otherwise the Jaeger trace.
struct TraceLink {
    label: String,
    href: String,
}

fn trace_link(links: &UiLinks, trace_id: &str, story_id: Option<&str>) -> TraceLink {
    TraceLink {
        label: trace_id.chars().take(8).collect(),
        href: match story_id {
            Some(id) => format!("/stories/{id}"),
            None => jaeger_trace_url(links, trace_id),
        },
    }
}

struct AlertRowView {
    kind: String,
    service: String,
    template_id: String,
    template: String,
    counts: String,
    started: String,
    last_seen: String,
    active: bool,
    examples: Vec<TraceLink>,
}

fn alert_row(a: &LogAlertView, links: &UiLinks) -> AlertRowView {
    AlertRowView {
        kind: a.kind.clone(),
        service: a.service.clone(),
        template_id: a.template_id.clone(),
        template: a.template.clone(),
        counts: if a.kind == "spike" {
            format!("{} vs {:.1}", a.peak_count, a.baseline_per_window)
        } else {
            "—".to_string()
        },
        started: fmt_time(a.started_at_ns),
        last_seen: fmt_time(a.last_at_ns),
        active: a.active,
        examples: a
            .example_traces
            .iter()
            .map(|ExampleTrace { trace_id, story_id }| {
                trace_link(links, trace_id, story_id.as_deref())
            })
            .collect(),
    }
}

#[derive(Template, WebTemplate)]
#[template(path = "alerts.html")]
struct AlertsPage {
    since: String,
    kind: String,
    service: String,
    rows: Vec<AlertRowView>,
}

async fn alerts_page<R: Repo>(
    State(s): State<UiState<R>>,
    q: Result<Query<AlertsQuery>, QueryRejection>,
) -> Response {
    let Query(q) = match q {
        Ok(q) => q,
        Err(r) => return error_page(StatusCode::BAD_REQUEST, r.body_text()),
    };
    let f = match alert_filter(q.since.as_deref(), q.kind.as_deref(), q.service.as_deref()) {
        Ok(f) => f,
        Err(e) => return error_page(StatusCode::BAD_REQUEST, e),
    };
    match s.app.repo.log_alerts(&f).await {
        Ok(alerts) => AlertsPage {
            since: since_or(q.since.as_deref(), "24h").to_string(),
            kind: f.kind.clone().unwrap_or_default(),
            service: f.service.clone().unwrap_or_default(),
            rows: alerts.iter().map(|a| alert_row(a, &s.links)).collect(),
        }
        .into_response(),
        Err(e) => {
            s.app.unavailable(e);
            error_page(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable")
        }
    }
}

struct TemplateRowView {
    template_id: String,
    service: String,
    template: String,
    count: u64,
    first_seen: String,
    alerting: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "templates.html")]
struct TemplatesPage {
    since: String,
    since_param: String,
    service: String,
    q: String,
    rows: Vec<TemplateRowView>,
}

async fn templates_page<R: Repo>(
    State(s): State<UiState<R>>,
    q: Result<Query<TemplatesQuery>, QueryRejection>,
) -> Response {
    let Query(q) = match q {
        Ok(q) => q,
        Err(r) => return error_page(StatusCode::BAD_REQUEST, r.body_text()),
    };
    let f = match template_filter(q.since.as_deref(), q.service.as_deref(), q.q.as_deref()) {
        Ok(f) => f,
        Err(e) => return error_page(StatusCode::BAD_REQUEST, e),
    };
    match s.app.repo.log_templates(&f).await {
        Ok(templates) => {
            let since = since_or(q.since.as_deref(), "1h").to_string();
            TemplatesPage {
                since_param: url_component(&since),
                since,
                service: f.service.clone().unwrap_or_default(),
                q: f.q.clone().unwrap_or_default(),
                rows: templates
                    .into_iter()
                    .map(|t| TemplateRowView {
                        first_seen: fmt_time(t.first_seen_ns),
                        template_id: t.template_id,
                        service: t.service,
                        template: t.template,
                        count: t.count,
                        alerting: t.alerting,
                    })
                    .collect(),
            }
            .into_response()
        }
        Err(e) => {
            s.app.unavailable(e);
            error_page(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable")
        }
    }
}

struct HitView {
    time: String,
    link: TraceLink,
}

#[derive(Template, WebTemplate)]
#[template(path = "template.html")]
struct TemplatePage {
    since: String,
    template: String,
    service: String,
    alerting: bool,
    count: u64,
    first_seen: String,
    last_seen: String,
    sample: String,
    spark: String,
    hits: Vec<HitView>,
    alerts: Vec<AlertRowView>,
}

async fn template_page<R: Repo>(
    State(s): State<UiState<R>>,
    Path(id): Path<String>,
    q: Result<Query<SinceQuery>, QueryRejection>,
) -> Response {
    let Query(q) = match q {
        Ok(q) => q,
        Err(r) => return error_page(StatusCode::BAD_REQUEST, r.body_text()),
    };
    let since_raw = since_or(q.since.as_deref(), "24h").to_string();
    let (Ok(id), Ok(since)) = (parse_fingerprint(&id), parse_since(&since_raw)) else {
        return error_page(StatusCode::BAD_REQUEST, "invalid template id or since");
    };
    match s.app.repo.log_template(&id, since).await {
        Ok(Some(d)) => TemplatePage {
            since: since_raw,
            spark: SparkWindow::new(since).render(&d.buckets, 600, 60),
            template: d.template.template,
            service: d.template.service,
            alerting: d.template.alerting,
            count: d.template.count,
            first_seen: fmt_time(d.template.first_seen_ns),
            last_seen: fmt_time(d.template.last_seen_ns),
            sample: d.sample,
            hits: d
                .recent
                .iter()
                .map(|h| HitView {
                    time: fmt_time(h.ts_ns),
                    link: trace_link(&s.links, &h.trace_id, h.story_id.as_deref()),
                })
                .collect(),
            alerts: d.alerts.iter().map(|a| alert_row(a, &s.links)).collect(),
        }
        .into_response(),
        Ok(None) => error_page(StatusCode::NOT_FOUND, "no such log template"),
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
    use crate::params::{AlertFilter, GroupFilter, TemplateFilter};
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
    async fn empty_since_renders_the_default_window() {
        let (status, body) = html(FakeRepo::default(), "/?since=&kind=&service=").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("name=\"since\" value=\"1h\""), "{body}");
        let (status, body) = html(FakeRepo::default(), "/service-map?since=").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("in the last 1h"));
        // No such group in the fake: 404 proves `since=` was accepted rather than a 400.
        let (status, _) = html(FakeRepo::default(), "/groups/42?since=").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
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
                log_id: "1".into(),
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

    fn log(ts_ns: i64, span: &str, service: &str, body: &str) -> TraceLogRow {
        TraceLogRow {
            log_id: "1".into(),
            ts_ns,
            span_id: span.into(),
            service_name: service.into(),
            severity_number: 9,
            severity_text: "INFO".into(),
            body: body.into(),
        }
    }

    fn span(id: &str, service: &str, name: &str) -> TraceSpanRow {
        TraceSpanRow {
            span_id: id.into(),
            parent_span_id: String::new(),
            service_name: service.into(),
            span_name: name.into(),
            kind: "server".into(),
            start_ns: 0,
            duration_ns: 10,
            status: "unset".into(),
            status_message: String::new(),
        }
    }

    fn logs_repo() -> FakeRepo {
        let trace = TraceView {
            trace_id: "ab".repeat(16),
            spans: vec![
                span("s1", "checkout", "PlaceOrder"),
                span("s2", "payment", "charge"),
            ],
            // Deliberately out of order; the page sorts by time.
            logs: vec![
                log(3_000_000, "s1", "checkout", "order placed"),
                log(1_000_000, "s1", "checkout", "[PlaceOrder]"),
                log(2_500_000, "s2", "payment", "Charge request received."),
                log(2_000_000, "unknown", "payment", "no span here"),
            ],
        };
        FakeRepo {
            story: Some(StoryView::from_record(record())),
            trace: Some(trace),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn story_logs_are_time_ordered_with_service_and_span() {
        let (status, body) = html(logs_repo(), &format!("/stories/{}", "ab".repeat(16))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("4 logs linked to this trace · 2 spans in 2 services"));
        let order: Vec<usize> = [
            "[PlaceOrder]",
            "no span here",
            "Charge request received.",
            "order placed",
        ]
        .iter()
        .map(|b| body.find(b).unwrap_or_else(|| panic!("missing {b}")))
        .collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "not time ordered: {order:?}"
        );
        assert!(body.contains("<td class=\"muted\">PlaceOrder</td>"));
        assert!(
            body.contains("<td class=\"muted\">—</td>"),
            "unknown span shows a dash"
        );
        assert!(body.contains("00:00:00.001"), "millisecond clock");
        let id = "ab".repeat(16);
        assert!(body.contains(&format!("href=\"/stories/{id}?log_service=payment#logs\"")));
        assert!(body.contains("all <span class=\"n\">4</span>"));
        assert!(body.contains("payment <span class=\"n\">2</span>"));
    }

    #[tokio::test]
    async fn story_logs_filter_by_service() {
        let id = "ab".repeat(16);
        let (status, body) = html(logs_repo(), &format!("/stories/{id}?log_service=payment")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Charge request received.") && body.contains("no span here"));
        assert!(!body.contains("order placed") && !body.contains("[PlaceOrder]"));
        assert!(body.contains("showing payment only"));
        assert!(
            body.contains(
                "chip on\" href=\"/stories/{id}?log_service=payment#logs\""
                    .replace("{id}", &id)
                    .as_str()
            )
        );
        let (_, none) = html(logs_repo(), &format!("/stories/{id}?log_service=nope")).await;
        assert!(none.contains("No logs from nope in this trace."));
    }

    #[tokio::test]
    async fn story_without_linked_logs_explains_why() {
        let repo = FakeRepo {
            story: Some(StoryView::from_record(record())),
            trace: Some(TraceView {
                trace_id: "ab".repeat(16),
                spans: vec![span("s1", "load-generator", "POST")],
                logs: vec![],
            }),
            ..Default::default()
        };
        let (_, body) = html(repo, &format!("/stories/{}", "ab".repeat(16))).await;
        assert!(body.contains("0 logs linked to this trace · 1 span in 1 service"));
        assert!(body.contains("No logs are linked to this trace."));
        assert!(!body.contains("class=\"chips\""));
    }

    #[test]
    fn unspecified_severity_shows_a_dash() {
        let names = HashMap::new();
        let mut l = log(0, "s", "frontend-proxy", "GET /");
        l.severity_text = String::new();
        l.severity_number = 0;
        assert_eq!(log_view(&l, &names, &HashMap::new()).severity, "—");
        l.severity_number = 17;
        assert_eq!(log_view(&l, &names, &HashMap::new()).severity, "17");
    }

    #[test]
    fn url_component_encodes_reserved_characters() {
        assert_eq!(url_component("frontend-proxy"), "frontend-proxy");
        assert_eq!(url_component("a b&c\"<"), "a%20b%26c%22%3C");
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

    fn alert_view(kind: &str, traces: Vec<ExampleTrace>) -> LogAlertView {
        LogAlertView {
            alert_id: "a1".into(),
            kind: kind.into(),
            template_id: "17393964261140422938".into(),
            service: "payment".into(),
            template: "Payment request failed <*>".into(),
            started_at_ns: 1_700_000_000_000_000_000,
            last_at_ns: 1_700_000_060_000_000_000,
            window_count: 40,
            peak_count: 42,
            baseline_per_window: 1.25,
            active: true,
            example_traces: traces,
        }
    }

    fn example_traces() -> Vec<ExampleTrace> {
        vec![
            ExampleTrace {
                trace_id: "ab".repeat(16),
                story_id: Some("ab".repeat(16)),
            },
            ExampleTrace {
                trace_id: "cd".repeat(16),
                story_id: None,
            },
        ]
    }

    #[tokio::test]
    async fn alerts_page_empty_state_and_bad_params() {
        let (status, body) = html(FakeRepo::default(), "/alerts").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("No log alerts in this window."));
        assert!(body.contains("name=\"since\" value=\"24h\""));
        assert!(body.contains("href=\"/alerts\">Log alerts</a>"));
        for uri in [
            "/alerts?kind=error",
            "/alerts?since=8d",
            "/alerts?since=1h&since=2h",
        ] {
            let (status, body) = html(FakeRepo::default(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert!(body.contains("<html"));
        }
        let repo = FakeRepo {
            fail: true,
            ..Default::default()
        };
        assert_eq!(
            html(repo, "/alerts").await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn alerts_page_lists_alerts_and_prefers_story_links() {
        let repo = FakeRepo {
            alerts: vec![
                alert_view("spike", example_traces()),
                alert_view("new", vec![]),
            ],
            ..Default::default()
        };
        let (status, body) = html(repo, "/alerts?kind=spike&service=payment").await;
        assert_eq!(status, StatusCode::OK);
        // The template is escaped, the counts are one-decimal, `new` shows a dash.
        assert!(
            body.contains("Payment request failed &#60;*&#62;")
                || body.contains("Payment request failed &lt;*&gt;")
        );
        assert!(body.contains("42 vs 1.2") || body.contains("42 vs 1.3"));
        assert!(body.contains("<span class=\"badge spike\">spike</span>"));
        assert!(body.contains("<span class=\"badge new\">new</span>"));
        assert!(body.contains("<td>—</td>"));
        assert!(body.contains("2023-11-14 22:13:20 UTC"));
        assert!(body.contains("<span class=\"badge active\">active</span>"));
        assert!(body.contains("/templates/17393964261140422938"));
        // Story when one exists, Jaeger otherwise.
        assert!(body.contains(&format!("href=\"/stories/{}\"", "ab".repeat(16))));
        assert!(body.contains(&format!(
            "href=\"http://localhost:8080/jaeger/ui/trace/{}\"",
            "cd".repeat(16)
        )));
        assert!(!body.contains(&format!("/jaeger/ui/trace/{}\"", "ab".repeat(16))));
        assert!(body.contains("<option value=\"spike\" selected>"));
    }

    #[tokio::test]
    async fn alerts_filters_reach_the_repo() {
        let repo = Arc::new(FakeRepo::default());
        let app = ui_router(repo.clone(), ApiMetrics::default(), links());
        let res = app
            .oneshot(
                Request::get("/alerts?since=5m&kind=new&service=cart")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let f = repo.last_alert_filter.lock().unwrap().clone().unwrap();
        assert_eq!(
            (f.since_secs, f.kind.as_deref(), f.service.as_deref()),
            (300, Some("new"), Some("cart"))
        );
    }

    fn template_view(count: u64, alerting: bool) -> LogTemplateView {
        LogTemplateView {
            template_id: "17393964261140422938".into(),
            service: "payment".into(),
            template: "Found <script>alert(1)</script> products".into(),
            count,
            first_seen_ns: 1_700_000_000_000_000_000,
            last_seen_ns: 1_700_000_060_000_000_000,
            max_severity: 17,
            alerting,
        }
    }

    #[tokio::test]
    async fn templates_page_renders_rows_and_escapes() {
        let repo = FakeRepo {
            templates: vec![template_view(9, true)],
            ..Default::default()
        };
        let (status, body) = html(repo, "/templates?since=30m&q=Found").await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body.contains("<script>alert(1)</script>"));
        assert!(
            body.contains("&#60;script&#62;alert(1)") || body.contains("&lt;script&gt;alert(1)")
        );
        assert!(body.contains("href=\"/templates/17393964261140422938?since=30m\""));
        assert!(body.contains("<span class=\"badge alerting\">alerting</span>"));
        assert!(body.contains("<td>9</td>"));
        assert!(body.contains("name=\"q\" value=\"Found\""));
    }

    #[tokio::test]
    async fn templates_page_empty_default_and_bad_params() {
        let (status, body) = html(FakeRepo::default(), "/templates").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("No log templates in this window."));
        assert!(body.contains("name=\"since\" value=\"1h\""));
        let long = "x".repeat(201);
        for uri in [
            "/templates?since=0m".to_string(),
            format!("/templates?q={long}"),
        ] {
            let (status, body) = html(FakeRepo::default(), &uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(body.contains("<html"));
        }
        // A hostile search term is attribute-escaped and not reflected into links.
        let (_, body) = html(FakeRepo::default(), "/templates?q=%22%3E%3Cscript%3E").await;
        assert!(!body.contains("\"><script>"));
    }

    fn template_detail(hits: Vec<TemplateHitView>) -> FakeRepo {
        FakeRepo {
            template_detail: Some(LogTemplateDetail {
                template: template_view(3, true),
                sample: "Found <b>3</b> products".into(),
                bucket_secs: 60,
                buckets: vec![(1_700_000_040, 3)],
                recent: hits,
                alerts: vec![alert_view("spike", example_traces())],
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn template_page_renders_sparkline_hits_and_alerts() {
        let hit = |trace: &str, story: Option<&str>| TemplateHitView {
            ts_ns: 1_700_000_000_000_000_000,
            trace_id: trace.into(),
            span_id: "01".repeat(8),
            severity_number: 17,
            story_id: story.map(str::to_string),
        };
        let repo = template_detail(vec![
            hit(&"ab".repeat(16), Some(&"ab".repeat(16))),
            hit(&"ef".repeat(16), None),
        ]);
        let (status, body) = html(repo, "/templates/17393964261140422938").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("<svg"));
        assert!(body.contains("in the last 24h") && body.contains("3 hits"));
        // Sample and template are escaped.
        assert!(!body.contains("<b>3</b>") && !body.contains("<script>alert(1)"));
        assert!(body.contains("&#60;b&#62;3") || body.contains("&lt;b&gt;3"));
        assert!(body.contains(&format!("href=\"/stories/{}\"", "ab".repeat(16))));
        assert!(body.contains(&format!(
            "href=\"http://localhost:8080/jaeger/ui/trace/{}\"",
            "ef".repeat(16)
        )));
        assert!(body.contains("<span class=\"badge spike\">spike</span>"));
        assert!(body.contains("42 vs 1.2") || body.contains("42 vs 1.3"));
    }

    #[tokio::test]
    async fn template_page_404_and_bad_params() {
        assert_eq!(
            html(FakeRepo::default(), "/templates/17393964261140422938")
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        for uri in ["/templates/zz", "/templates/17393964261140422938?since=8d"] {
            let (status, body) = html(FakeRepo::default(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert!(body.contains("<html"));
        }
    }

    fn template_for(log_id: &str, template: &str, alert: Option<&str>) -> TraceLogTemplate {
        TraceLogTemplate {
            log_id: log_id.into(),
            template_id: "17393964261140422938".into(),
            template: template.into(),
            alert: alert.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn story_page_shows_template_column_with_badge() {
        let mut repo = logs_repo();
        repo.trace.as_mut().unwrap().logs[0].log_id = "77".into();
        repo.trace_templates = vec![template_for(
            "77",
            "order <script>x</script> placed",
            Some("spike"),
        )];
        let (status, body) = html(repo, &format!("/stories/{}", "ab".repeat(16))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("<th>Template</th>"));
        assert!(body.contains("href=\"/templates/17393964261140422938\""));
        assert!(!body.contains("<script>x</script>"));
        assert!(body.contains("<span class=\"badge spike\">spike</span>"));
        // The other logs have no template.
        assert!(body.contains("<span class=\"muted\">—</span>"));
    }

    #[test]
    fn long_templates_are_truncated_to_80_chars() {
        let t = template_for("1", &"é".repeat(100), None);
        let map: HashMap<&str, &TraceLogTemplate> = [("1", &t)].into();
        let v = log_view(&log(0, "s", "svc", "b"), &HashMap::new(), &map);
        let cell = v.template.unwrap();
        assert_eq!(cell.text.chars().count(), 80);
        assert!(cell.text.ends_with('…'));
        assert_eq!(cell.full.chars().count(), 100);
        assert_eq!(truncate_chars("short", 80), "short");
    }

    #[tokio::test]
    async fn story_page_survives_template_lookup_failure() {
        // The repo fails on every call, so exercise the handler's fallback through a repo
        // that only fails the template lookup.
        struct FlakyTemplates(FakeRepo);
        impl Repo for FlakyTemplates {
            async fn story_groups(&self, f: &GroupFilter) -> anyhow::Result<Vec<GroupView>> {
                self.0.story_groups(f).await
            }
            async fn story_group(&self, fp: &str, s: u32) -> anyhow::Result<Option<GroupDetail>> {
                self.0.story_group(fp, s).await
            }
            async fn story(&self, id: &str) -> anyhow::Result<Option<StoryView>> {
                self.0.story(id).await
            }
            async fn trace(&self, id: &str) -> anyhow::Result<TraceView> {
                self.0.trace(id).await
            }
            async fn service_map(&self, s: u32) -> anyhow::Result<Vec<EdgeView>> {
                self.0.service_map(s).await
            }
            async fn log_alerts(&self, f: &AlertFilter) -> anyhow::Result<Vec<LogAlertView>> {
                self.0.log_alerts(f).await
            }
            async fn log_templates(
                &self,
                f: &TemplateFilter,
            ) -> anyhow::Result<Vec<LogTemplateView>> {
                self.0.log_templates(f).await
            }
            async fn log_template(
                &self,
                id: &str,
                s: u32,
            ) -> anyhow::Result<Option<LogTemplateDetail>> {
                self.0.log_template(id, s).await
            }
            async fn trace_log_templates(&self, _: &str) -> anyhow::Result<Vec<TraceLogTemplate>> {
                anyhow::bail!("templates down")
            }
        }
        let metrics = ApiMetrics::default();
        let app = ui_router(
            Arc::new(FlakyTemplates(logs_repo())),
            metrics.clone(),
            links(),
        );
        let res = app
            .oneshot(
                Request::get(format!("/stories/{}", "ab".repeat(16)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("order placed") && body.contains("<th>Template</th>"));
        assert!(!body.contains("href=\"/templates/"));
        assert_eq!(metrics.repo_errors.get(), 1);
    }
}
