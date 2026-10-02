//! OTLP request -> Kafka records: one routing key per record, each record within a byte budget.

use prost::Message;
use tayga_model::attrs::service_name;
use tayga_model::envelope::{Envelope, Kind};
use tayga_model::ids::hex_id;
use tayga_model::otlp::collector::logs::v1::ExportLogsServiceRequest;
use tayga_model::otlp::collector::trace::v1::ExportTraceServiceRequest;
use tayga_model::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use tayga_model::otlp::resource::v1::Resource;
use tayga_model::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};
use tayga_model::split::{Routed, RoutingKey, split_logs, split_traces};

#[derive(Debug, Clone, PartialEq)]
pub struct OutRecord {
    pub key: Vec<u8>,
    /// `tayga-key` header value: "trace" or "service".
    pub key_kind: &'static str,
    pub kind: Kind,
    pub payload: Vec<u8>,
}

/// Records for one OTLP request plus what happened to items that did not go out keyed by trace.
#[derive(Debug, Default, PartialEq)]
pub struct Converted {
    pub records: Vec<OutRecord>,
    /// Spans/logs routed by service name because their trace id was missing or invalid.
    pub routed_by_service: usize,
    /// Spans/logs dropped because a record holding only that item exceeds the byte budget.
    pub dropped_oversized: usize,
}

pub fn now_unix_nano() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

pub fn trace_records(
    req: ExportTraceServiceRequest,
    now_unix_nano: u64,
    max_record_bytes: usize,
) -> Converted {
    convert(split_traces(req), now_unix_nano, max_record_bytes)
}

pub fn log_records(
    req: ExportLogsServiceRequest,
    now_unix_nano: u64,
    max_record_bytes: usize,
) -> Converted {
    convert(split_logs(req), now_unix_nano, max_record_bytes)
}

fn convert<S: Signal>(routed: Vec<Routed<S>>, now: u64, max_record_bytes: usize) -> Converted {
    let mut out = Converted::default();
    for Routed { key, mut request } in routed {
        if matches!(key, RoutingKey::Service(_)) {
            out.routed_by_service += request.item_count();
        }
        let mut payloads = Vec::new();
        fit(
            request,
            now,
            max_record_bytes,
            &mut payloads,
            &mut out.dropped_oversized,
        );
        let key_bytes = key.to_bytes();
        out.records
            .extend(payloads.into_iter().map(|payload| OutRecord {
                key: key_bytes.clone(),
                key_kind: key.kind_str(),
                kind: S::KIND,
                payload,
            }));
    }
    if out.routed_by_service > 0 {
        tracing::debug!(
            signal = S::KIND.as_str(),
            count = out.routed_by_service,
            "items without a valid trace id routed by service"
        );
    }
    out
}

/// Encodes `req` as one envelope if it fits the budget, otherwise halves it (keeping order and
/// resource/scope wrappers) until every part fits. A single item that alone exceeds the budget is
/// dropped: retrying cannot make it fit, so it must not fail the rest of the request.
fn fit<S: Signal>(
    req: S,
    now: u64,
    max_record_bytes: usize,
    out: &mut Vec<Vec<u8>>,
    dropped: &mut usize,
) {
    let size = envelope_len(now, req.encoded_len());
    if size <= max_record_bytes {
        out.push(req.envelope(now).encode());
        return;
    }
    match halve(req) {
        Ok((a, b)) => {
            fit(a, now, max_record_bytes, out, dropped);
            fit(b, now, max_record_bytes, out, dropped);
        }
        Err(mut single) => {
            let (service, trace_id) = single.describe();
            tracing::warn!(
                signal = S::KIND.as_str(),
                service = %service,
                trace_id = %trace_id,
                size,
                max_record_bytes,
                "dropping item larger than the kafka record budget"
            );
            *dropped += single.item_count();
        }
    }
}

/// Exact encoded size of `Envelope { received_at_unix_nano: now, payload: Some(req) }`.
fn envelope_len(now: u64, request_len: usize) -> usize {
    let header = Envelope {
        received_at_unix_nano: now,
        payload: None,
    }
    .encoded_len();
    // oneof field tag (2 or 3, wire type LEN) is one byte, then the length varint.
    header + 1 + prost::encoding::encoded_len_varint(request_len as u64) + request_len
}

/// Splits the items into two requests of nearly equal item count; `Err` if fewer than two items.
fn halve<S: Signal>(mut req: S) -> Result<(S, S), S> {
    let total = req.item_count();
    if total < 2 {
        return Err(req);
    }
    let mid = total / 2;
    let mut seen = 0;
    let (mut left, mut right) = (S::default(), S::default());
    for mut resource in std::mem::take(req.resources()) {
        let scopes = std::mem::take(S::scopes(&mut resource));
        let (mut left_res, mut right_res) = (None, None);
        for mut scope in scopes {
            let mut items = std::mem::take(S::items(&mut scope));
            let tail = items.split_off(mid.saturating_sub(seen).min(items.len()));
            seen += items.len() + tail.len();
            for (part, side) in [(items, &mut left_res), (tail, &mut right_res)] {
                if part.is_empty() {
                    continue;
                }
                let res = side.get_or_insert_with(|| resource.clone());
                let mut shell = scope.clone();
                *S::items(&mut shell) = part;
                S::scopes(res).push(shell);
            }
        }
        left.resources().extend(left_res);
        right.resources().extend(right_res);
    }
    Ok((left, right))
}

/// Uniform access to the resource/scope/item nesting of trace and log requests.
trait Signal: Message + Default + Sized {
    const KIND: Kind;
    type Resource: Clone;
    type Scope: Clone;
    type Item;

    fn resources(&mut self) -> &mut Vec<Self::Resource>;
    fn scopes(r: &mut Self::Resource) -> &mut Vec<Self::Scope>;
    fn items(s: &mut Self::Scope) -> &mut Vec<Self::Item>;
    fn resource_of(r: &Self::Resource) -> Option<&Resource>;
    fn trace_id_of(i: &Self::Item) -> &[u8];
    fn envelope(self, now: u64) -> Envelope;

    fn item_count(&mut self) -> usize {
        self.resources()
            .iter_mut()
            .flat_map(|r| Self::scopes(r).iter_mut())
            .map(|s| Self::items(s).len())
            .sum()
    }

    /// Service name and trace id (hex) of the first item, for logging.
    fn describe(&mut self) -> (String, String) {
        for r in self.resources().iter_mut() {
            let service = service_name(Self::resource_of(r));
            for s in Self::scopes(r).iter_mut() {
                if let Some(item) = Self::items(s).first() {
                    return (service, hex_id(Self::trace_id_of(item)));
                }
            }
        }
        (String::new(), String::new())
    }
}

impl Signal for ExportTraceServiceRequest {
    const KIND: Kind = Kind::Traces;
    type Resource = ResourceSpans;
    type Scope = ScopeSpans;
    type Item = Span;

    fn resources(&mut self) -> &mut Vec<ResourceSpans> {
        &mut self.resource_spans
    }
    fn scopes(r: &mut ResourceSpans) -> &mut Vec<ScopeSpans> {
        &mut r.scope_spans
    }
    fn items(s: &mut ScopeSpans) -> &mut Vec<Span> {
        &mut s.spans
    }
    fn resource_of(r: &ResourceSpans) -> Option<&Resource> {
        r.resource.as_ref()
    }
    fn trace_id_of(i: &Span) -> &[u8] {
        &i.trace_id
    }
    fn envelope(self, now: u64) -> Envelope {
        Envelope::traces(self, now)
    }
}

impl Signal for ExportLogsServiceRequest {
    const KIND: Kind = Kind::Logs;
    type Resource = ResourceLogs;
    type Scope = ScopeLogs;
    type Item = LogRecord;

    fn resources(&mut self) -> &mut Vec<ResourceLogs> {
        &mut self.resource_logs
    }
    fn scopes(r: &mut ResourceLogs) -> &mut Vec<ScopeLogs> {
        &mut r.scope_logs
    }
    fn items(s: &mut ScopeLogs) -> &mut Vec<LogRecord> {
        &mut s.log_records
    }
    fn resource_of(r: &ResourceLogs) -> Option<&Resource> {
        r.resource.as_ref()
    }
    fn trace_id_of(i: &LogRecord) -> &[u8] {
        &i.trace_id
    }
    fn envelope(self, now: u64) -> Envelope {
        Envelope::logs(self, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tayga_model::envelope::Payload;
    use tayga_model::otlp::common::v1::{
        AnyValue, InstrumentationScope, KeyValue, any_value::Value,
    };

    const BIG: usize = 1 << 20;

    #[test]
    fn one_record_per_trace_with_decodable_envelope() {
        let spans = vec![
            Span {
                trace_id: vec![1; 16],
                ..Default::default()
            },
            Span {
                trace_id: vec![2; 16],
                ..Default::default()
            },
        ];
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    spans,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let out = trace_records(req, 5, BIG).records;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key, vec![1; 16]);
        assert_eq!(out[0].key_kind, "trace");
        assert_eq!(out[0].kind, Kind::Traces);
        let env = Envelope::decode(&out[1].payload).unwrap();
        assert_eq!(env.received_at_unix_nano, 5);
        assert_eq!(env.kind(), Some(Kind::Traces));
    }

    #[test]
    fn empty_logs_request_yields_no_records() {
        assert!(
            log_records(ExportLogsServiceRequest::default(), 1, BIG)
                .records
                .is_empty()
        );
    }

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

    fn span(trace: u8, name: String, pad: usize) -> Span {
        Span {
            trace_id: vec![trace; 16],
            name,
            trace_state: "x".repeat(pad),
            ..Default::default()
        }
    }

    /// (service, scope, span name) of every span in the record payload, in order.
    fn spans_of(payload: &[u8]) -> Vec<(String, String, String)> {
        let Some(Payload::Traces(req)) = Envelope::decode(payload).unwrap().payload else {
            panic!("not a traces envelope");
        };
        req.resource_spans
            .iter()
            .flat_map(|rs| {
                let svc = service_name(rs.resource.as_ref());
                rs.scope_spans.iter().flat_map(move |ss| {
                    let scope = ss
                        .scope
                        .as_ref()
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                    let svc = svc.clone();
                    ss.spans
                        .iter()
                        .map(move |s| (svc.clone(), scope.clone(), s.name.clone()))
                })
            })
            .collect()
    }

    #[test]
    fn oversized_trace_is_chunked_under_budget_preserving_order_and_wrappers() {
        let scope = |name: &str| {
            Some(InstrumentationScope {
                name: name.into(),
                ..Default::default()
            })
        };
        let mut n = 0;
        let mut spans = |count: usize| {
            (0..count)
                .map(|_| {
                    n += 1;
                    span(4, format!("s{n:03}"), 200)
                })
                .collect::<Vec<_>>()
        };
        let req = ExportTraceServiceRequest {
            resource_spans: vec![
                ResourceSpans {
                    resource: resource("frontend"),
                    scope_spans: vec![
                        ScopeSpans {
                            scope: scope("http"),
                            spans: spans(30),
                            ..Default::default()
                        },
                        ScopeSpans {
                            scope: scope("grpc"),
                            spans: spans(25),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                ResourceSpans {
                    resource: resource("checkout"),
                    scope_spans: vec![ScopeSpans {
                        scope: scope("grpc"),
                        spans: spans(45),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
        };
        let expected = spans_of(&Envelope::traces(req.clone(), 9).encode());
        assert_eq!(expected.len(), 100);
        let budget = 4_000;

        let out = trace_records(req, 9, budget);
        assert!(
            out.records.len() > 1,
            "expected chunking, got {} record(s)",
            out.records.len()
        );
        assert_eq!(out.dropped_oversized, 0);
        assert!(
            out.records.iter().all(|r| r.payload.len() <= budget
                && r.key == vec![4; 16]
                && r.key_kind == "trace")
        );
        let got: Vec<_> = out
            .records
            .iter()
            .flat_map(|r| spans_of(&r.payload))
            .collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn single_oversized_span_is_dropped_and_the_rest_published() {
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: resource("ad"),
                scope_spans: vec![ScopeSpans {
                    spans: vec![span(1, "huge".into(), 10_000), span(2, "ok".into(), 10)],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let out = trace_records(req, 1, 2_000);
        assert_eq!(out.dropped_oversized, 1);
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0].key, vec![2; 16]);
        assert_eq!(
            spans_of(&out.records[0].payload),
            vec![("ad".into(), String::new(), "ok".into())]
        );
    }

    #[test]
    fn oversized_service_routed_logs_are_chunked() {
        let logs = (0..50)
            .map(|i| LogRecord {
                body: Some(AnyValue {
                    value: Some(Value::StringValue(format!("{i:0>100}"))),
                }),
                ..Default::default()
            })
            .collect();
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: resource("cart"),
                scope_logs: vec![ScopeLogs {
                    log_records: logs,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let out = log_records(req, 1, 1_000);
        assert_eq!(out.routed_by_service, 50);
        assert_eq!(out.dropped_oversized, 0);
        assert!(out.records.len() > 1);
        assert!(
            out.records
                .iter()
                .all(|r| r.payload.len() <= 1_000 && r.key == b"cart" && r.key_kind == "service")
        );
        let total: usize = out
            .records
            .iter()
            .map(|r| match Envelope::decode(&r.payload).unwrap().payload {
                Some(Payload::Logs(l)) => l
                    .resource_logs
                    .iter()
                    .flat_map(|r| &r.scope_logs)
                    .map(|s| s.log_records.len())
                    .sum(),
                _ => 0,
            })
            .sum();
        assert_eq!(total, 50);
    }

    #[test]
    fn envelope_len_matches_encoding() {
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    spans: vec![span(3, "a".into(), 300)],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        for now in [0, 1, u64::MAX] {
            assert_eq!(
                envelope_len(now, req.encoded_len()),
                Envelope::traces(req.clone(), now).encode().len()
            );
        }
        assert_eq!(
            envelope_len(0, 0),
            Envelope::logs(ExportLogsServiceRequest::default(), 0)
                .encode()
                .len()
        );
    }
}
