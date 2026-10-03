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
    assert_eq!(migrate(&s).await.unwrap(), vec![1, 2, 3]);
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
    assert_eq!(migrate(&s).await.unwrap(), vec![1, 2, 3]);
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
    assert_eq!(migrate(&s).await.unwrap(), vec![1, 2, 3]);
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
    assert_eq!(migrate(&s).await.unwrap(), vec![1, 2, 3]);
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
