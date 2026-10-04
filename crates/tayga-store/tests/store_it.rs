use tayga_store::ClickHouseSettings;
use tayga_store::migrate::migrate;
use tayga_store::rows::{LogRow, SpanRow};
use tayga_store::store::Store;

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
    assert_eq!(migrate(&s).await.unwrap(), vec![1, 2, 3, 4]);
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
    assert_eq!(migrate(&s).await.unwrap(), vec![1, 2, 3, 4]);
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

    let eps = store.endpoint_stats(60).await.unwrap();
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].traces, 60, "error trace excluded");
    let ops = store.op_stats(60).await.unwrap();
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
    assert_eq!(migrate(&s).await.unwrap(), vec![1, 2, 3, 4]);
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
    assert_eq!(migrate(&s).await.unwrap(), vec![1, 2, 3, 4]);
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

    let eps = store.endpoint_stats(60).await.unwrap();
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].traces, 59, "slow-story trace excluded");
    let ops = store.op_stats(60).await.unwrap();
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
