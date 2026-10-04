//! ChRepo against a real ClickHouse. Self-seeding: it migrates a fresh, uniquely named database,
//! inserts its own rows with random ids and `now`-based timestamps, reads them back and drops
//! the database. Runs on the empty `make it` ClickHouse and against the live stack alike.

use tayga_api::params::{AlertFilter, GroupFilter, TemplateFilter};
use tayga_api::repo::{ChRepo, Repo};
use tayga_store::ClickHouseSettings;
use tayga_store::logs::{LogAlertRow, LogHitRow, LogTemplateRow};
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
    let log_id: u64 = rand::random::<u32>().into();
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
            log_id,
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
    assert_eq!(trace.logs[0].log_id, log_id.to_string());

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

#[tokio::test]
#[ignore = "requires ClickHouse: make it, or TAYGA_IT_CLICKHOUSE against the live stack"]
async fn reads_seeded_log_templates_alerts_and_trace_links() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let now = now_ns();
    let min = 60_000_000_000_i64;
    let tmpl_a: u64 = rand::random();
    let tmpl_b: u64 = tmpl_a ^ 1;
    let (trace_story, trace_plain, trace_other) = (hex32(), hex32(), hex32());

    let template = |id: u64, service: &str, body: &str| LogTemplateRow {
        template_id: id,
        service: service.into(),
        template: body.into(),
        first_seen: now - 30 * min,
        last_seen: now,
        count: 1000,
        max_severity: 17,
        sample: body.replace("<*>", "42"),
        version: 1,
    };
    // A stale first version must lose to the newer one under FINAL.
    let mut stale = template(tmpl_a, "payment", "old text");
    stale.version = 0;
    store
        .upsert_templates(&[
            stale,
            template(tmpl_a, "payment", "Payment request failed <*>"),
            template(tmpl_b, "cart", "Cart emptied <*>"),
        ])
        .await
        .unwrap();

    let hit = |log_id: u64, tmpl: u64, service: &str, ts: i64, trace: &str| LogHitRow {
        log_id,
        template_id: tmpl,
        service: service.into(),
        ts,
        severity_number: 17,
        trace_id: trace.into(),
        span_id: "0000000000000002".into(),
    };
    let base: u64 = rand::random::<u32>().into();
    store
        .insert_log_hits(&[
            hit(base, tmpl_a, "payment", now - 2 * min, &trace_story),
            // The same log replayed must count once.
            hit(base, tmpl_a, "payment", now - 2 * min, &trace_story),
            hit(base + 1, tmpl_a, "payment", now - min, &trace_plain),
            hit(
                base + 2,
                tmpl_a,
                "payment",
                now - 40 * 3600 * 1_000_000_000,
                "",
            ),
            hit(base + 3, tmpl_b, "cart", now - min, &trace_other),
        ])
        .await
        .unwrap();

    let alert =
        |id: &str, kind: i8, tmpl: u64, service: &str, started: i64, last: i64| LogAlertRow {
            alert_id: id.into(),
            kind,
            template_id: tmpl,
            service: service.into(),
            template: "Payment request failed <*>".into(),
            started_at: started,
            last_at: last,
            window_count: 30,
            peak_count: 35,
            baseline_per_window: 0.5,
            example_trace_ids: vec![trace_story.clone(), trace_plain.clone()],
            version: 1,
        };
    let ids = [hex32(), hex32(), hex32()];
    store
        .insert_alerts(&[
            // Active spike, and a `new` alert on the same template that has since ended.
            alert(&ids[0], 2, tmpl_a, "payment", now - 3 * min, now),
            alert(
                &ids[1],
                1,
                tmpl_a,
                "payment",
                now - 30 * min,
                now - 25 * min,
            ),
            alert(
                &ids[2],
                1,
                tmpl_b,
                "cart",
                now - 20 * 3600 * 1_000_000_000,
                now - 20 * 3600 * 1_000_000_000,
            ),
        ])
        .await
        .unwrap();
    // Only the first example has an error story (story_id equals trace_id).
    store
        .insert_rows(
            "error_stories",
            &[StoryRow {
                story_id: trace_story.clone(),
                fingerprint: 1,
                kind: 1,
                ts: now - min,
                trace_id: trace_story.clone(),
                endpoint_service: "frontend".into(),
                endpoint_name: "POST /api/checkout".into(),
                rc_service: "payment".into(),
                rc_span_name: "Charge".into(),
                rc_span_kind: "server".into(),
                rc_span_id: "0000000000000002".into(),
                rc_message: "x".into(),
                rc_exception_type: String::new(),
                summary: "s".into(),
                duration_ns: 1,
                path_services: vec![],
                path_spans: "[]".into(),
                critical_path: "{}".into(),
                baseline_diff: String::new(),
                logs: "[]".into(),
                also_failed: "[]".into(),
                span_count: 1,
                flags: vec![],
            }],
        )
        .await
        .unwrap();

    let r = ChRepo::new(&s);
    let af = |since_secs, kind: Option<&str>, service: Option<&str>| AlertFilter {
        since_secs,
        kind: kind.map(Into::into),
        service: service.map(Into::into),
    };

    // Alerts: newest last_at first, 24h drops nothing here but kind and service filter.
    let alerts = r.log_alerts(&af(86_400, None, None)).await.unwrap();
    assert_eq!(
        alerts
            .iter()
            .map(|a| a.alert_id.as_str())
            .collect::<Vec<_>>(),
        [ids[0].as_str(), ids[1].as_str(), ids[2].as_str()]
    );
    assert!(alerts[0].active && !alerts[1].active && !alerts[2].active);
    assert_eq!(alerts[0].kind, "spike");
    assert_eq!(alerts[0].template_id, tmpl_a.to_string());
    assert_eq!(alerts[0].example_traces[0].trace_id, trace_story);
    assert_eq!(
        alerts[0].example_traces[0].story_id.as_deref(),
        Some(trace_story.as_str())
    );
    assert_eq!(alerts[0].example_traces[1].story_id, None);
    assert_eq!(
        r.log_alerts(&af(86_400, Some("new"), None))
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        r.log_alerts(&af(86_400, None, Some("cart")))
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        r.log_alerts(&af(3600, None, None)).await.unwrap().len(),
        2,
        "window applies"
    );

    // Templates: window count is distinct logs; service and q filter; alerting is the 10 min rule.
    let tf = |since_secs, service: Option<&str>, q: Option<&str>| TemplateFilter {
        since_secs,
        service: service.map(Into::into),
        q: q.map(Into::into),
    };
    let ts = r.log_templates(&tf(3600, None, None)).await.unwrap();
    assert_eq!(ts.len(), 2);
    assert_eq!(ts[0].template_id, tmpl_a.to_string());
    assert_eq!(ts[0].template, "Payment request failed <*>");
    assert_eq!(
        ts[0].count, 2,
        "replay deduplicated, old hit outside the window"
    );
    assert!(ts[0].alerting);
    assert!(!ts[1].alerting, "cart alert is old");
    assert_eq!(ts[1].count, 1);
    assert_eq!(
        r.log_templates(&tf(172_800, None, None)).await.unwrap()[0].count,
        3
    );
    let by_service = r
        .log_templates(&tf(3600, Some("cart"), None))
        .await
        .unwrap();
    assert_eq!(by_service.len(), 1);
    assert_eq!(by_service[0].service, "cart");
    let by_q = r
        .log_templates(&tf(3600, None, Some("PAYMENT REQ")))
        .await
        .unwrap();
    assert_eq!(by_q.len(), 1);
    assert_eq!(by_q[0].template_id, tmpl_a.to_string());
    assert!(
        r.log_templates(&tf(3600, None, Some("no such text")))
            .await
            .unwrap()
            .is_empty()
    );
    // Quotes in user input are bound, not interpolated.
    assert!(
        r.log_templates(&tf(3600, None, Some("' OR 1=1 --")))
            .await
            .unwrap()
            .is_empty()
    );

    // Detail.
    let d = r
        .log_template(&tmpl_a.to_string(), 86_400)
        .await
        .unwrap()
        .expect("exists");
    assert_eq!(d.template.template, "Payment request failed <*>");
    assert_eq!(d.sample, "Payment request failed 42");
    assert_eq!(d.template.count, 2);
    assert!(d.template.alerting);
    assert_eq!(d.bucket_secs, 720);
    assert_eq!(d.buckets.iter().map(|b| b.1).sum::<u64>(), 2);
    assert!(d.buckets.iter().all(|b| b.0 % 720 == 0));
    assert_eq!(d.recent.len(), 3, "recent ignores the window, one per log");
    assert!(d.recent.windows(2).all(|w| w[0].ts_ns >= w[1].ts_ns));
    assert_eq!(d.recent[0].trace_id, trace_plain);
    assert_eq!(d.alerts.len(), 2);
    assert_eq!(d.alerts[0].alert_id, ids[0]);
    assert!(r.log_template("1", 3600).await.unwrap().is_none());

    // Trace links: the story trace sits inside the spike (and the ended `new` alert's lead time).
    let t = r.trace_log_templates(&trace_story).await.unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].log_id, base.to_string());
    assert_eq!(t[0].template_id, tmpl_a.to_string());
    assert_eq!(t[0].template, "Payment request failed <*>");
    assert_eq!(t[0].alert.as_deref(), Some("spike"));
    // The cart alert ended 20h ago, so no alert is active at the cart trace's time.
    let c = r.trace_log_templates(&trace_other).await.unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].alert, None);
    assert!(r.trace_log_templates(&hex32()).await.unwrap().is_empty());

    Store::new(&s)
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}
