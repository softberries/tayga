//! Analysis-friendly trace model built from OTLP, with at-least-once duplicates dropped.

use serde::Serialize;
use std::collections::HashSet;
use tayga_model::attrs::{any_value_to_string, attrs_to_pairs, service_name};
use tayga_model::envelope::{Envelope, Payload};
use tayga_model::ids::{TraceId, hex_id};
use tayga_model::otlp::collector::logs::v1::ExportLogsServiceRequest;
use tayga_model::otlp::collector::trace::v1::ExportTraceServiceRequest;

/// OTLP `SeverityNumber` for ERROR (17..=20 are error levels, 21.. fatal).
pub const SEVERITY_ERROR: i32 = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpanKind {
    Unspecified,
    Internal,
    Server,
    Client,
    Producer,
    Consumer,
}

impl SpanKind {
    pub fn from_otlp(kind: i32) -> Self {
        match kind {
            1 => Self::Internal,
            2 => Self::Server,
            3 => Self::Client,
            4 => Self::Producer,
            5 => Self::Consumer,
            _ => Self::Unspecified,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Internal => "internal",
            Self::Server => "server",
            Self::Client => "client",
            Self::Producer => "producer",
            Self::Consumer => "consumer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusCode {
    Unset,
    Ok,
    Error,
}

impl StatusCode {
    pub fn from_otlp(code: i32) -> Self {
        match code {
            1 => Self::Ok,
            2 => Self::Error,
            _ => Self::Unset,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventRec {
    pub ts_ns: u64,
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpanRec {
    pub span_id: String,
    pub parent_span_id: String,
    pub service: String,
    pub name: String,
    pub kind: SpanKind,
    pub start_ns: u64,
    /// Never before `start_ns` (clamped on ingest).
    pub end_ns: u64,
    pub status: StatusCode,
    pub status_message: String,
    pub attrs: Vec<(String, String)>,
    pub events: Vec<EventRec>,
}

impl SpanRec {
    pub fn duration_ns(&self) -> u64 {
        self.end_ns.saturating_sub(self.start_ns)
    }

    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// `service:span_name`, the unit baselines are kept for.
    pub fn op(&self) -> String {
        format!("{}:{}", self.service, self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogRec {
    pub ts_ns: u64,
    pub span_id: String,
    pub severity_number: i32,
    pub severity_text: String,
    pub body: String,
    pub service: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Endpoint {
    pub service: String,
    pub name: String,
}

const PATH_ATTRS: [&str; 5] = [
    "http.route",
    "url.path",
    "url.full",
    "http.url",
    "http.target",
];

/// Root span name, plus a normalized path when the name alone does not identify the route (spec §9.1).
pub fn endpoint_name(span: &SpanRec) -> String {
    if span.name.contains('/') {
        return span.name.clone();
    }
    PATH_ATTRS
        .iter()
        .find_map(|k| span.attr(k).filter(|v| !v.is_empty()))
        .map(|raw| format!("{} {}", span.name, normalize_path(raw)))
        .unwrap_or_else(|| span.name.clone())
}

/// Strips scheme/host, query and fragment; replaces segments with a digit or > 24 chars by `<*>`.
pub fn normalize_path(raw: &str) -> String {
    // Query and fragment first: they may themselves contain `://` or `/`.
    let raw = raw.split(['?', '#']).next().unwrap_or("");
    let path = match raw.find("://") {
        Some(i) => {
            let rest = &raw[i + 3..];
            rest.find('/').map_or("/", |j| &rest[j..])
        }
        None => raw,
    };
    let joined = path
        .split('/')
        .map(|seg| {
            if seg.len() > 24 || seg.bytes().any(|b| b.is_ascii_digit()) {
                "<*>"
            } else {
                seg
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() {
        "/".to_string()
    } else {
        joined
    }
}

/// All spans and logs of one trace, deduplicated by span id (logs by ts + span id + body).
#[derive(Debug, Clone, Default)]
pub struct TraceBundle {
    pub trace_id: String,
    pub spans: Vec<SpanRec>,
    pub logs: Vec<LogRec>,
    seen_spans: HashSet<String>,
    seen_logs: HashSet<(u64, String, String)>,
}

impl TraceBundle {
    pub fn new(trace_id: impl Into<String>) -> Self {
        Self {
            trace_id: trace_id.into(),
            ..Default::default()
        }
    }

    /// Returns the number of spans added (duplicates skipped).
    pub fn add_traces(&mut self, req: &ExportTraceServiceRequest) -> usize {
        let mut added = 0;
        for rs in &req.resource_spans {
            let service = service_name(rs.resource.as_ref());
            for ss in &rs.scope_spans {
                for s in &ss.spans {
                    let span_id = hex_id(&s.span_id);
                    if !span_id.is_empty() && !self.seen_spans.insert(span_id.clone()) {
                        continue;
                    }
                    let status = s.status.as_ref();
                    self.spans.push(SpanRec {
                        span_id,
                        parent_span_id: hex_id(&s.parent_span_id),
                        service: service.clone(),
                        name: s.name.clone(),
                        kind: SpanKind::from_otlp(s.kind),
                        start_ns: s.start_time_unix_nano,
                        end_ns: s.end_time_unix_nano.max(s.start_time_unix_nano),
                        status: status
                            .map_or(StatusCode::Unset, |st| StatusCode::from_otlp(st.code)),
                        status_message: status.map(|st| st.message.clone()).unwrap_or_default(),
                        attrs: attrs_to_pairs(&s.attributes),
                        events: s
                            .events
                            .iter()
                            .map(|e| EventRec {
                                ts_ns: e.time_unix_nano,
                                name: e.name.clone(),
                                attrs: attrs_to_pairs(&e.attributes),
                            })
                            .collect(),
                    });
                    added += 1;
                }
            }
        }
        added
    }

    /// Returns the number of log records added (duplicates skipped).
    pub fn add_logs(&mut self, req: &ExportLogsServiceRequest) -> usize {
        let mut added = 0;
        for rl in &req.resource_logs {
            let service = service_name(rl.resource.as_ref());
            for sl in &rl.scope_logs {
                for r in &sl.log_records {
                    let ts_ns = if r.time_unix_nano != 0 {
                        r.time_unix_nano
                    } else {
                        r.observed_time_unix_nano
                    };
                    let span_id = hex_id(&r.span_id);
                    let body = r.body.as_ref().map(any_value_to_string).unwrap_or_default();
                    if !self
                        .seen_logs
                        .insert((ts_ns, span_id.clone(), body.clone()))
                    {
                        continue;
                    }
                    self.logs.push(LogRec {
                        ts_ns,
                        span_id,
                        severity_number: r.severity_number,
                        severity_text: r.severity_text.clone(),
                        body,
                        service: service.clone(),
                    });
                    added += 1;
                }
            }
        }
        added
    }

    pub fn add_envelope(&mut self, env: &Envelope) -> usize {
        match &env.payload {
            Some(Payload::Traces(t)) => self.add_traces(t),
            Some(Payload::Logs(l)) => self.add_logs(l),
            None => 0,
        }
    }
}

/// Trace id of the first span or log record carrying a valid one.
pub fn trace_id_of(env: &Envelope) -> Option<TraceId> {
    match &env.payload {
        Some(Payload::Traces(t)) => t
            .resource_spans
            .iter()
            .flat_map(|r| &r.scope_spans)
            .flat_map(|s| &s.spans)
            .find_map(|s| TraceId::from_slice(&s.trace_id)),
        Some(Payload::Logs(l)) => l
            .resource_logs
            .iter()
            .flat_map(|r| &r.scope_logs)
            .flat_map(|s| &s.log_records)
            .find_map(|r| TraceId::from_slice(&r.trace_id)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{attr, span};
    use tayga_model::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
    use tayga_model::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use tayga_model::otlp::resource::v1::Resource;
    use tayga_model::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span, Status};

    fn resource(svc: &str) -> Option<Resource> {
        Some(Resource {
            attributes: vec![KeyValue {
                key: "service.name".into(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue(svc.into())),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })
    }

    fn traces(spans: Vec<Span>) -> ExportTraceServiceRequest {
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: resource("checkout"),
                scope_spans: vec![ScopeSpans {
                    spans,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    fn otlp_span(id: u8, parent: Option<u8>, start: u64, end: u64) -> Span {
        Span {
            trace_id: vec![7; 16],
            span_id: vec![id; 8],
            parent_span_id: parent.map(|p| vec![p; 8]).unwrap_or_default(),
            name: format!("op{id}"),
            kind: 3,
            start_time_unix_nano: start,
            end_time_unix_nano: end,
            status: Some(Status {
                code: 2,
                message: "boom".into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn spans_are_converted() {
        let mut b = TraceBundle::new("t");
        assert_eq!(
            b.add_traces(&traces(vec![otlp_span(1, Some(9), 10, 30)])),
            1
        );
        let s = &b.spans[0];
        assert_eq!(s.span_id, "0101010101010101");
        assert_eq!(s.parent_span_id, "0909090909090909");
        assert_eq!(s.service, "checkout");
        assert_eq!(s.kind, SpanKind::Client);
        assert_eq!(s.status, StatusCode::Error);
        assert_eq!(s.status_message, "boom");
        assert_eq!(s.duration_ns(), 20);
        assert_eq!(s.op(), "checkout:op1");
    }

    #[test]
    fn duplicate_span_ids_are_dropped() {
        let mut b = TraceBundle::new("t");
        let req = traces(vec![otlp_span(1, None, 0, 5), otlp_span(2, Some(1), 1, 4)]);
        assert_eq!(b.add_traces(&req), 2);
        assert_eq!(b.add_traces(&req), 0);
        assert_eq!(b.spans.len(), 2);
    }

    #[test]
    fn end_before_start_is_clamped() {
        let mut b = TraceBundle::new("t");
        b.add_traces(&traces(vec![otlp_span(1, None, 50, 10)]));
        assert_eq!(b.spans[0].end_ns, 50);
        assert_eq!(b.spans[0].duration_ns(), 0);
    }

    #[test]
    fn logs_use_observed_time_and_are_deduplicated() {
        let rec = LogRecord {
            observed_time_unix_nano: 77,
            severity_number: 17,
            body: Some(AnyValue {
                value: Some(Value::StringValue("declined".into())),
            }),
            trace_id: vec![7; 16],
            span_id: vec![1; 8],
            ..Default::default()
        };
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: resource("payment"),
                scope_logs: vec![ScopeLogs {
                    log_records: vec![rec.clone(), rec],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let mut b = TraceBundle::new("t");
        assert_eq!(b.add_envelope(&Envelope::logs(req, 0)), 1);
        assert_eq!(b.logs[0].ts_ns, 77);
        assert_eq!(b.logs[0].service, "payment");
        assert_eq!(b.logs[0].span_id, "0101010101010101");
        assert_eq!(b.logs[0].body, "declined");
    }

    #[test]
    fn kinds_and_codes_map_from_otlp() {
        assert_eq!(SpanKind::from_otlp(2), SpanKind::Server);
        assert_eq!(SpanKind::from_otlp(99), SpanKind::Unspecified);
        assert_eq!(SpanKind::Consumer.as_str(), "consumer");
        assert_eq!(StatusCode::from_otlp(1), StatusCode::Ok);
        assert_eq!(StatusCode::from_otlp(0), StatusCode::Unset);
    }

    #[test]
    fn trace_id_of_finds_first_valid_id() {
        let env = Envelope::traces(traces(vec![otlp_span(1, None, 0, 1)]), 0);
        assert_eq!(trace_id_of(&env), Some(TraceId([7; 16])));
        assert_eq!(
            trace_id_of(&Envelope::logs(ExportLogsServiceRequest::default(), 0)),
            None
        );
    }

    #[test]
    fn normalize_path_cases() {
        assert_eq!(
            normalize_path("http://frontend-proxy:8080/_next/static/chunks/3e712z4qjmeoq.js"),
            "/_next/static/chunks/<*>"
        );
        assert_eq!(
            normalize_path("http://h:1/icons/CartIcon.svg?w=32&q=75"),
            "/icons/CartIcon.svg"
        );
        assert_eq!(
            normalize_path("/api/products/OLJCESPC7Z#x"),
            "/api/products/<*>"
        );
        assert_eq!(normalize_path("http://frontend-proxy:8080"), "/");
        assert_eq!(normalize_path("/api/cart"), "/api/cart");
        assert_eq!(normalize_path("/redirect?to=http://x/y"), "/redirect");
        assert_eq!(normalize_path("http://h?next=/a/b"), "/");
    }

    #[test]
    fn endpoint_name_cases() {
        let root = span("a", "", "frontend-proxy", "GET", 0, 1);
        assert_eq!(endpoint_name(&root), "GET");
        let with_url = attr(
            root.clone(),
            "http.url",
            "http://frontend-proxy:8080/api/cart?x=1",
        );
        assert_eq!(endpoint_name(&with_url), "GET /api/cart");
        let route_first = attr(
            attr(root, "url.full", "http://h/other"),
            "http.route",
            "/api/checkout",
        );
        assert_eq!(endpoint_name(&route_first), "GET /api/checkout");
        let grpc = span("b", "", "cart", "oteldemo.CartService/GetCart", 0, 1);
        assert_eq!(endpoint_name(&grpc), "oteldemo.CartService/GetCart");
    }
}
