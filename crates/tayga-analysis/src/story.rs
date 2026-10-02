//! One closed trace → summary, edges and (for failing or slow traces) a story (spec §9.5–§9.6).

use crate::baseline::{Baseline, BaselineDiff, Thresholds, diff, is_slow};
use crate::critical_path::{CriticalPath, critical_path};
use crate::fingerprint::{fingerprint, mask};
use crate::model::{Endpoint, LogRec, SpanKind, StatusCode, TraceBundle};
use crate::rootcause::{error_flags, find_root_cause};
use crate::summary::{EdgeObs, TraceSummary, service_edges, trace_summary};
use crate::tree::{FLAG_INCOMPLETE, SpanTree};
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::HashMap;

pub const MAX_STORY_LOGS: usize = 50;
pub const TOP_CONTRIBUTORS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StoryKind {
    Error,
    Slow,
}

impl StoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Slow => "slow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpanRef {
    pub span_id: String,
    pub service: String,
    pub name: String,
    pub kind: SpanKind,
    pub status: StatusCode,
    pub start_ns: u64,
    pub duration_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RootCauseInfo {
    pub span: SpanRef,
    pub message: String,
    pub exception_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PathSegment {
    pub span_id: String,
    pub service: String,
    pub name: String,
    pub start_ns: u64,
    pub end_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Contributor {
    pub span_id: String,
    pub service: String,
    pub name: String,
    pub self_time_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryLog {
    pub ts_ns: u64,
    pub service: String,
    pub span_id: String,
    pub severity_number: i32,
    pub severity_text: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Story {
    /// The trace id: one story per trace, so replays overwrite instead of duplicating.
    pub story_id: String,
    pub fingerprint: u64,
    pub kind: StoryKind,
    pub ts_ns: u64,
    pub duration_ns: u64,
    pub trace_id: String,
    pub endpoint: Endpoint,
    pub root_cause: RootCauseInfo,
    pub summary: String,
    /// Services along root → root cause, consecutive duplicates collapsed.
    pub path_services: Vec<String>,
    pub path_spans: Vec<SpanRef>,
    pub critical_path: Vec<PathSegment>,
    pub top_contributors: Vec<Contributor>,
    pub baseline_diff: Option<BaselineDiff>,
    /// At most `MAX_STORY_LOGS`, most severe first.
    pub logs: Vec<StoryLog>,
    pub also_failed: Vec<SpanRef>,
    pub span_count: u32,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub summary: TraceSummary,
    pub edges: Vec<EdgeObs>,
    pub story: Option<Story>,
}

struct StoryInput {
    kind: StoryKind,
    cause: usize,
    message: String,
    exception_type: String,
    summary_line: String,
    path: Vec<usize>,
    also_failed: Vec<usize>,
}

/// `None` only for a bundle without spans (e.g. logs only).
pub fn analyze(
    bundle: &TraceBundle,
    baselines: &HashMap<Endpoint, Baseline>,
    t: &Thresholds,
    window_flags: &[String],
) -> Option<Analysis> {
    let tree = SpanTree::build(bundle)?;
    let is_err = error_flags(&tree);
    let root_cause = find_root_cause(&tree, &is_err);
    let summary = trace_summary(&tree, root_cause.is_some());
    let edges = service_edges(&tree, &is_err);
    let baseline = baselines.get(&summary.endpoint).filter(|b| b.trusted(t));
    let story = match root_cause {
        Some(rc) => {
            let cp = critical_path(&tree);
            let input = StoryInput {
                kind: StoryKind::Error,
                cause: rc.span,
                message: rc.message,
                exception_type: rc.exception_type,
                summary_line: rc.summary,
                path: rc.path,
                also_failed: rc.also_failed,
            };
            Some(build_story(
                &tree,
                &summary,
                &cp,
                input,
                baseline,
                t,
                window_flags,
            ))
        }
        None => match baseline {
            Some(b) if is_slow(summary.duration_ns, b, t) => {
                let cp = critical_path(&tree);
                let (cause, self_ns) = cp
                    .self_time
                    .first()
                    .copied()
                    .unwrap_or((tree.root, summary.duration_ns));
                let c = tree.span(cause);
                let summary_line = format!(
                    "{} {} took {} ms (p99 {} ms); most time in {} {} ({} ms on the critical path)",
                    summary.endpoint.service,
                    summary.endpoint.name,
                    ms(summary.duration_ns),
                    ms(b.p99_ns as u64),
                    c.service,
                    c.name,
                    ms(self_ns)
                );
                let input = StoryInput {
                    kind: StoryKind::Slow,
                    cause,
                    message: "slow".to_string(),
                    exception_type: String::new(),
                    summary_line,
                    path: tree.path_to(cause),
                    also_failed: Vec::new(),
                };
                Some(build_story(
                    &tree,
                    &summary,
                    &cp,
                    input,
                    Some(b),
                    t,
                    window_flags,
                ))
            }
            _ => None,
        },
    };
    Some(Analysis {
        summary,
        edges,
        story,
    })
}

fn ms(ns: u64) -> String {
    format!("{:.1}", ns as f64 / 1e6)
}

fn span_ref(tree: &SpanTree, i: usize) -> SpanRef {
    let s = tree.span(i);
    SpanRef {
        span_id: s.span_id.clone(),
        service: s.service.clone(),
        name: s.name.clone(),
        kind: s.kind,
        status: s.status,
        start_ns: s.start_ns,
        duration_ns: s.duration_ns(),
    }
}

fn build_story(
    tree: &SpanTree,
    summary: &TraceSummary,
    cp: &CriticalPath,
    input: StoryInput,
    baseline: Option<&Baseline>,
    t: &Thresholds,
    window_flags: &[String],
) -> Story {
    let cause = tree.span(input.cause);
    let fingerprint = fingerprint(&[
        input.kind.as_str(),
        summary.endpoint.service.as_str(),
        summary.endpoint.name.as_str(),
        cause.service.as_str(),
        cause.name.as_str(),
        mask(&input.message).as_str(),
    ]);
    let mut path_services: Vec<String> = Vec::new();
    for &i in &input.path {
        let service = &tree.span(i).service;
        if path_services.last() != Some(service) {
            path_services.push(service.clone());
        }
    }
    let mut logs: Vec<&LogRec> = tree.bundle.logs.iter().collect();
    logs.sort_by_key(|l| (Reverse(l.severity_number), l.ts_ns));
    let mut flags = window_flags.to_vec();
    if tree.incomplete {
        flags.push(FLAG_INCOMPLETE.to_string());
    }
    Story {
        story_id: summary.trace_id.clone(),
        fingerprint,
        kind: input.kind,
        ts_ns: summary.ts_ns,
        duration_ns: summary.duration_ns,
        trace_id: summary.trace_id.clone(),
        endpoint: summary.endpoint.clone(),
        root_cause: RootCauseInfo {
            span: span_ref(tree, input.cause),
            message: input.message,
            exception_type: input.exception_type,
        },
        summary: input.summary_line,
        path_services,
        path_spans: input.path.iter().map(|&i| span_ref(tree, i)).collect(),
        critical_path: cp
            .segments
            .iter()
            .map(|seg| {
                let s = tree.span(seg.span);
                PathSegment {
                    span_id: s.span_id.clone(),
                    service: s.service.clone(),
                    name: s.name.clone(),
                    start_ns: seg.start_ns,
                    end_ns: seg.end_ns,
                }
            })
            .collect(),
        top_contributors: cp
            .self_time
            .iter()
            .take(TOP_CONTRIBUTORS)
            .map(|&(i, self_time_ns)| {
                let s = tree.span(i);
                Contributor {
                    span_id: s.span_id.clone(),
                    service: s.service.clone(),
                    name: s.name.clone(),
                    self_time_ns,
                }
            })
            .collect(),
        baseline_diff: baseline.map(|b| diff(summary, b, t)),
        logs: logs
            .into_iter()
            .take(MAX_STORY_LOGS)
            .map(|l| StoryLog {
                ts_ns: l.ts_ns,
                service: l.service.clone(),
                span_id: l.span_id.clone(),
                severity_number: l.severity_number,
                severity_text: l.severity_text.clone(),
                body: l.body.clone(),
            })
            .collect(),
        also_failed: input
            .also_failed
            .iter()
            .map(|&i| span_ref(tree, i))
            .collect(),
        span_count: u32::try_from(tree.bundle.spans.len()).unwrap_or(u32::MAX),
        flags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::baseline::OpStats;
    use crate::testutil::{bundle, err, kind, log, span};

    fn checkout_failure(message: &str) -> TraceBundle {
        let mut b = bundle(vec![
            kind(
                span("a", "", "frontend", "POST /api/checkout", 0, 100),
                SpanKind::Server,
            ),
            err(
                kind(
                    span(
                        "b",
                        "a",
                        "checkout",
                        "oteldemo.PaymentService/Charge",
                        10,
                        90,
                    ),
                    SpanKind::Client,
                ),
                "rpc error",
            ),
            err(
                kind(
                    span(
                        "c",
                        "b",
                        "payment",
                        "oteldemo.PaymentService/Charge",
                        20,
                        80,
                    ),
                    SpanKind::Server,
                ),
                message,
            ),
        ]);
        b.logs = vec![
            log("c", 9, "charging card", 25),
            log("c", 17, "charge failed", 70),
        ];
        b
    }

    #[test]
    fn error_trace_produces_error_story() {
        let a = analyze(
            &checkout_failure("Invalid token 1234"),
            &HashMap::new(),
            &Thresholds::default(),
            &[],
        )
        .unwrap();
        assert!(a.summary.is_error);
        assert_eq!(a.edges.len(), 2);
        let s = a.story.unwrap();
        assert_eq!(s.kind, StoryKind::Error);
        assert_eq!(s.story_id, "t1");
        assert_eq!(s.root_cause.span.service, "payment");
        assert_eq!(s.root_cause.message, "Invalid token 1234");
        assert_eq!(s.path_services, vec!["frontend", "checkout", "payment"]);
        assert_eq!(s.path_spans.len(), 3);
        assert_eq!(
            s.summary,
            "payment oteldemo.PaymentService/Charge failed: Invalid token 1234"
        );
        assert_eq!(s.logs[0].body, "charge failed");
        assert_eq!(s.logs.len(), 2);
        assert!(s.baseline_diff.is_none());
        assert_eq!(s.span_count, 3);
        assert_eq!(s.endpoint.name, "POST /api/checkout");
        let total: u64 = s.critical_path.iter().map(|p| p.end_ns - p.start_ns).sum();
        assert_eq!(total, 100);
        assert!(s.top_contributors.len() <= TOP_CONTRIBUTORS);
    }

    #[test]
    fn fingerprint_ignores_variable_numbers() {
        let t = Thresholds::default();
        let a = analyze(
            &checkout_failure("Invalid token 1234"),
            &HashMap::new(),
            &t,
            &[],
        )
        .unwrap();
        let b = analyze(
            &checkout_failure("Invalid token 98765"),
            &HashMap::new(),
            &t,
            &[],
        )
        .unwrap();
        assert_eq!(a.story.unwrap().fingerprint, b.story.unwrap().fingerprint);
    }

    #[test]
    fn flags_combine_window_and_tree() {
        let mut b = checkout_failure("x");
        b.spans.push(span("z", "missing", "ad", "GetAds", 5, 6));
        let a = analyze(
            &b,
            &HashMap::new(),
            &Thresholds::default(),
            &["truncated".to_string()],
        )
        .unwrap();
        assert_eq!(
            a.story.unwrap().flags,
            vec!["truncated".to_string(), "incomplete".to_string()]
        );
    }

    #[test]
    fn slow_trace_with_trusted_baseline_blames_biggest_self_time() {
        let ms = 1_000_000;
        let b = bundle(vec![
            span("r", "", "frontend", "GET /", 0, 1_000 * ms),
            span("c", "r", "cart", "GetCart", 100 * ms, 900 * ms),
        ]);
        let baseline = Baseline {
            traces: 100,
            p50_ns: 20.0 * ms as f64,
            p95_ns: 50.0 * ms as f64,
            p99_ns: 100.0 * ms as f64,
            ops: HashMap::from([(
                "frontend:GET /".to_string(),
                OpStats {
                    presence: 1.0,
                    p95_ns: 50.0 * ms as f64,
                },
            )]),
        };
        let endpoint = Endpoint {
            service: "frontend".into(),
            name: "GET /".into(),
        };
        let map = HashMap::from([(endpoint, baseline)]);
        let a = analyze(&b, &map, &Thresholds::default(), &[]).unwrap();
        let s = a.story.unwrap();
        assert_eq!(s.kind, StoryKind::Slow);
        assert_eq!(s.root_cause.span.service, "cart");
        assert_eq!(s.root_cause.message, "slow");
        assert_eq!(s.path_services, vec!["frontend", "cart"]);
        assert!(s.summary.starts_with(
            "frontend GET / took 1000.0 ms (p99 100.0 ms); most time in cart GetCart (800.0 ms"
        ));
        let d = s.baseline_diff.unwrap();
        assert_eq!(d.new_ops, vec!["cart:GetCart".to_string()]);
        assert_eq!(d.slower_ops[0].op, "frontend:GET /");
    }

    #[test]
    fn healthy_trace_without_baseline_has_no_story() {
        let b = bundle(vec![
            span("r", "", "frontend", "GET /", 0, 10),
            span("c", "r", "cart", "GetCart", 1, 5),
        ]);
        let a = analyze(&b, &HashMap::new(), &Thresholds::default(), &[]).unwrap();
        assert!(a.story.is_none());
        assert!(!a.summary.is_error);
        assert_eq!(a.edges.len(), 1);
    }

    #[test]
    fn logs_only_bundle_is_not_analysed() {
        let mut b = TraceBundle::new("t");
        b.logs.push(log("", 17, "x", 1));
        assert!(analyze(&b, &HashMap::new(), &Thresholds::default(), &[]).is_none());
    }

    #[test]
    fn story_logs_are_capped() {
        let mut b = checkout_failure("x");
        b.logs = (0..80).map(|i| log("c", 9, "info", i)).collect();
        let s = analyze(&b, &HashMap::new(), &Thresholds::default(), &[])
            .unwrap()
            .story
            .unwrap();
        assert_eq!(s.logs.len(), MAX_STORY_LOGS);
    }
}
