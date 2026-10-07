//! Read side over the tables written by the writer and the assembler.

use crate::model::*;
use crate::params::{
    AlertFilter, GroupFilter, HEALTH_BASELINE_SECS, SeriesQuery, TemplateFilter, TraceFilter,
    Window, health_baseline_end, now_ms,
};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use tayga_store::ClickHouseSettings;
use tayga_store::metrics_store::MetricPointRow;
use tayga_store::store::Store;
use tokio::sync::OnceCell;

pub trait Repo: Send + Sync + 'static {
    fn story_groups(
        &self,
        f: &GroupFilter,
    ) -> impl Future<Output = anyhow::Result<Vec<GroupView>>> + Send;
    fn story_group(
        &self,
        fingerprint: &str,
        window: Window,
    ) -> impl Future<Output = anyhow::Result<Option<GroupDetail>>> + Send;
    fn story(
        &self,
        story_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<StoryView>>> + Send;
    fn trace(&self, trace_id: &str) -> impl Future<Output = anyhow::Result<TraceView>> + Send;
    fn service_map(
        &self,
        window: Window,
    ) -> impl Future<Output = anyhow::Result<Vec<EdgeView>>> + Send;
    fn log_alerts(
        &self,
        f: &AlertFilter,
    ) -> impl Future<Output = anyhow::Result<Vec<LogAlertView>>> + Send;
    fn log_templates(
        &self,
        f: &TemplateFilter,
    ) -> impl Future<Output = anyhow::Result<Vec<LogTemplateListItem>>> + Send;
    fn log_template(
        &self,
        template_id: &str,
        window: Window,
    ) -> impl Future<Output = anyhow::Result<Option<LogTemplateDetail>>> + Send;
    fn trace_log_templates(
        &self,
        trace_id: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<TraceLogTemplate>>> + Send;
    /// The silence setting of a template, `None` when it was never set.
    fn silence(
        &self,
        template_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<SilenceSetting>>> + Send;
    /// Stores the silence setting; `false` when the template does not exist.
    fn put_silence(
        &self,
        template_id: &str,
        setting: SilenceSetting,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
    fn overview(&self, window: Window)
    -> impl Future<Output = anyhow::Result<OverviewView>> + Send;
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
        window: Window,
    ) -> impl Future<Output = anyhow::Result<Option<ServiceView>>> + Send;
    /// Edges as `service_map`, plus per-node RED and health.
    fn service_graph(
        &self,
        window: Window,
    ) -> impl Future<Output = anyhow::Result<ServiceMapView>> + Send;
    fn search(&self, q: &str) -> impl Future<Output = anyhow::Result<SearchView>> + Send;
    /// The last sample per series and `q.window.step()` bucket (see `Store::metric_buckets`).
    fn metric_buckets(
        &self,
        q: &SeriesQuery,
    ) -> impl Future<Output = anyhow::Result<Vec<MetricPointRow>>> + Send;
}

/// A template counts as alerting while one of its alerts was last seen this recently.
const ALERT_ACTIVE_MIN: u32 = 10;
/// A template's alerts are listed over this many seconds up to the window's end. Alerts are
/// kept 7 days (TTL), so this shows all of them.
const MAX_ALERT_AGE_SECS: i64 = 7 * 24 * 3600;
/// A trace's log matches an alert that started at most this long after the log.
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

/// Rows in a window: binds its two bounds (unix seconds, see `bind_rows` and `bind_capped`).
/// Half-open, so a row exactly at the end never starts a bucket past the window. `{col}` names
/// the time column.
fn in_window(col: &str) -> String {
    format!("{col} >= toDateTime(?) AND {col} < toDateTime(?)")
}

/// The bucket (unix seconds) a time column falls in: `step` wide on the epoch grid, so a live
/// window that moves keeps its bucket edges (the window clips the first and last bucket).
/// Binds `step` (see `bind_bucket`).
fn bucket_of(col: &str) -> String {
    format!("toUInt32(toStartOfInterval({col}, toIntervalSecond(?)))")
}

/// Binds `[start, upper)` for a row list (trace search; the alerts list and recent hits bind
/// `upper` directly): a live window also lists
/// rows stamped up to `LIVE_SLACK_SECS` ahead, so a producer clock running ahead hides nothing.
fn bind_rows(q: clickhouse::query::Query, w: Window) -> clickhouse::query::Query {
    q.bind(w.start).bind(w.upper())
}

/// Binds `[start, end)` for a bucketed or aggregated read: its totals then equal the sum of the
/// buckets the UI draws (the grid ends with the bucket holding `end - 1`), and rates divide by
/// the window's own length.
fn bind_capped(q: clickhouse::query::Query, w: Window) -> clickhouse::query::Query {
    q.bind(w.start).bind(w.end)
}

fn bind_bucket(q: clickhouse::query::Query, step: u32) -> clickhouse::query::Query {
    q.bind(step)
}

/// Filtered stories as a subquery so outer aliases never shadow filter columns. Binds the
/// window, then kind, service and fingerprint twice each.
const FILTERED: &str = "SELECT * FROM error_stories FINAL WHERE ts >= toDateTime(?) AND ts < toDateTime(?) \
     AND (? = '' OR toString(kind) = ?) AND (? = '' OR rc_service = ?) AND (? = '' OR toString(fingerprint) = ?)";

const GROUP_COLUMNS: &str = "toString(fingerprint) AS fingerprint, toString(any(kind)) AS kind, \
     argMax(summary, ts) AS summary, any(rc_service) AS rc_service, any(rc_span_name) AS rc_span_name, \
     any(endpoint_service) AS endpoint_service, any(endpoint_name) AS endpoint_name, count() AS stories, \
     toUnixTimestamp64Nano(min(ts)) AS first_seen_ns, toUnixTimestamp64Nano(max(ts)) AS last_seen_ns, \
     argMax(story_id, ts) AS sample_story_id";

pub struct ChRepo {
    client: clickhouse::Client,
    store: Store,
    /// The live minute's health baseline, single-flight (sub-project 4 spec §3.8).
    health: HealthCache,
}

/// Per-service p99 over `[end - HEALTH_BASELINE_SECS, end)`.
type Baseline = Arc<HashMap<String, f64>>;

/// One health baseline, by its end, behind a single-flight cell: concurrent requests for the
/// same end run one query and share its result (sub-project 4 spec §3.8).
///
/// The std mutex is held only to pick or install the cell, never across an `.await` (spec §6).
/// The query runs inside the caller's future, not a spawned task, so the request timeout still
/// bounds it; if that future is dropped (client gone, timeout, `try_join!` failure), the cell
/// releases its permit and the next waiter runs the query. An `Err` stores nothing, and the
/// caller whose query failed gets the original error (so a ClickHouse timeout still maps to 504).
#[derive(Default)]
struct HealthCache {
    slot: Mutex<Option<(i64, Arc<OnceCell<Baseline>>)>>,
}

impl HealthCache {
    /// The baseline ending at `end`, loading it with `load` unless the slot's cell for `end`
    /// already holds it or another caller is loading it.
    /// - The slot's end: share its cell.
    /// - A newer end, or an empty slot: install a new cell (the slot only moves forward).
    /// - An older end (a past window): a local cell, never installed, so the live minute stays.
    async fn get<F, Fut>(&self, end: i64, load: F) -> anyhow::Result<Baseline>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = anyhow::Result<HashMap<String, f64>>>,
    {
        let cell = {
            let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            match slot.as_ref() {
                Some((e, cell)) if *e == end => Arc::clone(cell),
                Some((e, _)) if *e > end => Arc::new(OnceCell::new()),
                _ => {
                    let cell = Arc::new(OnceCell::new());
                    *slot = Some((end, Arc::clone(&cell)));
                    cell
                }
            }
        };
        let p99 = cell
            .get_or_try_init(|| async { load().await.map(Arc::new) })
            .await?;
        Ok(Arc::clone(p99))
    }
}

/// Search results per kind (⌘K).
const SEARCH_LIMIT: u32 = 8;
/// Story groups are searched over the stories' full retention (7 days).
const STORY_SEARCH_SECS: u32 = 7 * 24 * 3600;
/// The overview's data lag only counts when recorded this recently.
const DATA_LAG_FRESH_SECS: u32 = 300;
const DATA_LAG_METRIC: &str = "tayga_logminer_data_lag_seconds";

/// Seconds the trace search scans past each window edge, so every version of a trace whose
/// versions lie at most this far apart is read and deduplicated together (see `TRACE_SEARCH`).
/// Span start times within one trace spread up to 52 s on the live stack (2026-10-07, last
/// hour); a re-assembly moves `ts` to another span's start of the same trace.
const TRACE_VERSION_SLACK_SECS: i64 = 600;

/// Trace search over the newest version of every trace in the window (plan 10). `{SCAN}` is the
/// service clause that holds for every version of a trace (the touched-service set, or `1`),
/// `{KEEP}` the one that depends on the version (the endpoint service, or `1`).
///
/// It reads the rows within `TRACE_VERSION_SLACK_SECS` of the window instead of
/// `trace_summaries FINAL`, which read the whole table (`ORDER BY trace_id`, so the `ts` filter
/// pruned nothing; 4.5 M rows and 237 MiB per default request). Those rows are deduplicated per
/// trace by `argMax(…, (span_count, ts))`: the `ReplacingMergeTree` version, then the newer `ts`.
/// The filters a version can change (migration 0003: a re-assembly can pick another root, which
/// changes `ts`, the endpoint, the duration and the error flag) run after the dedup, on the
/// newest version, as `FINAL` did; only the touched-service set, stable per trace, runs before it.
///
/// The result equals `FINAL`'s except at these edges:
/// - versions more than `TRACE_VERSION_SLACK_SECS` apart: when the newest version lies outside
///   the scan, an older one inside the window is returned (`FINAL` omits the trace), and it can
///   push a row out of the `LIMIT`. A trace id reused across far-apart requests does this: a
///   `flagd` `EventStream` trace id had versions at 17:46, 05:32 and 06:32, all of 2 spans;
/// - a `span_count` tie: `FINAL` keeps the last inserted version, this the newer `ts` (the same
///   when the later insert has the later `ts`, as in the reused id above), and any one of rows
///   equal in both (replays of the same trace, so equal);
/// - rows with equal `ts` are ordered by `trace_id`; `FINAL`'s order among them was unspecified,
///   so a `LIMIT` that cuts such a tie may now keep another, equally valid, subset.
const TRACE_SEARCH: &str = "SELECT trace_id, toUnixTimestamp64Nano(ts) AS ts_ns, endpoint_service, \
     endpoint_name, duration_ns, is_error, span_count \
     FROM (SELECT trace_id, tupleElement(v, 1) AS ts, tupleElement(v, 2) AS endpoint_service, \
     tupleElement(v, 3) AS endpoint_name, tupleElement(v, 4) AS duration_ns, \
     tupleElement(v, 5) AS is_error, tupleElement(v, 6) AS span_count \
     FROM (SELECT trace_id, argMax((ts, endpoint_service, endpoint_name, duration_ns, is_error, \
     span_count), (span_count, ts)) AS v FROM trace_summaries \
     WHERE ts >= toDateTime(?) AND ts < toDateTime(?) AND {SCAN} GROUP BY trace_id)) \
     WHERE ts >= toDateTime(?) AND ts < toDateTime(?) AND {KEEP} \
     AND (? = '' OR endpoint_name = ?) AND duration_ns >= ? AND duration_ns <= ? \
     AND (? = 0 OR is_error = 1) ORDER BY ts DESC, trace_id LIMIT ?";
/// The trace's endpoint service matches.
const SERVICE_IS_ENDPOINT: &str = "(? = '' OR endpoint_service = ?)";
/// The trace has a span of the service since the window's start. No upper bound: a trace that
/// starts inside the window may reach the service after its end.
const SERVICE_TOUCHED: &str = "trace_id IN (SELECT trace_id FROM spans \
     WHERE service_name = ? AND start_ts >= toDateTime(?))";

/// `TRACE_SEARCH` with its service clauses: the touched-service set before the dedup, or the
/// endpoint service after it. Bound parameters, in order: scan start and end, the touched
/// service and window start (touched only), window start and upper bound, the endpoint service
/// twice (not touched), the endpoint twice, min and max duration, errors only, limit.
fn trace_search_sql(touched: bool) -> String {
    let (scan, keep) = if touched {
        (SERVICE_TOUCHED, "1")
    } else {
        ("1", SERVICE_IS_ENDPOINT)
    };
    TRACE_SEARCH.replace("{SCAN}", scan).replace("{KEEP}", keep)
}

impl ChRepo {
    pub fn new(s: &ClickHouseSettings) -> Self {
        Self {
            client: clickhouse::Client::default()
                .with_url(&s.url)
                .with_database(&s.database),
            store: Store::new(s),
            health: HealthCache::default(),
        }
    }

    /// Sends `max_execution_time` = `secs` with every query, so ClickHouse stops a read that runs
    /// longer (error 159; the route answers 504). `0` sends nothing (no limit).
    pub fn with_max_execution_time(self, secs: u64) -> Self {
        if secs == 0 {
            return self;
        }
        let value = secs.to_string();
        Self {
            client: self
                .client
                .with_setting("max_execution_time", value.as_str()),
            store: self.store.with_setting("max_execution_time", &value),
            health: self.health,
        }
    }

    /// The map's health baseline for a window ending at `window_end`: per service, the p99 of
    /// the server and consumer spans of the `HEALTH_BASELINE_SECS` before the minute floor of the
    /// earlier of the end and now. Kept for that minute and computed once per minute however many
    /// requests ask at once (sub-project 4 spec §3.8). A failed query caches nothing.
    async fn health_baseline(&self, window_end: i64) -> anyhow::Result<Baseline> {
        let end = health_baseline_end(window_end, now_ms().div_euclid(1000));
        self.health
            .get(end, || async {
                let rows: Vec<BaselineRow> = self
                    .client
                    .query(
                        "SELECT toString(service_name) AS service, quantile(0.99)(duration_ns) AS p99_ns \
                         FROM spans WHERE kind IN ('server', 'consumer') \
                         AND start_ts >= toDateTime(?) AND start_ts < toDateTime(?) \
                         GROUP BY service",
                    )
                    .bind(end - i64::from(HEALTH_BASELINE_SECS))
                    .bind(end)
                    .fetch_all()
                    .await?;
                Ok(rows.into_iter().map(|r| (r.service, r.p99_ns)).collect())
            })
            .await
    }

    /// Stories per bucket and kind under a group filter.
    async fn kind_buckets(&self, f: &GroupFilter) -> anyhow::Result<StoriesSeries> {
        let kind = f.kind.as_deref().unwrap_or_default();
        let service = f.service.as_deref().unwrap_or_default();
        let step = f.window.step();
        let q = self.client.query(&format!(
            "SELECT {} AS bucket, toString(kind) AS kind, count() AS n FROM ({FILTERED}) \
             GROUP BY bucket, kind ORDER BY bucket",
            bucket_of("ts")
        ));
        let rows: Vec<KindBucketRow> = bind_capped(bind_bucket(q, step), f.window)
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
            bind_capped(q, f.window)
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
        let step = f.window.step();
        let top: Vec<&str> = groups.iter().map(|g| g.fingerprint.as_str()).collect();
        // Placeholder order: the bucket, the FILTERED binds, then the fingerprint list.
        let query = self.client.query(&format!(
            "SELECT toString(fingerprint) AS fingerprint, {} AS bucket, count() AS stories \
             FROM ({FILTERED} AND has(?, toString(fingerprint))) GROUP BY fingerprint, bucket ORDER BY bucket",
            bucket_of("ts")
        ));
        let query = bind_bucket(query, step);
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

    /// The stories among `trace_ids` (story_id equals trace_id), mapped to their kind.
    async fn story_kinds_among<'a>(
        &self,
        trace_ids: impl Iterator<Item = &'a str>,
    ) -> anyhow::Result<HashMap<String, String>> {
        let mut traces: Vec<&str> = trace_ids.collect();
        traces.sort_unstable();
        traces.dedup();
        if traces.is_empty() {
            return Ok(HashMap::new());
        }
        Ok(self
            .client
            .query(
                "SELECT story_id, toString(kind) AS kind FROM error_stories FINAL \
                 WHERE story_id IN ?",
            )
            .bind(&traces)
            .fetch_all::<StoryKindRow>()
            .await?
            .into_iter()
            .map(|r| (r.story_id, r.kind))
            .collect())
    }

    /// Alerts that overlap the window (seen after its start, started by its end), newest first,
    /// resolving which example traces have an error story. `active` is as of the window's end.
    async fn alerts(
        &self,
        w: Window,
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
                 toUInt8(last_at > toDateTime(?) - toIntervalMinute({ALERT_ACTIVE_MIN})) AS active, example_trace_ids, \
                 baseline_day, baseline_week \
                 FROM (SELECT * FROM log_alerts FINAL WHERE last_at >= toDateTime(?) AND started_at < toDateTime(?) \
                 AND (? = '' OR toString(kind) = ?) AND (? = '' OR service = ?) AND (? = '' OR toString(template_id) = ?)) \
                 ORDER BY last_at DESC LIMIT {limit}"
            ))
            .bind(w.end)
            .bind(w.start)
            .bind(w.upper())
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

/// Template ids with a `new` or `spike` alert active at the window's end, by the alerts list's
/// rule (`active` as of `end`, started before the list's upper bound): binds `end`, then
/// `upper`. Silence alerts are left out: `alerting` flags new or spiking activity, while a
/// silent template shows through `silence_enabled` and the alert list.
const ALERTING_AT: &str = "SELECT template_id FROM log_alerts FINAL \
     WHERE kind != 'silence' AND last_at > toDateTime(?) - toIntervalMinute({ACTIVE}) \
     AND started_at < toDateTime(?)";

/// Templates with at least one hit in the window, as a subquery so the outer aliases never
/// shadow the filter columns. `alerting` is as of the window's end.
const TEMPLATES_IN_WINDOW: &str = "SELECT toString(t.template_id) AS template_id, t.service AS service, \
     t.template AS template, h.hits AS count, toUnixTimestamp64Nano(t.first_seen) AS first_seen_ns, \
     toUnixTimestamp64Nano(t.last_seen) AS last_seen_ns, t.max_severity AS max_severity, \
     toUInt8(t.template_id IN ({ALERTING_AT})) AS alerting \
     FROM (SELECT template_id, uniqExact(log_id) AS hits FROM log_template_hits \
       WHERE ts >= toDateTime(?) AND ts < toDateTime(?) AND (? = '' OR service = ?) GROUP BY template_id) AS h \
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
        window: Window,
    ) -> anyhow::Result<Option<GroupDetail>> {
        let f = GroupFilter {
            window,
            kind: None,
            service: None,
        };
        let Some(group) = self.groups(&f, fingerprint).await?.into_iter().next() else {
            return Ok(None);
        };
        // The group's newest stories inside the window (row-list rule: up to `upper`).
        let examples: Vec<StorySummaryRow> = bind_rows(
            self.client
                .query(
                    "SELECT story_id, toUnixTimestamp64Nano(ts) AS ts_ns, trace_id, duration_ns, summary \
                     FROM error_stories FINAL WHERE fingerprint = toUInt64(?) \
                     AND ts >= toDateTime(?) AND ts < toDateTime(?) ORDER BY ts DESC LIMIT 20",
                )
                .bind(fingerprint),
            window,
        )
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
            f.window,
            f.kind.as_deref().unwrap_or_default(),
            f.service.as_deref().unwrap_or_default(),
            "",
            200,
        )
        .await
    }

    async fn log_templates(&self, f: &TemplateFilter) -> anyhow::Result<Vec<LogTemplateListItem>> {
        let service = f.service.as_deref().unwrap_or_default();
        let q = f.q.as_deref().unwrap_or_default();
        let sql = TEMPLATES_IN_WINDOW
            .replace("{ALERTING_AT}", ALERTING_AT)
            .replace("{ACTIVE}", &ALERT_ACTIVE_MIN.to_string());
        let rows: Vec<LogTemplateRow> = self
            .client
            .query(&sql)
            .bind(f.window.end)
            .bind(f.window.upper())
            .bind(f.window.start)
            .bind(f.window.end)
            .bind(service)
            .bind(service)
            .bind(service)
            .bind(service)
            .bind(q)
            .bind(q)
            .fetch_all()
            .await?;
        let step = f.window.step();
        // One grouped query for every listed template (at most 200), not one per row.
        let mut by_template: HashMap<String, Vec<(u32, u64)>> = HashMap::new();
        if !rows.is_empty() {
            let ids: Vec<String> = rows.iter().map(|r| r.template_id.clone()).collect();
            let q = self.client.query(&format!(
                "SELECT toString(template_id) AS template_id, {} AS bucket, uniqExact(log_id) AS hits \
                 FROM log_template_hits \
                 WHERE template_id IN (SELECT toUInt64(arrayJoin(?))) AND {} \
                 GROUP BY template_id, bucket ORDER BY template_id, bucket",
                bucket_of("ts"),
                in_window("ts")
            ));
            let q = bind_bucket(q, step).bind(ids);
            let hits: Vec<TemplateListBucketRow> = bind_capped(q, f.window).fetch_all().await?;
            for h in hits {
                by_template
                    .entry(h.template_id)
                    .or_default()
                    .push((h.bucket, h.hits));
            }
        }
        let silent: HashSet<String> = if rows.is_empty() {
            HashSet::new()
        } else {
            self.store
                .silence_enabled()
                .await?
                .into_iter()
                .map(|(id, _)| id.to_string())
                .collect()
        };
        Ok(rows
            .into_iter()
            .map(|r| {
                let buckets = by_template.remove(&r.template_id).unwrap_or_default();
                LogTemplateListItem {
                    silence_enabled: silent.contains(&r.template_id),
                    template: LogTemplateView::from_row(r),
                    bucket_secs: step,
                    buckets,
                }
            })
            .collect())
    }

    async fn log_template(
        &self,
        template_id: &str,
        window: Window,
    ) -> anyhow::Result<Option<LogTemplateDetail>> {
        let step = window.step();
        let alerting = ALERTING_AT.replace("{ACTIVE}", &ALERT_ACTIVE_MIN.to_string());
        let rows: Vec<TemplateDetailRow> = self
            .client
            .query(&format!(
                "SELECT toString(t.template_id) AS template_id, t.service AS service, t.template AS template, \
                 t.count AS count, toUnixTimestamp64Nano(t.first_seen) AS first_seen_ns, \
                 toUnixTimestamp64Nano(t.last_seen) AS last_seen_ns, t.max_severity AS max_severity, \
                 toUInt8(t.template_id IN ({alerting})) AS alerting, t.sample AS sample \
                 FROM (SELECT * FROM log_templates FINAL WHERE template_id = toUInt64(?) LIMIT 1) AS t"
            ))
            .bind(window.end)
            .bind(window.upper())
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
        let q = self.client.query(&format!(
            "SELECT {} AS bucket, uniqExact(log_id) AS hits \
             FROM log_template_hits WHERE template_id = toUInt64(?) AND {} \
             GROUP BY bucket ORDER BY bucket",
            bucket_of("ts"),
            in_window("ts")
        ));
        let q = bind_bucket(q, step).bind(template_id);
        let buckets: Vec<TemplateBucketRow> = bind_capped(q, window).fetch_all().await?;
        // The latest hits as of the window's end, inside the window or before it.
        let hits: Vec<TemplateHitRow> = self
            .client
            .query(
                "SELECT toUnixTimestamp64Nano(ts) AS ts_ns, trace_id, span_id, severity_number \
                 FROM log_template_hits WHERE template_id = toUInt64(?) AND ts < toDateTime(?) \
                 ORDER BY ts DESC LIMIT 1 BY log_id LIMIT 20",
            )
            .bind(template_id)
            .bind(window.upper())
            .fetch_all()
            .await?;
        let stories = self
            .story_ids_among(hits.iter().map(|h| h.trace_id.as_str()))
            .await?;
        let recent = hits
            .into_iter()
            .map(|h| TemplateHitView::from_row(h, &stories))
            .collect();
        let alerts_window = Window {
            start: window.end - MAX_ALERT_AGE_SECS,
            ..window
        };
        let alerts = self.alerts(alerts_window, "", "", template_id, 20).await?;
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

    /// A trace shows no templates until its logs are stored (writer lag; transient).
    async fn trace_log_templates(&self, trace_id: &str) -> anyhow::Result<Vec<TraceLogTemplate>> {
        // The trace's log time range: `logs` has a bloom index on `trace_id`, and a hit carries
        // its log's own `ts`, so bounding the hits by it lets ClickHouse skip other days' parts
        // (8.8M rows read unbounded vs 8.1k bounded, live 2026-10-06). No stored log, no links.
        let range: Vec<(i64, i64)> = self
            .client
            .query(
                "SELECT toUnixTimestamp64Nano(min(ts)), toUnixTimestamp64Nano(max(ts)) FROM logs \
                 WHERE trace_id = ? HAVING count() > 0",
            )
            .bind(trace_id)
            .fetch_all()
            .await?;
        let Some(&(first_ns, last_ns)) = range.first() else {
            return Ok(Vec::new());
        };
        let rows: Vec<TraceTemplateRow> = self
            .client
            .query(
                "SELECT toString(h.log_id) AS log_id, toString(h.template_id) AS template_id, t.template AS template, \
                 toUnixTimestamp64Nano(h.ts) AS ts_ns \
                 FROM (SELECT log_id, template_id, ts FROM log_template_hits WHERE trace_id = ? \
                 AND ts >= fromUnixTimestamp64Nano(?) AND ts <= fromUnixTimestamp64Nano(?) \
                 LIMIT 1 BY log_id) AS h \
                 INNER JOIN (SELECT template_id, template FROM log_templates FINAL) AS t ON t.template_id = h.template_id \
                 ORDER BY h.ts, h.log_id LIMIT 1000",
            )
            .bind(trace_id)
            .bind(first_ns)
            .bind(last_ns)
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
                 WHERE kind != 'silence' AND toString(template_id) IN ? \
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

    async fn silence(&self, template_id: &str) -> anyhow::Result<Option<SilenceSetting>> {
        let id: u64 = template_id.parse()?;
        Ok(self
            .store
            .silence_get(id)
            .await?
            .map(|(enabled, minutes)| SilenceSetting { enabled, minutes }))
    }

    async fn put_silence(
        &self,
        template_id: &str,
        setting: SilenceSetting,
    ) -> anyhow::Result<bool> {
        let id: u64 = template_id.parse()?;
        let known: u64 = self
            .client
            .query("SELECT count() FROM log_templates WHERE template_id = ?")
            .bind(id)
            .fetch_one()
            .await?;
        if known == 0 {
            return Ok(false);
        }
        self.store
            .silence_put(id, setting.enabled, setting.minutes)
            .await?;
        Ok(true)
    }

    async fn service_map(&self, window: Window) -> anyhow::Result<Vec<EdgeView>> {
        let q = self.client.query(&format!(
            "SELECT parent_service, child_service, sum(calls) AS calls, sum(errors) AS errors, \
             sum(duration_ns_sum) AS duration_ns_sum FROM service_edges \
             WHERE {} GROUP BY parent_service, child_service \
             ORDER BY calls DESC LIMIT 500",
            in_window("minute")
        ));
        let rows: Vec<EdgeRow> = bind_capped(q, window).fetch_all().await?;
        Ok(rows.into_iter().map(EdgeView::from_row).collect())
    }

    async fn overview(&self, window: Window) -> anyhow::Result<OverviewView> {
        let step = window.step();
        let stories = self
            .kind_buckets(&GroupFilter {
                window,
                kind: None,
                service: None,
            })
            .await?;
        // Alerts active at the window's end.
        let active_alerts: u64 = self
            .client
            .query(&format!(
                "SELECT count() FROM log_alerts FINAL \
                 WHERE last_at > toDateTime(?) - toIntervalMinute({ALERT_ACTIVE_MIN}) AND started_at < toDateTime(?)"
            ))
            .bind(window.end)
            .bind(window.upper())
            .fetch_one()
            .await?;
        let q = self.client.query(&format!(
            "SELECT {} AS bucket, count() AS n FROM spans WHERE {} GROUP BY bucket ORDER BY bucket",
            bucket_of("start_ts"),
            in_window("start_ts")
        ));
        let spans: Vec<CountBucketRow> = bind_capped(bind_bucket(q, step), window)
            .fetch_all()
            .await?;
        // Per replica (`instance`; one series without it), the lag recorded last before the
        // window's end within the freshness bound; the slowest of those.
        let lag: Vec<f64> = self
            .client
            .query(
                "SELECT v FROM (SELECT argMax(value, ts) AS v FROM metric_samples \
                 WHERE metric = ? AND isFinite(value) \
                 AND ts > toDateTime(?) - toIntervalSecond(?) AND ts < toDateTime(?) \
                 GROUP BY labels['instance']) ORDER BY v DESC LIMIT 1",
            )
            .bind(DATA_LAG_METRIC)
            .bind(window.end)
            .bind(DATA_LAG_FRESH_SECS)
            .bind(window.upper())
            .fetch_all()
            .await?;
        let total_spans: u64 = spans.iter().map(|b| b.n).sum();
        Ok(OverviewView {
            bucket_secs: step,
            error_stories: stories.error.iter().map(|b| b.1).sum(),
            slow_stories: stories.slow.iter().map(|b| b.1).sum(),
            active_alerts,
            spans_per_sec: total_spans as f64 / f64::from(window.secs().max(1)),
            data_lag_secs: lag.into_iter().next(),
            stories,
            spans: spans
                .into_iter()
                .map(|b| (b.bucket, b.n as f64 / f64::from(step)))
                .collect(),
        })
    }

    async fn stories_series(&self, f: &GroupFilter) -> anyhow::Result<StoriesSeries> {
        self.kind_buckets(f).await
    }

    async fn traces_search(&self, f: &TraceFilter) -> anyhow::Result<Vec<TraceHitView>> {
        let service = f.service.as_deref().unwrap_or_default();
        let endpoint = f.endpoint.as_deref().unwrap_or_default();
        let touched = f.touched && !service.is_empty();
        let mut q = self
            .client
            .query(&trace_search_sql(touched))
            .bind(f.window.start - TRACE_VERSION_SLACK_SECS)
            .bind(f.window.upper() + TRACE_VERSION_SLACK_SECS);
        if touched {
            q = q.bind(service).bind(f.window.start);
        }
        q = bind_rows(q, f.window);
        if !touched {
            q = q.bind(service).bind(service);
        }
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
            .story_kinds_among(rows.iter().map(|r| r.trace_id.as_str()))
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

    async fn service(&self, name: &str, window: Window) -> anyhow::Result<Option<ServiceView>> {
        let step = window.step();
        let q = self.client.query(&format!(
            "SELECT {} AS bucket, count() AS spans, \
             countIf(kind IN ('server', 'consumer')) AS calls, \
             countIf(kind IN ('server', 'consumer') AND status_code = 'error') AS errors, \
             quantilesIf(0.5, 0.95, 0.99)(duration_ns, kind IN ('server', 'consumer')) AS q \
             FROM spans WHERE service_name = ? AND {} \
             GROUP BY bucket ORDER BY bucket",
            bucket_of("start_ts"),
            in_window("start_ts")
        ));
        let q = bind_bucket(q, step).bind(name);
        let rows: Vec<ServiceBucketRow> = bind_capped(q, window).fetch_all().await?;
        Ok(ServiceView::from_rows(name, step, rows))
    }

    async fn service_graph(&self, window: Window) -> anyhow::Result<ServiceMapView> {
        let window_rows = async {
            let rows: Vec<NodeWindowRow> = self
                .client
                .query(
                    "SELECT toString(service_name) AS service, count() AS calls, \
                     countIf(status_code = 'error') AS errors, quantile(0.99)(duration_ns) AS p99_ns \
                     FROM spans WHERE kind IN ('server', 'consumer') \
                     AND start_ts >= toDateTime(?) AND start_ts < toDateTime(?) \
                     GROUP BY service ORDER BY service LIMIT 500",
                )
                .bind(window.start)
                .bind(window.end)
                .fetch_all()
                .await?;
            anyhow::Ok(rows)
        };
        // The three queries run concurrently. A failed baseline query caches nothing; when another
        // query fails first, `try_join!` drops the baseline future, and a waiter on the same
        // minute (if any) runs the baseline query instead.
        let (edges, rows, baseline) = tokio::try_join!(
            self.service_map(window),
            window_rows,
            self.health_baseline(window.end)
        )?;
        Ok(ServiceMapView {
            edges,
            nodes: rows
                .into_iter()
                .map(|r| {
                    let row = NodeRow {
                        baseline_p99_ns: baseline.get(&r.service).copied().unwrap_or(0.0),
                        service: r.service,
                        calls: r.calls,
                        errors: r.errors,
                        p99_ns: r.p99_ns,
                    };
                    NodeView::from_row(row, window.secs())
                })
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

    async fn metric_buckets(&self, q: &SeriesQuery) -> anyhow::Result<Vec<MetricPointRow>> {
        Ok(self
            .store
            .metric_buckets(
                q.job.as_deref(),
                &q.metric,
                &q.labels,
                (q.window.start, q.window.end),
                q.window.step(),
            )
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_health_baseline_ends_at_the_minute_floor() {
        let now = 1_791_310_400;
        assert_eq!(health_baseline_end(1_791_310_363, now), 1_791_310_320);
        assert_eq!(health_baseline_end(1_791_310_320, now), 1_791_310_320);
        assert_eq!(health_baseline_end(1_791_310_379, now), 1_791_310_320);
    }

    #[test]
    fn a_window_ending_ahead_of_now_keys_the_current_minute() {
        // `until` may run up to 60 s ahead; its minute must not key next minute's baseline.
        let now = 1_791_310_363;
        assert_eq!(health_baseline_end(now + 30, now), 1_791_310_320);
        assert_eq!(health_baseline_end(now + 60, now), 1_791_310_320);
        assert_eq!(health_baseline_end(now, now), 1_791_310_320);
    }

    #[tokio::test]
    async fn a_failed_health_baseline_caches_nothing() {
        let s = ClickHouseSettings {
            url: "http://127.0.0.1:1".into(),
            database: "tayga".into(),
        };
        let repo = ChRepo::new(&s);
        assert!(repo.health_baseline(1_791_310_363).await.is_err());
        let slot = repo
            .health
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert!(slot.as_ref().is_none_or(|(_, cell)| !cell.initialized()));
    }

    mod health_cache {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        const END: i64 = 1_791_310_320;
        const QUERY: Duration = Duration::from_secs(5);

        type Load =
            std::pin::Pin<Box<dyn Future<Output = anyhow::Result<HashMap<String, f64>>> + Send>>;

        /// A loader that counts its runs and takes `QUERY` to answer `{"svc": end}`.
        fn loader(loads: &Arc<AtomicUsize>, end: i64) -> impl FnOnce() -> Load + Send + 'static {
            let loads = Arc::clone(loads);
            move || {
                Box::pin(async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(QUERY).await;
                    Ok(HashMap::from([("svc".to_string(), end as f64)]))
                })
            }
        }

        fn slot_end(cache: &HealthCache) -> Option<i64> {
            cache
                .slot
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .map(|(e, _)| *e)
        }

        #[tokio::test(start_paused = true)]
        async fn concurrent_calls_for_one_end_load_once() {
            let cache = HealthCache::default();
            let loads = Arc::new(AtomicUsize::new(0));
            let (a, b, c) = tokio::join!(
                cache.get(END, loader(&loads, END)),
                cache.get(END, loader(&loads, END)),
                cache.get(END, loader(&loads, END)),
            );
            let (a, b, c) = (a.unwrap(), b.unwrap(), c.unwrap());
            assert_eq!(loads.load(Ordering::SeqCst), 1);
            assert!(Arc::ptr_eq(&a, &b) && Arc::ptr_eq(&b, &c));
            assert_eq!(a.get("svc"), Some(&(END as f64)));
            // Later calls in the minute reuse it.
            cache.get(END, loader(&loads, END)).await.unwrap();
            assert_eq!(loads.load(Ordering::SeqCst), 1);
        }

        #[tokio::test(start_paused = true)]
        async fn a_failed_load_caches_nothing_and_keeps_the_error() {
            let cache = HealthCache::default();
            let loads = Arc::new(AtomicUsize::new(0));
            let failing = {
                let loads = Arc::clone(&loads);
                move || async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(QUERY).await;
                    Err(anyhow::Error::new(std::io::Error::other("boom")))
                }
            };
            let err = cache.get(END, failing).await.unwrap_err();
            // The original error, so the route's timeout downcast still works.
            assert!(err.downcast_ref::<std::io::Error>().is_some());
            assert_eq!(loads.load(Ordering::SeqCst), 1);
            cache.get(END, loader(&loads, END)).await.unwrap();
            assert_eq!(loads.load(Ordering::SeqCst), 2);
        }

        #[tokio::test(start_paused = true)]
        async fn an_aborted_leader_lets_a_waiter_load() {
            let cache = Arc::new(HealthCache::default());
            let loads = Arc::new(AtomicUsize::new(0));
            let leader = tokio::spawn({
                let (cache, load) = (Arc::clone(&cache), loader(&loads, END));
                async move { cache.get(END, load).await }
            });
            tokio::time::sleep(Duration::from_millis(1)).await;
            let waiter = tokio::spawn({
                let (cache, load) = (Arc::clone(&cache), loader(&loads, END));
                async move { cache.get(END, load).await }
            });
            tokio::time::sleep(Duration::from_millis(1)).await;
            assert_eq!(loads.load(Ordering::SeqCst), 1, "the waiter waits");
            leader.abort();
            assert!(leader.await.unwrap_err().is_cancelled());
            let p99 = waiter.await.unwrap().unwrap();
            assert_eq!(p99.get("svc"), Some(&(END as f64)));
            assert_eq!(loads.load(Ordering::SeqCst), 2);
        }

        #[tokio::test(start_paused = true)]
        async fn an_older_end_never_replaces_the_slot_and_a_newer_one_does() {
            let cache = HealthCache::default();
            let loads = Arc::new(AtomicUsize::new(0));
            cache.get(END, loader(&loads, END)).await.unwrap();
            assert_eq!(slot_end(&cache), Some(END));

            let older = cache.get(END - 60, loader(&loads, END - 60)).await.unwrap();
            assert_eq!(older.get("svc"), Some(&((END - 60) as f64)));
            assert_eq!(loads.load(Ordering::SeqCst), 2);
            assert_eq!(slot_end(&cache), Some(END));
            cache.get(END, loader(&loads, END)).await.unwrap();
            assert_eq!(loads.load(Ordering::SeqCst), 2, "the live minute stayed");

            cache.get(END + 60, loader(&loads, END + 60)).await.unwrap();
            assert_eq!(loads.load(Ordering::SeqCst), 3);
            assert_eq!(slot_end(&cache), Some(END + 60));
            cache.get(END + 60, loader(&loads, END + 60)).await.unwrap();
            assert_eq!(loads.load(Ordering::SeqCst), 3);
        }
    }

    #[test]
    fn trace_search_reads_no_final_and_binds_twelve_values() {
        for touched in [false, true] {
            let sql = trace_search_sql(touched);
            assert!(!sql.contains("FINAL") && !sql.contains('{'), "{sql}");
            assert_eq!(sql.matches('?').count(), 12, "{sql}");
            assert_eq!(sql.contains("FROM spans"), touched, "{sql}");
        }
    }

    #[test]
    fn max_execution_time_is_sent_with_every_query_unless_zero() {
        let s = ClickHouseSettings {
            url: "http://127.0.0.1:1".into(),
            database: "tayga".into(),
        };
        let repo = ChRepo::new(&s).with_max_execution_time(15);
        assert_eq!(repo.client.get_setting("max_execution_time"), Some("15"));
        assert_eq!(
            repo.store.client().get_setting("max_execution_time"),
            Some("15")
        );
        let off = ChRepo::new(&s).with_max_execution_time(0);
        assert_eq!(off.client.get_setting("max_execution_time"), None);
    }

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
