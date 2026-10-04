//! Metric history: samples scraped from Tayga's own `/metrics` endpoints (spec §7).

use crate::store::Store;
use serde::{Deserialize, Serialize};

/// Most rows `metric_points` returns, so one chart query stays bounded in memory.
pub const MAX_METRIC_POINTS: u32 = 200_000;

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct MetricSampleRow {
    /// Unix milliseconds (DateTime64(3)).
    pub ts: i64,
    pub job: String,
    pub metric: String,
    pub labels: Vec<(String, String)>,
    pub value: f64,
}

/// One stored sample of a metric, as read back for series math.
#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct MetricPointRow {
    pub ts_ms: i64,
    /// The scrape job. Part of the series identity, so two jobs exposing the same metric
    /// and labels are never merged into one counter.
    pub job: String,
    pub labels: Vec<(String, String)>,
    pub value: f64,
}

impl Store {
    pub async fn insert_metric_samples(
        &self,
        rows: &[MetricSampleRow],
    ) -> clickhouse::error::Result<()> {
        self.insert_rows("metric_samples", rows).await
    }

    /// Samples of `metric` from the last `since_secs` seconds, oldest first, optionally
    /// limited to one job. Over `MAX_METRIC_POINTS` rows, the newest ones are kept.
    pub async fn metric_points(
        &self,
        job: Option<&str>,
        metric: &str,
        since_secs: u32,
    ) -> clickhouse::error::Result<Vec<MetricPointRow>> {
        let job_clause = if job.is_some() { "AND job = ? " } else { "" };
        let sql = format!(
            "SELECT toUnixTimestamp64Milli(ts) AS ts_ms, toString(job) AS job, labels, value \
             FROM metric_samples \
             WHERE metric = ? {job_clause}AND ts > now64(3) - toIntervalSecond(?) \
             ORDER BY ts DESC LIMIT ?"
        );
        let mut q = self.client().query(&sql).bind(metric);
        if let Some(job) = job {
            q = q.bind(job);
        }
        let mut rows: Vec<MetricPointRow> = q
            .bind(since_secs)
            .bind(MAX_METRIC_POINTS)
            .fetch_all()
            .await?;
        rows.reverse();
        Ok(rows)
    }

    /// The last finite sample of `metric` per series (job and label set) and `step_secs`
    /// bucket over the last `since_secs` seconds, oldest first. `ts_ms` is the bucket start
    /// (epoch-aligned), so the series math sees one point per series and step whatever the
    /// window; at most `MAX_METRIC_POINTS` rows. `labels` keeps only series carrying every
    /// listed label with that value.
    pub async fn metric_buckets(
        &self,
        job: Option<&str>,
        metric: &str,
        labels: &[(String, String)],
        since_secs: u32,
        step_secs: u32,
    ) -> clickhouse::error::Result<Vec<MetricPointRow>> {
        let job_clause = if job.is_some() { "AND job = ? " } else { "" };
        let label_clause = "AND labels[?] = ? ".repeat(labels.len());
        // The inner alias differs from `value` so the WHERE clause reads the column.
        let sql = format!(
            "SELECT job, labels, ts_ms, last AS value FROM ( \
             SELECT toString(job) AS job, labels, \
             toInt64(toUnixTimestamp(toStartOfInterval(ts, toIntervalSecond(?)))) * 1000 AS ts_ms, \
             argMax(value, ts) AS last FROM metric_samples \
             WHERE metric = ? {job_clause}{label_clause}AND isFinite(value) \
             AND ts > now64(3) - toIntervalSecond(?) GROUP BY job, labels, ts_ms) \
             ORDER BY ts_ms LIMIT ?"
        );
        let mut q = self
            .client()
            .query(&sql)
            .bind(step_secs.max(1))
            .bind(metric);
        if let Some(job) = job {
            q = q.bind(job);
        }
        for (k, v) in labels {
            q = q.bind(k.as_str()).bind(v.as_str());
        }
        q.bind(since_secs).bind(MAX_METRIC_POINTS).fetch_all().await
    }
}
