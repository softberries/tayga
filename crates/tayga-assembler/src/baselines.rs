//! Baselines aggregated by ClickHouse from `trace_summaries` (spec §9.4).

use std::collections::HashMap;
use tayga_analysis::baseline::{Baseline, OpStats, Thresholds, slow_limit_ns};
use tayga_analysis::model::Endpoint;
use tayga_store::rows::{EndpointStatsRow, OpStatsRow};
use tayga_store::store::{EndpointCaps, Store};

/// A previous baseline is carried through at most this many baseline windows.
pub const MAX_CARRY_WINDOWS: u64 = 2;

/// One baseline refresh: the baselines, the endpoints carried over (with the time in seconds
/// the carry started) and the traces the duration caps excluded.
#[derive(Debug, Default, PartialEq)]
pub struct Refresh {
    pub baselines: HashMap<Endpoint, Baseline>,
    pub carried: HashMap<Endpoint, u64>,
    pub excluded: u64,
}

/// Folds the query rows into baselines. An endpoint that had a trusted previous baseline but
/// now has fewer than `min_baseline_traces` kept traces (a sustained slowdown: its traces are
/// slow-storied or above the cap) keeps the previous baseline unchanged, for at most
/// `MAX_CARRY_WINDOWS` windows from the first carry; after that the new data is accepted. An
/// endpoint with no traces in the window at all is dropped. `carried_since` is the previous
/// refresh's `Refresh::carried`; `now_secs` is any monotonic clock in seconds.
pub fn build(
    endpoints: Vec<EndpointStatsRow>,
    ops: Vec<OpStatsRow>,
    previous: &HashMap<Endpoint, Baseline>,
    carried_since: &HashMap<Endpoint, u64>,
    now_secs: u64,
    window_minutes: u32,
    t: &Thresholds,
) -> Refresh {
    let max_carry_secs = MAX_CARRY_WINDOWS * u64::from(window_minutes) * 60;
    let mut out = Refresh::default();
    for r in endpoints {
        out.excluded += r.excluded;
        if r.seen == 0 {
            continue;
        }
        let endpoint = Endpoint {
            service: r.endpoint_service,
            name: r.endpoint_name,
        };
        if r.kept < t.min_baseline_traces
            && let Some(prev) = previous.get(&endpoint).filter(|b| b.trusted(t))
        {
            let since = carried_since.get(&endpoint).copied().unwrap_or(now_secs);
            if now_secs.saturating_sub(since) < max_carry_secs {
                out.baselines.insert(endpoint.clone(), prev.clone());
                out.carried.insert(endpoint, since);
                continue;
            }
        }
        // No kept traces (or NaN quantiles from none) means no baseline: a 0-trace baseline
        // would later turn into a cap of 0 and lock the endpoint out. The next refresh then
        // learns it through the 10 x p50 bootstrap.
        if r.kept == 0 || ![r.p50, r.p95, r.p99].iter().all(|q| q.is_finite()) {
            continue;
        }
        let baseline = Baseline {
            traces: r.kept,
            p50_ns: r.p50,
            p95_ns: r.p95,
            p99_ns: r.p99,
            ops: HashMap::new(),
        };
        out.baselines.insert(endpoint, baseline);
    }
    for r in ops {
        let endpoint = Endpoint {
            service: r.endpoint_service,
            name: r.endpoint_name,
        };
        if out.carried.contains_key(&endpoint) {
            continue;
        }
        if let Some(b) = out.baselines.get_mut(&endpoint)
            && b.traces > 0
        {
            b.ops.insert(
                r.op,
                OpStats {
                    presence: r.present as f64 / b.traces as f64,
                    p95_ns: r.p95,
                },
            );
        }
    }
    out
}

/// Duration caps from the previous refresh: each endpoint's slow limit, from **trusted** previous
/// baselines only. An untrusted baseline (fewer than `min_baseline_traces`, e.g. the few fast
/// survivors left when a carry expires mid-slowdown) does not cap: capping at its old fast
/// limit would exclude the slow majority again, keep the endpoint untrusted and never adopt the
/// new level. Endpoints without a trusted previous baseline get no cap here and fall back to the
/// query's 10 x p50 bootstrap cap, which still drops single outliers.
pub fn caps_from(previous: &HashMap<Endpoint, Baseline>, t: &Thresholds) -> EndpointCaps {
    let mut caps = EndpointCaps::default();
    for (endpoint, b) in previous {
        if !b.trusted(t) {
            continue;
        }
        let limit = slow_limit_ns(b, t);
        // A NaN or non-positive limit would cap everything at 0; leave it to the bootstrap.
        if !limit.is_finite() || limit <= 0.0 {
            continue;
        }
        caps.keys
            .push(format!("{}\0{}", endpoint.service, endpoint.name));
        caps.caps_ns.push(limit.ceil() as u64);
    }
    caps
}

/// Loads the baselines, excluding traces above the previous limit.
pub async fn load(
    store: &Store,
    window_minutes: u32,
    previous: &HashMap<Endpoint, Baseline>,
    carried_since: &HashMap<Endpoint, u64>,
    now_secs: u64,
    t: &Thresholds,
) -> anyhow::Result<Refresh> {
    let caps = caps_from(previous, t);
    let (endpoints, ops) = tokio::try_join!(
        store.endpoint_stats(window_minutes, &caps),
        store.op_stats(window_minutes, &caps),
    )?;
    Ok(build(
        endpoints,
        ops,
        previous,
        carried_since,
        now_secs,
        window_minutes,
        t,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: u32 = 60;
    const WINDOW_SECS: u64 = 3600;

    fn ep() -> Endpoint {
        Endpoint {
            service: "frontend".into(),
            name: "GET /".into(),
        }
    }

    fn row(seen: u64, kept: u64, p99: f64) -> EndpointStatsRow {
        EndpointStatsRow {
            endpoint_service: "frontend".into(),
            endpoint_name: "GET /".into(),
            seen,
            kept,
            excluded: 0,
            p50: 1.0,
            p95: 2.0,
            p99,
        }
    }

    fn prev(traces: u64, p99: f64) -> HashMap<Endpoint, Baseline> {
        HashMap::from([(
            ep(),
            Baseline {
                traces,
                p99_ns: p99,
                ..Baseline::default()
            },
        )])
    }

    fn run(
        rows: Vec<EndpointStatsRow>,
        previous: &HashMap<Endpoint, Baseline>,
        carried: &HashMap<Endpoint, u64>,
        now: u64,
    ) -> Refresh {
        build(
            rows,
            vec![],
            previous,
            carried,
            now,
            WINDOW,
            &Thresholds::default(),
        )
    }

    #[test]
    fn presence_is_share_of_endpoint_traces() {
        let ops = vec![
            OpStatsRow {
                endpoint_service: "frontend".into(),
                endpoint_name: "GET /".into(),
                op: "cart:Get".into(),
                present: 50,
                p95: 7.0,
            },
            OpStatsRow {
                endpoint_service: "other".into(),
                endpoint_name: "x".into(),
                op: "y".into(),
                present: 1,
                p95: 1.0,
            },
        ];
        let r = build(
            vec![row(200, 200, 3.0)],
            ops,
            &HashMap::new(),
            &HashMap::new(),
            0,
            WINDOW,
            &Thresholds::default(),
        );
        assert_eq!(r.baselines.len(), 1);
        let b = &r.baselines[&ep()];
        assert_eq!(b.traces, 200);
        assert_eq!(b.p99_ns, 3.0);
        assert_eq!(
            b.ops["cart:Get"],
            OpStats {
                presence: 0.25,
                p95_ns: 7.0
            }
        );
    }

    #[test]
    fn fully_excluded_endpoint_keeps_its_trusted_previous_baseline() {
        let previous = prev(100, 100.0);
        let r = run(vec![row(100, 0, f64::NAN)], &previous, &HashMap::new(), 10);
        assert_eq!(r.baselines, previous);
        assert_eq!(r.carried, HashMap::from([(ep(), 10)]));
    }

    #[test]
    fn partially_kept_endpoint_is_carried_only_with_a_trusted_previous() {
        let previous = prev(100, 100.0);
        let r = run(vec![row(100, 30, 9.0)], &previous, &HashMap::new(), 0);
        assert_eq!(r.baselines, previous);
        let r = run(vec![row(100, 30, 9.0)], &HashMap::new(), &HashMap::new(), 0);
        assert_eq!(r.baselines[&ep()].traces, 30);
        assert_eq!(r.baselines[&ep()].p99_ns, 9.0);
        assert!(r.carried.is_empty());
        let r = run(
            vec![row(100, 30, 9.0)],
            &prev(10, 100.0),
            &HashMap::new(),
            0,
        );
        assert_eq!(
            r.baselines[&ep()].traces,
            30,
            "untrusted previous is not carried"
        );
    }

    #[test]
    fn endpoint_absent_from_the_rows_is_dropped() {
        let r = run(vec![], &prev(100, 100.0), &HashMap::new(), 0);
        assert!(r.baselines.is_empty());
        let r = run(vec![row(0, 0, 0.0)], &prev(100, 100.0), &HashMap::new(), 0);
        assert!(r.baselines.is_empty());
    }

    #[test]
    fn enough_kept_traces_replace_the_carried_baseline() {
        let previous = prev(100, 100.0);
        let carried = HashMap::from([(ep(), 0)]);
        let r = run(vec![row(100, 50, 9.0)], &previous, &carried, 100);
        assert_eq!(r.baselines[&ep()].traces, 50);
        assert_eq!(r.baselines[&ep()].p99_ns, 9.0);
        assert!(r.carried.is_empty());
    }

    #[test]
    fn carry_stops_after_two_windows() {
        let previous = prev(100, 100.0);
        let carried = HashMap::from([(ep(), 0)]);
        let r = run(
            vec![row(100, 0, f64::NAN)],
            &previous,
            &carried,
            2 * WINDOW_SECS - 1,
        );
        assert_eq!(r.baselines, previous);
        let r = run(
            vec![row(100, 0, f64::NAN)],
            &previous,
            &carried,
            2 * WINDOW_SECS,
        );
        assert!(r.baselines.is_empty(), "no kept traces, so no baseline");
        assert!(r.carried.is_empty());
        // Next refresh: no previous baseline, so no cap and the bootstrap applies.
        assert_eq!(
            caps_from(&r.baselines, &Thresholds::default()),
            EndpointCaps::default()
        );
    }

    #[test]
    fn original_baseline_survives_two_consecutive_refreshes() {
        let original = prev(100, 100.0);
        let slow = || vec![row(100, 0, f64::NAN)];
        let first = run(slow(), &original, &HashMap::new(), 0);
        let second = run(slow(), &first.baselines, &first.carried, 60);
        assert_eq!(second.baselines, original);
        assert_eq!(
            second.carried,
            HashMap::from([(ep(), 0)]),
            "carry start kept"
        );
    }

    #[test]
    fn restart_shaped_fold_with_no_kept_traces_inserts_no_baseline() {
        let r = run(
            vec![row(100, 0, f64::NAN)],
            &HashMap::new(),
            &HashMap::new(),
            0,
        );
        assert!(r.baselines.is_empty());
        assert!(r.carried.is_empty());
        let mut nan = row(100, 60, f64::NAN);
        nan.p50 = f64::NAN;
        let r = run(vec![nan], &HashMap::new(), &HashMap::new(), 0);
        assert!(
            r.baselines.is_empty(),
            "non-finite quantiles insert nothing"
        );
    }

    #[test]
    fn caps_skip_baselines_without_a_finite_positive_limit() {
        let mut previous = prev(0, f64::NAN);
        assert_eq!(
            caps_from(&previous, &Thresholds::default()),
            EndpointCaps::default()
        );
        previous = prev(100, 0.0);
        let t = Thresholds {
            slow_trace_margin_ms: 0.0,
            ..Thresholds::default()
        };
        assert_eq!(caps_from(&previous, &t), EndpointCaps::default());
    }

    #[test]
    fn excluded_traces_are_summed() {
        let mut a = row(100, 90, 1.0);
        a.excluded = 10;
        let mut b = row(50, 50, 1.0);
        b.endpoint_name = "other".into();
        b.excluded = 2;
        let r = run(vec![a, b], &HashMap::new(), &HashMap::new(), 0);
        assert_eq!(r.excluded, 12);
    }

    #[test]
    fn caps_skip_untrusted_baselines() {
        let t = Thresholds::default();
        let below = t.min_baseline_traces - 1;
        assert_eq!(
            caps_from(&prev(below, 100_000_000.0), &t),
            EndpointCaps::default(),
            "an untrusted baseline leaves the endpoint to the bootstrap"
        );
        assert_eq!(
            caps_from(&prev(t.min_baseline_traces, 100_000_000.0), &t)
                .keys
                .len(),
            1
        );
    }

    #[test]
    fn after_a_carry_expires_with_few_kept_the_next_refresh_adopts_the_new_level() {
        let t = Thresholds::default();
        let fast = prev(600, 100.0);
        let carried = HashMap::from([(ep(), 0)]);
        // Carry expired: 30 fast survivors (0 < kept < min) become an untrusted baseline.
        let mut few = row(600, 30, 100.0);
        few.excluded = 570;
        let expired = run(vec![few], &fast, &carried, 2 * WINDOW_SECS);
        let b = &expired.baselines[&ep()];
        assert_eq!(b.traces, 30);
        assert!(!b.trusted(&t));
        assert!(expired.carried.is_empty());
        // Next refresh: no cap from the untrusted baseline, so the query's 10 x p50 bootstrap
        // keeps the (now unstoried) slow majority and the endpoint learns the slow level.
        assert_eq!(caps_from(&expired.baselines, &t), EndpointCaps::default());
        let slow = row(600, 590, 5_000.0);
        let next = run(
            vec![slow],
            &expired.baselines,
            &expired.carried,
            2 * WINDOW_SECS + 60,
        );
        let b = &next.baselines[&ep()];
        assert_eq!((b.traces, b.p99_ns), (590, 5_000.0));
        assert!(b.trusted(&t));
        assert!(next.carried.is_empty());
    }

    #[test]
    fn caps_use_the_slow_limit_of_each_previous_baseline() {
        let caps = caps_from(&prev(100, 100_000_000.0), &Thresholds::default());
        assert_eq!(caps.keys, vec!["frontend\0GET /".to_string()]);
        assert_eq!(caps.caps_ns, vec![200_000_000]);
        assert_eq!(
            caps_from(&HashMap::new(), &Thresholds::default()),
            EndpointCaps::default()
        );
    }
}
