//! Per-trace summary row (baseline input) and cross-service edges (dependency map).

use crate::model::Endpoint;
use crate::tree::SpanTree;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceSummary {
    pub trace_id: String,
    /// Root span start.
    pub ts_ns: u64,
    pub endpoint: Endpoint,
    pub duration_ns: u64,
    pub is_error: bool,
    /// `service:span_name` → longest duration in this trace, sorted by op.
    pub op_durations: Vec<(String, u64)>,
    pub span_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeObs {
    pub parent: String,
    pub child: String,
    /// The child span is an error span.
    pub error: bool,
    pub duration_ns: u64,
}

pub fn trace_summary(tree: &SpanTree, is_error: bool) -> TraceSummary {
    let mut ops: BTreeMap<String, u64> = BTreeMap::new();
    for s in &tree.bundle.spans {
        let d = ops.entry(s.op()).or_default();
        *d = (*d).max(s.duration_ns());
    }
    let root = tree.span(tree.root);
    TraceSummary {
        trace_id: tree.bundle.trace_id.clone(),
        ts_ns: root.start_ns,
        endpoint: tree.endpoint(),
        duration_ns: root.duration_ns(),
        is_error,
        op_durations: ops.into_iter().collect(),
        span_count: u32::try_from(tree.bundle.spans.len()).unwrap_or(u32::MAX),
    }
}

/// One observation per span whose parent belongs to another service.
pub fn service_edges(tree: &SpanTree, is_err: &[bool]) -> Vec<EdgeObs> {
    (0..tree.bundle.spans.len())
        .filter_map(|i| {
            let p = tree.parent[i]?;
            let (parent, child) = (tree.span(p), tree.span(i));
            (parent.service != child.service).then(|| EdgeObs {
                parent: parent.service.clone(),
                child: child.service.clone(),
                error: is_err[i],
                duration_ns: child.duration_ns(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{attr, bundle, span};

    #[test]
    fn summary_takes_root_and_max_per_op() {
        let b = bundle(vec![
            attr(
                span("r", "", "frontend-proxy", "GET", 100, 400),
                "http.url",
                "http://h/api/cart",
            ),
            span("a", "r", "cart", "GetCart", 110, 150),
            span("b", "r", "cart", "GetCart", 160, 260),
        ]);
        let t = SpanTree::build(&b).unwrap();
        let s = trace_summary(&t, true);
        assert_eq!(s.trace_id, "t1");
        assert_eq!(s.ts_ns, 100);
        assert_eq!(s.duration_ns, 300);
        assert_eq!(s.span_count, 3);
        assert!(s.is_error);
        assert_eq!(
            s.endpoint,
            Endpoint {
                service: "frontend-proxy".into(),
                name: "GET /api/cart".into()
            }
        );
        assert_eq!(
            s.op_durations,
            vec![
                ("cart:GetCart".to_string(), 100),
                ("frontend-proxy:GET".to_string(), 300)
            ]
        );
    }

    #[test]
    fn edges_only_cross_services() {
        let b = bundle(vec![
            span("r", "", "frontend", "GET", 0, 100),
            span("a", "r", "frontend", "render", 10, 20),
            span("b", "r", "cart", "GetCart", 30, 60),
        ]);
        let t = SpanTree::build(&b).unwrap();
        let edges = service_edges(&t, &[false, false, true]);
        assert_eq!(
            edges,
            vec![EdgeObs {
                parent: "frontend".into(),
                child: "cart".into(),
                error: true,
                duration_ns: 30
            }]
        );
    }
}
