//! Closed traces → rows and Kafka messages. Analysis panics are isolated per trace (spec §13).

use crate::convert::{story_row, summary_row};
use crate::window::ClosedTrace;
use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use tayga_analysis::baseline::{Baseline, Thresholds};
use tayga_analysis::model::Endpoint;
use tayga_analysis::story::analyze;
use tayga_store::rows::{ServiceEdgeRow, StoryRow, TraceSummaryRow};

#[derive(Debug, Default)]
pub struct Outputs {
    pub summaries: Vec<TraceSummaryRow>,
    pub edges: Vec<ServiceEdgeRow>,
    pub stories: Vec<StoryRow>,
    /// (Kafka key = decimal fingerprint, story JSON) for `tayga.stories`.
    pub story_messages: Vec<(String, String)>,
    /// Traces whose analysis panicked.
    pub failed: usize,
}

impl Outputs {
    pub fn is_empty(&self) -> bool {
        self.summaries.is_empty() && self.edges.is_empty() && self.stories.is_empty()
    }
}

pub fn process(
    closed: &[ClosedTrace],
    baselines: &HashMap<Endpoint, Baseline>,
    t: &Thresholds,
) -> Outputs {
    let mut out = Outputs::default();
    let mut edges: BTreeMap<(u32, String, String), (u64, u64, u64)> = BTreeMap::new();
    for ct in closed {
        let analysis = match catch_unwind(AssertUnwindSafe(|| {
            analyze(&ct.bundle, baselines, t, &ct.flags)
        })) {
            Ok(Some(a)) => a,
            Ok(None) => continue,
            Err(_) => {
                tracing::error!(trace_id = %ct.bundle.trace_id, "analysis panicked; trace skipped");
                out.failed += 1;
                continue;
            }
        };
        let minute = (analysis.summary.ts_ns / 1_000_000_000 / 60 * 60) as u32;
        for e in &analysis.edges {
            let entry = edges
                .entry((minute, e.parent.clone(), e.child.clone()))
                .or_default();
            entry.0 += 1;
            entry.1 += u64::from(e.error);
            entry.2 += e.duration_ns;
        }
        out.summaries.push(summary_row(&analysis.summary));
        if let Some(story) = &analysis.story {
            match (story_row(story), serde_json::to_string(story)) {
                (Ok(row), Ok(json)) => {
                    out.stories.push(row);
                    out.story_messages
                        .push((story.fingerprint.to_string(), json));
                }
                (Err(e), _) | (_, Err(e)) => {
                    tracing::error!(error = %e, trace_id = %story.trace_id, "story serialization failed")
                }
            }
        }
    }
    out.edges = edges
        .into_iter()
        .map(
            |((minute, parent_service, child_service), (calls, errors, duration_ns_sum))| {
                ServiceEdgeRow {
                    minute,
                    parent_service,
                    child_service,
                    calls,
                    errors,
                    duration_ns_sum,
                }
            },
        )
        .collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tayga_analysis::model::{SpanKind, SpanRec, StatusCode, TraceBundle};

    fn span(id: &str, parent: &str, service: &str, start: u64, end: u64, failed: bool) -> SpanRec {
        SpanRec {
            span_id: id.into(),
            parent_span_id: parent.into(),
            service: service.into(),
            name: format!("{service}-op"),
            kind: SpanKind::Server,
            start_ns: start,
            end_ns: end,
            status: if failed {
                StatusCode::Error
            } else {
                StatusCode::Unset
            },
            status_message: if failed {
                "boom 42".into()
            } else {
                String::new()
            },
            attrs: Vec::new(),
            events: Vec::new(),
        }
    }

    fn closed(trace: &str, spans: Vec<SpanRec>) -> ClosedTrace {
        let mut bundle = TraceBundle::new(trace);
        bundle.spans = spans;
        ClosedTrace {
            partition: 0,
            bundle,
            flags: Vec::new(),
        }
    }

    #[test]
    fn processes_summaries_edges_and_stories() {
        let base = 1_790_925_000_000_000_000; // a minute boundary in ns
        let traces = vec![
            closed(
                "t1",
                vec![
                    span("a", "", "frontend", base, base + 100, false),
                    span("b", "a", "cart", base + 10, base + 50, false),
                ],
            ),
            closed(
                "t2",
                vec![
                    span("a", "", "frontend", base + 5, base + 100, false),
                    span("b", "a", "cart", base + 10, base + 50, true),
                ],
            ),
            closed("t3", Vec::new()),
        ];
        let out = process(&traces, &HashMap::new(), &Thresholds::default());
        assert_eq!(out.summaries.len(), 2);
        assert!(out.summaries.iter().all(|s| s.span_count == 2));
        assert_eq!(out.stories.len(), 1);
        assert_eq!(out.stories[0].rc_service, "cart");
        assert_eq!(out.stories[0].rc_span_id, "b");
        assert_eq!(out.stories[0].kind, 1);
        assert_eq!(out.story_messages.len(), 1);
        assert_eq!(
            out.story_messages[0].0,
            out.stories[0].fingerprint.to_string()
        );
        let json: serde_json::Value = serde_json::from_str(&out.story_messages[0].1).unwrap();
        assert_eq!(json["root_cause"]["span"]["service"], "cart");
        assert_eq!(json["kind"], "error");
        assert_eq!(
            json["fingerprint"],
            serde_json::Value::String(out.stories[0].fingerprint.to_string())
        );
        assert_eq!(out.edges.len(), 1);
        let e = &out.edges[0];
        assert_eq!(
            (
                e.parent_service.as_str(),
                e.child_service.as_str(),
                e.calls,
                e.errors
            ),
            ("frontend", "cart", 2, 1)
        );
        assert_eq!(e.minute as u64, base / 1_000_000_000);
        assert_eq!(out.failed, 0);
    }

    #[test]
    fn story_row_serializes_nested_json() {
        let traces = vec![closed(
            "t2",
            vec![
                span("a", "", "frontend", 0, 100, false),
                span("b", "a", "cart", 10, 50, true),
            ],
        )];
        let row = &process(&traces, &HashMap::new(), &Thresholds::default()).stories[0];
        let cp: serde_json::Value = serde_json::from_str(&row.critical_path).unwrap();
        assert!(cp["segments"].is_array() && cp["top"].is_array());
        assert_eq!(row.baseline_diff, "");
        assert_eq!(
            row.path_services,
            vec!["frontend".to_string(), "cart".to_string()]
        );
        assert_eq!(row.rc_span_kind, "server");
    }
}
