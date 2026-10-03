use crate::migrate::ClickHouseSettings;
use crate::rows::{EndpointStatsRow, LogRow, OpStatsRow, SpanRow};
use clickhouse::{Client, RowOwned, RowWrite};

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

    /// Root-duration quantiles per endpoint over non-error traces in the window.
    pub async fn endpoint_stats(
        &self,
        window_minutes: u32,
    ) -> clickhouse::error::Result<Vec<EndpointStatsRow>> {
        self.client
            .query(
                "SELECT endpoint_service, endpoint_name, count() AS traces, \
                 quantile(0.5)(duration_ns) AS p50, quantile(0.95)(duration_ns) AS p95, \
                 quantile(0.99)(duration_ns) AS p99 \
                 FROM trace_summaries FINAL WHERE ts > now() - INTERVAL ? MINUTE AND is_error = 0 \
                 GROUP BY endpoint_service, endpoint_name",
            )
            .bind(window_minutes)
            .fetch_all()
            .await
    }

    /// Per endpoint and op: traces containing the op and its p95 duration.
    pub async fn op_stats(
        &self,
        window_minutes: u32,
    ) -> clickhouse::error::Result<Vec<OpStatsRow>> {
        self.client
            .query(
                "SELECT endpoint_service, endpoint_name, op, count() AS present, quantile(0.95)(d) AS p95 \
                 FROM trace_summaries FINAL ARRAY JOIN mapKeys(op_durations) AS op, mapValues(op_durations) AS d \
                 WHERE ts > now() - INTERVAL ? MINUTE AND is_error = 0 \
                 GROUP BY endpoint_service, endpoint_name, op",
            )
            .bind(window_minutes)
            .fetch_all()
            .await
    }
}
