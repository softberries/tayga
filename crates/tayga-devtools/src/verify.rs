//! Compares per-trace span counts in ClickHouse with the demo's Jaeger.

use std::collections::HashSet;

/// Distinct span ids across all traces in a Jaeger `/api/traces/{id}` response.
pub fn jaeger_span_count(resp: &serde_json::Value) -> usize {
    let ids: HashSet<&str> = resp["data"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|t| t["spans"].as_array().into_iter().flatten())
        .filter_map(|s| s["spanID"].as_str())
        .collect();
    ids.len()
}

/// Samples traces older than 60 s (so both stores are complete) and compares span counts.
/// Returns the mismatches as (trace_id, tayga, jaeger).
pub async fn verify_raw(
    clickhouse_url: &str,
    jaeger_url: &str,
    samples: u32,
) -> anyhow::Result<(usize, Vec<(String, u64, usize)>)> {
    let ch = clickhouse::Client::default().with_url(clickhouse_url).with_database("tayga");
    let traces: Vec<(String, u64)> = ch
        .query(
            "SELECT trace_id, uniqExact(span_id) FROM spans \
             WHERE trace_id != '' AND start_ts BETWEEN now() - INTERVAL 10 MINUTE AND now() - INTERVAL 60 SECOND \
             GROUP BY trace_id HAVING max(start_ts) < now() - INTERVAL 60 SECOND \
             ORDER BY cityHash64(trace_id) LIMIT ?",
        )
        .bind(samples)
        .fetch_all()
        .await?;
    let http = reqwest::Client::new();
    let mut mismatches = Vec::new();
    for (trace_id, ours) in &traces {
        let resp: serde_json::Value =
            http.get(format!("{jaeger_url}/api/traces/{trace_id}")).send().await?.json().await?;
        let theirs = jaeger_span_count(&resp);
        if *ours as usize != theirs {
            mismatches.push((trace_id.clone(), *ours, theirs));
        }
    }
    Ok((traces.len(), mismatches))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_distinct_span_ids() {
        let v = serde_json::json!({"data":[{"traceID":"t","spans":[{"spanID":"a"},{"spanID":"b"},{"spanID":"a"}]}]});
        assert_eq!(jaeger_span_count(&v), 2);
    }

    #[test]
    fn missing_data_is_zero() {
        assert_eq!(jaeger_span_count(&serde_json::json!({"errors":[{"code":404}]})), 0);
    }
}
