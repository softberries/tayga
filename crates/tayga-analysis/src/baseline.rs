//! "Normal shape" of an endpoint and how one trace differs from it (spec §9.4).

use crate::model::Endpoint;
use crate::summary::TraceSummary;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const MS: f64 = 1_000_000.0;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub min_baseline_traces: u64,
    pub new_op_presence: f64,
    pub missing_op_presence: f64,
    pub slower_op_factor: f64,
    pub slower_op_margin_ms: f64,
    pub slow_trace_factor: f64,
    pub slow_trace_margin_ms: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            min_baseline_traces: 50,
            new_op_presence: 0.01,
            missing_op_presence: 0.95,
            slower_op_factor: 2.0,
            slower_op_margin_ms: 50.0,
            slow_trace_factor: 1.5,
            slow_trace_margin_ms: 100.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OpStats {
    /// Share of baseline traces containing the op.
    pub presence: f64,
    pub p95_ns: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Baseline {
    pub traces: u64,
    pub p50_ns: f64,
    pub p95_ns: f64,
    pub p99_ns: f64,
    pub ops: HashMap<String, OpStats>,
}

impl Baseline {
    pub fn trusted(&self, t: &Thresholds) -> bool {
        self.traces >= t.min_baseline_traces
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct BaselineDiff {
    pub new_ops: Vec<String>,
    pub missing_ops: Vec<String>,
    pub slower_ops: Vec<SlowerOp>,
}

impl BaselineDiff {
    pub fn is_empty(&self) -> bool {
        self.new_ops.is_empty() && self.missing_ops.is_empty() && self.slower_ops.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SlowerOp {
    pub op: String,
    pub duration_ns: u64,
    pub baseline_p95_ns: u64,
}

pub fn diff(summary: &TraceSummary, b: &Baseline, t: &Thresholds) -> BaselineDiff {
    let present: HashSet<&str> = summary
        .op_durations
        .iter()
        .map(|(op, _)| op.as_str())
        .collect();
    let mut d = BaselineDiff::default();
    for (op, duration_ns) in &summary.op_durations {
        match b.ops.get(op) {
            Some(s) if s.presence >= t.new_op_presence => {
                let limit =
                    (s.p95_ns * t.slower_op_factor).max(s.p95_ns + t.slower_op_margin_ms * MS);
                if *duration_ns as f64 > limit {
                    d.slower_ops.push(SlowerOp {
                        op: op.clone(),
                        duration_ns: *duration_ns,
                        baseline_p95_ns: s.p95_ns as u64,
                    });
                }
            }
            _ => d.new_ops.push(op.clone()),
        }
    }
    d.missing_ops = b
        .ops
        .iter()
        .filter(|(op, s)| s.presence > t.missing_op_presence && !present.contains(op.as_str()))
        .map(|(op, _)| op.clone())
        .collect();
    d.missing_ops.sort();
    d
}

pub fn is_slow(duration_ns: u64, b: &Baseline, t: &Thresholds) -> bool {
    let limit = (b.p99_ns * t.slow_trace_factor).max(b.p99_ns + t.slow_trace_margin_ms * MS);
    b.trusted(t) && duration_ns as f64 > limit
}

/// Same aggregation as the ClickHouse baseline query, for tests and fixtures (non-error traces only).
pub fn baselines_from_summaries(summaries: &[TraceSummary]) -> HashMap<Endpoint, Baseline> {
    let mut groups: HashMap<&Endpoint, Vec<&TraceSummary>> = HashMap::new();
    for s in summaries.iter().filter(|s| !s.is_error) {
        groups.entry(&s.endpoint).or_default().push(s);
    }
    groups
        .into_iter()
        .map(|(endpoint, list)| {
            let mut durations: Vec<u64> = list.iter().map(|s| s.duration_ns).collect();
            durations.sort_unstable();
            let mut per_op: HashMap<&str, Vec<u64>> = HashMap::new();
            for s in &list {
                for (op, d) in &s.op_durations {
                    per_op.entry(op.as_str()).or_default().push(*d);
                }
            }
            let n = list.len() as f64;
            let ops = per_op
                .into_iter()
                .map(|(op, mut v)| {
                    v.sort_unstable();
                    (
                        op.to_string(),
                        OpStats {
                            presence: v.len() as f64 / n,
                            p95_ns: quantile(&v, 0.95),
                        },
                    )
                })
                .collect();
            let baseline = Baseline {
                traces: list.len() as u64,
                p50_ns: quantile(&durations, 0.5),
                p95_ns: quantile(&durations, 0.95),
                p99_ns: quantile(&durations, 0.99),
                ops,
            };
            (endpoint.clone(), baseline)
        })
        .collect()
}

/// Nearest-rank quantile of a sorted, non-empty slice.
fn quantile(sorted: &[u64], q: f64) -> f64 {
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1] as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep() -> Endpoint {
        Endpoint {
            service: "frontend".into(),
            name: "GET /".into(),
        }
    }

    fn summary(duration_ms: u64, ops: &[(&str, u64)], is_error: bool) -> TraceSummary {
        TraceSummary {
            trace_id: "t".into(),
            ts_ns: 0,
            endpoint: ep(),
            duration_ns: duration_ms * 1_000_000,
            is_error,
            op_durations: ops
                .iter()
                .map(|(o, d)| (o.to_string(), d * 1_000_000))
                .collect(),
            span_count: 1,
        }
    }

    fn baseline() -> Baseline {
        Baseline {
            traces: 100,
            p50_ns: 40.0 * MS,
            p95_ns: 80.0 * MS,
            p99_ns: 100.0 * MS,
            ops: HashMap::from([
                (
                    "frontend:GET".to_string(),
                    OpStats {
                        presence: 1.0,
                        p95_ns: 80.0 * MS,
                    },
                ),
                (
                    "cart:GetCart".to_string(),
                    OpStats {
                        presence: 0.99,
                        p95_ns: 10.0 * MS,
                    },
                ),
                (
                    "ad:GetAds".to_string(),
                    OpStats {
                        presence: 0.005,
                        p95_ns: 5.0 * MS,
                    },
                ),
            ]),
        }
    }

    #[test]
    fn diff_finds_new_missing_and_slower_ops() {
        let s = summary(
            200,
            &[("frontend:GET", 200), ("ad:GetAds", 1), ("email:Send", 3)],
            false,
        );
        let d = diff(&s, &baseline(), &Thresholds::default());
        assert_eq!(
            d.new_ops,
            vec!["ad:GetAds".to_string(), "email:Send".to_string()]
        );
        assert_eq!(d.missing_ops, vec!["cart:GetCart".to_string()]);
        assert_eq!(
            d.slower_ops,
            vec![SlowerOp {
                op: "frontend:GET".into(),
                duration_ns: 200_000_000,
                baseline_p95_ns: 80_000_000
            }]
        );
    }

    #[test]
    fn slower_needs_both_factor_and_margin() {
        // p95 10 ms: 2× = 20 ms, +50 ms = 60 ms → limit 60 ms.
        let d = diff(
            &summary(50, &[("cart:GetCart", 50)], false),
            &baseline(),
            &Thresholds::default(),
        );
        assert!(d.slower_ops.is_empty());
        let d = diff(
            &summary(70, &[("cart:GetCart", 70)], false),
            &baseline(),
            &Thresholds::default(),
        );
        assert_eq!(d.slower_ops.len(), 1);
    }

    #[test]
    fn slow_trace_rule_and_trust() {
        let t = Thresholds::default();
        let b = baseline();
        // p99 100 ms: ×1.5 = 150 ms, +100 ms = 200 ms → limit 200 ms.
        assert!(!is_slow(190 * 1_000_000, &b, &t));
        assert!(is_slow(201 * 1_000_000, &b, &t));
        let untrusted = Baseline { traces: 10, ..b };
        assert!(!is_slow(10_000 * 1_000_000, &untrusted, &t));
    }

    #[test]
    fn baselines_from_summaries_skip_errors_and_compute_presence() {
        let mut list: Vec<TraceSummary> = (1..=10)
            .map(|i| summary(i * 10, &[("frontend:GET", i * 10)], false))
            .collect();
        list[0]
            .op_durations
            .push(("cart:GetCart".into(), 5_000_000));
        list.push(summary(10_000, &[("frontend:GET", 10_000)], true));
        let map = baselines_from_summaries(&list);
        let b = &map[&ep()];
        assert_eq!(b.traces, 10);
        assert_eq!(b.p50_ns, 50.0 * MS);
        assert_eq!(b.p99_ns, 100.0 * MS);
        assert_eq!(b.ops["frontend:GET"].presence, 1.0);
        assert_eq!(b.ops["cart:GetCart"].presence, 0.1);
    }
}
