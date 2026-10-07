use crate::migrate::ClickHouseSettings;
use crate::rows::{EndpointStatsRow, LogRow, OpStatsRow, SpanRow};
use clickhouse::{Client, RowOwned, RowWrite};

/// Slow stories are looked up over the baseline window plus this slack, so a trace near the
/// window edge whose story landed slightly earlier is still excluded.
pub const SLOW_STORY_LOOKBACK_SLACK_MIN: u32 = 10;

#[derive(Clone)]
pub struct Store {
    client: Client,
}

impl Store {
    pub fn new(s: &ClickHouseSettings) -> Self {
        Self {
            client: Client::default()
                .with_url(&s.url)
                .with_database(&s.database),
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The same store with a ClickHouse setting sent on every query (e.g. `max_execution_time`).
    pub fn with_setting(self, name: &str, value: &str) -> Self {
        Self {
            client: self.client.with_setting(name, value),
        }
    }

    pub async fn insert_rows<T>(&self, table: &str, rows: &[T]) -> clickhouse::error::Result<()>
    where
        T: RowOwned + RowWrite,
    {
        if rows.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert::<T>(table).await?;
        for row in rows {
            insert.write(row).await?;
        }
        insert.end().await
    }

    pub async fn insert_spans(&self, rows: &[SpanRow]) -> clickhouse::error::Result<()> {
        self.insert_rows("spans", rows).await
    }

    pub async fn insert_logs(&self, rows: &[LogRow]) -> clickhouse::error::Result<()> {
        self.insert_rows("logs", rows).await
    }

    /// Per endpoint over the non-error window traces: `seen` (all), `kept` (no slow story and
    /// at or below the cap), `excluded` (no slow story, above the cap), and the root-duration
    /// quantiles over the kept traces. An endpoint whose traces are all excluded still gets a
    /// row (`kept = 0`), so the caller can tell a slowdown from a vanished endpoint.
    pub async fn endpoint_stats(
        &self,
        window_minutes: u32,
        caps: &EndpointCaps,
    ) -> clickhouse::error::Result<Vec<EndpointStatsRow>> {
        let sql = format!(
            "{} \
             SELECT endpoint_service, endpoint_name, count() AS seen, \
             countIf(is_kept) AS kept, countIf(is_capped) AS excluded, \
             quantileIf(0.5)(duration_ns, is_kept) AS p50, \
             quantileIf(0.95)(duration_ns, is_kept) AS p95, \
             quantileIf(0.99)(duration_ns, is_kept) AS p99 \
             FROM ({}) GROUP BY endpoint_service, endpoint_name",
            baseline_with(false),
            baseline_candidates(false)
        );
        bind_baseline(self.client.query(&sql), window_minutes, caps)
            .fetch_all()
            .await
    }

    /// Per endpoint and op: kept traces containing the op and its p95 duration. Uses the same
    /// kept set as `endpoint_stats`, so presence ratios stay within 0..=1.
    pub async fn op_stats(
        &self,
        window_minutes: u32,
        caps: &EndpointCaps,
    ) -> clickhouse::error::Result<Vec<OpStatsRow>> {
        let sql = format!(
            "{} \
             SELECT endpoint_service, endpoint_name, op, count() AS present, quantile(0.95)(d) AS p95 \
             FROM (SELECT endpoint_service, endpoint_name, op_durations \
             FROM ({}) WHERE is_kept) \
             ARRAY JOIN mapKeys(op_durations) AS op, mapValues(op_durations) AS d \
             GROUP BY endpoint_service, endpoint_name, op",
            baseline_with(true),
            baseline_candidates(true)
        );
        bind_baseline(self.client.query(&sql), window_minutes, caps)
            .fetch_all()
            .await
    }
}

/// Per-endpoint duration caps in ns, as parallel arrays bound to the baseline queries. A key is
/// `service\0name`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EndpointCaps {
    pub keys: Vec<String>,
    pub caps_ns: Vec<u64>,
}

/// The `WITH` clause of the baseline queries. Bound parameters, in order: cap keys, cap values,
/// window, slow-story lookback.
///
/// `latest` is the newest version of every trace of the window: `argMax` by `span_count`, the
/// `ReplacingMergeTree` version, over the rows the `ts` filter keeps. `FINAL` read the whole
/// table instead (`ORDER BY trace_id`, so the filter prunes nothing): 8.8 M rows and 478 MiB per
/// `endpoint_stats` against 0.39 M rows and 21 MiB (`docs/perf/sp4-performance.md`, ClickHouse). `with_ops`
/// carries `op_durations`, which only `op_stats` reads.
///
/// Two edges differ from `FINAL` (spec §3.8):
/// - on a `span_count` tie `argMax` takes any of the tied rows, `FINAL` the last inserted; tied
///   rows are replays of the same trace, so they are equal;
/// - the `ts` filter runs before the dedup, so a trace whose newer version lies outside the
///   window is still counted, by its version inside it (`FINAL` dropped it).
fn baseline_with(with_ops: bool) -> String {
    let (ops_in, ops_out) = if with_ops {
        (", op_durations", ", tupleElement(v, 5) AS op_durations")
    } else {
        ("", "")
    };
    format!(
        "WITH CAST(? AS Array(String)) AS cap_keys, \
         CAST(? AS Array(UInt64)) AS cap_vals, \
         latest AS (SELECT trace_id, tupleElement(v, 1) AS endpoint_service, \
         tupleElement(v, 2) AS endpoint_name, tupleElement(v, 3) AS duration_ns, \
         tupleElement(v, 4) AS is_error{ops_out} \
         FROM (SELECT trace_id, \
         argMax((endpoint_service, endpoint_name, duration_ns, is_error{ops_in}), span_count) AS v \
         FROM trace_summaries WHERE ts > now() - INTERVAL ? MINUTE GROUP BY trace_id)), \
         slow_ids AS (SELECT trace_id FROM error_stories \
         WHERE kind = 'slow' AND ts > now() - INTERVAL ? MINUTE), \
         p50s AS (SELECT endpoint_service, endpoint_name, quantile(0.5)(duration_ns) AS p50 \
         FROM latest WHERE is_error = 0 AND trace_id NOT IN (SELECT trace_id FROM slow_ids) \
         GROUP BY endpoint_service, endpoint_name)"
    )
}

/// Every non-error trace of the window with its endpoint's cap: the previous limit when there
/// is one, otherwise 10 x the p50 of the window's slow-story-free traces (bootstrap).
/// `is_kept`: no slow story and within the cap. `is_capped`: no slow story but above the cap.
fn baseline_candidates(with_ops: bool) -> String {
    let ops = if with_ops {
        "t.op_durations AS op_durations, "
    } else {
        ""
    };
    format!(
        "SELECT t.endpoint_service AS endpoint_service, \
         t.endpoint_name AS endpoint_name, t.duration_ns AS duration_ns, {ops}\
         indexOf(cap_keys, concat(t.endpoint_service, '\\0', t.endpoint_name)) AS cap_idx, \
         if(cap_idx > 0, arrayElement(cap_vals, cap_idx), toUInt64(ceil(10 * p50s.p50))) AS cap_ns, \
         t.trace_id IN (SELECT trace_id FROM slow_ids) AS has_slow, \
         NOT has_slow AND t.duration_ns <= cap_ns AS is_kept, \
         NOT has_slow AND t.duration_ns > cap_ns AS is_capped \
         FROM latest AS t \
         LEFT JOIN p50s ON t.endpoint_service = p50s.endpoint_service \
         AND t.endpoint_name = p50s.endpoint_name \
         WHERE t.is_error = 0"
    )
}

fn bind_baseline(
    q: clickhouse::query::Query,
    window_minutes: u32,
    caps: &EndpointCaps,
) -> clickhouse::query::Query {
    q.bind(&caps.keys)
        .bind(&caps.caps_ns)
        .bind(window_minutes)
        .bind(window_minutes.saturating_add(SLOW_STORY_LOOKBACK_SLACK_MIN))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_sql_reads_no_final_and_binds_four_values() {
        for ops in [false, true] {
            let sql = format!("{} {}", baseline_with(ops), baseline_candidates(ops));
            assert!(!sql.contains("FINAL"), "{sql}");
            assert_eq!(sql.matches('?').count(), 4, "{sql}");
            assert_eq!(sql.contains("op_durations"), ops, "{sql}");
        }
    }
}
