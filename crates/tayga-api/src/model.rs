//! API rows (ClickHouse result shapes, field names = SQL aliases) and response views.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct StoryGroupRow {
    pub fingerprint: String,
    pub kind: String,
    pub summary: String,
    pub rc_service: String,
    pub rc_span_name: String,
    pub endpoint_service: String,
    pub endpoint_name: String,
    pub stories: u64,
    pub first_seen_ns: i64,
    pub last_seen_ns: i64,
    pub sample_story_id: String,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct GroupBucketRow {
    pub fingerprint: String,
    /// Unix seconds at the bucket start (epoch-aligned to the bucket width).
    pub bucket: u32,
    pub stories: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupView {
    #[serde(flatten)]
    pub group: StoryGroupRow,
    /// Width of each entry in `buckets`, in seconds (see `params::bucket_secs`).
    pub bucket_secs: u32,
    /// (bucket start in unix seconds, stories), ascending; empty buckets are omitted.
    pub buckets: Vec<(u32, u64)>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct StorySummaryRow {
    pub story_id: String,
    pub ts_ns: i64,
    pub trace_id: String,
    pub duration_ns: u64,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupDetail {
    pub group: GroupView,
    pub examples: Vec<StorySummaryRow>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct StoryRecordRow {
    pub story_id: String,
    pub fingerprint: String,
    pub kind: String,
    pub ts_ns: i64,
    pub trace_id: String,
    pub endpoint_service: String,
    pub endpoint_name: String,
    pub rc_service: String,
    pub rc_span_name: String,
    pub rc_span_kind: String,
    pub rc_span_id: String,
    pub rc_message: String,
    pub rc_exception_type: String,
    pub summary: String,
    pub duration_ns: u64,
    pub path_services: Vec<String>,
    pub path_spans: String,
    pub critical_path: String,
    pub baseline_diff: String,
    pub logs: String,
    pub also_failed: String,
    pub span_count: u32,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryView {
    pub story_id: String,
    pub fingerprint: String,
    pub kind: String,
    pub ts_ns: i64,
    pub duration_ns: u64,
    pub trace_id: String,
    pub endpoint_service: String,
    pub endpoint_name: String,
    pub root_cause: RootCauseView,
    pub summary: String,
    pub path_services: Vec<String>,
    pub path_spans: serde_json::Value,
    /// `{"segments": [...], "top": [...]}`
    pub critical_path: serde_json::Value,
    pub baseline_diff: Option<serde_json::Value>,
    pub logs: serde_json::Value,
    pub also_failed: serde_json::Value,
    pub span_count: u32,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RootCauseView {
    pub service: String,
    pub span_name: String,
    pub span_kind: String,
    pub span_id: String,
    pub message: String,
    pub exception_type: String,
}

fn json_or_null(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or(serde_json::Value::Null)
}

impl StoryView {
    pub fn from_record(r: StoryRecordRow) -> Self {
        Self {
            story_id: r.story_id,
            fingerprint: r.fingerprint,
            kind: r.kind,
            ts_ns: r.ts_ns,
            duration_ns: r.duration_ns,
            trace_id: r.trace_id,
            endpoint_service: r.endpoint_service,
            endpoint_name: r.endpoint_name,
            root_cause: RootCauseView {
                service: r.rc_service,
                span_name: r.rc_span_name,
                span_kind: r.rc_span_kind,
                span_id: r.rc_span_id,
                message: r.rc_message,
                exception_type: r.rc_exception_type,
            },
            summary: r.summary,
            path_services: r.path_services,
            path_spans: json_or_null(&r.path_spans),
            critical_path: json_or_null(&r.critical_path),
            baseline_diff: (!r.baseline_diff.is_empty()).then(|| json_or_null(&r.baseline_diff)),
            logs: json_or_null(&r.logs),
            also_failed: json_or_null(&r.also_failed),
            span_count: r.span_count,
            flags: r.flags,
        }
    }
}

/// One span as read from ClickHouse; `TraceSpanRow` is its response shape.
#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct SpanChRow {
    pub span_id: String,
    pub parent_span_id: String,
    pub service_name: String,
    pub span_name: String,
    pub kind: String,
    pub start_ns: i64,
    pub duration_ns: u64,
    pub status: String,
    pub status_message: String,
    pub attrs: Vec<(String, String)>,
    pub resource: Vec<(String, String)>,
    pub events_ts_ns: Vec<i64>,
    pub events_name: Vec<String>,
    pub events_attrs: Vec<Vec<(String, String)>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpanEvent {
    pub ts_ns: i64,
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceSpanRow {
    pub span_id: String,
    pub parent_span_id: String,
    pub service_name: String,
    pub span_name: String,
    pub kind: String,
    pub start_ns: i64,
    pub duration_ns: u64,
    pub status: String,
    pub status_message: String,
    /// Span attributes, as `[key, value]` pairs.
    pub attrs: Vec<(String, String)>,
    /// Resource attributes, as `[key, value]` pairs.
    pub resource: Vec<(String, String)>,
    pub events: Vec<SpanEvent>,
    /// Duration minus the time covered by direct children (clipped to this span); see
    /// `fill_self_ns`.
    pub self_ns: u64,
}

impl TraceSpanRow {
    /// The response shape of a stored span; `self_ns` starts at the full duration.
    pub fn from_ch(r: SpanChRow) -> Self {
        let events = r
            .events_ts_ns
            .into_iter()
            .zip(r.events_name)
            .zip(r.events_attrs)
            .map(|((ts_ns, name), attrs)| SpanEvent { ts_ns, name, attrs })
            .collect();
        Self {
            span_id: r.span_id,
            parent_span_id: r.parent_span_id,
            service_name: r.service_name,
            span_name: r.span_name,
            kind: r.kind,
            start_ns: r.start_ns,
            duration_ns: r.duration_ns,
            status: r.status,
            status_message: r.status_message,
            attrs: r.attrs,
            resource: r.resource,
            events,
            self_ns: r.duration_ns,
        }
    }
}

fn end_ns(start: i64, dur: u64) -> i64 {
    start.saturating_add(i64::try_from(dur).unwrap_or(i64::MAX))
}

/// Sets each span's `self_ns`: its duration minus the union of its direct children's
/// intervals, each clipped to the span. Overlapping children count once; a span listed as its
/// own parent is ignored.
pub fn fill_self_ns(spans: &mut [TraceSpanRow]) {
    let mut children: std::collections::HashMap<String, Vec<(i64, i64)>> =
        std::collections::HashMap::new();
    for s in spans.iter() {
        if !s.parent_span_id.is_empty() && s.parent_span_id != s.span_id {
            children
                .entry(s.parent_span_id.clone())
                .or_default()
                .push((s.start_ns, end_ns(s.start_ns, s.duration_ns)));
        }
    }
    for s in spans.iter_mut() {
        let (start, end) = (s.start_ns, end_ns(s.start_ns, s.duration_ns));
        let mut iv: Vec<(i64, i64)> = children
            .get(&s.span_id)
            .into_iter()
            .flatten()
            .map(|&(a, b)| (a.max(start), b.min(end)))
            .filter(|(a, b)| a < b)
            .collect();
        iv.sort_unstable();
        let mut covered: u64 = 0;
        let mut cur: Option<(i64, i64)> = None;
        for (a, b) in iv {
            match cur {
                Some((ca, cb)) if a <= cb => cur = Some((ca, cb.max(b))),
                _ => {
                    if let Some((ca, cb)) = cur {
                        covered += cb.abs_diff(ca);
                    }
                    cur = Some((a, b));
                }
            }
        }
        if let Some((ca, cb)) = cur {
            covered += cb.abs_diff(ca);
        }
        s.self_ns = s.duration_ns.saturating_sub(covered);
    }
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct TraceLogRow {
    /// Decimal string of the u64 log id, to join logs to `log_template_hits`.
    pub log_id: String,
    pub ts_ns: i64,
    pub span_id: String,
    pub service_name: String,
    pub severity_number: u8,
    pub severity_text: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct LogAlertRow {
    pub alert_id: String,
    pub kind: String,
    pub template_id: String,
    pub service: String,
    pub template: String,
    pub started_at_ns: i64,
    pub last_at_ns: i64,
    pub window_count: u64,
    pub peak_count: u64,
    pub baseline_per_window: f64,
    pub active: u8,
    pub example_trace_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExampleTrace {
    pub trace_id: String,
    /// Set when an error story exists for the trace (story_id equals trace_id).
    pub story_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogAlertView {
    pub alert_id: String,
    pub kind: String,
    pub template_id: String,
    pub service: String,
    pub template: String,
    pub started_at_ns: i64,
    pub last_at_ns: i64,
    pub window_count: u64,
    pub peak_count: u64,
    pub baseline_per_window: f64,
    pub active: bool,
    pub example_traces: Vec<ExampleTrace>,
}

impl LogAlertView {
    /// `stories` holds the story ids that exist among the example traces.
    pub fn from_row(r: LogAlertRow, stories: &HashSet<String>) -> Self {
        Self {
            alert_id: r.alert_id,
            kind: r.kind,
            template_id: r.template_id,
            service: r.service,
            template: r.template,
            started_at_ns: r.started_at_ns,
            last_at_ns: r.last_at_ns,
            window_count: r.window_count,
            peak_count: r.peak_count,
            baseline_per_window: r.baseline_per_window,
            active: r.active != 0,
            example_traces: r
                .example_trace_ids
                .into_iter()
                .map(|t| ExampleTrace {
                    story_id: stories.contains(&t).then(|| t.clone()),
                    trace_id: t,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct LogTemplateRow {
    pub template_id: String,
    pub service: String,
    pub template: String,
    pub count: u64,
    pub first_seen_ns: i64,
    pub last_seen_ns: i64,
    pub max_severity: u8,
    pub alerting: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogTemplateView {
    pub template_id: String,
    pub service: String,
    pub template: String,
    pub count: u64,
    pub first_seen_ns: i64,
    pub last_seen_ns: i64,
    pub max_severity: u8,
    pub alerting: bool,
}

impl LogTemplateView {
    pub fn from_row(r: LogTemplateRow) -> Self {
        Self {
            template_id: r.template_id,
            service: r.service,
            template: r.template,
            count: r.count,
            first_seen_ns: r.first_seen_ns,
            last_seen_ns: r.last_seen_ns,
            max_severity: r.max_severity,
            alerting: r.alerting != 0,
        }
    }
}

/// One row of `GET /log-templates`: the template plus its hits per bucket over the window,
/// for the table's sparkline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogTemplateListItem {
    #[serde(flatten)]
    pub template: LogTemplateView,
    pub bucket_secs: u32,
    /// (bucket start in unix seconds, distinct hits), ascending; empty buckets are omitted.
    pub buckets: Vec<(u32, u64)>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct TemplateListBucketRow {
    pub template_id: String,
    pub bucket: u32,
    pub hits: u64,
}

/// A `LogTemplateRow` plus the sample body, as one row of the detail query.
#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct TemplateDetailRow {
    pub template_id: String,
    pub service: String,
    pub template: String,
    pub count: u64,
    pub first_seen_ns: i64,
    pub last_seen_ns: i64,
    pub max_severity: u8,
    pub alerting: u8,
    pub sample: String,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct TemplateHitRow {
    pub ts_ns: i64,
    pub trace_id: String,
    pub span_id: String,
    pub severity_number: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TemplateHitView {
    pub ts_ns: i64,
    pub trace_id: String,
    pub span_id: String,
    pub severity_number: u8,
    /// Set when an error story exists for the trace (story_id equals trace_id).
    pub story_id: Option<String>,
}

impl TemplateHitView {
    /// `stories` holds the story ids that exist among the hit traces.
    pub fn from_row(r: TemplateHitRow, stories: &HashSet<String>) -> Self {
        Self {
            story_id: stories.contains(&r.trace_id).then(|| r.trace_id.clone()),
            ts_ns: r.ts_ns,
            trace_id: r.trace_id,
            span_id: r.span_id,
            severity_number: r.severity_number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct TemplateBucketRow {
    pub bucket: u32,
    pub hits: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogTemplateDetail {
    pub template: LogTemplateView,
    pub sample: String,
    pub bucket_secs: u32,
    /// (bucket start in unix seconds, hits), ascending; empty buckets are omitted.
    pub buckets: Vec<(u32, u64)>,
    pub recent: Vec<TemplateHitView>,
    pub alerts: Vec<LogAlertView>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct TraceTemplateRow {
    pub log_id: String,
    pub template_id: String,
    pub template: String,
    pub ts_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceLogTemplate {
    pub log_id: String,
    pub template_id: String,
    pub template: String,
    /// `new` or `spike` when that template has an alert active at the trace's time.
    pub alert: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceView {
    pub trace_id: String,
    pub spans: Vec<TraceSpanRow>,
    pub logs: Vec<TraceLogRow>,
    /// Set when a story exists for the trace (story_id equals trace_id).
    pub story_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct EdgeRow {
    pub parent_service: String,
    pub child_service: String,
    pub calls: u64,
    pub errors: u64,
    pub duration_ns_sum: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EdgeView {
    pub parent: String,
    pub child: String,
    pub calls: u64,
    pub errors: u64,
    pub error_rate: f64,
    pub avg_duration_ns: u64,
}

impl EdgeView {
    pub fn from_row(r: EdgeRow) -> Self {
        let (error_rate, avg_duration_ns) = if r.calls == 0 {
            (0.0, 0)
        } else {
            (
                r.errors as f64 / r.calls as f64,
                r.duration_ns_sum / r.calls,
            )
        };
        Self {
            parent: r.parent_service,
            child: r.child_service,
            calls: r.calls,
            errors: r.errors,
            error_rate,
            avg_duration_ns,
        }
    }
}

/// Story count in one bucket for one kind.
#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct KindBucketRow {
    pub bucket: u32,
    pub kind: String,
    pub n: u64,
}

/// Stories per bucket, split by kind: (bucket start in unix seconds, stories), ascending;
/// empty buckets are omitted.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StoriesSeries {
    pub bucket_secs: u32,
    pub error: Vec<(u32, u64)>,
    pub slow: Vec<(u32, u64)>,
}

impl StoriesSeries {
    pub fn from_rows(bucket_secs: u32, mut rows: Vec<KindBucketRow>) -> Self {
        rows.sort_by_key(|r| r.bucket);
        let pick = |kind: &str| {
            rows.iter()
                .filter(|r| r.kind == kind)
                .map(|r| (r.bucket, r.n))
                .collect()
        };
        Self {
            bucket_secs,
            error: pick("error"),
            slow: pick("slow"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct CountBucketRow {
    pub bucket: u32,
    pub n: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct OverviewView {
    pub bucket_secs: u32,
    pub error_stories: u64,
    pub slow_stories: u64,
    /// Log alerts last seen in the past 10 minutes.
    pub active_alerts: u64,
    /// Spans stored in the window divided by its length.
    pub spans_per_sec: f64,
    /// The newest `tayga_logminer_data_lag_seconds` recorded in the past 5 minutes.
    pub data_lag_secs: Option<f64>,
    /// Per-bucket stories by kind.
    pub stories: StoriesSeries,
    /// (bucket start in unix seconds, spans per second in that bucket), ascending.
    pub spans: Vec<(u32, f64)>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct TraceHitRow {
    pub trace_id: String,
    pub ts_ns: i64,
    pub endpoint_service: String,
    pub endpoint_name: String,
    pub duration_ns: u64,
    pub is_error: u8,
    pub span_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceHitView {
    pub trace_id: String,
    pub ts_ns: i64,
    pub endpoint_service: String,
    pub endpoint_name: String,
    pub duration_ns: u64,
    pub is_error: bool,
    pub span_count: u32,
    /// Set when a story exists for the trace (story_id equals trace_id).
    pub story_id: Option<String>,
    /// The story's kind, `"error"` or `"slow"`, when `story_id` is set. An error story can
    /// exist for a trace whose `is_error` is false (the failure was caught downstream).
    pub story_kind: Option<String>,
}

/// A story id with its kind, for resolving which traces have a story and of what kind.
#[derive(Debug, Clone, PartialEq, clickhouse::Row, Deserialize)]
pub struct StoryKindRow {
    pub story_id: String,
    pub kind: String,
}

impl TraceHitView {
    /// `stories` maps story id (equal to its trace id) to the story kind.
    pub fn from_row(r: TraceHitRow, stories: &HashMap<String, String>) -> Self {
        let kind = stories.get(&r.trace_id).cloned();
        Self {
            story_id: kind.as_ref().map(|_| r.trace_id.clone()),
            story_kind: kind,
            trace_id: r.trace_id,
            ts_ns: r.ts_ns,
            endpoint_service: r.endpoint_service,
            endpoint_name: r.endpoint_name,
            duration_ns: r.duration_ns,
            is_error: r.is_error != 0,
            span_count: r.span_count,
        }
    }
}

/// RED over a service's server and consumer spans in one bucket. `spans` counts every span of
/// the service, so a bucket with spans but no calls is still evidence that the service exists.
#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct ServiceBucketRow {
    pub bucket: u32,
    pub spans: u64,
    pub calls: u64,
    pub errors: u64,
    /// p50, p95 and p99 of the calls' duration in ns.
    pub q: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RedPoint {
    /// Bucket start in unix seconds.
    pub bucket: u32,
    /// Calls per second.
    pub rate: f64,
    pub error_ratio: f64,
    pub p50_ns: f64,
    pub p95_ns: f64,
    pub p99_ns: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServiceView {
    pub service: String,
    pub bucket_secs: u32,
    /// Server and consumer spans in the window.
    pub calls: u64,
    pub errors: u64,
    /// Buckets with at least one call, ascending.
    pub buckets: Vec<RedPoint>,
}

impl ServiceView {
    /// `None` when the service has no spans in the window.
    pub fn from_rows(
        service: &str,
        bucket_secs: u32,
        mut rows: Vec<ServiceBucketRow>,
    ) -> Option<Self> {
        if rows.iter().all(|r| r.spans == 0) {
            return None;
        }
        rows.sort_by_key(|r| r.bucket);
        let q = |r: &ServiceBucketRow, i: usize| r.q.get(i).copied().unwrap_or(0.0);
        Some(Self {
            service: service.to_string(),
            bucket_secs,
            calls: rows.iter().map(|r| r.calls).sum(),
            errors: rows.iter().map(|r| r.errors).sum(),
            buckets: rows
                .iter()
                .filter(|r| r.calls > 0)
                .map(|r| RedPoint {
                    bucket: r.bucket,
                    rate: r.calls as f64 / f64::from(bucket_secs.max(1)),
                    error_ratio: r.errors as f64 / r.calls as f64,
                    p50_ns: q(r, 0),
                    p95_ns: q(r, 1),
                    p99_ns: q(r, 2),
                })
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct NodeRow {
    pub service: String,
    pub calls: u64,
    pub errors: u64,
    pub p99_ns: f64,
    pub baseline_p99_ns: f64,
}

/// A service-map node: RED over its server and consumer spans in the window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeView {
    pub service: String,
    pub calls: u64,
    /// Calls per second over the window.
    pub rate: f64,
    pub error_ratio: f64,
    pub p99_ns: f64,
    /// p99 over the last 24h (`params::HEALTH_BASELINE_SECS`).
    pub baseline_p99_ns: f64,
    /// `ok`, `slow` or `error` (`params::health`).
    pub health: String,
}

impl NodeView {
    pub fn from_row(r: NodeRow, since_secs: u32) -> Self {
        let error_ratio = if r.calls == 0 {
            0.0
        } else {
            r.errors as f64 / r.calls as f64
        };
        Self {
            health: crate::params::health(error_ratio, r.p99_ns, r.baseline_p99_ns).to_string(),
            rate: r.calls as f64 / f64::from(since_secs.max(1)),
            service: r.service,
            calls: r.calls,
            error_ratio,
            p99_ns: r.p99_ns,
            baseline_p99_ns: r.baseline_p99_ns,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ServiceMapView {
    pub edges: Vec<EdgeView>,
    /// Services with server or consumer spans in the window, by name.
    pub nodes: Vec<NodeView>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct SearchTemplate {
    pub template_id: String,
    pub service: String,
    pub template: String,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct SearchGroup {
    pub fingerprint: String,
    pub kind: String,
    pub summary: String,
    pub stories: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SearchView {
    pub services: Vec<String>,
    pub templates: Vec<SearchTemplate>,
    pub groups: Vec<SearchGroup>,
    /// The query itself, lowercased, when it is a 32-hex trace id (not checked for existence).
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeriesView {
    pub metric: String,
    /// `rate`, `gauge`, `q50` or `q99`.
    pub kind: String,
    pub bucket_secs: u32,
    /// (bucket start in unix ms, value), ascending; `null` where a quantile has no data.
    pub points: Vec<(i64, Option<f64>)>,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn record() -> StoryRecordRow {
        StoryRecordRow {
            story_id: "ab".repeat(16),
            fingerprint: "17393964261140422938".into(),
            kind: "error".into(),
            ts_ns: 1,
            trace_id: "ab".repeat(16),
            endpoint_service: "load-generator".into(),
            endpoint_name: "user_checkout_single".into(),
            rc_service: "payment".into(),
            rc_span_name: "charge".into(),
            rc_span_kind: "internal".into(),
            rc_span_id: "c879b3702407e63a".into(),
            rc_message: "Invalid token".into(),
            rc_exception_type: String::new(),
            summary: "payment charge failed: Invalid token".into(),
            duration_ns: 10,
            path_services: vec!["load-generator".into(), "checkout".into(), "payment".into()],
            path_spans: "[]".into(),
            critical_path: r#"{"segments":[{"span_id":"a"},{"span_id":"b"}],"top":[]}"#.into(),
            baseline_diff: String::new(),
            logs: "[]".into(),
            also_failed: "[]".into(),
            span_count: 3,
            flags: vec![],
        }
    }

    #[test]
    fn story_view_parses_json_columns() {
        let v = StoryView::from_record(record());
        assert_eq!(v.root_cause.span_id, "c879b3702407e63a");
        assert!(v.baseline_diff.is_none());
        assert_eq!(v.critical_path["segments"][1]["span_id"], "b");
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["fingerprint"], "17393964261140422938");
    }

    #[test]
    fn alert_view_marks_examples_with_stories() {
        let row = LogAlertRow {
            alert_id: "a".into(),
            kind: "spike".into(),
            template_id: "7".into(),
            service: "payment".into(),
            template: "t".into(),
            started_at_ns: 1,
            last_at_ns: 2,
            window_count: 5,
            peak_count: 6,
            baseline_per_window: 0.5,
            active: 1,
            example_trace_ids: vec!["x".into(), "y".into()],
        };
        let stories: HashSet<String> = ["y".to_string()].into();
        let v = LogAlertView::from_row(row, &stories);
        assert!(v.active);
        assert_eq!(v.example_traces[0].story_id, None);
        assert_eq!(v.example_traces[1].story_id.as_deref(), Some("y"));
    }

    #[test]
    fn trace_hit_story_kind() {
        let row = |id: &str, err: u8| TraceHitRow {
            trace_id: id.into(),
            ts_ns: 1,
            endpoint_service: "frontend".into(),
            endpoint_name: "GET /".into(),
            duration_ns: 2,
            is_error: err,
            span_count: 3,
        };
        let stories: HashMap<String, String> = [
            ("e".to_string(), "error".to_string()),
            ("s".to_string(), "slow".to_string()),
        ]
        .into();
        // An error story on a trace whose summary is not an error keeps its kind.
        let e = TraceHitView::from_row(row("e", 0), &stories);
        assert_eq!(
            (e.story_id.as_deref(), e.story_kind.as_deref(), e.is_error),
            (Some("e"), Some("error"), false)
        );
        let s = TraceHitView::from_row(row("s", 0), &stories);
        assert_eq!(
            (s.story_id.as_deref(), s.story_kind.as_deref()),
            (Some("s"), Some("slow"))
        );
        let n = TraceHitView::from_row(row("n", 1), &stories);
        assert_eq!((n.story_id, n.story_kind, n.is_error), (None, None, true));
    }

    #[test]
    fn edge_view_rates() {
        let e = EdgeView::from_row(EdgeRow {
            parent_service: "a".into(),
            child_service: "b".into(),
            calls: 4,
            errors: 1,
            duration_ns_sum: 40,
        });
        assert_eq!((e.error_rate, e.avg_duration_ns), (0.25, 10));
        let idle = EdgeRow {
            parent_service: "a".into(),
            child_service: "b".into(),
            calls: 0,
            errors: 0,
            duration_ns_sum: 0,
        };
        assert_eq!(EdgeView::from_row(idle).avg_duration_ns, 0);
    }

    fn span(id: &str, parent: &str, start: i64, dur: u64) -> TraceSpanRow {
        TraceSpanRow::from_ch(SpanChRow {
            span_id: id.into(),
            parent_span_id: parent.into(),
            service_name: "s".into(),
            span_name: "n".into(),
            kind: "server".into(),
            start_ns: start,
            duration_ns: dur,
            status: "unset".into(),
            status_message: String::new(),
            attrs: vec![],
            resource: vec![],
            events_ts_ns: vec![5, 6],
            events_name: vec!["exception".into(), "retry".into()],
            events_attrs: vec![vec![("exception.message".into(), "boom".into())], vec![]],
        })
    }

    #[test]
    fn events_zip_into_structs() {
        let s = span("a", "", 0, 10);
        assert_eq!(s.events.len(), 2);
        assert_eq!(s.events[0].name, "exception");
        assert_eq!(s.events[0].attrs[0].1, "boom");
        assert_eq!(s.events[1].ts_ns, 6);
        assert_eq!(s.self_ns, 10);
    }

    #[test]
    fn self_time_subtracts_the_union_of_clipped_children() {
        let mut spans = vec![
            span("root", "", 0, 100),
            // Overlapping children [10, 40) and [30, 50): union 40.
            span("a", "root", 10, 30),
            span("b", "root", 30, 20),
            // Sticks out past the root's end: clipped to [90, 100).
            span("c", "root", 90, 50),
            // Starts before the root: clipped to [0, 5).
            span("d", "root", -20, 25),
            // A grandchild does not count against the root.
            span("e", "a", 12, 5),
            // Self-parented and orphaned spans are ignored.
            span("loop", "loop", 0, 7),
            span("orphan", "missing", 0, 3),
        ];
        fill_self_ns(&mut spans);
        let by = |id: &str| spans.iter().find(|s| s.span_id == id).unwrap().self_ns;
        assert_eq!(by("root"), 100 - 40 - 10 - 5);
        assert_eq!(by("a"), 25);
        assert_eq!(by("b"), 20);
        assert_eq!(by("loop"), 7);
        assert_eq!(by("orphan"), 3);
    }

    #[test]
    fn self_time_never_underflows_and_handles_zero_length() {
        let mut spans = vec![span("p", "", 0, 0), span("c", "p", 0, 10)];
        fill_self_ns(&mut spans);
        assert_eq!(spans[0].self_ns, 0);
        let mut spans = vec![
            span("p", "", 0, 10),
            span("c", "p", 0, 10),
            span("c2", "p", 0, 10),
        ];
        fill_self_ns(&mut spans);
        assert_eq!(spans[0].self_ns, 0);
    }

    #[test]
    fn stories_series_splits_kinds_in_bucket_order() {
        let r = |bucket, kind: &str, n| KindBucketRow {
            bucket,
            kind: kind.into(),
            n,
        };
        let s = StoriesSeries::from_rows(
            60,
            vec![r(120, "error", 2), r(60, "slow", 1), r(60, "error", 3)],
        );
        assert_eq!(s.error, vec![(60, 3), (120, 2)]);
        assert_eq!(s.slow, vec![(60, 1)]);
    }

    #[test]
    fn service_view_rates_and_absence() {
        let row = |bucket, spans, calls, errors| ServiceBucketRow {
            bucket,
            spans,
            calls,
            errors,
            q: vec![1.0, 2.0, 3.0],
        };
        assert!(ServiceView::from_rows("x", 60, vec![]).is_none());
        let v =
            ServiceView::from_rows("x", 60, vec![row(120, 5, 0, 0), row(60, 10, 6, 3)]).unwrap();
        assert_eq!((v.calls, v.errors), (6, 3));
        assert_eq!(v.buckets.len(), 1, "a bucket without calls is dropped");
        assert_eq!(v.buckets[0].rate, 0.1);
        assert_eq!(v.buckets[0].error_ratio, 0.5);
        assert_eq!((v.buckets[0].p50_ns, v.buckets[0].p99_ns), (1.0, 3.0));
        // A service with only client spans exists but has no RED points.
        let v = ServiceView::from_rows("x", 60, vec![row(60, 4, 0, 0)]).unwrap();
        assert!(v.buckets.is_empty());
    }

    #[test]
    fn node_view_health() {
        let n = |calls, errors, p99, base| {
            NodeView::from_row(
                NodeRow {
                    service: "s".into(),
                    calls,
                    errors,
                    p99_ns: p99,
                    baseline_p99_ns: base,
                },
                10,
            )
        };
        assert_eq!(n(100, 5, 1.0, 1.0).health, "error");
        assert_eq!(n(100, 0, 3.0, 1.0).health, "slow");
        assert_eq!(n(100, 0, 1.0, 1.0).health, "ok");
        assert_eq!(n(100, 0, 1.0, 1.0).rate, 10.0);
        assert_eq!(n(0, 0, 0.0, 0.0).error_ratio, 0.0);
    }
}
