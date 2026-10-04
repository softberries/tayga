//! Read side over the tables written by the writer and the assembler.

use crate::model::*;
use crate::params::{
    AlertFilter, GroupFilter, HEALTH_BASELINE_SECS, SeriesQuery, TemplateFilter, TraceFilter,
    bucket_secs,
};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use tayga_store::ClickHouseSettings;
use tayga_store::metrics_store::MetricPointRow;
use tayga_store::store::Store;

pub trait Repo: Send + Sync + 'static {
    fn story_groups(
        &self,
        f: &GroupFilter,
    ) -> impl Future<Output = anyhow::Result<Vec<GroupView>>> + Send;
    fn story_group(
        &self,
        fingerprint: &str,
        since_secs: u32,
    ) -> impl Future<Output = anyhow::Result<Option<GroupDetail>>> + Send;
    fn story(
        &self,
        story_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<StoryView>>> + Send;
    fn trace(&self, trace_id: &str) -> impl Future<Output = anyhow::Result<TraceView>> + Send;
    fn service_map(
        &self,
        since_secs: u32,
    ) -> impl Future<Output = anyhow::Result<Vec<EdgeView>>> + Send;
    fn log_alerts(
        &self,
        f: &AlertFilter,
    ) -> impl Future<Output = anyhow::Result<Vec<LogAlertView>>> + Send;
    fn log_templates(
        &self,
        f: &TemplateFilter,
    ) -> impl Future<Output = anyhow::Result<Vec<LogTemplateView>>> + Send;
    fn log_template(
        &self,
        template_id: &str,
        since_secs: u32,
    ) -> impl Future<Output = anyhow::Result<Option<LogTemplateDetail>>> + Send;
    fn trace_log_templates(
        &self,
        trace_id: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<TraceLogTemplate>>> + Send;
    fn overview(
        &self,
        since_secs: u32,
    ) -> impl Future<Output = anyhow::Result<OverviewView>> + Send;
    fn stories_series(
        &self,
        f: &GroupFilter,
    ) -> impl Future<Output = anyhow::Result<StoriesSeries>> + Send;
    fn traces_search(
        &self,
        f: &TraceFilter,
    ) -> impl Future<Output = anyhow::Result<Vec<TraceHitView>>> + Send;
    fn services(&self) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send;
    fn service(
        &self,
        name: &str,
        since_secs: u32,
    ) -> impl Future<Output = anyhow::Result<Option<ServiceView>>> + Send;
    /// Edges as `service_map`, plus per-node RED and health.
    fn service_graph(
        &self,
        since_secs: u32,
    ) -> impl Future<Output = anyhow::Result<ServiceMapView>> + Send;
    fn search(&self, q: &str) -> impl Future<Output = anyhow::Result<SearchView>> + Send;
    /// The last sample per series and `step_secs` bucket (see `Store::metric_buckets`).
    fn metric_buckets(
        &self,
        q: &SeriesQuery,
        step_secs: u32,
    ) -> impl Future<Output = anyhow::Result<Vec<MetricPointRow>>> + Send;
}

/// A template counts as alerting while one of its alerts was last seen this recently.
const ALERT_ACTIVE_MIN: u32 = 10;
/// A trace's log matches an alert that started at most this long after the log.
/// Alerts are kept 7 days (TTL), so this window shows all of them.
const MAX_ALERT_AGE_SECS: u32 = 7 * 24 * 3600;
const ALERT_LEAD_MIN: u32 = 5;

/// Picks the alert kind per template id, `spike` over `new` when both match.
pub fn prefer_spike(rows: Vec<(String, String)>) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    for (template_id, kind) in rows {
        let e = out.entry(template_id).or_insert_with(|| kind.clone());
        if kind == "spike" {
            *e = kind;
        }
    }
    out
}

/// Attaches bucketed counts to their groups, preserving group order.
pub fn merge_buckets(
    groups: Vec<StoryGroupRow>,
    rows: Vec<GroupBucketRow>,
    bucket_secs: u32,
) -> Vec<GroupView> {
    let mut by_fp: HashMap<String, Vec<(u32, u64)>> = HashMap::new();
    for r in rows {
        by_fp
            .entry(r.fingerprint)
            .or_default()
            .push((r.bucket, r.stories));
    }
    groups
        .into_iter()
        .map(|group| {
            let mut buckets = by_fp.remove(&group.fingerprint).unwrap_or_default();
            buckets.sort_unstable();
            GroupView {
                group,
                bucket_secs,
                buckets,
            }
        })
        .collect()
}

/// Filtered stories as a subquery so outer aliases never shadow filter columns.
const FILTERED: &str = "SELECT * FROM error_stories FINAL WHERE ts > now64(9) - toIntervalSecond(?) \
     AND (? = '' OR toString(kind) = ?) AND (? = '' OR rc_service = ?) AND (? = '' OR toString(fingerprint) = ?)";

const GROUP_COLUMNS: &str = "toString(fingerprint) AS fingerprint, toString(any(kind)) AS kind, \
     argMax(summary, ts) AS summary, any(rc_service) AS rc_service, any(rc_span_name) AS rc_span_name, \
     any(endpoint_service) AS endpoint_service, any(endpoint_name) AS endpoint_name, count() AS stories, \
     toUnixTimestamp64Nano(min(ts)) AS first_seen_ns, toUnixTimestamp64Nano(max(ts)) AS last_seen_ns, \
     argMax(story_id, ts) AS sample_story_id";

pub struct ChRepo {
    client: clickhouse::Client,
    store: Store,
}

/// Search results per kind (⌘K).
const SEARCH_LIMIT: u32 = 8;
/// Story groups are searched over the stories' full retention (7 days).
const STORY_SEARCH_SECS: u32 = 7 * 24 * 3600;
/// The overview's data lag only counts when recorded this recently.
const DATA_LAG_FRESH_SECS: u32 = 300;
const DATA_LAG_METRIC: &str = "tayga_logminer_data_lag_seconds";

/// Trace search over `trace_summaries`; `{SERVICE}` is the service clause.
const TRACE_SEARCH: &str = "SELECT trace_id, toUnixTimestamp64Nano(ts) AS ts_ns, endpoint_service, endpoint_name, \
     duration_ns, is_error, span_count FROM trace_summaries FINAL \
     WHERE ts > now64(9) - toIntervalSecond(?) AND {SERVICE} \
     AND (? = '' OR endpoint_name = ?) AND duration_ns >= ? AND duration_ns <= ? \
     AND (? = 0 OR is_error = 1) ORDER BY ts DESC LIMIT ?";
/// The trace's endpoint service matches.
const SERVICE_IS_ENDPOINT: &str = "(? = '' OR endpoint_service = ?)";
/// The trace has a span of the service in the window.
const SERVICE_TOUCHED: &str = "trace_id IN (SELECT trace_id FROM spans \
     WHERE service_name = ? AND start_ts > now64(9) - toIntervalSecond(?))";

impl ChRepo {
    pub fn new(s: &ClickHouseSettings) -> Self {
        Self {
            client: clickhouse::Client::default()
                .with_url(&s.url)
                .with_database(&s.database),
            store: Store::new(s),
        }
    }

    /// Stories per bucket and kind under a group filter.
    async fn kind_buckets(&self, f: &GroupFilter, step: u32) -> anyhow::Result<StoriesSeries> {
        let kind = f.kind.as_deref().unwrap_or_default();
        let service = f.service.as_deref().unwrap_or_default();
        let rows: Vec<KindBucketRow> = self
            .client
            .query(&format!(
                "SELECT toUInt32(toStartOfInterval(ts, toIntervalSecond(?))) AS bucket, \
                 toString(kind) AS kind, count() AS n FROM ({FILTERED}) GROUP BY bucket, kind ORDER BY bucket"
            ))
            .bind(step)
            .bind(f.since_secs)
            .bind(kind)
            .bind(kind)
            .bind(service)
            .bind(service)
            .bind("")
            .bind("")
            .fetch_all()
            .await?;
        Ok(StoriesSeries::from_rows(step, rows))
    }

    async fn groups(&self, f: &GroupFilter, fingerprint: &str) -> anyhow::Result<Vec<GroupView>> {
        let kind = f.kind.clone().unwrap_or_default();
        let service = f.service.clone().unwrap_or_default();
        let bind = |q: clickhouse::query::Query| {
            q.bind(f.since_secs)
                .bind(kind.as_str())
                .bind(kind.as_str())
                .bind(service.as_str())
                .bind(service.as_str())
                .bind(fingerprint)
                .bind(fingerprint)
        };
        let groups: Vec<StoryGroupRow> = bind(self.client.query(&format!(
            "SELECT {GROUP_COLUMNS} FROM ({FILTERED}) GROUP BY fingerprint ORDER BY stories DESC LIMIT 100"
        )))
        .fetch_all()
        .await?;
        if groups.is_empty() {
            return Ok(Vec::new());
        }
        // Only the groups kept above, bucketed to about 120 points per group.
        let step = bucket_secs(f.since_secs);
        let top: Vec<&str> = groups.iter().map(|g| g.fingerprint.as_str()).collect();
        // Placeholder order: bucket width, the FILTERED binds, then the fingerprint list.
        let query = self
            .client
            .query(&format!(
                "SELECT toString(fingerprint) AS fingerprint, \
                 toUInt32(toStartOfInterval(ts, toIntervalSecond(?))) AS bucket, count() AS stories \
                 FROM ({FILTERED} AND has(?, toString(fingerprint))) GROUP BY fingerprint, bucket ORDER BY bucket"
            ))
            .bind(step);
        let rows: Vec<GroupBucketRow> = bind(query).bind(&top).fetch_all().await?;
        Ok(merge_buckets(groups, rows, step))
    }

    /// The subset of `trace_ids` that has an error story (story_id equals trace_id).
    async fn story_ids_among<'a>(
        &self,
        trace_ids: impl Iterator<Item = &'a str>,
    ) -> anyhow::Result<HashSet<String>> {
        let mut traces: Vec<&str> = trace_ids.collect();
        traces.sort_unstable();
        traces.dedup();
        if traces.is_empty() {
            return Ok(HashSet::new());
        }
        Ok(self
            .client
            .query("SELECT story_id FROM error_stories FINAL WHERE story_id IN ?")
            .bind(&traces)
            .fetch_all::<String>()
            .await?
            .into_iter()
            .collect())
    }

    /// Alerts newest first, resolving which example traces have an error story.
    async fn alerts(
        &self,
        since_secs: u32,
        kind: &str,
        service: &str,
        template_id: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<LogAlertView>> {
        let rows: Vec<LogAlertRow> = self
            .client
            .query(&format!(
                "SELECT alert_id, toString(kind) AS kind, toString(template_id) AS template_id, service, template, \
                 toUnixTimestamp64Nano(started_at) AS started_at_ns, toUnixTimestamp64Nano(last_at) AS last_at_ns, \
                 window_count, peak_count, baseline_per_window, \
                 toUInt8(last_at > now64(9) - toIntervalMinute({ALERT_ACTIVE_MIN})) AS active, example_trace_ids \
                 FROM (SELECT * FROM log_alerts FINAL WHERE last_at > now64(9) - toIntervalSecond(?) \
                 AND (? = '' OR toString(kind) = ?) AND (? = '' OR service = ?) AND (? = '' OR toString(template_id) = ?)) \
                 ORDER BY last_at DESC LIMIT {limit}"
            ))
            .bind(since_secs)
            .bind(kind)
            .bind(kind)
            .bind(service)
            .bind(service)
            .bind(template_id)
            .bind(template_id)
            .fetch_all()
            .await?;
        let stories = self
            .story_ids_among(
                rows.iter()
                    .flat_map(|r| r.example_trace_ids.iter().map(String::as_str)),
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| LogAlertView::from_row(r, &stories))
            .collect())
    }
}

/// Templates with at least one hit in the window, as a subquery so the outer aliases never
/// shadow the filter columns.
const TEMPLATES_IN_WINDOW: &str = "SELECT toString(t.template_id) AS template_id, t.service AS service, \
     t.template AS template, h.hits AS count, toUnixTimestamp64Nano(t.first_seen) AS first_seen_ns, \
     toUnixTimestamp64Nano(t.last_seen) AS last_seen_ns, t.max_severity AS max_severity, \
     toUInt8(t.template_id IN (SELECT template_id FROM log_alerts FINAL \
       WHERE last_at > now64(9) - toIntervalMinute({ACTIVE}))) AS alerting \
     FROM (SELECT template_id, uniqExact(log_id) AS hits FROM log_template_hits \
       WHERE ts > now64(9) - toIntervalSecond(?) AND (? = '' OR service = ?) GROUP BY template_id) AS h \
     INNER JOIN (SELECT * FROM log_templates FINAL WHERE (? = '' OR service = ?) \
       AND (? = '' OR positionCaseInsensitive(template, ?) > 0)) AS t ON t.template_id = h.template_id \
     ORDER BY count DESC LIMIT 200";

impl Repo for ChRepo {
    async fn story_groups(&self, f: &GroupFilter) -> anyhow::Result<Vec<GroupView>> {
        self.groups(f, "").await
    }

    async fn story_group(
        &self,
        fingerprint: &str,
        since_secs: u32,
    ) -> anyhow::Result<Option<GroupDetail>> {
        let f = GroupFilter {
            since_secs,
            kind: None,
            service: None,
        };
        let Some(group) = self.groups(&f, fingerprint).await?.into_iter().next() else {
            return Ok(None);
        };
        let examples: Vec<StorySummaryRow> = self
            .client
            .query(
                "SELECT story_id, toUnixTimestamp64Nano(ts) AS ts_ns, trace_id, duration_ns, summary \
                 FROM error_stories FINAL WHERE fingerprint = toUInt64(?) ORDER BY ts DESC LIMIT 20",
            )
            .bind(fingerprint)
            .fetch_all()
            .await?;
        Ok(Some(GroupDetail { group, examples }))
    }

    async fn story(&self, story_id: &str) -> anyhow::Result<Option<StoryView>> {
        let rows: Vec<StoryRecordRow> = self
            .client
            .query(
                "SELECT story_id, toString(fingerprint) AS fingerprint, toString(kind) AS kind, \
                 toUnixTimestamp64Nano(ts) AS ts_ns, trace_id, endpoint_service, endpoint_name, rc_service, \
                 rc_span_name, rc_span_kind, rc_span_id, rc_message, rc_exception_type, summary, duration_ns, \
                 path_services, path_spans, critical_path, baseline_diff, logs, also_failed, span_count, flags \
                 FROM error_stories FINAL WHERE story_id = ? LIMIT 1",
            )
            .bind(story_id)
            .fetch_all()
            .await?;
        Ok(rows.into_iter().next().map(StoryView::from_record))
    }

    async fn trace(&self, trace_id: &str) -> anyhow::Result<TraceView> {
        let rows: Vec<SpanChRow> = self
            .client
            .query(
                "SELECT span_id, parent_span_id, service_name, span_name, toString(kind) AS kind, \
                 toUnixTimestamp64Nano(start_ts) AS start_ns, duration_ns, toString(status_code) AS status, status_message, \
                 span_attrs AS attrs, resource_attrs AS resource, \
                 arrayMap(t -> toUnixTimestamp64Nano(t), `events.ts`) AS events_ts_ns, \
                 `events.name` AS events_name, `events.attrs` AS events_attrs \
                 FROM spans WHERE trace_id = ? ORDER BY start_ts LIMIT 1 BY span_id LIMIT 10000",
            )
            .bind(trace_id)
            .fetch_all()
            .await?;
        let mut spans: Vec<TraceSpanRow> = rows.into_iter().map(TraceSpanRow::from_ch).collect();
        fill_self_ns(&mut spans);
        let logs: Vec<TraceLogRow> = self
            .client
            .query(
                "SELECT toString(log_id) AS log_id, toUnixTimestamp64Nano(ts) AS ts_ns, span_id, service_name, severity_number, severity_text, body \
                 FROM logs WHERE trace_id = ? ORDER BY ts LIMIT 1 BY log_id LIMIT 1000",
            )
            .bind(trace_id)
            .fetch_all()
            .await?;
        let story_id = self
            .story_ids_among(std::iter::once(trace_id))
            .await?
            .into_iter()
            .next();
        Ok(TraceView {
            trace_id: trace_id.to_string(),
            spans,
            logs,
            story_id,
        })
    }

    async fn log_alerts(&self, f: &AlertFilter) -> anyhow::Result<Vec<LogAlertView>> {
        self.alerts(
            f.since_secs,
            f.kind.as_deref().unwrap_or_default(),
            f.service.as_deref().unwrap_or_default(),
            "",
            200,
        )
        .await
    }

    async fn log_templates(&self, f: &TemplateFilter) -> anyhow::Result<Vec<LogTemplateView>> {
        let service = f.service.as_deref().unwrap_or_default();
        let q = f.q.as_deref().unwrap_or_default();
        let rows: Vec<LogTemplateRow> = self
            .client
            .query(&TEMPLATES_IN_WINDOW.replace("{ACTIVE}", &ALERT_ACTIVE_MIN.to_string()))
            .bind(f.since_secs)
            .bind(service)
            .bind(service)
            .bind(service)
            .bind(service)
            .bind(q)
            .bind(q)
            .fetch_all()
            .await?;
        Ok(rows.into_iter().map(LogTemplateView::from_row).collect())
    }

    async fn log_template(
        &self,
        template_id: &str,
        since_secs: u32,
    ) -> anyhow::Result<Option<LogTemplateDetail>> {
        let step = bucket_secs(since_secs);
        let rows: Vec<TemplateDetailRow> = self
            .client
            .query(&format!(
                "SELECT toString(t.template_id) AS template_id, t.service AS service, t.template AS template, \
                 t.count AS count, toUnixTimestamp64Nano(t.first_seen) AS first_seen_ns, \
                 toUnixTimestamp64Nano(t.last_seen) AS last_seen_ns, t.max_severity AS max_severity, \
                 toUInt8(t.template_id IN (SELECT template_id FROM log_alerts FINAL \
                   WHERE last_at > now64(9) - toIntervalMinute({ALERT_ACTIVE_MIN}))) AS alerting, t.sample AS sample \
                 FROM (SELECT * FROM log_templates FINAL WHERE template_id = toUInt64(?) LIMIT 1) AS t"
            ))
            .bind(template_id)
            .fetch_all()
            .await?;
        let Some(found) = rows.into_iter().next() else {
            return Ok(None);
        };
        let sample = found.sample;
        let row = LogTemplateRow {
            template_id: found.template_id,
            service: found.service,
            template: found.template,
            count: found.count,
            first_seen_ns: found.first_seen_ns,
            last_seen_ns: found.last_seen_ns,
            max_severity: found.max_severity,
            alerting: found.alerting,
        };
        let buckets: Vec<TemplateBucketRow> = self
            .client
            .query(
                "SELECT toUInt32(toStartOfInterval(ts, toIntervalSecond(?))) AS bucket, uniqExact(log_id) AS hits \
                 FROM log_template_hits WHERE template_id = toUInt64(?) AND ts > now64(9) - toIntervalSecond(?) \
                 GROUP BY bucket ORDER BY bucket",
            )
            .bind(step)
            .bind(template_id)
            .bind(since_secs)
            .fetch_all()
            .await?;
        let hits: Vec<TemplateHitRow> = self
            .client
            .query(
                "SELECT toUnixTimestamp64Nano(ts) AS ts_ns, trace_id, span_id, severity_number \
                 FROM log_template_hits WHERE template_id = toUInt64(?) \
                 ORDER BY ts DESC LIMIT 1 BY log_id LIMIT 20",
            )
            .bind(template_id)
            .fetch_all()
            .await?;
        let stories = self
            .story_ids_among(hits.iter().map(|h| h.trace_id.as_str()))
            .await?;
        let recent = hits
            .into_iter()
            .map(|h| TemplateHitView::from_row(h, &stories))
            .collect();
        let alerts = self
            .alerts(MAX_ALERT_AGE_SECS, "", "", template_id, 20)
            .await?;
        let mut template = LogTemplateView::from_row(row);
        // The window's distinct hits, not the lifetime counter kept on the template row.
        template.count = buckets.iter().map(|b| b.hits).sum();
        Ok(Some(LogTemplateDetail {
            template,
            sample,
            bucket_secs: step,
            buckets: buckets.into_iter().map(|b| (b.bucket, b.hits)).collect(),
            recent,
            alerts,
        }))
    }

    async fn trace_log_templates(&self, trace_id: &str) -> anyhow::Result<Vec<TraceLogTemplate>> {
        let rows: Vec<TraceTemplateRow> = self
            .client
            .query(
                "SELECT toString(h.log_id) AS log_id, toString(h.template_id) AS template_id, t.template AS template, \
                 toUnixTimestamp64Nano(h.ts) AS ts_ns \
                 FROM (SELECT log_id, template_id, ts FROM log_template_hits WHERE trace_id = ? LIMIT 1 BY log_id) AS h \
                 INNER JOIN (SELECT template_id, template FROM log_templates FINAL) AS t ON t.template_id = h.template_id \
                 ORDER BY h.ts, h.log_id LIMIT 1000",
            )
            .bind(trace_id)
            .fetch_all()
            .await?;
        let Some(trace_ns) = rows.iter().map(|r| r.ts_ns).min() else {
            return Ok(Vec::new());
        };
        let mut ids: Vec<&str> = rows.iter().map(|r| r.template_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        // Active at the trace's time: started by then (or shortly after) and not yet over.
        let alerts: Vec<(String, String)> = self
            .client
            .query(&format!(
                "SELECT toString(template_id) AS template_id, toString(kind) AS kind FROM log_alerts FINAL \
                 WHERE toString(template_id) IN ? \
                 AND started_at <= fromUnixTimestamp64Nano(?) + toIntervalMinute({ALERT_LEAD_MIN}) \
                 AND last_at >= fromUnixTimestamp64Nano(?) - toIntervalMinute({ALERT_ACTIVE_MIN})"
            ))
            .bind(&ids)
            .bind(trace_ns)
            .bind(trace_ns)
            .fetch_all()
            .await?;
        let kinds = prefer_spike(alerts);
        Ok(rows
            .into_iter()
            .map(|r| TraceLogTemplate {
                alert: kinds.get(&r.template_id).cloned(),
                log_id: r.log_id,
                template_id: r.template_id,
                template: r.template,
            })
            .collect())
    }

    async fn service_map(&self, since_secs: u32) -> anyhow::Result<Vec<EdgeView>> {
        let rows: Vec<EdgeRow> = self
            .client
            .query(
                "SELECT parent_service, child_service, sum(calls) AS calls, sum(errors) AS errors, \
                 sum(duration_ns_sum) AS duration_ns_sum FROM service_edges \
                 WHERE minute > now() - toIntervalSecond(?) GROUP BY parent_service, child_service \
                 ORDER BY calls DESC LIMIT 500",
            )
            .bind(since_secs)
            .fetch_all()
            .await?;
        Ok(rows.into_iter().map(EdgeView::from_row).collect())
    }

    async fn overview(&self, since_secs: u32) -> anyhow::Result<OverviewView> {
        let step = bucket_secs(since_secs);
        let stories = self
            .kind_buckets(
                &GroupFilter {
                    since_secs,
                    kind: None,
                    service: None,
                },
                step,
            )
            .await?;
        let active_alerts: u64 = self
            .client
            .query(&format!(
                "SELECT count() FROM log_alerts FINAL WHERE last_at > now64(9) - toIntervalMinute({ALERT_ACTIVE_MIN})"
            ))
            .fetch_one()
            .await?;
        let spans: Vec<CountBucketRow> = self
            .client
            .query(
                "SELECT toUInt32(toStartOfInterval(start_ts, toIntervalSecond(?))) AS bucket, count() AS n \
                 FROM spans WHERE start_ts > now64(9) - toIntervalSecond(?) GROUP BY bucket ORDER BY bucket",
            )
            .bind(step)
            .bind(since_secs)
            .fetch_all()
            .await?;
        let lag: Vec<f64> = self
            .client
            .query(
                "SELECT value FROM metric_samples WHERE metric = ? AND isFinite(value) \
                 AND ts > now64(3) - toIntervalSecond(?) ORDER BY ts DESC LIMIT 1",
            )
            .bind(DATA_LAG_METRIC)
            .bind(DATA_LAG_FRESH_SECS)
            .fetch_all()
            .await?;
        let total_spans: u64 = spans.iter().map(|b| b.n).sum();
        Ok(OverviewView {
            bucket_secs: step,
            error_stories: stories.error.iter().map(|b| b.1).sum(),
            slow_stories: stories.slow.iter().map(|b| b.1).sum(),
            active_alerts,
            spans_per_sec: total_spans as f64 / f64::from(since_secs.max(1)),
            data_lag_secs: lag.into_iter().next(),
            stories,
            spans: spans
                .into_iter()
                .map(|b| (b.bucket, b.n as f64 / f64::from(step)))
                .collect(),
        })
    }

    async fn stories_series(&self, f: &GroupFilter) -> anyhow::Result<StoriesSeries> {
        self.kind_buckets(f, bucket_secs(f.since_secs)).await
    }

    async fn traces_search(&self, f: &TraceFilter) -> anyhow::Result<Vec<TraceHitView>> {
        let service = f.service.as_deref().unwrap_or_default();
        let endpoint = f.endpoint.as_deref().unwrap_or_default();
        let touched = f.touched && !service.is_empty();
        let sql = TRACE_SEARCH.replace(
            "{SERVICE}",
            if touched {
                SERVICE_TOUCHED
            } else {
                SERVICE_IS_ENDPOINT
            },
        );
        let mut q = self.client.query(&sql).bind(f.since_secs);
        q = if touched {
            q.bind(service).bind(f.since_secs)
        } else {
            q.bind(service).bind(service)
        };
        let rows: Vec<TraceHitRow> = q
            .bind(endpoint)
            .bind(endpoint)
            .bind(f.min_ns)
            .bind(f.max_ns)
            .bind(u8::from(f.errors_only))
            .bind(f.limit)
            .fetch_all()
            .await?;
        let stories = self
            .story_ids_among(rows.iter().map(|r| r.trace_id.as_str()))
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| TraceHitView::from_row(r, &stories))
            .collect())
    }

    async fn services(&self) -> anyhow::Result<Vec<String>> {
        Ok(self
            .client
            .query(
                "SELECT DISTINCT toString(service_name) FROM spans \
                 WHERE start_ts > now64(9) - toIntervalHour(24) ORDER BY 1 LIMIT 500",
            )
            .fetch_all()
            .await?)
    }

    async fn service(&self, name: &str, since_secs: u32) -> anyhow::Result<Option<ServiceView>> {
        let step = bucket_secs(since_secs);
        let rows: Vec<ServiceBucketRow> = self
            .client
            .query(
                "SELECT toUInt32(toStartOfInterval(start_ts, toIntervalSecond(?))) AS bucket, count() AS spans, \
                 countIf(kind IN ('server', 'consumer')) AS calls, \
                 countIf(kind IN ('server', 'consumer') AND status_code = 'error') AS errors, \
                 quantilesIf(0.5, 0.95, 0.99)(duration_ns, kind IN ('server', 'consumer')) AS q \
                 FROM spans WHERE service_name = ? AND start_ts > now64(9) - toIntervalSecond(?) \
                 GROUP BY bucket ORDER BY bucket",
            )
            .bind(step)
            .bind(name)
            .bind(since_secs)
            .fetch_all()
            .await?;
        Ok(ServiceView::from_rows(name, step, rows))
    }

    async fn service_graph(&self, since_secs: u32) -> anyhow::Result<ServiceMapView> {
        let edges = self.service_map(since_secs).await?;
        // The window and the 24h baseline in one scan over the longer of the two.
        let rows: Vec<NodeRow> = self
            .client
            .query(
                "SELECT toString(service_name) AS service, \
                 countIf(start_ts > now64(9) - toIntervalSecond(?)) AS calls, \
                 countIf(start_ts > now64(9) - toIntervalSecond(?) AND status_code = 'error') AS errors, \
                 quantileIf(0.99)(duration_ns, start_ts > now64(9) - toIntervalSecond(?)) AS p99_ns, \
                 quantileIf(0.99)(duration_ns, start_ts > now64(9) - toIntervalSecond(?)) AS baseline_p99_ns \
                 FROM spans WHERE kind IN ('server', 'consumer') AND start_ts > now64(9) - toIntervalSecond(?) \
                 GROUP BY service HAVING calls > 0 ORDER BY service LIMIT 500",
            )
            .bind(since_secs)
            .bind(since_secs)
            .bind(since_secs)
            .bind(HEALTH_BASELINE_SECS)
            .bind(since_secs.max(HEALTH_BASELINE_SECS))
            .fetch_all()
            .await?;
        Ok(ServiceMapView {
            edges,
            nodes: rows
                .into_iter()
                .map(|r| NodeView::from_row(r, since_secs))
                .collect(),
        })
    }

    async fn search(&self, q: &str) -> anyhow::Result<SearchView> {
        let services: Vec<String> = self
            .client
            .query(&format!(
                "SELECT DISTINCT toString(service_name) FROM spans \
                 WHERE start_ts > now64(9) - toIntervalHour(24) AND positionCaseInsensitiveUTF8(service_name, ?) > 0 \
                 ORDER BY 1 LIMIT {SEARCH_LIMIT}"
            ))
            .bind(q)
            .fetch_all()
            .await?;
        let templates: Vec<SearchTemplate> = self
            .client
            .query(&format!(
                "SELECT toString(template_id) AS template_id, service, template FROM log_templates FINAL \
                 WHERE positionCaseInsensitiveUTF8(template, ?) > 0 ORDER BY last_seen DESC LIMIT {SEARCH_LIMIT}"
            ))
            .bind(q)
            .fetch_all()
            .await?;
        let groups: Vec<SearchGroup> = self
            .client
            .query(&format!(
                "SELECT toString(fingerprint) AS fingerprint, toString(any(kind)) AS kind, \
                 argMax(summary, ts) AS summary, count() AS stories \
                 FROM (SELECT fingerprint, kind, summary, ts FROM error_stories FINAL \
                   WHERE ts > now64(9) - toIntervalSecond({STORY_SEARCH_SECS}) AND positionCaseInsensitiveUTF8(summary, ?) > 0) \
                 GROUP BY fingerprint ORDER BY stories DESC LIMIT {SEARCH_LIMIT}"
            ))
            .bind(q)
            .fetch_all()
            .await?;
        Ok(SearchView {
            services,
            templates,
            groups,
            trace_id: crate::params::parse_hex_id(q).ok(),
        })
    }

    async fn metric_buckets(
        &self,
        q: &SeriesQuery,
        step_secs: u32,
    ) -> anyhow::Result<Vec<MetricPointRow>> {
        Ok(self
            .store
            .metric_buckets(
                q.job.as_deref(),
                &q.metric,
                &q.labels,
                q.since_secs,
                step_secs,
            )
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(fp: &str) -> StoryGroupRow {
        StoryGroupRow {
            fingerprint: fp.into(),
            kind: "error".into(),
            summary: "s".into(),
            rc_service: "payment".into(),
            rc_span_name: "charge".into(),
            endpoint_service: "e".into(),
            endpoint_name: "n".into(),
            stories: 1,
            first_seen_ns: 0,
            last_seen_ns: 0,
            sample_story_id: "x".into(),
        }
    }

    #[test]
    fn spike_wins_over_new() {
        let m = prefer_spike(vec![
            ("1".into(), "new".into()),
            ("1".into(), "spike".into()),
            ("2".into(), "spike".into()),
            ("2".into(), "new".into()),
            ("3".into(), "new".into()),
        ]);
        assert_eq!(m["1"], "spike");
        assert_eq!(m["2"], "spike");
        assert_eq!(m["3"], "new");
    }

    #[test]
    fn merge_buckets_attaches_sorted_points_and_keeps_order() {
        let rows = vec![
            GroupBucketRow {
                fingerprint: "2".into(),
                bucket: 120,
                stories: 3,
            },
            GroupBucketRow {
                fingerprint: "1".into(),
                bucket: 60,
                stories: 1,
            },
            GroupBucketRow {
                fingerprint: "2".into(),
                bucket: 60,
                stories: 2,
            },
        ];
        let out = merge_buckets(vec![group("2"), group("1"), group("3")], rows, 60);
        assert_eq!(
            out.iter()
                .map(|g| g.group.fingerprint.as_str())
                .collect::<Vec<_>>(),
            ["2", "1", "3"]
        );
        assert_eq!(out[0].buckets, vec![(60, 2), (120, 3)]);
        assert_eq!(out[0].bucket_secs, 60);
        assert!(out[2].buckets.is_empty());
    }
}
