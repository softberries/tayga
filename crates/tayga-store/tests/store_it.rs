use tayga_store::ClickHouseSettings;
use tayga_store::migrate::migrate;
use tayga_store::rows::{LogRow, SpanRow};
use tayga_store::store::{EndpointCaps, Store};

fn settings() -> ClickHouseSettings {
    let url =
        std::env::var("TAYGA_IT_CLICKHOUSE").unwrap_or_else(|_| "http://localhost:18123".into());
    let suffix: u32 = rand::random();
    ClickHouseSettings {
        url,
        database: format!("tayga_it_{suffix}"),
    }
}

/// Rows must be recent: the tables carry a 3-day TTL and expired parts are dropped on insert.
fn now_ns() -> i64 {
    static NOW: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *NOW.get_or_init(|| {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        i64::try_from(d.as_nanos()).unwrap()
    })
}

fn span(id: &str) -> SpanRow {
    SpanRow {
        trace_id: "ab".repeat(16),
        span_id: id.into(),
        parent_span_id: String::new(),
        service_name: "payment".into(),
        span_name: "Charge".into(),
        kind: 2,
        start_ts: now_ns(),
        duration_ns: 10,
        status_code: 2,
        status_message: "failed".into(),
        resource_attrs: vec![("service.name".into(), "payment".into())],
        span_attrs: vec![],
        events_ts: vec![now_ns() + 1],
        events_name: vec!["exception".into()],
        events_attrs: vec![vec![("exception.message".into(), "boom".into())]],
    }
}

#[tokio::test]
#[ignore = "requires ClickHouse: make it"]
async fn migrate_is_idempotent_and_rows_roundtrip() {
    let s = settings();
    assert_eq!(
        migrate(&s).await.unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    assert!(migrate(&s).await.unwrap().is_empty());

    let store = Store::new(&s);
    let rows = vec![span("01"), span("02"), span("01")];
    store.insert_spans(&rows).await.unwrap();
    store
        .insert_logs(&[LogRow {
            log_id: 1,
            ts: now_ns(),
            observed_ts: now_ns(),
            trace_id: "ab".repeat(16),
            span_id: "01".into(),
            severity_number: 17,
            severity_text: "ERROR".into(),
            service_name: "payment".into(),
            body: "declined".into(),
            resource_attrs: vec![],
            log_attrs: vec![],
        }])
        .await
        .unwrap();

    let back: Vec<SpanRow> = store
        .client()
        .query("SELECT ?fields FROM spans FINAL ORDER BY span_id")
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(
        back.len(),
        2,
        "duplicate (same sort key) collapses under FINAL"
    );
    assert_eq!(back[0], span("01"));

    let n: u64 = store
        .client()
        .query("SELECT count() FROM logs")
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(n, 1);
    store
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

use tayga_store::rows::{ServiceEdgeRow, StoryRow, TraceSummaryRow};

fn summary_row(i: u64, op_present: bool) -> TraceSummaryRow {
    let mut ops = vec![("frontend:GET".to_string(), 1_000 + i)];
    if op_present {
        ops.push(("cart:GetCart".to_string(), 10 * i));
    }
    TraceSummaryRow {
        trace_id: format!("t{i}"),
        ts: now_ns(),
        endpoint_service: "frontend".into(),
        endpoint_name: "GET /".into(),
        duration_ns: 1_000 + i,
        is_error: 0,
        op_durations: ops,
        span_count: 2,
    }
}

fn story_row(id: &str) -> StoryRow {
    StoryRow {
        story_id: id.into(),
        fingerprint: 42,
        kind: 1,
        ts: now_ns(),
        trace_id: id.into(),
        endpoint_service: "frontend".into(),
        endpoint_name: "GET /".into(),
        rc_service: "payment".into(),
        rc_span_name: "Charge".into(),
        rc_span_kind: "server".into(),
        rc_message: "Invalid token".into(),
        rc_exception_type: String::new(),
        summary: "payment Charge failed: Invalid token".into(),
        duration_ns: 100,
        path_services: vec!["frontend".into(), "payment".into()],
        path_spans: "[]".into(),
        critical_path: "{}".into(),
        baseline_diff: String::new(),
        logs: "[]".into(),
        also_failed: "[]".into(),
        span_count: 3,
        flags: vec!["incomplete".into()],
        rc_span_id: "00f067aa0ba902b7".into(),
    }
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn analysis_tables_roundtrip_and_baseline_queries() {
    let s = settings();
    assert_eq!(
        migrate(&s).await.unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    let store = Store::new(&s);

    let mut summaries: Vec<TraceSummaryRow> = (0..60).map(|i| summary_row(i, i % 2 == 0)).collect();
    summaries.push(TraceSummaryRow {
        is_error: 1,
        duration_ns: 9_999_999,
        ..summary_row(99, true)
    });
    store
        .insert_rows("trace_summaries", &summaries)
        .await
        .unwrap();
    // Replay the non-error summaries as a separate part: FINAL must dedupe them.
    store
        .insert_rows("trace_summaries", &summaries[..60])
        .await
        .unwrap();
    let minute = (now_ns() / 1_000_000_000 / 60 * 60) as u32;
    let edge = ServiceEdgeRow {
        minute,
        parent_service: "frontend".into(),
        child_service: "cart".into(),
        calls: 2,
        errors: 1,
        duration_ns_sum: 30,
    };
    store
        .insert_rows("service_edges", &[edge.clone(), edge])
        .await
        .unwrap();
    let story = story_row("t1");
    store
        .insert_rows("error_stories", &[story.clone(), story.clone()])
        .await
        .unwrap();

    let eps = store
        .endpoint_stats(60, &EndpointCaps::default())
        .await
        .unwrap();
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].kept, 60, "error trace excluded");
    let ops = store.op_stats(60, &EndpointCaps::default()).await.unwrap();
    let cart = ops.iter().find(|o| o.op == "cart:GetCart").unwrap();
    assert_eq!(cart.present, 30);
    let back: Vec<StoryRow> = store
        .client()
        .query("SELECT ?fields FROM error_stories FINAL")
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(back, vec![story]);
    let calls: u64 = store
        .client()
        .query("SELECT sum(calls) FROM service_edges")
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(calls, 4);
    store
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

/// Re-assembling a trace may pick another root: endpoint, ts and fingerprint change. Rows must
/// still collapse per trace, keeping the most complete version.
#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn replayed_trace_collapses_to_most_complete_row() {
    let s = settings();
    assert_eq!(
        migrate(&s).await.unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    let store = Store::new(&s);

    let full = TraceSummaryRow {
        endpoint_name: "GET /api/cart".into(),
        span_count: 5,
        ..summary_row(7, true)
    };
    let partial = TraceSummaryRow {
        ts: now_ns() + 1_000,
        endpoint_service: "cart".into(),
        endpoint_name: "GetCart".into(),
        span_count: 3,
        ..summary_row(7, false)
    };
    // Separate inserts = separate parts; the more complete row arrives first.
    store
        .insert_rows("trace_summaries", std::slice::from_ref(&full))
        .await
        .unwrap();
    store
        .insert_rows("trace_summaries", std::slice::from_ref(&partial))
        .await
        .unwrap();
    let back: Vec<TraceSummaryRow> = store
        .client()
        .query("SELECT ?fields FROM trace_summaries FINAL WHERE trace_id = 't7'")
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(back, vec![full]);

    let full_story = StoryRow {
        span_count: 5,
        ..story_row("t7")
    };
    let partial_story = StoryRow {
        fingerprint: 43,
        ts: now_ns() + 1_000,
        endpoint_service: "cart".into(),
        endpoint_name: "GetCart".into(),
        span_count: 3,
        ..story_row("t7")
    };
    store
        .insert_rows("error_stories", std::slice::from_ref(&full_story))
        .await
        .unwrap();
    store
        .insert_rows("error_stories", std::slice::from_ref(&partial_story))
        .await
        .unwrap();
    let back: Vec<StoryRow> = store
        .client()
        .query("SELECT ?fields FROM error_stories FINAL WHERE story_id = 't7'")
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(back, vec![full_story]);
    store
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

/// A trace that produced a slow story must leave both baseline queries, so the endpoint trace
/// count and every op's presence count shrink together. Error-kind stories do not exclude.
#[tokio::test]
#[ignore = "requires ClickHouse: make it"]
async fn slow_story_traces_are_excluded_from_baselines() {
    let s = settings();
    assert_eq!(
        migrate(&s).await.unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    let store = Store::new(&s);

    let summaries: Vec<TraceSummaryRow> = (0..60).map(|i| summary_row(i, i % 2 == 0)).collect();
    store
        .insert_rows("trace_summaries", &summaries)
        .await
        .unwrap();
    // t2 carries the cart op and produced a slow story; t3 produced only an error story.
    let slow = StoryRow {
        kind: 2,
        ..story_row("t2")
    };
    let error_only = story_row("t3");
    store
        .insert_rows("error_stories", &[slow, error_only])
        .await
        .unwrap();

    let eps = store
        .endpoint_stats(60, &EndpointCaps::default())
        .await
        .unwrap();
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].kept, 59, "slow-story trace excluded");
    assert_eq!(eps[0].seen, 60, "slow-story trace still counted as seen");
    let ops = store.op_stats(60, &EndpointCaps::default()).await.unwrap();
    let root = ops.iter().find(|o| o.op == "frontend:GET").unwrap();
    assert_eq!(root.present, 59, "presence denominator matches traces");
    let cart = ops.iter().find(|o| o.op == "cart:GetCart").unwrap();
    assert_eq!(
        cart.present, 29,
        "slow-story trace excluded from op presence"
    );
    store
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

const MS_NS: u64 = 1_000_000;

fn timed_row(service: &str, name: &str, id: &str, duration_ms: u64) -> TraceSummaryRow {
    TraceSummaryRow {
        trace_id: format!("{service}-{id}"),
        ts: now_ns(),
        endpoint_service: service.into(),
        endpoint_name: name.into(),
        duration_ns: duration_ms * MS_NS,
        is_error: 0,
        op_durations: vec![(format!("{service}:op"), duration_ms * MS_NS / 2)],
        span_count: 2,
    }
}

/// Endpoint `frontend` GET /: 64 traces of 100..=150 ms plus one 5 s outlier.
/// Endpoint `cart` Get: 64 traces of 10..=20 ms, nothing unusual.
async fn seed_outlier_endpoints(store: &Store) {
    let mut rows: Vec<TraceSummaryRow> = (0..64)
        .map(|i| timed_row("frontend", "GET /", &i.to_string(), 100 + i % 51))
        .collect();
    rows.push(timed_row("frontend", "GET /", "outlier", 5_000));
    rows.extend((0..64).map(|i| timed_row("cart", "Get", &i.to_string(), 10 + i % 11)));
    store.insert_rows("trace_summaries", &rows).await.unwrap();
}

fn stats_of<'a>(
    eps: &'a [tayga_store::rows::EndpointStatsRow],
    service: &str,
) -> &'a tayga_store::rows::EndpointStatsRow {
    eps.iter().find(|e| e.endpoint_service == service).unwrap()
}

/// The previous refresh's limit for an endpoint caps this refresh, so one missed outlier cannot
/// raise p99. The uncapped endpoint is unchanged and `op_stats` uses the same trace set.
#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn one_outlier_does_not_raise_p99_with_previous_cap() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    seed_outlier_endpoints(&store).await;

    let none = EndpointCaps::default();
    let before = store.endpoint_stats(60, &none).await.unwrap();
    // No previous baseline: the 10 x p50 bootstrap cap already drops the outlier.
    assert_eq!(stats_of(&before, "frontend").kept, 64);

    let caps = EndpointCaps {
        keys: vec!["frontend\0GET /".into(), "cart\0Get".into()],
        caps_ns: vec![250 * MS_NS, 1_000 * MS_NS],
    };
    let eps = store.endpoint_stats(60, &caps).await.unwrap();
    let frontend = stats_of(&eps, "frontend");
    assert_eq!(frontend.kept, 64);
    assert!(frontend.p99 <= (150 * MS_NS) as f64, "p99 {}", frontend.p99);
    assert_eq!(stats_of(&eps, "cart"), stats_of(&before, "cart"));
    assert_eq!(frontend.excluded, 1);

    // A tight cap proves the bound key matches (the bootstrap alone would keep these).
    let tight = EndpointCaps {
        keys: vec!["frontend\0GET /".into()],
        caps_ns: vec![120 * MS_NS],
    };
    let tight_eps = store.endpoint_stats(60, &tight).await.unwrap();
    assert!(stats_of(&tight_eps, "frontend").kept < 64);
    assert_eq!(stats_of(&tight_eps, "cart").kept, 64);
    assert!(stats_of(&tight_eps, "frontend").excluded > 1);

    let ops = store.op_stats(60, &caps).await.unwrap();
    let present = |service: &str| {
        ops.iter()
            .find(|o| o.endpoint_service == service)
            .unwrap()
            .present
    };
    assert_eq!(present("frontend"), 64, "op presence uses the capped set");
    assert_eq!(present("cart"), 64);
    let frontend_op = ops
        .iter()
        .find(|o| o.endpoint_service == "frontend")
        .unwrap();
    assert!(frontend_op.p95 <= (75 * MS_NS) as f64);
    store
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

/// A brand-new endpoint has no previous limit: its first baseline is capped at 10 x p50.
#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn bootstrap_cap_is_ten_times_p50() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    seed_outlier_endpoints(&store).await;

    let none = EndpointCaps::default();
    let eps = store.endpoint_stats(60, &none).await.unwrap();
    let frontend = stats_of(&eps, "frontend");
    assert_eq!(frontend.kept, 64, "5 s outlier is above 10 x p50");
    assert!(frontend.p99 <= (150 * MS_NS) as f64, "p99 {}", frontend.p99);
    assert_eq!(stats_of(&eps, "cart").kept, 64);
    assert_eq!(frontend.excluded, 1);
    let ops = store.op_stats(60, &none).await.unwrap();
    assert!(
        ops.iter()
            .filter(|o| o.endpoint_service == "frontend")
            .all(|o| o.present == 64)
    );
    store
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

/// An endpoint whose traces are all above the cap (or slow-storied) still reports `seen`.
#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn all_traces_above_cap_still_reported() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let rows: Vec<TraceSummaryRow> = (0..65)
        .map(|i| timed_row("frontend", "GET /", &i.to_string(), 5_000))
        .collect();
    store.insert_rows("trace_summaries", &rows).await.unwrap();
    let caps = EndpointCaps {
        keys: vec!["frontend\0GET /".into()],
        caps_ns: vec![250 * MS_NS],
    };
    let eps = store.endpoint_stats(60, &caps).await.unwrap();
    assert_eq!(eps.len(), 1);
    let e = &eps[0];
    assert_eq!((e.seen, e.kept, e.excluded), (65, 0, 65));
    assert!(store.op_stats(60, &caps).await.unwrap().is_empty());

    let stories: Vec<StoryRow> = (0..65)
        .map(|i| StoryRow {
            kind: 2,
            ..story_row(&format!("frontend-{i}"))
        })
        .collect();
    store.insert_rows("error_stories", &stories).await.unwrap();
    let eps = store.endpoint_stats(60, &caps).await.unwrap();
    let e = &eps[0];
    assert_eq!(
        (e.seen, e.kept, e.excluded),
        (65, 0, 0),
        "slow-storied, not capped"
    );
    store
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

use tayga_store::logs::{LogAlertRow, LogHitRow, LogTemplateRow};

const MIN_NS: i64 = 60 * 1_000_000_000;

async fn log_store() -> (ClickHouseSettings, Store) {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    (s, store)
}

async fn drop_db(s: &ClickHouseSettings, store: &Store) {
    store
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

fn template(id: u64, service: &str, first_seen: i64) -> LogTemplateRow {
    LogTemplateRow {
        template_id: id,
        service: service.into(),
        template: format!("template {id} <*>"),
        first_seen,
        last_seen: now_ns(),
        count: 1,
        max_severity: 9,
        sample: "sample".into(),
        version: 1,
    }
}

fn hit(log_id: u64, template_id: u64, ts: i64, trace: &str) -> LogHitRow {
    LogHitRow {
        log_id,
        template_id,
        service: "checkout".into(),
        ts,
        severity_number: 9,
        trace_id: trace.into(),
        span_id: String::new(),
    }
}

fn alert(id: &str, kind: i8, template_id: u64, last_at: i64) -> LogAlertRow {
    LogAlertRow {
        alert_id: id.into(),
        kind,
        template_id,
        service: "checkout".into(),
        template: "t".into(),
        started_at: last_at,
        last_at,
        window_count: 12,
        peak_count: 12,
        baseline_per_window: 1.5,
        example_trace_ids: vec!["tr1".into()],
        version: 1,
        baseline_day: Some(3.0),
        baseline_week: None,
    }
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn hits_are_idempotent_by_log_id() {
    let (s, store) = log_store().await;
    let hits: Vec<LogHitRow> = (1..=3)
        .map(|i| hit(i, 7, now_ns() - i as i64, ""))
        .collect();
    store.insert_log_hits(&hits).await.unwrap();
    store.insert_log_hits(&hits).await.unwrap();
    let n: u64 = store
        .client()
        .query("SELECT uniqExact(log_id) FROM log_template_hits WHERE template_id = 7")
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(n, 3);
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn template_windows_counts_current_and_baseline() {
    let (s, store) = log_store().await;
    let now = now_ns();
    store
        .upsert_templates(&[template(1, "checkout", now - 120 * MIN_NS)])
        .await
        .unwrap();
    let mut hits = Vec::new();
    let mut id = 0;
    for i in 0..12 {
        id += 1;
        hits.push(hit(id, 1, now - MIN_NS - i * 1_000_000, ""));
    }
    for i in 0..6 {
        id += 1;
        hits.push(hit(id, 1, now - (30 + i) * MIN_NS, ""));
    }
    for i in 0..4 {
        id += 1;
        hits.push(hit(id, 1, now - 120 * MIN_NS - i, ""));
    }
    store.insert_log_hits(&hits).await.unwrap();

    let w = store.template_windows(5, 60, 10).await.unwrap();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].template_id, 1);
    assert_eq!(w[0].service, "checkout");
    assert_eq!(w[0].first_seen_ns, now - 120 * MIN_NS);
    assert_eq!(w[0].current, 12);
    assert_eq!(w[0].baseline_total, 6);
    assert!(store.template_windows(5, 60, 13).await.unwrap().is_empty());
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn covered_minute_buckets_are_distinct_minutes_with_any_hit() {
    let (s, store) = log_store().await;
    let now = now_ns();
    assert!(
        store
            .covered_minute_buckets(5, 60)
            .await
            .unwrap()
            .is_empty()
    );
    let at = |m: i64| now - m * MIN_NS - MIN_NS / 2;
    let mut hits = Vec::new();
    let mut id = 0;
    // Template 1 covers baseline minutes 6..=15, template 2 minutes 46..=55: a 30-minute gap.
    for m in 6..=15 {
        id += 1;
        hits.push(hit(id, 1, at(m), ""));
    }
    for m in 46..=55 {
        id += 1;
        hits.push(hit(id, 2, at(m), ""));
        id += 1;
        hits.push(hit(id, 2, at(m) + 1_000, "")); // same minute: counted once
    }
    // Outside the baseline window: the spike window and before it.
    id += 1;
    hits.push(hit(id, 1, at(1), ""));
    id += 1;
    hits.push(hit(id, 1, at(70), ""));
    store.insert_log_hits(&hits).await.unwrap();
    let buckets = store.covered_minute_buckets(5, 60).await.unwrap();
    assert_eq!(buckets.len(), 20);
    assert!(
        buckets.windows(2).all(|w| w[0] < w[1]),
        "ascending, distinct"
    );
    let now_min = now / MIN_NS;
    assert!(buckets.iter().all(|&m| m > now_min - 66 && m < now_min - 4));
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn new_candidates_and_alerts() {
    let (s, store) = log_store().await;
    let now = now_ns();
    let old_first = now - 60 * MIN_NS;
    store
        .upsert_templates(&[
            template(1, "checkout", old_first),
            template(2, "checkout", now - 2 * MIN_NS),
        ])
        .await
        .unwrap();
    let since = now - 10 * MIN_NS;
    let c = store.new_template_candidates(since).await.unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].template_id, 2);
    assert_eq!(c[0].service_oldest_ns, old_first);
    let first = c[0].first_seen_ns;
    assert!(
        store
            .new_template_candidates(first)
            .await
            .unwrap()
            .is_empty(),
        "the bound is exclusive"
    );
    assert_eq!(
        store
            .new_template_candidates(first - 1)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store.new_template_candidates(0).await.unwrap().len(),
        2,
        "the bound is in data time, not a wall-clock window"
    );

    store
        .insert_alerts(&[alert("new:2", 1, 2, now)])
        .await
        .unwrap();
    assert!(
        store
            .new_template_candidates(0)
            .await
            .unwrap()
            .iter()
            .all(|r| r.template_id == 1)
    );
    assert!(
        store
            .new_template_candidates(since)
            .await
            .unwrap()
            .is_empty()
    );
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn data_now_is_the_latest_recent_hit_or_zero() {
    let (s, store) = log_store().await;
    assert_eq!(store.data_now_ns().await.unwrap(), 0, "no hits");
    let now = now_ns();
    store
        .insert_log_hits(&[
            hit(1, 1, now - 30 * MIN_NS, ""),
            hit(2, 1, now - 5 * MIN_NS, ""),
            hit(3, 1, now + 60 * MIN_NS, ""),
        ])
        .await
        .unwrap();
    assert_eq!(
        store.data_now_ns().await.unwrap(),
        now - 5 * MIN_NS,
        "a log stamped an hour ahead is ignored"
    );
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn logminer_state_round_trips_and_keeps_latest() {
    let (s, store) = log_store().await;
    assert_eq!(store.state_get("k").await.unwrap(), None);
    store.state_put("k", 1).await.unwrap();
    store.state_put("k", 2).await.unwrap();
    store.state_put("other", 9).await.unwrap();
    assert_eq!(store.state_get("k").await.unwrap(), Some(2));
    assert_eq!(store.state_get("other").await.unwrap(), Some(9));
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn templates_upsert_latest_version_wins() {
    let (s, store) = log_store().await;
    let now = now_ns();
    let v1 = template(1, "checkout", now - MIN_NS);
    let v2 = LogTemplateRow {
        count: 50,
        version: 2,
        ..v1.clone()
    };
    store
        .upsert_templates(std::slice::from_ref(&v2))
        .await
        .unwrap();
    store.upsert_templates(&[v1]).await.unwrap();
    assert_eq!(store.load_templates().await.unwrap(), vec![v2]);
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn active_spike_alerts_filters_by_last_at() {
    let (s, store) = log_store().await;
    let now = now_ns();
    let fresh = alert("spike:1", 2, 1, now - MIN_NS);
    store
        .insert_alerts(&[
            fresh.clone(),
            alert("spike:2", 2, 2, now - 30 * MIN_NS),
            alert("new:3", 1, 3, now),
        ])
        .await
        .unwrap();
    assert_eq!(store.active_spike_alerts(10).await.unwrap(), vec![fresh]);
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn example_traces_are_distinct_newest_first_and_skip_empty() {
    let (s, store) = log_store().await;
    let now = now_ns();
    store
        .insert_log_hits(&[
            hit(1, 1, now - 5 * MIN_NS, "ta"),
            hit(2, 1, now - 4 * MIN_NS, "tb"),
            hit(3, 1, now - 3 * MIN_NS, ""),
            hit(4, 1, now - 2 * MIN_NS, "ta"),
            hit(5, 2, now - MIN_NS, "other"),
            hit(6, 1, now - 120 * MIN_NS, "old"),
        ])
        .await
        .unwrap();
    assert_eq!(
        store.example_traces(1, 60, 10).await.unwrap(),
        vec!["ta".to_string(), "tb".to_string()]
    );
    assert_eq!(store.example_traces(1, 60, 1).await.unwrap().len(), 1);
    drop_db(&s, &store).await;
}

use tayga_store::metrics_store::{MetricPointRow, MetricSampleRow};

fn sample(
    ts: i64,
    job: &str,
    metric: &str,
    labels: &[(&str, &str)],
    value: f64,
) -> MetricSampleRow {
    MetricSampleRow {
        ts,
        job: job.into(),
        metric: metric.into(),
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        value,
    }
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn metric_samples_roundtrip_through_metric_buckets() {
    let (s, store) = log_store().await;
    let now_ms = now_ns() / 1_000_000;
    // One-second buckets: each sample below is its own point, at its second's start.
    let sec = |ms: i64| ms / 1000 * 1000;
    let m = "tayga_writer_rows_inserted_total";
    let le = "tayga_writer_batch_seconds_bucket";
    store
        .insert_metric_samples(&[
            sample(
                now_ms - 2_000,
                "tayga-writer",
                m,
                &[("kind", "spans")],
                10.0,
            ),
            sample(
                now_ms - 1_000,
                "tayga-writer",
                m,
                &[("kind", "spans")],
                25.0,
            ),
            sample(now_ms - 1_000, "other-job", m, &[("kind", "spans")], 7.0),
            // Outside a 60 s window.
            sample(
                now_ms - 3_600_000,
                "tayga-writer",
                m,
                &[("kind", "spans")],
                1.0,
            ),
            sample(now_ms - 1_000, "tayga-writer", "up", &[], 1.0),
            sample(now_ms - 2_000, "tayga-writer", le, &[("le", "+Inf")], 4.0),
            // A non-finite value is stored but never read back as a point.
            sample(
                now_ms - 1_000,
                "tayga-writer",
                le,
                &[("le", "+Inf")],
                f64::INFINITY,
            ),
        ])
        .await
        .unwrap();

    // Windows ending just after now, `secs` long.
    let end = now_ms / 1000 + 1;
    let last = |secs: i64| (end - secs, end);
    let writer = store
        .metric_buckets(Some("tayga-writer"), m, &[], last(60), 1)
        .await
        .unwrap();
    assert_eq!(
        writer,
        vec![
            MetricPointRow {
                ts_ms: sec(now_ms - 2_000),
                job: "tayga-writer".into(),
                labels: vec![("kind".into(), "spans".into())],
                value: 10.0,
            },
            MetricPointRow {
                ts_ms: sec(now_ms - 1_000),
                job: "tayga-writer".into(),
                labels: vec![("kind".into(), "spans".into())],
                value: 25.0,
            },
        ]
    );

    let all = store
        .metric_buckets(None, m, &[], last(60), 1)
        .await
        .unwrap();
    assert_eq!(all.len(), 3, "both jobs, old sample excluded: {all:?}");
    assert!(all.iter().any(|p| p.job == "other-job"));

    let wide = store
        .metric_buckets(Some("tayga-writer"), m, &[], last(7_200), 1)
        .await
        .unwrap();
    assert_eq!(wide.len(), 3);

    let inf = store
        .metric_buckets(None, le, &[], last(60), 1)
        .await
        .unwrap();
    assert_eq!(inf.len(), 1, "the infinite sample is dropped: {inf:?}");
    assert_eq!(inf[0].value, 4.0);
    assert_eq!(inf[0].labels, vec![("le".to_string(), "+Inf".to_string())]);

    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn metric_buckets_keep_the_last_value_per_series_and_step() {
    let (s, store) = log_store().await;
    let step = 60_i64;
    // The start of a bucket comfortably inside the window, so the points below share it.
    let now_ms = now_ns() / 1_000_000;
    let b0 = (now_ms / 1000 - 300) / step * step * 1000;
    let b1 = b0 + step * 1000;
    let m = "tayga_writer_rows_inserted_total";
    let spans = [("kind", "spans")];
    let logs = [("kind", "logs")];
    store
        .insert_metric_samples(&[
            // Bucket 0: the later sample (15) wins; NaN never wins.
            sample(b0 + 1_000, "tayga-writer", m, &spans, 10.0),
            sample(b0 + 30_000, "tayga-writer", m, &spans, 15.0),
            sample(b0 + 45_000, "tayga-writer", m, &spans, f64::NAN),
            sample(b0 + 2_000, "tayga-writer", m, &logs, 100.0),
            // Bucket 1: exactly at its start, then later (the later one wins).
            sample(b1, "tayga-writer", m, &spans, 33.0),
            sample(b1 + 5_000, "tayga-writer", m, &spans, 40.0),
            sample(b1 + 5_000, "other-job", m, &spans, 7.0),
            // Outside a 30 min window.
            sample(now_ms - 7_200_000, "tayga-writer", m, &spans, 1.0),
        ])
        .await
        .unwrap();

    // A 30 min window ending after now; its start is 7 s past a minute, and the buckets still
    // lie on the epoch grid.
    let end = b0 / 1000 + 6 * step + 7;
    let w30 = (end - 1800, end);
    let all = store
        .metric_buckets(Some("tayga-writer"), m, &[], w30, 60)
        .await
        .unwrap();
    let pt = |ts_ms, labels: &[(&str, &str)], value| MetricPointRow {
        ts_ms,
        job: "tayga-writer".into(),
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        value,
    };
    let mut got = all.clone();
    got.sort_by(|a, b| (a.ts_ms, &a.labels).cmp(&(b.ts_ms, &b.labels)));
    assert_eq!(
        got,
        vec![
            pt(b0, &logs, 100.0),
            pt(b0, &spans, 15.0),
            pt(b1, &spans, 40.0)
        ]
    );
    assert!(
        all.windows(2).all(|w| w[0].ts_ms <= w[1].ts_ms),
        "oldest first"
    );

    let filtered = store
        .metric_buckets(None, m, &[("kind".into(), "spans".into())], w30, 60)
        .await
        .unwrap();
    assert_eq!(filtered.len(), 3, "both jobs, spans only: {filtered:?}");
    assert!(filtered.iter().any(|p| p.job == "other-job"));
    // A label value with a quote is bound, not interpolated.
    assert!(
        store
            .metric_buckets(None, m, &[("kind".into(), "' OR 1=1 --".into())], w30, 60)
            .await
            .unwrap()
            .is_empty()
    );
    let wide_start = end - 3 * 3600;
    let wide = store
        .metric_buckets(Some("tayga-writer"), m, &[], (wide_start, end), 3600)
        .await
        .unwrap();
    assert!(
        wide.iter().any(|p| p.value == 1.0),
        "the 2h-old sample is in a 3h window"
    );
    assert!(
        wide.iter().all(|p| p.ts_ms % 3_600_000 == 0),
        "buckets lie on the epoch grid"
    );
    // A window in the past that ends exactly at `b1`: the sample at `b1` is outside it (the end
    // is exclusive), and every point lies in bucket 0 although the window starts mid-minute.
    let past_end = b0 / 1000 + step;
    let past = store
        .metric_buckets(Some("tayga-writer"), m, &[], (past_end - 90, past_end), 60)
        .await
        .unwrap();
    let mut got = past.clone();
    got.sort_by(|a, b| (a.ts_ms, &a.labels).cmp(&(b.ts_ms, &b.labels)));
    assert_eq!(
        got,
        vec![pt(b0, &logs, 100.0), pt(b0, &spans, 15.0),],
        "only samples before the window's end, in epoch-aligned buckets"
    );

    drop_db(&s, &store).await;
}

const DAY_SECS: u32 = 86_400;
const WEEK_SECS: u32 = 7 * DAY_SECS;

fn shifted(shift_secs: u32, back_min: i64) -> i64 {
    now_ns() - i64::from(shift_secs) * 1_000_000_000 - back_min * MIN_NS
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn minutes_mv_does_not_double_count_replayed_hits() {
    let (s, store) = log_store().await;
    // Ten hits inside one past minute (mid-minute, so none crosses a boundary).
    let minute_start = (now_ns() - 10 * MIN_NS).div_euclid(MIN_NS) * MIN_NS;
    let batch = |ids: std::ops::RangeInclusive<u64>| -> Vec<LogHitRow> {
        ids.map(|i| hit(i, 7, minute_start + 20_000_000_000 + i as i64, ""))
            .collect()
    };
    let merged = |store: &Store| {
        let c = store.client().clone();
        async move {
            c.query(
                "SELECT count(), uniqExactMerge(hits) FROM log_template_minutes \
                 WHERE template_id = 7 AND minute = fromUnixTimestamp(?)",
            )
            .bind(minute_start / 1_000_000_000)
            .fetch_one::<(u64, u64)>()
            .await
            .unwrap()
        }
    };
    store.insert_log_hits(&batch(1..=10)).await.unwrap();
    assert_eq!(merged(&store).await, (1, 10));
    // The same 10 log_ids again: a second state row lands in the table (the MV fires per
    // insert), but the merged distinct count stays 10.
    store.insert_log_hits(&batch(1..=10)).await.unwrap();
    let (state_rows, distinct) = merged(&store).await;
    assert_eq!(state_rows, 2, "each insert block adds a state row");
    assert_eq!(distinct, 10, "uniqExactMerge dedups the replay");
    // A partial replay with new ids only adds the new ones.
    store.insert_log_hits(&batch(6..=15)).await.unwrap();
    assert_eq!(merged(&store).await.1, 15);
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn seasonal_counts_cover_shifted_windows() {
    let (s, store) = log_store().await;
    let mut hits = Vec::new();
    // Template 1: 10 hits one day ago (inserted twice below), 4 hits a week ago.
    for i in 1..=10u64 {
        hits.push(hit(i, 1, shifted(DAY_SECS, 3) + i as i64, ""));
    }
    // Template 3 is active in the day window only, so template 2 has coverage but no row.
    for i in 101..=102u64 {
        hits.push(hit(i, 3, shifted(DAY_SECS, 2) + i as i64, ""));
    }
    store.insert_log_hits(&hits).await.unwrap();
    store.insert_log_hits(&hits).await.unwrap(); // replay
    // A hit outside the day window (30 minutes before it) must not count.
    store
        .insert_log_hits(&[hit(201, 1, shifted(DAY_SECS, 30), "")])
        .await
        .unwrap();

    let w = store
        .seasonal_counts(&[1, 2], 5, &[DAY_SECS, WEEK_SECS])
        .await
        .unwrap();
    assert_eq!(w.len(), 2);
    assert_eq!((w[0].shift_secs, w[0].covered), (DAY_SECS, true));
    assert_eq!(
        w[0].counts,
        vec![(1, 10), (2, 0)],
        "replay deduped, no row in a covered window is 0"
    );
    assert_eq!((w[1].shift_secs, w[1].covered), (WEEK_SECS, false));
    assert!(
        w[1].counts.is_empty(),
        "no rows at all: no coverage, no counts"
    );

    // Seed the week window: only template 3 and template 1 (4 hits).
    let mut week = vec![hit(301, 3, shifted(WEEK_SECS, 3), "")];
    for i in 1..=4u64 {
        week.push(hit(310 + i, 1, shifted(WEEK_SECS, 4) + i as i64, ""));
    }
    store.insert_log_hits(&week).await.unwrap();
    let w = store
        .seasonal_counts(&[1, 2], 5, &[DAY_SECS, WEEK_SECS])
        .await
        .unwrap();
    assert_eq!(w[0].counts, vec![(1, 10), (2, 0)]);
    assert!(w[1].covered);
    assert_eq!(w[1].counts, vec![(1, 4), (2, 0)]);
    assert!(
        store.seasonal_counts(&[], 5, &[DAY_SECS]).await.unwrap()[0]
            .counts
            .is_empty()
    );
    drop_db(&s, &store).await;
}

const BACKFILL_SQL: &str = include_str!("../migrations/0009_backfill_log_template_minutes.sql");
const MINUTES_MV_SQL: &str = "CREATE MATERIALIZED VIEW IF NOT EXISTS log_template_minutes_mv \
     TO log_template_minutes AS \
     SELECT template_id, toStartOfMinute(ts) AS minute, uniqExactState(log_id) AS hits \
     FROM log_template_hits GROUP BY template_id, minute";

/// `(state rows, distinct log_ids)` per minute offset (minutes after `base_ns`) for template 7.
async fn minute_counts(store: &Store, base_ns: i64) -> Vec<(i64, u64, u64)> {
    store
        .client()
        .query(
            "SELECT intDiv(toUnixTimestamp(minute) - ?, 60) AS m, count(), uniqExactMerge(hits) \
             FROM log_template_minutes WHERE template_id = 7 GROUP BY m ORDER BY m",
        )
        .bind(base_ns / 1_000_000_000)
        .fetch_all::<(i64, u64, u64)>()
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn backfill_fills_minutes_before_the_view_without_double_counting() {
    let (s, store) = log_store().await;
    let c = store.client().clone();
    let base = (now_ns() - 40 * MIN_NS).div_euclid(MIN_NS) * MIN_NS;
    let at = |minute: i64, i: u64| base + minute * MIN_NS + 1_000_000_000 + i as i64;
    let hits = |minute: i64, ids: std::ops::RangeInclusive<u64>| -> Vec<LogHitRow> {
        ids.map(|i| hit(i, 7, at(minute, i), "")).collect()
    };
    // Upgrade shape: hits from before the view existed (minutes 0 and 10, and the first half of
    // minute 20), then the view is created mid-minute 20 and sees the rest.
    c.query("DROP VIEW log_template_minutes_mv")
        .execute()
        .await
        .unwrap();
    for h in [hits(0, 1..=4), hits(10, 11..=13), hits(20, 21..=25)] {
        store.insert_log_hits(&h).await.unwrap();
    }
    c.query(MINUTES_MV_SQL).execute().await.unwrap();
    store.insert_log_hits(&hits(20, 26..=30)).await.unwrap();
    store.insert_log_hits(&hits(30, 31..=33)).await.unwrap();
    assert_eq!(
        minute_counts(&store, base).await,
        vec![(20, 1, 5), (30, 1, 3)],
        "the view saw only its own inserts"
    );

    c.query(BACKFILL_SQL).execute().await.unwrap();
    let want = vec![(0, 1, 4), (10, 1, 3), (20, 2, 10), (30, 1, 3)];
    assert_eq!(
        minute_counts(&store, base).await,
        want,
        "older minutes filled, the boundary minute completed, later minutes not re-inserted"
    );
    // A re-run only re-reads the (new) earliest minute; uniqExactMerge counts its ids once.
    c.query(BACKFILL_SQL).execute().await.unwrap();
    let rerun = minute_counts(&store, base).await;
    assert_eq!(
        rerun[0],
        (0, 2, 4),
        "a second state row, same distinct count"
    );
    assert_eq!(&rerun[1..], &want[1..]);
    // After background merges the states combine and the distinct counts are unchanged.
    c.query("OPTIMIZE TABLE log_template_minutes FINAL")
        .execute()
        .await
        .unwrap();
    let merged: Vec<(i64, u64)> = minute_counts(&store, base)
        .await
        .into_iter()
        .map(|(m, _, n)| (m, n))
        .collect();
    assert_eq!(merged, vec![(0, 4), (10, 3), (20, 10), (30, 3)]);

    // An empty minutes table (no hit since the view was created) backfills every hit.
    c.query("TRUNCATE TABLE log_template_minutes")
        .execute()
        .await
        .unwrap();
    c.query(BACKFILL_SQL).execute().await.unwrap();
    let all: Vec<(i64, u64)> = minute_counts(&store, base)
        .await
        .into_iter()
        .map(|(m, _, n)| (m, n))
        .collect();
    assert_eq!(all, vec![(0, 4), (10, 3), (20, 10), (30, 3)]);
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn silence_settings_round_trip_and_latest_wins() {
    let (s, store) = log_store().await;
    assert_eq!(store.silence_get(1).await.unwrap(), None);
    store.silence_put(1, true, 10).await.unwrap();
    store.silence_put(1, true, 30).await.unwrap();
    store.silence_put(2, true, 5).await.unwrap();
    store.silence_put(2, false, 5).await.unwrap();
    store.silence_put(3, true, 1440).await.unwrap();
    assert_eq!(store.silence_get(1).await.unwrap(), Some((true, 30)));
    assert_eq!(store.silence_get(2).await.unwrap(), Some((false, 5)));
    assert_eq!(store.silence_get(99).await.unwrap(), None);
    assert_eq!(
        store.silence_enabled().await.unwrap(),
        vec![(1, 30), (3, 1440)]
    );
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn silence_inputs_read_template_and_service_last_hit() {
    let (s, store) = log_store().await;
    let now = now_ns();
    store
        .upsert_templates(&[
            template(1, "checkout", now - 60 * MIN_NS),
            template(2, "checkout", now - 50 * MIN_NS),
            template(3, "checkout", now - 40 * MIN_NS),
            template(4, "ghost", now - 30 * MIN_NS),
        ])
        .await
        .unwrap();
    store
        .insert_log_hits(&[
            hit(1, 1, now - 20 * MIN_NS, ""),
            hit(2, 1, now - 15 * MIN_NS, ""),
            hit(3, 2, now - MIN_NS, ""),
        ])
        .await
        .unwrap();
    let mut got = store.silence_inputs(&[1, 3, 4, 77]).await.unwrap();
    got.sort_by_key(|i| i.template_id);
    let ns = |v: i64| Some(v);
    assert_eq!(got.len(), 3, "unknown id 77 is left out");
    assert_eq!(got[0].template_id, 1);
    assert_eq!(got[0].service, "checkout");
    assert_eq!(got[0].t_last_ns, ns(now - 15 * MIN_NS));
    assert_eq!(got[0].s_last_ns, ns(now - MIN_NS));
    assert_eq!(got[0].first_seen_ns, now - 60 * MIN_NS);
    // Template without hits in the hits TTL: falls back to its own `last_seen` (the helper
    // stamps it at insert time), so a long silence keeps one anchor.
    assert_eq!(got[1].template_id, 3);
    assert!(
        got[1].t_last_ns.is_some_and(|v| v >= now),
        "{:?}",
        got[1].t_last_ns
    );
    assert_eq!(got[1].s_last_ns, ns(now - MIN_NS));
    // Service without any hit: no `s_last`, so it is never judged silent.
    assert_eq!(got[2].template_id, 4);
    assert!(got[2].t_last_ns.is_some_and(|v| v >= now));
    assert_eq!(got[2].s_last_ns, None);
    assert!(store.silence_inputs(&[]).await.unwrap().is_empty());
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn enum_migration_keeps_old_alert_rows_and_accepts_silence() {
    let s = settings();
    let server = clickhouse::Client::default().with_url(&s.url);
    server
        .query(&format!("CREATE DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
    let db = server.with_database(&s.database);
    // Schema as of migration 0009 (log_alerts with the two-kind enum), then old rows.
    for stmt in [
        include_str!("../migrations/0004_log_templates.sql"),
        include_str!("../migrations/0008_log_alerts_seasonal.sql"),
    ] {
        for q in tayga_store::migrate::split_statements(stmt) {
            db.query(&q).execute().await.unwrap();
        }
    }
    db.query("CREATE TABLE schema_migrations (version UInt32, applied_at DateTime DEFAULT now()) ENGINE = MergeTree ORDER BY version")
        .execute()
        .await
        .unwrap();
    for v in 1..=9u32 {
        db.query("INSERT INTO schema_migrations (version) VALUES (?)")
            .bind(v)
            .execute()
            .await
            .unwrap();
    }
    let store = Store::new(&s);
    let now = now_ns();
    store
        .insert_alerts(&[alert("new:1", 1, 1, now), alert("spike:1", 2, 1, now)])
        .await
        .unwrap();

    assert_eq!(migrate(&s).await.unwrap(), vec![10, 11]);
    let kinds: Vec<(String, i8)> = store
        .client()
        .query("SELECT alert_id, kind FROM log_alerts FINAL ORDER BY alert_id")
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(kinds, vec![("new:1".into(), 1), ("spike:1".into(), 2)]);
    // The client caches the table schema per instance: a fresh one sees the widened enum.
    let store = Store::new(&s);
    store
        .insert_alerts(&[alert("silence:1", 3, 1, now)])
        .await
        .unwrap();
    let silence: Vec<String> = store
        .client()
        .query("SELECT toString(kind) FROM log_alerts FINAL WHERE kind = 'silence'")
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(silence, vec!["silence".to_string()]);

    // A partially applied 0010 (statements ran, version not recorded) re-runs cleanly.
    db_exec_all(
        &store,
        include_str!("../migrations/0010_log_template_silence.sql"),
    )
    .await;
    drop_db(&s, &store).await;
}

async fn db_exec_all(store: &Store, sql: &str) {
    for q in tayga_store::migrate::split_statements(sql) {
        store.client().query(&q).execute().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires ClickHouse: run against the live stack"]
async fn deliveries_round_trip_and_latest_wins() {
    use tayga_store::notifier::{DeliveryRow, STATUS_DELIVERED, STATUS_PENDING};
    let (s, store) = log_store().await;
    assert_eq!(store.delivery_get("a1", "slack").await.unwrap(), None);
    let row = |status, attempts, err: &str| DeliveryRow {
        alert_id: "a1".into(),
        target: "slack".into(),
        status,
        attempts,
        last_error: err.into(),
    };
    store
        .delivery_put(&row(STATUS_PENDING, 1, "http 503"))
        .await
        .unwrap();
    store
        .delivery_put(&row(STATUS_DELIVERED, 2, ""))
        .await
        .unwrap();
    store
        .delivery_put(&DeliveryRow {
            target: "hook".into(),
            ..row(STATUS_PENDING, 1, "")
        })
        .await
        .unwrap();
    assert_eq!(
        store.delivery_get("a1", "slack").await.unwrap(),
        Some(row(STATUS_DELIVERED, 2, ""))
    );
    assert_eq!(
        store
            .delivery_get("a1", "hook")
            .await
            .unwrap()
            .unwrap()
            .status,
        STATUS_PENDING
    );
    assert_eq!(store.delivery_get("a2", "slack").await.unwrap(), None);
    drop_db(&s, &store).await;
}
