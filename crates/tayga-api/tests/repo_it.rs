//! ChRepo against a real ClickHouse. Self-seeding: it migrates a fresh, uniquely named database,
//! inserts its own rows with random ids and `now`-based timestamps, reads them back and drops
//! the database. Runs on the empty `make it` ClickHouse and against the live stack alike.

use tayga_api::params::{
    AlertFilter, GroupFilter, SeriesKind, SeriesQuery, TemplateFilter, TraceFilter,
};
use tayga_api::repo::{ChRepo, Repo};
use tayga_store::ClickHouseSettings;
use tayga_store::logs::{LogAlertRow, LogHitRow, LogTemplateRow};
use tayga_store::metrics_store::MetricSampleRow;
use tayga_store::migrate::migrate;
use tayga_store::rows::{LogRow, ServiceEdgeRow, SpanRow, StoryRow, TraceSummaryRow};
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
    assert!(
        d.recent
            .iter()
            .all(|h| h.story_id.is_some() == (h.trace_id == trace_story)),
        "only the story trace's hits carry a story id"
    );
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

fn story_row(id: &str, kind: i8, ts: i64, rc_service: &str, summary: &str) -> StoryRow {
    StoryRow {
        story_id: id.into(),
        fingerprint: rand::random(),
        kind,
        ts,
        trace_id: id.into(),
        endpoint_service: "frontend".into(),
        endpoint_name: "POST /api/checkout".into(),
        rc_service: rc_service.into(),
        rc_span_name: "Charge".into(),
        rc_span_kind: "server".into(),
        rc_span_id: "0000000000000002".into(),
        rc_message: "x".into(),
        rc_exception_type: String::new(),
        summary: summary.into(),
        duration_ns: 1,
        path_services: vec![],
        path_spans: "[]".into(),
        critical_path: "{}".into(),
        baseline_diff: String::new(),
        logs: "[]".into(),
        also_failed: "[]".into(),
        span_count: 1,
        flags: vec![],
    }
}

fn summary(
    trace_id: &str,
    ts: i64,
    service: &str,
    name: &str,
    ms: u64,
    err: bool,
) -> TraceSummaryRow {
    TraceSummaryRow {
        trace_id: trace_id.into(),
        ts,
        endpoint_service: service.into(),
        endpoint_name: name.into(),
        duration_ns: ms * 1_000_000,
        is_error: u8::from(err),
        op_durations: vec![],
        span_count: 2,
    }
}

/// Server span of `service` at `start` lasting `ms`, failed when `err`, with kind `kind`.
fn rspan(service: &str, kind: i8, start: i64, ms: u64, err: bool) -> SpanRow {
    let mut s = span(&hex32(), &hex32()[..16], "", service, start);
    s.kind = kind;
    s.duration_ns = ms * 1_000_000;
    s.status_code = if err { 2 } else { 1 };
    s
}

#[tokio::test]
#[ignore = "requires ClickHouse: make it, or TAYGA_IT_CLICKHOUSE against the live stack"]
async fn reads_seeded_overview_traces_services_and_search() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let now = now_ns();
    let sec = 1_000_000_000_i64;
    let (t1, t2, t_old) = (hex32(), hex32(), hex32());

    // Trace t1: a frontend root with attributes and an event, and two overlapping children.
    let mut root = span(&t1, "00000000000000a1", "", "frontend", now - 60 * sec);
    root.duration_ns = 100;
    root.span_attrs = vec![("http.method".into(), "POST".into())];
    root.events_ts = vec![now - 60 * sec + 5];
    root.events_name = vec!["exception".into()];
    root.events_attrs = vec![vec![("exception.message".into(), "boom".into())]];
    let mut c1 = span(
        &t1,
        "00000000000000a2",
        "00000000000000a1",
        "payment",
        now - 60 * sec + 10,
    );
    c1.duration_ns = 30;
    let mut c2 = span(
        &t1,
        "00000000000000a3",
        "00000000000000a1",
        "payment",
        now - 60 * sec + 30,
    );
    c2.duration_ns = 20;
    // Trace t2 touches `oksvc` but its endpoint is frontend.
    let mut t2_span = rspan("oksvc", 5, now - 50 * sec, 2, false);
    t2_span.trace_id = t2.clone();

    let mut spans = vec![root, c1, c2, t2_span];
    // slowsvc: 200 fast spans 2h ago (the 24h baseline) and one slow span in the window.
    spans.extend((0..200).map(|_| rspan("slowsvc", 2, now - 7200 * sec, 1, false)));
    spans.push(rspan("slowsvc", 2, now - 30 * sec, 10, false));
    // errsvc: 1 of 10 server spans failed (10% >= 5%).
    spans.extend((0..10).map(|i| rspan("errsvc", 2, now - 20 * sec, 1, i == 0)));
    // oksvc: consumer spans count as calls.
    spans.extend((0..4).map(|_| rspan("oksvc", 5, now - 10 * sec, 2, false)));
    // loadgen: client spans only, so it has no calls.
    spans.push(rspan("loadgen", 3, now - 10 * sec, 2, false));
    store.insert_spans(&spans).await.unwrap();
    let spans_in_hour = spans
        .iter()
        .filter(|s| s.start_ts > now - 3600 * sec)
        .count() as u64;

    store
        .insert_rows(
            "trace_summaries",
            &[
                summary(
                    &t1,
                    now - 60 * sec,
                    "frontend",
                    "POST /api/checkout",
                    5,
                    true,
                ),
                summary(&t2, now - 50 * sec, "frontend", "GET /", 50, false),
                summary(&t_old, now - 7200 * sec, "frontend", "GET /", 1, true),
            ],
        )
        .await
        .unwrap();
    let slow_id = hex32();
    store
        .insert_rows(
            "error_stories",
            &[
                story_row(
                    &t1,
                    1,
                    now - 60 * sec,
                    "payment",
                    "payment Charge failed: Invalid token",
                ),
                story_row(&slow_id, 2, now - 40 * sec, "cart", "cart slow"),
            ],
        )
        .await
        .unwrap();
    let alert = |id: &str, last_at: i64| LogAlertRow {
        alert_id: id.into(),
        kind: 2,
        template_id: 1,
        service: "payment".into(),
        template: "t".into(),
        started_at: last_at - 60 * sec,
        last_at,
        window_count: 1,
        peak_count: 1,
        baseline_per_window: 0.0,
        example_trace_ids: vec![],
        version: 1,
    };
    store
        .insert_alerts(&[alert(&hex32(), now), alert(&hex32(), now - 3600 * sec)])
        .await
        .unwrap();
    let tmpl: u64 = rand::random();
    store
        .upsert_templates(&[LogTemplateRow {
            template_id: tmpl,
            service: "payment".into(),
            template: "Payment request failed <*>".into(),
            first_seen: now - 60 * sec,
            last_seen: now,
            count: 3,
            max_severity: 17,
            sample: "Payment request failed 42".into(),
            version: 1,
        }])
        .await
        .unwrap();
    let now_ms = now / 1_000_000;
    let lag = |ts, v| MetricSampleRow {
        ts,
        job: "tayga-logminer".into(),
        metric: "tayga_logminer_data_lag_seconds".into(),
        labels: vec![],
        value: v,
    };
    store
        .insert_metric_samples(&[lag(now_ms - 20_000, 3.0), lag(now_ms - 10_000, 1.5)])
        .await
        .unwrap();

    let r = ChRepo::new(&s);

    // Overview.
    let o = r.overview(3600).await.unwrap();
    assert_eq!(o.bucket_secs, 60);
    assert_eq!(
        (o.error_stories, o.slow_stories, o.active_alerts),
        (1, 1, 1)
    );
    assert_eq!(o.data_lag_secs, Some(1.5));
    let spans_sum: f64 = o.spans.iter().map(|b| b.1 * 60.0).sum();
    assert!(
        (spans_sum - spans_in_hour as f64).abs() < 1e-6,
        "{spans_sum} vs {spans_in_hour}"
    );
    assert!((o.spans_per_sec - spans_in_hour as f64 / 3600.0).abs() < 1e-9);
    assert!(o.stories.error.iter().all(|b| b.0 % 60 == 0));

    // Stories series with kind and service filters.
    let gf = |kind: Option<&str>, service: Option<&str>| GroupFilter {
        since_secs: 3600,
        kind: kind.map(Into::into),
        service: service.map(Into::into),
    };
    let all = r.stories_series(&gf(None, None)).await.unwrap();
    assert_eq!((all.error.len(), all.slow.len()), (1, 1));
    let slow = r.stories_series(&gf(Some("slow"), None)).await.unwrap();
    assert!(slow.error.is_empty() && slow.slow.len() == 1);
    let cart = r.stories_series(&gf(None, Some("cart"))).await.unwrap();
    assert!(cart.error.is_empty() && cart.slow[0].1 == 1);

    // Trace search.
    let tf = TraceFilter {
        since_secs: 3600,
        service: None,
        touched: false,
        endpoint: None,
        min_ns: 0,
        max_ns: u64::MAX,
        errors_only: false,
        limit: 500,
    };
    let ids = |v: Vec<tayga_api::model::TraceHitView>| {
        v.into_iter().map(|h| h.trace_id).collect::<Vec<_>>()
    };
    let hits = r.traces_search(&tf).await.unwrap();
    assert_eq!(hits.len(), 2, "old trace outside the window");
    assert_eq!(hits[0].trace_id, t2, "newest first");
    assert_eq!(hits[1].story_id.as_deref(), Some(t1.as_str()));
    assert_eq!(hits[1].story_kind.as_deref(), Some("error"));
    assert!(hits[0].story_id.is_none() && hits[0].story_kind.is_none() && hits[1].is_error);
    assert_eq!(hits[1].duration_ns, 5_000_000);
    assert_eq!(
        ids(r
            .traces_search(&TraceFilter {
                errors_only: true,
                ..tf.clone()
            })
            .await
            .unwrap()),
        [t1.as_str()]
    );
    assert_eq!(
        ids(r
            .traces_search(&TraceFilter {
                min_ns: 10_000_000,
                ..tf.clone()
            })
            .await
            .unwrap()),
        [t2.as_str()]
    );
    assert_eq!(
        ids(r
            .traces_search(&TraceFilter {
                max_ns: 10_000_000,
                ..tf.clone()
            })
            .await
            .unwrap()),
        [t1.as_str()]
    );
    assert_eq!(
        ids(r
            .traces_search(&TraceFilter {
                endpoint: Some("GET /".into()),
                ..tf.clone()
            })
            .await
            .unwrap()),
        [t2.as_str()]
    );
    assert_eq!(
        ids(r
            .traces_search(&TraceFilter {
                limit: 1,
                ..tf.clone()
            })
            .await
            .unwrap()),
        [t2.as_str()]
    );
    assert_eq!(
        r.traces_search(&TraceFilter {
            since_secs: 3 * 3600,
            ..tf.clone()
        })
        .await
        .unwrap()
        .len(),
        3
    );
    // Endpoint-service match versus "trace touched the service".
    let oksvc = |touched| TraceFilter {
        service: Some("oksvc".into()),
        touched,
        ..tf.clone()
    };
    assert!(r.traces_search(&oksvc(false)).await.unwrap().is_empty());
    assert_eq!(
        ids(r.traces_search(&oksvc(true)).await.unwrap()),
        [t2.as_str()]
    );
    let frontend = TraceFilter {
        service: Some("frontend".into()),
        ..tf.clone()
    };
    assert_eq!(r.traces_search(&frontend).await.unwrap().len(), 2);
    assert_eq!(
        ids(r
            .traces_search(&TraceFilter {
                touched: true,
                ..frontend
            })
            .await
            .unwrap()),
        [t1.as_str()]
    );
    // A slow story's trace (not an error) reports kind "slow".
    store
        .insert_rows(
            "trace_summaries",
            &[summary(
                &slow_id,
                now - 40 * sec,
                "slowsvc",
                "GET /slow",
                900,
                false,
            )],
        )
        .await
        .unwrap();
    let slow_hits = r
        .traces_search(&TraceFilter {
            endpoint: Some("GET /slow".into()),
            ..tf.clone()
        })
        .await
        .unwrap();
    assert_eq!(
        slow_hits
            .iter()
            .map(|h| (h.story_id.as_deref(), h.story_kind.as_deref(), h.is_error))
            .collect::<Vec<_>>(),
        [(Some(slow_id.as_str()), Some("slow"), false)]
    );

    // Extended trace.
    let t = r.trace(&t1).await.unwrap();
    assert_eq!(t.story_id.as_deref(), Some(t1.as_str()));
    let root = t
        .spans
        .iter()
        .find(|s| s.span_id == "00000000000000a1")
        .unwrap();
    assert_eq!(
        root.attrs,
        vec![("http.method".to_string(), "POST".to_string())]
    );
    assert_eq!(
        root.resource,
        vec![("service.name".to_string(), "frontend".to_string())]
    );
    assert_eq!(root.events.len(), 1);
    assert_eq!(root.events[0].ts_ns, now - 60 * sec + 5);
    assert_eq!(root.events[0].name, "exception");
    assert_eq!(root.events[0].attrs[0].1, "boom");
    // Children cover [10, 50) of the root's 100 ns.
    assert_eq!(root.self_ns, 60);
    assert!(r.trace(&t2).await.unwrap().story_id.is_none());

    // Services.
    let names = r.services().await.unwrap();
    assert_eq!(
        names,
        [
            "errsvc", "frontend", "loadgen", "oksvc", "payment", "slowsvc"
        ]
    );
    let err = r.service("errsvc", 3600).await.unwrap().expect("exists");
    assert_eq!((err.calls, err.errors, err.bucket_secs), (10, 1, 60));
    assert_eq!(
        err.buckets
            .iter()
            .map(|b| b.error_ratio)
            .fold(0.0, f64::max),
        0.1
    );
    assert!(
        err.buckets
            .iter()
            .all(|b| b.p99_ns > 0.0 && b.p50_ns <= b.p99_ns)
    );
    let ok = r.service("oksvc", 3600).await.unwrap().unwrap();
    assert_eq!(ok.calls, 5, "consumer spans count");
    let lg = r
        .service("loadgen", 3600)
        .await
        .unwrap()
        .expect("client-only still exists");
    assert!(lg.buckets.is_empty() && lg.calls == 0);
    assert!(r.service("nosuch", 3600).await.unwrap().is_none());
    assert!(r.service("' OR 1=1 --", 3600).await.unwrap().is_none());

    // Service map nodes and health.
    let g = r.service_graph(3600).await.unwrap();
    let node = |name: &str| g.nodes.iter().find(|n| n.service == name).cloned();
    assert_eq!(node("errsvc").unwrap().health, "error");
    let slow = node("slowsvc").unwrap();
    assert_eq!(slow.calls, 1, "the 2h-old spans are only the baseline");
    assert_eq!(slow.health, "slow", "{slow:?}");
    assert_eq!(node("oksvc").unwrap().health, "ok");
    assert!(node("loadgen").is_none(), "no server or consumer spans");
    assert!(g.edges.is_empty());

    // Search.
    let found = r.search("SLOWSV").await.unwrap();
    assert_eq!(found.services, ["slowsvc"]);
    assert!(found.trace_id.is_none());
    let found = r.search("payment req").await.unwrap();
    assert_eq!(found.templates.len(), 1);
    assert_eq!(found.templates[0].template_id, tmpl.to_string());
    let found = r.search("invalid TOKEN").await.unwrap();
    assert_eq!(found.groups.len(), 1);
    assert_eq!(found.groups[0].kind, "error");
    assert_eq!(found.groups[0].stories, 1);
    let found = r.search(&t1.to_uppercase()).await.unwrap();
    assert_eq!(found.trace_id.as_deref(), Some(t1.as_str()));
    let none = r.search("' OR 1=1 --").await.unwrap();
    assert!(none.services.is_empty() && none.templates.is_empty() && none.groups.is_empty());

    // Pipeline series points: the last value per series and step.
    let pts = r
        .metric_buckets(
            &SeriesQuery {
                since_secs: 3600,
                metric: "tayga_logminer_data_lag_seconds".into(),
                job: Some("tayga-logminer".into()),
                kind: SeriesKind::Gauge,
                labels: vec![],
            },
            3600,
        )
        .await
        .unwrap();
    assert!(!pts.is_empty() && pts.len() <= 2, "{pts:?}");
    assert_eq!(pts.last().unwrap().value, 1.5);

    Store::new(&s)
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}
