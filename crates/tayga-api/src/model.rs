//! API rows (ClickHouse result shapes, field names = SQL aliases) and response views.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

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

    /// Span ids on the critical path (for highlighting in the waterfall).
    pub fn critical_span_ids(&self) -> Vec<String> {
        self.critical_path["segments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s["span_id"].as_str().map(str::to_string))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
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
        assert_eq!(
            v.critical_span_ids(),
            vec!["a".to_string(), "b".to_string()]
        );
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
}
