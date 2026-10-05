//! Metric history: samples scraped from Tayga's own `/metrics` endpoints (spec §7).

use crate::store::Store;
use serde::{Deserialize, Serialize};

/// Most rows `metric_buckets` returns, so one chart query stays bounded in memory.
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

    /// The last finite sample of `metric` per series (job and label set) and `step_secs`
    /// bucket in the window `(start, end]` (unix seconds), oldest first. `ts_ms` is the bucket
    /// start; buckets start at `start`, so the series math sees one point per series and step
    /// whatever the window; at most `MAX_METRIC_POINTS` rows. `labels` keeps only series
    /// carrying every listed label with that value.
    pub async fn metric_buckets(
        &self,
        job: Option<&str>,
        metric: &str,
        labels: &[(String, String)],
        (start, end): (i64, i64),
        step_secs: u32,
    ) -> clickhouse::error::Result<Vec<MetricPointRow>> {
        let job_clause = if job.is_some() { "AND job = ? " } else { "" };
        let label_clause = "AND labels[?] = ? ".repeat(labels.len());
        // The inner alias differs from `value` so the WHERE clause reads the column.
        let sql = format!(
            "SELECT job, labels, ts_ms, last AS value FROM ( \
             SELECT toString(job) AS job, labels, \
             (toInt64(?) + intDiv(toInt64(toUnixTimestamp(ts)) - toInt64(?), ?) * ?) * 1000 AS ts_ms, \
             argMax(value, ts) AS last FROM metric_samples \
             WHERE metric = ? {job_clause}{label_clause}AND isFinite(value) \
             AND ts > toDateTime(?) AND ts <= toDateTime(?) GROUP BY job, labels, ts_ms) \
             ORDER BY ts_ms LIMIT ?"
        );
        let step = step_secs.max(1);
        let mut q = self
            .client()
            .query(&sql)
            .bind(start)
            .bind(start)
            .bind(step)
            .bind(step)
            .bind(metric);
        if let Some(job) = job {
            q = q.bind(job);
        }
        for (k, v) in labels {
            q = q.bind(k.as_str()).bind(v.as_str());
        }
        q.bind(start)
            .bind(end)
            .bind(MAX_METRIC_POINTS)
            .fetch_all()
            .await
    }
}
