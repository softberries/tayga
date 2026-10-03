//! ChRepo against a real ClickHouse. Self-seeding: it migrates a fresh, uniquely named database,
//! inserts its own rows with random ids and `now`-based timestamps, reads them back and drops
//! the database. Runs on the empty `make it` ClickHouse and against the live stack alike.

use tayga_api::params::GroupFilter;
use tayga_api::repo::{ChRepo, Repo};
use tayga_store::ClickHouseSettings;
use tayga_store::migrate::migrate;
use tayga_store::rows::{LogRow, ServiceEdgeRow, SpanRow, StoryRow};
use tayga_store::store::Store;

fn settings() -> ClickHouseSettings {
    let url =
        std::env::var("TAYGA_IT_CLICKHOUSE").unwrap_or_else(|_| "http://localhost:18123".into());
    let suffix: u32 = rand::random();
    ClickHouseSettings {
        url,
        database: format!("tayga_api_it_{suffix}"),
    }
}

fn hex32() -> String {
    format!("{:032x}", rand::random::<u128>())
}

/// Rows must be recent: the tables carry TTLs and every read filters on a `since` window.
fn now_ns() -> i64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch");
    i64::try_from(d.as_nanos()).expect("nanos fit i64")
}

fn span(trace_id: &str, span_id: &str, parent: &str, service: &str, start: i64) -> SpanRow {
    SpanRow {
        trace_id: trace_id.into(),
        span_id: span_id.into(),
        parent_span_id: parent.into(),
        service_name: service.into(),
        span_name: format!("{service} op"),
        kind: 2,
        start_ts: start,
        duration_ns: 1_000_000,
        status_code: 2,
        status_message: "failed".into(),
        resource_attrs: vec![("service.name".into(), service.into())],
        span_attrs: vec![],
        events_ts: vec![],
        events_name: vec![],
        events_attrs: vec![],
    }
}

#[tokio::test]
#[ignore = "requires ClickHouse: make it, or TAYGA_IT_CLICKHOUSE against the live stack"]
async fn reads_seeded_groups_story_trace_and_map() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let now = now_ns();
    let fingerprint: u64 = rand::random();
    let trace_id = hex32();
    let story_ids = [hex32(), hex32()];

    let stories: Vec<StoryRow> = story_ids
        .iter()
        .enumerate()
        .map(|(i, id)| StoryRow {
            story_id: id.clone(),
            fingerprint,
            kind: 1,
            ts: now - 1_000_000_000 * (i as i64 + 1),
            trace_id: trace_id.clone(),
            endpoint_service: "frontend".into(),
            endpoint_name: "POST /api/checkout".into(),
            rc_service: "payment".into(),
            rc_span_name: "Charge".into(),
            rc_span_kind: "server".into(),
            rc_message: "Invalid token".into(),
            rc_exception_type: String::new(),
            summary: "payment Charge failed: Invalid token".into(),
            duration_ns: 2_000_000,
            path_services: vec!["frontend".into(), "payment".into()],
            path_spans: "[]".into(),
            critical_path: "{}".into(),
            baseline_diff: String::new(),
            logs: "[]".into(),
            also_failed: "[]".into(),
            span_count: 2,
            flags: vec![],
            rc_span_id: "0000000000000002".into(),
        })
        .collect();
    store.insert_rows("error_stories", &stories).await.unwrap();
    let root = span(
        &trace_id,
        "0000000000000001",
        "",
        "frontend",
        now - 5_000_000,
    );
    let child = span(
        &trace_id,
        "0000000000000002",
        "0000000000000001",
        "payment",
        now - 4_000_000,
    );
    // The replayed root must collapse to one span on read.
    store
        .insert_spans(&[root.clone(), child, root])
        .await
        .unwrap();
    store
        .insert_logs(&[LogRow {
            log_id: rand::random(),
            ts: now - 4_000_000,
            observed_ts: now,
            trace_id: trace_id.clone(),
            span_id: "0000000000000002".into(),
            severity_number: 17,
            severity_text: "ERROR".into(),
            service_name: "payment".into(),
            body: "declined".into(),
            resource_attrs: vec![],
            log_attrs: vec![],
        }])
        .await
        .unwrap();
    let minute = u32::try_from(now / 1_000_000_000 / 60 * 60).unwrap();
    store
        .insert_rows(
            "service_edges",
            &[ServiceEdgeRow {
                minute,
                parent_service: "frontend".into(),
                child_service: "payment".into(),
                calls: 3,
                errors: 1,
                duration_ns_sum: 3_000_000,
            }],
        )
        .await
        .unwrap();

    let r = ChRepo::new(&s);
    let groups = r
        .story_groups(&GroupFilter {
            since_secs: 7 * 86_400,
            kind: None,
            service: None,
        })
        .await
        .unwrap();
    assert_eq!(groups.len(), 1);
    let g = &groups[0];
    assert_eq!(g.group.fingerprint, fingerprint.to_string());
    assert_eq!(g.group.stories, 2);
    assert_eq!(g.group.sample_story_id, story_ids[0]);
    assert_eq!(g.bucket_secs, 5040);
    assert_eq!(g.buckets.iter().map(|b| b.1).sum::<u64>(), 2);
    assert!(g.buckets.iter().all(|b| b.0 % 5040 == 0));

    let filtered = r
        .story_groups(&GroupFilter {
            since_secs: 3600,
            kind: Some("slow".into()),
            service: None,
        })
        .await
        .unwrap();
    assert!(filtered.is_empty(), "kind filter applies");

    let detail = r
        .story_group(&g.group.fingerprint, 3600)
        .await
        .unwrap()
        .expect("group exists");
    assert_eq!(detail.examples.len(), 2);
    assert!(r.story_group("1", 3600).await.unwrap().is_none());

    let story = r.story(&story_ids[0]).await.unwrap().expect("story exists");
    assert_eq!(story.fingerprint, g.group.fingerprint);
    assert_eq!(story.trace_id, trace_id);
    assert!(r.story(&"0".repeat(32)).await.unwrap().is_none());

    let trace = r.trace(&trace_id).await.unwrap();
    assert_eq!(trace.spans.len(), 2, "spans deduplicated");
    assert_eq!(trace.logs.len(), 1);

    let edges = r.service_map(3600).await.unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].calls, 3);

    Store::new(&s)
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}
