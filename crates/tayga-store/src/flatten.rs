use crate::rows::{LogRow, SpanRow};
use tayga_model::attrs::{any_value_to_string, attrs_to_pairs, service_name};
use tayga_model::envelope::{Envelope, Payload};
use tayga_model::ids::hex_id;
use tayga_model::otlp::collector::logs::v1::ExportLogsServiceRequest;
use tayga_model::otlp::collector::trace::v1::ExportTraceServiceRequest;
use xxhash_rust::xxh3::Xxh3;

fn nanos(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

pub fn span_rows(req: &ExportTraceServiceRequest) -> Vec<SpanRow> {
    let mut out = Vec::new();
    for rs in &req.resource_spans {
        let service = service_name(rs.resource.as_ref());
        let resource_attrs = rs
            .resource
            .as_ref()
            .map(|r| attrs_to_pairs(&r.attributes))
            .unwrap_or_default();
        for ss in &rs.scope_spans {
            for s in &ss.spans {
                let status = s.status.as_ref();
                out.push(SpanRow {
                    trace_id: hex_id(&s.trace_id),
                    span_id: hex_id(&s.span_id),
                    parent_span_id: hex_id(&s.parent_span_id),
                    service_name: service.clone(),
                    span_name: s.name.clone(),
                    kind: i8::try_from(s.kind).unwrap_or(0),
                    start_ts: nanos(s.start_time_unix_nano),
                    duration_ns: s.end_time_unix_nano.saturating_sub(s.start_time_unix_nano),
                    status_code: status
                        .map(|st| i8::try_from(st.code).unwrap_or(0))
                        .unwrap_or(0),
                    status_message: status.map(|st| st.message.clone()).unwrap_or_default(),
                    resource_attrs: resource_attrs.clone(),
                    span_attrs: attrs_to_pairs(&s.attributes),
                    events_ts: s.events.iter().map(|e| nanos(e.time_unix_nano)).collect(),
                    events_name: s.events.iter().map(|e| e.name.clone()).collect(),
                    events_attrs: s
                        .events
                        .iter()
                        .map(|e| attrs_to_pairs(&e.attributes))
                        .collect(),
                });
            }
        }
    }
    out
}

pub fn log_rows(req: &ExportLogsServiceRequest) -> Vec<LogRow> {
    let mut out = Vec::new();
    for rl in &req.resource_logs {
        let service = service_name(rl.resource.as_ref());
        let resource_attrs = rl
            .resource
            .as_ref()
            .map(|r| attrs_to_pairs(&r.attributes))
            .unwrap_or_default();
        for sl in &rl.scope_logs {
            for r in &sl.log_records {
                let ts = if r.time_unix_nano != 0 {
                    r.time_unix_nano
                } else {
                    r.observed_time_unix_nano
                };
                let body = r.body.as_ref().map(any_value_to_string).unwrap_or_default();
                let mut h = Xxh3::new();
                h.update(&r.trace_id);
                h.update(&r.span_id);
                h.update(&ts.to_le_bytes());
                h.update(body.as_bytes());
                out.push(LogRow {
                    log_id: h.digest(),
                    ts: nanos(ts),
                    observed_ts: nanos(r.observed_time_unix_nano),
                    trace_id: hex_id(&r.trace_id),
                    span_id: hex_id(&r.span_id),
                    severity_number: u8::try_from(r.severity_number).unwrap_or(0),
                    severity_text: r.severity_text.clone(),
                    service_name: service.clone(),
                    body,
                    resource_attrs: resource_attrs.clone(),
                    log_attrs: attrs_to_pairs(&r.attributes),
                });
            }
        }
    }
    out
}

pub fn rows_from_envelope(env: &Envelope) -> (Vec<SpanRow>, Vec<LogRow>) {
    match &env.payload {
        Some(Payload::Traces(t)) => (span_rows(t), Vec::new()),
        Some(Payload::Logs(l)) => (Vec::new(), log_rows(l)),
        None => (Vec::new(), Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tayga_model::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
    use tayga_model::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use tayga_model::otlp::resource::v1::Resource;
    use tayga_model::otlp::trace::v1::{
        ResourceSpans, ScopeSpans, Span, Status, span::Event, status::StatusCode,
    };

    fn kv(k: &str, v: &str) -> KeyValue {
        KeyValue {
            key: k.into(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(v.into())),
            }),
            ..Default::default()
        }
    }

    fn res() -> Option<Resource> {
        Some(Resource {
            attributes: vec![kv("service.name", "payment")],
            ..Default::default()
        })
    }

    #[test]
    fn flatten_span_with_status_events_and_attrs() {
        let span = Span {
            trace_id: vec![0xab; 16],
            span_id: vec![0x01; 8],
            parent_span_id: vec![0x02; 8],
            name: "Charge".into(),
            kind: 2,
            start_time_unix_nano: 1_000,
            end_time_unix_nano: 4_500,
            attributes: vec![kv("rpc.method", "Charge")],
            events: vec![Event {
                time_unix_nano: 2_000,
                name: "exception".into(),
                attributes: vec![kv("exception.message", "boom")],
                ..Default::default()
            }],
            status: Some(Status {
                code: StatusCode::Error as i32,
                message: "failed".into(),
            }),
            ..Default::default()
        };
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: res(),
                scope_spans: vec![ScopeSpans {
                    spans: vec![span],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let rows = span_rows(&req);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.trace_id, "abababababababababababababababab");
        assert_eq!(r.span_id, "0101010101010101");
        assert_eq!(r.parent_span_id, "0202020202020202");
        assert_eq!(r.service_name, "payment");
        assert_eq!(r.span_name, "Charge");
        assert_eq!(r.kind, 2);
        assert_eq!(r.start_ts, 1_000);
        assert_eq!(r.duration_ns, 3_500);
        assert_eq!(r.status_code, 2);
        assert_eq!(r.status_message, "failed");
        assert_eq!(
            r.span_attrs,
            vec![("rpc.method".to_string(), "Charge".to_string())]
        );
        assert_eq!(
            r.resource_attrs,
            vec![("service.name".to_string(), "payment".to_string())]
        );
        assert_eq!(r.events_ts, vec![2_000]);
        assert_eq!(r.events_name, vec!["exception".to_string()]);
        assert_eq!(
            r.events_attrs,
            vec![vec![("exception.message".to_string(), "boom".to_string())]]
        );
    }

    #[test]
    fn flatten_span_with_empty_ids() {
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    spans: vec![Span {
                        start_time_unix_nano: 10,
                        end_time_unix_nano: 5,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let r = &span_rows(&req)[0];
        assert_eq!(r.trace_id, "");
        assert_eq!(r.parent_span_id, "");
        assert_eq!(r.service_name, "unknown_service");
        assert_eq!(r.duration_ns, 0, "end before start must not underflow");
        assert_eq!(r.status_code, 0);
    }

    #[test]
    fn flatten_log_uses_observed_time_when_time_missing_and_hashes_id() {
        let rec = LogRecord {
            observed_time_unix_nano: 77,
            severity_number: 17,
            severity_text: "ERROR".into(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("card declined".into())),
            }),
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            attributes: vec![kv("k", "v")],
            ..Default::default()
        };
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: res(),
                scope_logs: vec![ScopeLogs {
                    log_records: vec![rec.clone(), rec],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let rows = log_rows(&req);
        assert_eq!(rows[0].ts, 77);
        assert_eq!(rows[0].observed_ts, 77);
        assert_eq!(rows[0].severity_number, 17);
        assert_eq!(rows[0].body, "card declined");
        assert_eq!(rows[0].service_name, "payment");
        assert_eq!(rows[0].log_attrs, vec![("k".to_string(), "v".to_string())]);
        assert_ne!(rows[0].log_id, 0);
        assert_eq!(
            rows[0].log_id, rows[1].log_id,
            "identical records hash identically (dedup on replay)"
        );
    }

    #[test]
    fn envelope_dispatches_by_payload() {
        // Traces dispatch to span_rows
        let span = Span {
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            ..Default::default()
        };
        let trace_req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: res(),
                scope_spans: vec![ScopeSpans {
                    spans: vec![span],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let (spans, logs) = rows_from_envelope(&Envelope::traces(trace_req, 0));
        assert_eq!(spans.len(), 1, "Traces envelope should yield 1 span row");
        assert_eq!(logs.len(), 0, "Traces envelope should yield 0 log rows");

        // Logs dispatch to log_rows
        let log_rec = LogRecord {
            observed_time_unix_nano: 100,
            severity_number: 1,
            body: Some(AnyValue {
                value: Some(Value::StringValue("test".into())),
            }),
            ..Default::default()
        };
        let log_req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: res(),
                scope_logs: vec![ScopeLogs {
                    log_records: vec![log_rec],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let (spans, logs) = rows_from_envelope(&Envelope::logs(log_req, 0));
        assert_eq!(spans.len(), 0, "Logs envelope should yield 0 span rows");
        assert_eq!(logs.len(), 1, "Logs envelope should yield 1 log row");

        // None payload yields empty
        let (spans, logs) = rows_from_envelope(&Envelope {
            received_at_unix_nano: 0,
            payload: None,
        });
        assert_eq!(spans.len(), 0, "None payload should yield 0 span rows");
        assert_eq!(logs.len(), 0, "None payload should yield 0 log rows");
    }

    #[test]
    fn log_id_differs_when_body_differs() {
        let rec1 = LogRecord {
            observed_time_unix_nano: 100,
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            body: Some(AnyValue {
                value: Some(Value::StringValue("body1".into())),
            }),
            ..Default::default()
        };
        let rec2 = LogRecord {
            observed_time_unix_nano: 100,
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            body: Some(AnyValue {
                value: Some(Value::StringValue("body2".into())),
            }),
            ..Default::default()
        };
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: res(),
                scope_logs: vec![ScopeLogs {
                    log_records: vec![rec1, rec2],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let rows = log_rows(&req);
        assert_eq!(rows.len(), 2);
        assert_ne!(
            rows[0].log_id, rows[1].log_id,
            "different bodies should produce different log_ids"
        );
        assert_eq!(rows[0].body, "body1");
        assert_eq!(rows[1].body, "body2");
    }
}
