use crate::migrate::ClickHouseSettings;
use crate::rows::{EndpointStatsRow, LogRow, OpStatsRow, SpanRow};
use clickhouse::{Client, RowOwned, RowWrite};

/// Slow stories are looked up over the baseline window plus this slack, so a trace near the
/// window edge whose story landed slightly earlier is still excluded.
const SLOW_STORY_LOOKBACK_SLACK_MIN: u32 = 10;

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
            "{BASELINE_WITH} \
             SELECT endpoint_service, endpoint_name, count() AS seen, \
             countIf(is_kept) AS kept, countIf(is_capped) AS excluded, \
             quantileIf(0.5)(duration_ns, is_kept) AS p50, \
             quantileIf(0.95)(duration_ns, is_kept) AS p95, \
             quantileIf(0.99)(duration_ns, is_kept) AS p99 \
             FROM ({BASELINE_CANDIDATES}) GROUP BY endpoint_service, endpoint_name"
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
            "{BASELINE_WITH} \
             SELECT endpoint_service, endpoint_name, op, count() AS present, quantile(0.95)(d) AS p95 \
             FROM (SELECT endpoint_service, endpoint_name, op_durations \
             FROM ({BASELINE_CANDIDATES}) WHERE is_kept) \
             ARRAY JOIN mapKeys(op_durations) AS op, mapValues(op_durations) AS d \
             GROUP BY endpoint_service, endpoint_name, op"
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

/// Bound parameters, in order: cap keys, cap values, slow-story lookback, window (p50
/// subquery), window (candidates).
const BASELINE_WITH: &str = "WITH CAST(? AS Array(String)) AS cap_keys, \
     CAST(? AS Array(UInt64)) AS cap_vals, \
     slow_ids AS (SELECT trace_id FROM error_stories \
     WHERE kind = 'slow' AND ts > now() - INTERVAL ? MINUTE), \
     p50s AS (SELECT endpoint_service, endpoint_name, quantile(0.5)(duration_ns) AS p50 \
     FROM trace_summaries FINAL WHERE ts > now() - INTERVAL ? MINUTE AND is_error = 0 \
     AND trace_id NOT IN (SELECT trace_id FROM slow_ids) \
     GROUP BY endpoint_service, endpoint_name)";

/// Every non-error trace of the window with its endpoint's cap: the previous limit when there
/// is one, otherwise 10 x the p50 of the window's slow-story-free traces (bootstrap).
/// `is_kept`: no slow story and within the cap. `is_capped`: no slow story but above the cap.
const BASELINE_CANDIDATES: &str = "SELECT t.endpoint_service AS endpoint_service, \
     t.endpoint_name AS endpoint_name, t.duration_ns AS duration_ns, t.op_durations AS op_durations, \
     indexOf(cap_keys, concat(t.endpoint_service, '\\0', t.endpoint_name)) AS cap_idx, \
     if(cap_idx > 0, arrayElement(cap_vals, cap_idx), toUInt64(ceil(10 * p50s.p50))) AS cap_ns, \
     t.trace_id IN (SELECT trace_id FROM slow_ids) AS has_slow, \
     NOT has_slow AND t.duration_ns <= cap_ns AS is_kept, \
     NOT has_slow AND t.duration_ns > cap_ns AS is_capped \
     FROM trace_summaries AS t FINAL \
     LEFT JOIN p50s ON t.endpoint_service = p50s.endpoint_service AND t.endpoint_name = p50s.endpoint_name \
     WHERE t.ts > now() - INTERVAL ? MINUTE AND t.is_error = 0";

fn bind_baseline(
    q: clickhouse::query::Query,
    window_minutes: u32,
    caps: &EndpointCaps,
) -> clickhouse::query::Query {
    q.bind(&caps.keys)
        .bind(&caps.caps_ns)
        .bind(window_minutes.saturating_add(SLOW_STORY_LOOKBACK_SLACK_MIN))
        .bind(window_minutes)
        .bind(window_minutes)
}
