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

    /// Root-duration quantiles per endpoint over the baseline trace set (see `BASELINE_CANDIDATES`).
    pub async fn endpoint_stats(
        &self,
        window_minutes: u32,
        caps: &EndpointCaps,
    ) -> clickhouse::error::Result<Vec<EndpointStatsRow>> {
        let sql = format!(
            "{BASELINE_WITH} \
             SELECT endpoint_service, endpoint_name, count() AS traces, \
             quantile(0.5)(duration_ns) AS p50, quantile(0.95)(duration_ns) AS p95, \
             quantile(0.99)(duration_ns) AS p99 \
             FROM ({BASELINE_CANDIDATES}) WHERE duration_ns <= cap_ns \
             GROUP BY endpoint_service, endpoint_name"
        );
        bind_baseline(self.client.query(&sql), window_minutes, caps)
            .fetch_all()
            .await
    }

    /// Per endpoint and op: traces containing the op and its p95 duration. Uses the same trace
    /// set as `endpoint_stats`, so presence ratios stay within 0..=1.
    pub async fn op_stats(
        &self,
        window_minutes: u32,
        caps: &EndpointCaps,
    ) -> clickhouse::error::Result<Vec<OpStatsRow>> {
        let sql = format!(
            "{BASELINE_WITH} \
             SELECT endpoint_service, endpoint_name, op, count() AS present, quantile(0.95)(d) AS p95 \
             FROM (SELECT endpoint_service, endpoint_name, op_durations \
             FROM ({BASELINE_CANDIDATES}) WHERE duration_ns <= cap_ns) \
             ARRAY JOIN mapKeys(op_durations) AS op, mapValues(op_durations) AS d \
             GROUP BY endpoint_service, endpoint_name, op"
        );
        bind_baseline(self.client.query(&sql), window_minutes, caps)
            .fetch_all()
            .await
    }

    /// Traces that the duration caps removed from the baseline set: non-error, no slow story,
    /// but above their endpoint's cap. Same predicate as `endpoint_stats`, counted separately.
    pub async fn capped_traces(
        &self,
        window_minutes: u32,
        caps: &EndpointCaps,
    ) -> clickhouse::error::Result<u64> {
        let sql = format!(
            "{BASELINE_WITH} \
             SELECT count() FROM ({BASELINE_CANDIDATES}) WHERE duration_ns > cap_ns"
        );
        bind_baseline(self.client.query(&sql), window_minutes, caps)
            .fetch_one()
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

/// Bound parameters, in order: cap keys, cap values, window (p50 subquery), window (traces),
/// slow-story lookback.
const BASELINE_WITH: &str = "WITH CAST(? AS Array(String)) AS cap_keys, \
     CAST(? AS Array(UInt64)) AS cap_vals, \
     p50s AS (SELECT endpoint_service, endpoint_name, quantile(0.5)(duration_ns) AS p50 \
     FROM trace_summaries FINAL WHERE ts > now() - INTERVAL ? MINUTE AND is_error = 0 \
     GROUP BY endpoint_service, endpoint_name)";

/// Non-error traces of the window with no slow story, each with its endpoint's cap: the
/// previous limit when there is one, otherwise 10 x the window p50 (bootstrap).
const BASELINE_CANDIDATES: &str = "SELECT t.endpoint_service AS endpoint_service, \
     t.endpoint_name AS endpoint_name, t.duration_ns AS duration_ns, t.op_durations AS op_durations, \
     indexOf(cap_keys, concat(t.endpoint_service, '\\0', t.endpoint_name)) AS cap_idx, \
     if(cap_idx > 0, arrayElement(cap_vals, cap_idx), toUInt64(ceil(10 * p50s.p50))) AS cap_ns \
     FROM trace_summaries AS t FINAL \
     LEFT JOIN p50s ON t.endpoint_service = p50s.endpoint_service AND t.endpoint_name = p50s.endpoint_name \
     WHERE t.ts > now() - INTERVAL ? MINUTE AND t.is_error = 0 \
     AND t.trace_id NOT IN (SELECT trace_id FROM error_stories \
     WHERE kind = 'slow' AND ts > now() - INTERVAL ? MINUTE)";

fn bind_baseline(
    q: clickhouse::query::Query,
    window_minutes: u32,
    caps: &EndpointCaps,
) -> clickhouse::query::Query {
    q.bind(&caps.keys)
        .bind(&caps.caps_ns)
        .bind(window_minutes)
        .bind(window_minutes)
        .bind(window_minutes.saturating_add(SLOW_STORY_LOOKBACK_SLACK_MIN))
}
