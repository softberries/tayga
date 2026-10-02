//! ClickHouse row types. DateTime64(9) columns are i64 nanoseconds,
//! Enum8 columns are i8, Map columns are key/value pair vectors.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct SpanRow {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub service_name: String,
    pub span_name: String,
    /// OTLP SpanKind value (0 unspecified .. 5 consumer).
    pub kind: i8,
    pub start_ts: i64,
    pub duration_ns: u64,
    /// 0 unset, 1 ok, 2 error.
    pub status_code: i8,
    pub status_message: String,
    pub resource_attrs: Vec<(String, String)>,
    pub span_attrs: Vec<(String, String)>,
    #[serde(rename = "events.ts")]
    pub events_ts: Vec<i64>,
    #[serde(rename = "events.name")]
    pub events_name: Vec<String>,
    #[serde(rename = "events.attrs")]
    pub events_attrs: Vec<Vec<(String, String)>>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct LogRow {
    pub log_id: u64,
    pub ts: i64,
    pub observed_ts: i64,
    pub trace_id: String,
    pub span_id: String,
    pub severity_number: u8,
    pub severity_text: String,
    pub service_name: String,
    pub body: String,
    pub resource_attrs: Vec<(String, String)>,
    pub log_attrs: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct TraceSummaryRow {
    pub trace_id: String,
    pub ts: i64,
    pub endpoint_service: String,
    pub endpoint_name: String,
    pub duration_ns: u64,
    pub is_error: u8,
    pub op_durations: Vec<(String, u64)>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct ServiceEdgeRow {
    /// Unix seconds, floored to the minute.
    pub minute: u32,
    pub parent_service: String,
    pub child_service: String,
    pub calls: u64,
    pub errors: u64,
    pub duration_ns_sum: u64,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct StoryRow {
    pub story_id: String,
    pub fingerprint: u64,
    /// 1 error, 2 slow.
    pub kind: i8,
    pub ts: i64,
    pub trace_id: String,
    pub endpoint_service: String,
    pub endpoint_name: String,
    pub rc_service: String,
    pub rc_span_name: String,
    pub rc_span_kind: String,
    pub rc_message: String,
    pub rc_exception_type: String,
    pub summary: String,
    pub duration_ns: u64,
    pub path_services: Vec<String>,
    /// JSON documents (see `tayga_analysis::story::Story`).
    pub path_spans: String,
    pub critical_path: String,
    pub baseline_diff: String,
    pub logs: String,
    pub also_failed: String,
    pub span_count: u32,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Deserialize)]
pub struct EndpointStatsRow {
    pub endpoint_service: String,
    pub endpoint_name: String,
    pub traces: u64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Deserialize)]
pub struct OpStatsRow {
    pub endpoint_service: String,
    pub endpoint_name: String,
    pub op: String,
    pub present: u64,
    pub p95: f64,
}
