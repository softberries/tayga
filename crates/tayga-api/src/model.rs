//! API rows (ClickHouse result shapes, field names = SQL aliases) and response views.

use serde::{Deserialize, Serialize};

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
pub struct GroupMinuteRow {
    pub fingerprint: String,
    pub minute: u32,
    pub stories: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupView {
    #[serde(flatten)]
    pub group: StoryGroupRow,
    /// (unix minute start in seconds, stories)
    pub per_minute: Vec<(u32, u64)>,
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
    pub ts_ns: i64,
    pub span_id: String,
    pub service_name: String,
    pub severity_number: u8,
    pub severity_text: String,
    pub body: String,
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
