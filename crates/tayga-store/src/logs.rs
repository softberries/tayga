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
    /// 1 new, 2 spike, 3 silence.
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
    /// Seasonal comparators (spec 7a §2.3): hits of the same window 1 day / 7 days earlier.
    pub baseline_day: Option<f64>,
    pub baseline_week: Option<f64>,
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

/// One shifted copy of the spike window: whether it had any data at all (global coverage) and
/// the per-template distinct hit counts in it.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonalWindow {
    pub shift_secs: u32,
    pub covered: bool,
    /// `(template_id, hits)`; templates without a row in a covered window have 0 hits.
    pub counts: Vec<(u64, u64)>,
}

/// Detection inputs of one template for silence alerts (log time, ns). `t_last_ns` is the
/// template's newest hit, `s_last_ns` the newest hit over its service's templates; both are
/// `None` when no hit is left within the 3-day hits TTL.
#[derive(Debug, Clone, PartialEq)]
pub struct SilenceInput {
    pub template_id: u64,
    pub service: String,
    pub first_seen_ns: i64,
    pub t_last_ns: Option<i64>,
    pub s_last_ns: Option<i64>,
}

#[derive(clickhouse::Row, Deserialize)]
struct SilenceSettingRow {
    enabled: u8,
    minutes: u32,
}

#[derive(clickhouse::Row, Deserialize)]
struct SilenceEnabledRow {
    template_id: u64,
    minutes: u32,
}

#[derive(clickhouse::Row, Deserialize)]
struct SilenceTemplateRow {
    template_id: u64,
    service: String,
    first_seen_ns: i64,
    last_seen_ns: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct TemplateLastRow {
    template_id: u64,
    last_ns: i64,
}

#[derive(clickhouse::Row, Deserialize)]
struct ServiceLastRow {
    service: String,
    last_ns: i64,
}

#[derive(Debug, Clone, PartialEq, clickhouse::Row, Serialize, Deserialize)]
struct SeasonalCountRow {
    template_id: u64,
    hits: u64,
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
                "SELECT alert_id, kind, template_id, service, template, started_at, last_at, \
                 window_count, peak_count, baseline_per_window, example_trace_ids, version, \
                 baseline_day, baseline_week \
                 FROM log_alerts FINAL WHERE kind = 'spike' \
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

    /// Distinct clock minutes (unix minutes, ascending) within the `baseline_min` minutes before
    /// the last `spike_min` minutes that have at least one hit of any template. A minute with
    /// no logs at all means the pipeline was down. At most `baseline_min + 1` values.
    pub async fn covered_minute_buckets(
        &self,
        spike_min: u32,
        baseline_min: u32,
    ) -> clickhouse::error::Result<Vec<i64>> {
        self.client()
            .query(
                "SELECT DISTINCT toInt64(intDiv(toUnixTimestamp(toStartOfMinute(ts)), 60)) AS m \
                 FROM log_template_hits \
                 WHERE ts > now64(9) - toIntervalMinute(?) AND ts <= now64(9) - toIntervalMinute(?) \
                 ORDER BY m",
            )
            .bind(spike_min.saturating_add(baseline_min))
            .bind(spike_min)
            .fetch_all()
            .await
    }

    /// Per-template hit counts of the spike window shifted back by each of `shifts_secs`, from
    /// `log_template_minutes` (distinct `log_id`s, so replays do not count twice). The window is
    /// the `spike_min` whole minutes ending at the start of the current minute, minus the shift.
    /// `covered` is true when any template has a row in that window; callers must ignore the
    /// counts of an uncovered window (no data is not zero hits).
    pub async fn seasonal_counts(
        &self,
        template_ids: &[u64],
        spike_min: u32,
        shifts_secs: &[u32],
    ) -> clickhouse::error::Result<Vec<SeasonalWindow>> {
        let mut out = Vec::with_capacity(shifts_secs.len());
        for &shift in shifts_secs {
            let covered: Option<u8> = self
                .client()
                .query(
                    "SELECT 1 FROM log_template_minutes \
                     WHERE minute >= toStartOfMinute(now() - toIntervalSecond(?) - toIntervalMinute(?)) \
                       AND minute < toStartOfMinute(now() - toIntervalSecond(?)) LIMIT 1",
                )
                .bind(shift)
                .bind(spike_min)
                .bind(shift)
                .fetch_optional()
                .await?;
            let counts = if covered.is_some() && !template_ids.is_empty() {
                let rows: Vec<SeasonalCountRow> = self
                    .client()
                    .query(
                        "SELECT template_id, uniqExactMerge(hits) AS hits FROM log_template_minutes \
                         WHERE template_id IN ? \
                           AND minute >= toStartOfMinute(now() - toIntervalSecond(?) - toIntervalMinute(?)) \
                           AND minute < toStartOfMinute(now() - toIntervalSecond(?)) \
                         GROUP BY template_id",
                    )
                    .bind(template_ids)
                    .bind(shift)
                    .bind(spike_min)
                    .bind(shift)
                    .fetch_all()
                    .await?;
                template_ids
                    .iter()
                    .map(|&id| {
                        let hits = rows
                            .iter()
                            .find(|r| r.template_id == id)
                            .map_or(0, |r| r.hits);
                        (id, hits)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            out.push(SeasonalWindow {
                shift_secs: shift,
                covered: covered.is_some(),
                counts,
            });
        }
        Ok(out)
    }

    /// Latest hit `ts` in the last 3 days (the hits TTL) in ns, or 0 when there is none. The
    /// logminer's data clock for new-template detection.
    pub async fn data_now_ns(&self) -> clickhouse::error::Result<i64> {
        self.client()
            .query(
                // Logs stamped in the future (bad sender clock, crafted OTLP) must not move the
                // data clock ahead: the new-template watermark only ever advances.
                "SELECT toUnixTimestamp64Nano(max(ts)) FROM log_template_hits \
                 WHERE ts > now64(9) - toIntervalDay(3) AND ts <= now64(9) + toIntervalMinute(1)",
            )
            .fetch_one()
            .await
    }

    /// Templates first seen after `since_ns` (exclusive) with no `new` alert yet, with the
    /// oldest first-seen of their service (to tell a new service from a new template).
    pub async fn new_template_candidates(
        &self,
        since_ns: i64,
    ) -> clickhouse::error::Result<Vec<NewCandidateRow>> {
        self.client()
            .query(
                "SELECT t.template_id AS template_id, t.service AS service, t.template AS template, \
                 toUnixTimestamp64Nano(t.first_seen) AS first_seen_ns, \
                 toUnixTimestamp64Nano(s.oldest) AS service_oldest_ns \
                 FROM (SELECT template_id, service, template, first_seen FROM log_templates FINAL \
                       WHERE first_seen > fromUnixTimestamp64Nano(?)) AS t \
                 INNER JOIN (SELECT service, min(first_seen) AS oldest FROM log_templates FINAL GROUP BY service) AS s \
                     ON s.service = t.service \
                 WHERE t.template_id NOT IN (SELECT template_id FROM log_alerts WHERE kind = 'new')",
            )
            .bind(since_ns)
            .fetch_all()
            .await
    }

    /// Persisted logminer state value for `key`, or `None` when never stored.
    pub async fn state_get(&self, key: &str) -> clickhouse::error::Result<Option<i64>> {
        self.client()
            .query("SELECT value FROM logminer_state FINAL WHERE key = ?")
            .bind(key)
            .fetch_optional()
            .await
    }

    /// Stores `value` under `key`; the latest write wins.
    pub async fn state_put(&self, key: &str, value: i64) -> clickhouse::error::Result<()> {
        self.client()
            .query("INSERT INTO logminer_state (key, value, updated) VALUES (?, ?, now64(9))")
            .bind(key)
            .bind(value)
            .execute()
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

    /// Silence setting of a template as `(enabled, minutes)`, or `None` when never set.
    pub async fn silence_get(
        &self,
        template_id: u64,
    ) -> clickhouse::error::Result<Option<(bool, u32)>> {
        let row: Option<SilenceSettingRow> = self
            .client()
            .query("SELECT enabled, minutes FROM log_template_silence FINAL WHERE template_id = ?")
            .bind(template_id)
            .fetch_optional()
            .await?;
        Ok(row.map(|r| (r.enabled != 0, r.minutes)))
    }

    /// Stores the silence setting of a template; the latest write wins.
    pub async fn silence_put(
        &self,
        template_id: u64,
        enabled: bool,
        minutes: u32,
    ) -> clickhouse::error::Result<()> {
        self.client()
            .query(
                "INSERT INTO log_template_silence (template_id, enabled, minutes, updated) \
                 VALUES (?, ?, ?, now64(9))",
            )
            .bind(template_id)
            .bind(u8::from(enabled))
            .bind(minutes)
            .execute()
            .await
    }

    /// `(template_id, minutes)` of every template whose latest silence setting is enabled.
    pub async fn silence_enabled(&self) -> clickhouse::error::Result<Vec<(u64, u32)>> {
        let rows: Vec<SilenceEnabledRow> = self
            .client()
            .query(
                "SELECT template_id, minutes FROM log_template_silence FINAL \
                 WHERE enabled = 1 ORDER BY template_id",
            )
            .fetch_all()
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.template_id, r.minutes))
            .collect())
    }

    /// Silence detection inputs for `template_ids`, read from the hits within their 3-day TTL.
    /// Ids without a template row are left out.
    pub async fn silence_inputs(
        &self,
        template_ids: &[u64],
    ) -> clickhouse::error::Result<Vec<SilenceInput>> {
        if template_ids.is_empty() {
            return Ok(Vec::new());
        }
        let templates: Vec<SilenceTemplateRow> = self
            .client()
            .query(
                "SELECT template_id, service, toUnixTimestamp64Nano(first_seen) AS first_seen_ns, \
                 toUnixTimestamp64Nano(last_seen) AS last_seen_ns \
                 FROM log_templates FINAL WHERE template_id IN ? ORDER BY template_id",
            )
            .bind(template_ids)
            .fetch_all()
            .await?;
        let t_last: Vec<TemplateLastRow> = self
            .client()
            .query(
                "SELECT template_id, toUnixTimestamp64Nano(max(ts)) AS last_ns \
                 FROM log_template_hits WHERE template_id IN ? \
                 AND ts > now64(9) - toIntervalDay(3) GROUP BY template_id",
            )
            .bind(template_ids)
            .fetch_all()
            .await?;
        let mut services: Vec<&str> = templates.iter().map(|t| t.service.as_str()).collect();
        services.sort_unstable();
        services.dedup();
        let s_last: Vec<ServiceLastRow> = if services.is_empty() {
            Vec::new()
        } else {
            self.client()
                .query(
                    "SELECT service, toUnixTimestamp64Nano(max(ts)) AS last_ns \
                     FROM log_template_hits WHERE service IN ? \
                     AND ts > now64(9) - toIntervalDay(3) GROUP BY service",
                )
                .bind(services)
                .fetch_all()
                .await?
        };
        Ok(templates
            .into_iter()
            .map(|t| SilenceInput {
                // The newest hit inside the 3-day hits TTL, else the template's own `last_seen`
                // (30-day TTL): a silence longer than the hits TTL keeps the same anchor, so its
                // alert id does not change when the last hit ages out.
                t_last_ns: t_last
                    .iter()
                    .find(|r| r.template_id == t.template_id)
                    .map(|r| r.last_ns)
                    .or((t.last_seen_ns > 0).then_some(t.last_seen_ns)),
                s_last_ns: s_last
                    .iter()
                    .find(|r| r.service == t.service)
                    .map(|r| r.last_ns),
                template_id: t.template_id,
                service: t.service,
                first_seen_ns: t.first_seen_ns,
            })
            .collect())
    }
}
