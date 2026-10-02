//! Regroups an OTLP export request into one request per routing key
//! (trace id, or service name for data without a valid trace id).

use crate::attrs::service_name;
use crate::ids::TraceId;
use crate::otlp::collector::logs::v1::ExportLogsServiceRequest;
use crate::otlp::collector::trace::v1::ExportTraceServiceRequest;
use crate::otlp::logs::v1::{ResourceLogs, ScopeLogs};
use crate::otlp::trace::v1::{ResourceSpans, ScopeSpans};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RoutingKey {
    Trace(TraceId),
    Service(String),
}

impl RoutingKey {
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            RoutingKey::Trace(id) => id.0.to_vec(),
            RoutingKey::Service(name) => name.as_bytes().to_vec(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Routed<T> {
    pub key: RoutingKey,
    pub request: T,
}

struct Group<R> {
    key: RoutingKey,
    resources: Vec<R>,
    last: Option<(usize, usize)>,
}

fn group_for<'a, R>(
    groups: &'a mut Vec<Group<R>>,
    index: &mut HashMap<RoutingKey, usize>,
    key: RoutingKey,
) -> &'a mut Group<R> {
    let i = *index.entry(key.clone()).or_insert_with(|| {
        groups.push(Group { key, resources: Vec::new(), last: None });
        groups.len() - 1
    });
    &mut groups[i]
}

pub fn split_traces(req: ExportTraceServiceRequest) -> Vec<Routed<ExportTraceServiceRequest>> {
    let mut groups: Vec<Group<ResourceSpans>> = Vec::new();
    let mut index = HashMap::new();
    for (ri, rs) in req.resource_spans.into_iter().enumerate() {
        let service = service_name(rs.resource.as_ref());
        for (si, ss) in rs.scope_spans.into_iter().enumerate() {
            for span in ss.spans {
                let key = TraceId::from_slice(&span.trace_id)
                    .map(RoutingKey::Trace)
                    .unwrap_or_else(|| RoutingKey::Service(service.clone()));
                let g = group_for(&mut groups, &mut index, key);
                if g.last.map(|(r, _)| r) != Some(ri) {
                    g.resources.push(ResourceSpans {
                        resource: rs.resource.clone(),
                        schema_url: rs.schema_url.clone(),
                        ..Default::default()
                    });
                }
                let res = g.resources.last_mut().expect("resource pushed above");
                if g.last != Some((ri, si)) {
                    res.scope_spans.push(ScopeSpans {
                        scope: ss.scope.clone(),
                        schema_url: ss.schema_url.clone(),
                        ..Default::default()
                    });
                }
                res.scope_spans.last_mut().expect("scope pushed above").spans.push(span);
                g.last = Some((ri, si));
            }
        }
    }
    groups
        .into_iter()
        .map(|g| Routed { key: g.key, request: ExportTraceServiceRequest { resource_spans: g.resources } })
        .collect()
}

pub fn split_logs(req: ExportLogsServiceRequest) -> Vec<Routed<ExportLogsServiceRequest>> {
    let mut groups: Vec<Group<ResourceLogs>> = Vec::new();
    let mut index = HashMap::new();
    for (ri, rl) in req.resource_logs.into_iter().enumerate() {
        let service = service_name(rl.resource.as_ref());
        for (si, sl) in rl.scope_logs.into_iter().enumerate() {
            for record in sl.log_records {
                let key = TraceId::from_slice(&record.trace_id)
                    .map(RoutingKey::Trace)
                    .unwrap_or_else(|| RoutingKey::Service(service.clone()));
                let g = group_for(&mut groups, &mut index, key);
                if g.last.map(|(r, _)| r) != Some(ri) {
                    g.resources.push(ResourceLogs {
                        resource: rl.resource.clone(),
                        schema_url: rl.schema_url.clone(),
                        ..Default::default()
                    });
                }
                let res = g.resources.last_mut().expect("resource pushed above");
                if g.last != Some((ri, si)) {
                    res.scope_logs.push(ScopeLogs {
                        scope: sl.scope.clone(),
                        schema_url: sl.schema_url.clone(),
                        ..Default::default()
                    });
                }
                res.scope_logs.last_mut().expect("scope pushed above").log_records.push(record);
                g.last = Some((ri, si));
            }
        }
    }
    groups
        .into_iter()
        .map(|g| Routed { key: g.key, request: ExportLogsServiceRequest { resource_logs: g.resources } })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otlp::common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value::Value};
    use crate::otlp::logs::v1::LogRecord;
    use crate::otlp::resource::v1::Resource;
    use crate::otlp::trace::v1::Span;

    fn resource(svc: &str) -> Option<Resource> {
        Some(Resource {
            attributes: vec![KeyValue {
                key: "service.name".into(),
                value: Some(AnyValue { value: Some(Value::StringValue(svc.into())) }),
                ..Default::default()
            }],
            ..Default::default()
        })
    }

    fn scope(name: &str) -> Option<InstrumentationScope> {
        Some(InstrumentationScope { name: name.into(), ..Default::default() })
    }

    fn span(trace: u8, name: &str) -> Span {
        Span { trace_id: vec![trace; 16], span_id: vec![trace; 8], name: name.into(), ..Default::default() }
    }

    fn span_names(r: &ExportTraceServiceRequest) -> Vec<(String, String, String)> {
        r.resource_spans
            .iter()
            .flat_map(|rs| {
                let svc = service_name(rs.resource.as_ref());
                rs.scope_spans.iter().flat_map(move |ss| {
                    let sc = ss.scope.as_ref().map(|s| s.name.clone()).unwrap_or_default();
                    let svc = svc.clone();
                    ss.spans.iter().map(move |s| (svc.clone(), sc.clone(), s.name.clone()))
                })
            })
            .collect()
    }

    #[test]
    fn split_traces_groups_by_trace_and_keeps_resource_and_scope() {
        let req = ExportTraceServiceRequest {
            resource_spans: vec![
                ResourceSpans {
                    resource: resource("frontend"),
                    scope_spans: vec![
                        ScopeSpans { scope: scope("http"), spans: vec![span(1, "a"), span(2, "b")], ..Default::default() },
                        ScopeSpans { scope: scope("grpc"), spans: vec![span(1, "c")], ..Default::default() },
                    ],
                    ..Default::default()
                },
                ResourceSpans {
                    resource: resource("checkout"),
                    scope_spans: vec![ScopeSpans { scope: scope("grpc"), spans: vec![span(2, "d")], ..Default::default() }],
                    ..Default::default()
                },
            ],
        };
        let out = split_traces(req);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key, RoutingKey::Trace(TraceId([1; 16])));
        assert_eq!(
            span_names(&out[0].request),
            vec![
                ("frontend".into(), "http".into(), "a".into()),
                ("frontend".into(), "grpc".into(), "c".into()),
            ]
        );
        assert_eq!(out[0].request.resource_spans.len(), 1);
        assert_eq!(out[1].key, RoutingKey::Trace(TraceId([2; 16])));
        assert_eq!(
            span_names(&out[1].request),
            vec![
                ("frontend".into(), "http".into(), "b".into()),
                ("checkout".into(), "grpc".into(), "d".into()),
            ]
        );
    }

    #[test]
    fn split_traces_invalid_trace_id_routes_by_service() {
        let mut bad = span(0, "internal");
        bad.trace_id = vec![];
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: resource("ad"),
                scope_spans: vec![ScopeSpans { spans: vec![bad, span(0, "zero")], ..Default::default() }],
                ..Default::default()
            }],
        };
        let out = split_traces(req);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, RoutingKey::Service("ad".into()));
        assert_eq!(out[0].key.to_bytes(), b"ad".to_vec());
        assert_eq!(span_names(&out[0].request).len(), 2);
    }

    #[test]
    fn split_empty_request_yields_nothing() {
        assert!(split_traces(ExportTraceServiceRequest::default()).is_empty());
        assert!(split_logs(ExportLogsServiceRequest::default()).is_empty());
    }

    #[test]
    fn split_logs_by_trace_or_service() {
        let with_trace = LogRecord { trace_id: vec![3; 16], ..Default::default() };
        let without = LogRecord::default();
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: resource("cart"),
                scope_logs: vec![ScopeLogs { log_records: vec![with_trace, without], ..Default::default() }],
                ..Default::default()
            }],
        };
        let out = split_logs(req);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key, RoutingKey::Trace(TraceId([3; 16])));
        assert_eq!(out[1].key, RoutingKey::Service("cart".into()));
        assert_eq!(out[1].request.resource_logs[0].scope_logs[0].log_records.len(), 1);
    }
}
