//! Baselines aggregated by ClickHouse from `trace_summaries` (spec §9.4).

use std::collections::HashMap;
use tayga_analysis::baseline::{Baseline, OpStats};
use tayga_analysis::model::Endpoint;
use tayga_store::rows::{EndpointStatsRow, OpStatsRow};
use tayga_store::store::Store;

pub fn build(
    endpoints: Vec<EndpointStatsRow>,
    ops: Vec<OpStatsRow>,
) -> HashMap<Endpoint, Baseline> {
    let mut map: HashMap<Endpoint, Baseline> = endpoints
        .into_iter()
        .map(|r| {
            let endpoint = Endpoint {
                service: r.endpoint_service,
                name: r.endpoint_name,
            };
            let baseline = Baseline {
                traces: r.traces,
                p50_ns: r.p50,
                p95_ns: r.p95,
                p99_ns: r.p99,
                ops: HashMap::new(),
            };
            (endpoint, baseline)
        })
        .collect();
    for r in ops {
        let endpoint = Endpoint {
            service: r.endpoint_service,
            name: r.endpoint_name,
        };
        if let Some(b) = map.get_mut(&endpoint)
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
    map
}

pub async fn load(
    store: &Store,
    window_minutes: u32,
) -> anyhow::Result<HashMap<Endpoint, Baseline>> {
    let (endpoints, ops) = tokio::try_join!(
        store.endpoint_stats(window_minutes),
        store.op_stats(window_minutes)
    )?;
    Ok(build(endpoints, ops))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_is_share_of_endpoint_traces() {
        let endpoints = vec![EndpointStatsRow {
            endpoint_service: "frontend".into(),
            endpoint_name: "GET /".into(),
            traces: 200,
            p50: 1.0,
            p95: 2.0,
            p99: 3.0,
        }];
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
        let map = build(endpoints, ops);
        assert_eq!(map.len(), 1);
        let b = &map[&Endpoint {
            service: "frontend".into(),
            name: "GET /".into(),
        }];
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
}
