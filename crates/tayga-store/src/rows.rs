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
