//! Log template storage: rows for `log_templates`, `log_template_hits` and `log_alerts`,
//! plus the detection queries over them.

use crate::store::Store;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct LogTemplateRow {
    pub template_id: u64,
    pub service: String,
    pub template: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub count: u64,
    pub max_severity: u8,
    pub sample: String,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct LogHitRow {
    pub log_id: u64,
    pub template_id: u64,
    pub service: String,
    pub ts: i64,
    pub severity_number: u8,
    pub trace_id: String,
    pub span_id: String,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct LogAlertRow {
    pub alert_id: String,
    /// 1 new, 2 spike.
    pub kind: i8,
    pub template_id: u64,
    pub service: String,
    pub template: String,
    pub started_at: i64,
    pub last_at: i64,
    pub window_count: u64,
    pub peak_count: u64,
    pub baseline_per_window: f64,
    pub example_trace_ids: Vec<String>,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct TemplateWindowRow {
    pub template_id: u64,
    pub service: String,
    pub template: String,
    pub first_seen_ns: i64,
    pub current: u64,
    pub baseline_total: u64,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
pub struct NewCandidateRow {
    pub template_id: u64,
    pub service: String,
    pub template: String,
    pub first_seen_ns: i64,
    pub service_oldest_ns: i64,
}

impl Store {
    pub async fn insert_log_hits(&self, rows: &[LogHitRow]) -> clickhouse::error::Result<()> {
        self.insert_rows("log_template_hits", rows).await
    }

    pub async fn upsert_templates(&self, rows: &[LogTemplateRow]) -> clickhouse::error::Result<()> {
        self.insert_rows("log_templates", rows).await
    }

    pub async fn insert_alerts(&self, rows: &[LogAlertRow]) -> clickhouse::error::Result<()> {
        self.insert_rows("log_alerts", rows).await
    }

    pub async fn load_templates(&self) -> clickhouse::error::Result<Vec<LogTemplateRow>> {
        self.client()
            .query(
                "SELECT template_id, service, template, first_seen, last_seen, count, \
                 max_severity, sample, version FROM log_templates FINAL",
            )
            .fetch_all()
            .await
    }

    pub async fn active_spike_alerts(
        &self,
        active_min: u32,
    ) -> clickhouse::error::Result<Vec<LogAlertRow>> {
        self.client()
            .query(
                "SELECT * FROM log_alerts FINAL WHERE kind = 'spike' \
                 AND last_at > now64(9) - toIntervalMinute(?)",
            )
            .bind(active_min)
            .fetch_all()
            .await
    }

    /// Templates with at least `min_count` hits in the last `spike_min` minutes, with their
    /// hit total over the `baseline_min` minutes before that.
    pub async fn template_windows(
        &self,
        spike_min: u32,
        baseline_min: u32,
        min_count: u64,
    ) -> clickhouse::error::Result<Vec<TemplateWindowRow>> {
        self.client()
            .query(
                "SELECT h.template_id AS template_id, t.service AS service, t.template AS template, \
                 toUnixTimestamp64Nano(t.first_seen) AS first_seen_ns, h.current AS current, \
                 h.baseline_total AS baseline_total \
                 FROM ( \
                     SELECT template_id, \
                            uniqExactIf(log_id, ts > now64(9) - toIntervalMinute(?)) AS current, \
                            uniqExactIf(log_id, ts <= now64(9) - toIntervalMinute(?)) AS baseline_total \
                     FROM log_template_hits \
                     WHERE ts > now64(9) - toIntervalMinute(?) \
                     GROUP BY template_id \
                     HAVING current >= ? \
                 ) AS h \
                 INNER JOIN (SELECT template_id, service, template, first_seen FROM log_templates FINAL) AS t \
                     ON t.template_id = h.template_id",
            )
            .bind(spike_min)
            .bind(spike_min)
            .bind(spike_min.saturating_add(baseline_min))
            .bind(min_count)
            .fetch_all()
            .await
    }

    /// Templates first seen in the last `recent_min` minutes with no `new` alert yet, with the
    /// oldest first-seen of their service (to tell a new service from a new template).
    pub async fn new_template_candidates(
        &self,
        recent_min: u32,
    ) -> clickhouse::error::Result<Vec<NewCandidateRow>> {
        self.client()
            .query(
                "SELECT t.template_id AS template_id, t.service AS service, t.template AS template, \
                 toUnixTimestamp64Nano(t.first_seen) AS first_seen_ns, \
                 toUnixTimestamp64Nano(s.oldest) AS service_oldest_ns \
                 FROM (SELECT template_id, service, template, first_seen FROM log_templates FINAL \
                       WHERE first_seen > now64(9) - toIntervalMinute(?)) AS t \
                 INNER JOIN (SELECT service, min(first_seen) AS oldest FROM log_templates FINAL GROUP BY service) AS s \
                     ON s.service = t.service \
                 WHERE t.template_id NOT IN (SELECT template_id FROM log_alerts WHERE kind = 'new')",
            )
            .bind(recent_min)
            .fetch_all()
            .await
    }

    /// Distinct trace ids of recent hits of a template, newest first.
    pub async fn example_traces(
        &self,
        template_id: u64,
        since_min: u32,
        limit: u32,
    ) -> clickhouse::error::Result<Vec<String>> {
        self.client()
            .query(
                "SELECT trace_id FROM log_template_hits \
                 WHERE template_id = ? AND ts > now64(9) - toIntervalMinute(?) AND trace_id != '' \
                 ORDER BY ts DESC LIMIT 1 BY trace_id LIMIT ?",
            )
            .bind(template_id)
            .bind(since_min)
            .bind(limit)
            .fetch_all()
            .await
    }
}
