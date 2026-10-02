//! Regroups an OTLP export request into one request per routing key
//! (trace id, or service name for data without a valid trace id).

use crate::attrs::service_name;
use crate::ids::TraceId;
use crate::otlp::collector::logs::v1::ExportLogsServiceRequest;
use crate::otlp::collector::trace::v1::ExportTraceServiceRequest;
use crate::otlp::common::v1::InstrumentationScope;
use crate::otlp::logs::v1::{ResourceLogs, ScopeLogs};
use crate::otlp::resource::v1::Resource;
use crate::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};
use crate::otlp::logs::v1::LogRecord;
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

/// Trait to abstract over the differences between trace and log signal shapes.
trait SignalShape {
    type Item;
    type Resource;
    type Scope;
    type Request;

    /// Get the trace_id bytes from an item.
    fn item_trace_id(item: &Self::Item) -> &[u8];

    /// Consume a request and extract its resources.
    fn consume_resources(req: Self::Request) -> Vec<Self::Resource>;

    /// Consume a resource and extract its metadata and scopes.
    fn consume_scopes_from_resource(
        resource: Self::Resource,
    ) -> (Option<Resource>, String, Vec<Self::Scope>);

    /// Consume a scope and extract its metadata and items.
    fn consume_items_from_scope(
        scope: Self::Scope,
    ) -> (Option<InstrumentationScope>, String, Vec<Self::Item>);

    /// Create an empty resource shell with only resource and schema_url metadata.
    fn new_empty_resource(
        resource: &Option<Resource>,
        schema_url: String,
    ) -> Self::Resource;

    /// Create an empty scope shell with only scope and schema_url metadata.
    fn new_empty_scope(
        scope: &Option<InstrumentationScope>,
        schema_url: String,
    ) -> Self::Scope;

    /// Get mutable access to the scopes vector in a resource.
    fn get_scopes_mut(resource: &mut Self::Resource) -> &mut Vec<Self::Scope>;

    /// Add an item to a scope.
    fn add_item_to_scope(scope: &mut Self::Scope, item: Self::Item);

    /// Build a request from resources.
    fn build_request(resources: Vec<Self::Resource>) -> Self::Request;
}

struct TracesShape;

impl SignalShape for TracesShape {
    type Item = Span;
    type Resource = ResourceSpans;
    type Scope = ScopeSpans;
    type Request = ExportTraceServiceRequest;

    fn item_trace_id(item: &Span) -> &[u8] {
        &item.trace_id
    }

    fn consume_resources(req: ExportTraceServiceRequest) -> Vec<ResourceSpans> {
        req.resource_spans
    }

    fn consume_scopes_from_resource(
        resource: ResourceSpans,
    ) -> (Option<Resource>, String, Vec<ScopeSpans>) {
        (resource.resource, resource.schema_url, resource.scope_spans)
    }

    fn consume_items_from_scope(scope: ScopeSpans) -> (Option<InstrumentationScope>, String, Vec<Span>) {
        (scope.scope, scope.schema_url, scope.spans)
    }

    fn new_empty_resource(
        resource: &Option<Resource>,
        schema_url: String,
    ) -> ResourceSpans {
        ResourceSpans {
            resource: resource.clone(),
            schema_url,
            ..Default::default()
        }
    }

    fn new_empty_scope(
        scope: &Option<InstrumentationScope>,
        schema_url: String,
    ) -> ScopeSpans {
        ScopeSpans {
            scope: scope.clone(),
            schema_url,
            ..Default::default()
        }
    }

    fn get_scopes_mut(resource: &mut ResourceSpans) -> &mut Vec<ScopeSpans> {
        &mut resource.scope_spans
    }

    fn add_item_to_scope(scope: &mut ScopeSpans, item: Span) {
        scope.spans.push(item);
    }

    fn build_request(resources: Vec<ResourceSpans>) -> ExportTraceServiceRequest {
        ExportTraceServiceRequest { resource_spans: resources }
    }
}

struct LogsShape;

impl SignalShape for LogsShape {
    type Item = LogRecord;
    type Resource = ResourceLogs;
    type Scope = ScopeLogs;
    type Request = ExportLogsServiceRequest;

    fn item_trace_id(item: &LogRecord) -> &[u8] {
        &item.trace_id
    }

    fn consume_resources(req: ExportLogsServiceRequest) -> Vec<ResourceLogs> {
        req.resource_logs
    }

    fn consume_scopes_from_resource(
        resource: ResourceLogs,
    ) -> (Option<Resource>, String, Vec<ScopeLogs>) {
        (resource.resource, resource.schema_url, resource.scope_logs)
    }

    fn consume_items_from_scope(scope: ScopeLogs) -> (Option<InstrumentationScope>, String, Vec<LogRecord>) {
        (scope.scope, scope.schema_url, scope.log_records)
    }

    fn new_empty_resource(
        resource: &Option<Resource>,
        schema_url: String,
    ) -> ResourceLogs {
        ResourceLogs {
            resource: resource.clone(),
            schema_url,
            ..Default::default()
        }
    }

    fn new_empty_scope(
        scope: &Option<InstrumentationScope>,
        schema_url: String,
    ) -> ScopeLogs {
        ScopeLogs {
            scope: scope.clone(),
            schema_url,
            ..Default::default()
        }
    }

    fn get_scopes_mut(resource: &mut ResourceLogs) -> &mut Vec<ScopeLogs> {
        &mut resource.scope_logs
    }

    fn add_item_to_scope(scope: &mut ScopeLogs, item: LogRecord) {
        scope.log_records.push(item);
    }

    fn build_request(resources: Vec<ResourceLogs>) -> ExportLogsServiceRequest {
        ExportLogsServiceRequest { resource_logs: resources }
    }
}

/// Generic implementation of the regrouping algorithm.
fn split_impl<Shape: SignalShape>(req: Shape::Request) -> Vec<Routed<Shape::Request>> {
    let mut groups: Vec<Group<Shape::Resource>> = Vec::new();
    let mut index = HashMap::new();

    let resources = Shape::consume_resources(req);

    for (ri, rs) in resources.into_iter().enumerate() {
        let (resource_field, resource_schema_url, scopes) =
            Shape::consume_scopes_from_resource(rs);
        let service = service_name(resource_field.as_ref());

        for (si, ss) in scopes.into_iter().enumerate() {
            let (scope_field, scope_schema_url, items) = Shape::consume_items_from_scope(ss);

            for item in items {
                let key = TraceId::from_slice(Shape::item_trace_id(&item))
                    .map(RoutingKey::Trace)
                    .unwrap_or_else(|| RoutingKey::Service(service.clone()));

                let g = group_for(&mut groups, &mut index, key);

                if g.last.map(|(r, _)| r) != Some(ri) {
                    g.resources.push(Shape::new_empty_resource(
                        &resource_field,
                        resource_schema_url.clone(),
                    ));
                }

                let res = g.resources.last_mut().expect("resource pushed above");

                if g.last != Some((ri, si)) {
                    let new_scope =
                        Shape::new_empty_scope(&scope_field, scope_schema_url.clone());
                    Shape::get_scopes_mut(res).push(new_scope);
                }

                let scope =
                    Shape::get_scopes_mut(res).last_mut().expect("scope pushed above");
                Shape::add_item_to_scope(scope, item);

                g.last = Some((ri, si));
            }
        }
    }

    groups
        .into_iter()
        .map(|g| Routed {
            key: g.key,
            request: Shape::build_request(g.resources),
        })
        .collect()
}

pub fn split_traces(req: ExportTraceServiceRequest) -> Vec<Routed<ExportTraceServiceRequest>> {
    split_impl::<TracesShape>(req)
}

pub fn split_logs(req: ExportLogsServiceRequest) -> Vec<Routed<ExportLogsServiceRequest>> {
    split_impl::<LogsShape>(req)
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

    #[test]
    fn consecutive_spans_same_trace_same_scope_reuse_scope_shell() {
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: resource("api"),
                scope_spans: vec![ScopeSpans {
                    scope: scope("grpc"),
                    spans: vec![span(5, "span1"), span(5, "span2")],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let out = split_traces(req);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, RoutingKey::Trace(TraceId([5; 16])));
        let req = &out[0].request;
        assert_eq!(req.resource_spans.len(), 1);
        assert_eq!(req.resource_spans[0].scope_spans.len(), 1);
        assert_eq!(req.resource_spans[0].scope_spans[0].spans.len(), 2);
    }

    #[test]
    fn routing_key_trace_to_bytes_is_raw_bytes() {
        let key = RoutingKey::Trace(TraceId([7; 16]));
        assert_eq!(key.to_bytes(), vec![7u8; 16]);
    }
}
