//! ChRepo against a real ClickHouse. Self-seeding: it migrates a fresh, uniquely named database,
//! inserts its own rows with random ids and `now`-based timestamps, reads them back and drops
//! the database. Runs on the empty `make it` ClickHouse and against the live stack alike.

use tayga_api::model::{OverviewView, SilenceSetting};
use tayga_api::params::{
    AlertFilter, GroupFilter, SeriesKind, SeriesQuery, TemplateFilter, TraceFilter, Window,
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

/// The window of the last `secs` seconds, ending just after now (as a request without `until`).
fn last(secs: i64) -> Window {
    let end = now_ns() / 1_000_000_000 + 1;
    Window {
        start: end - secs,
        end,
        live: true,
    }
}

/// Whether every bucket start lies on the epoch grid (a multiple of `step`) and the bucket
/// overlaps the window `[start, upper)`.
fn on_grid(w: Window, step: u32, buckets: impl IntoIterator<Item = u32>) -> bool {
    let step = i64::from(step);
    buckets.into_iter().all(|b| {
        let b = i64::from(b);
        b % step == 0 && b + step > w.start && b < w.upper()
    })
}

/// Rows must be recent: the tables carry TTLs and every read filters on a window.
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
    let week = last(7 * 86_400);
    let groups = r
        .story_groups(&GroupFilter {
            window: week,
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
    assert!(on_grid(week, 5040, g.buckets.iter().map(|b| b.0)));

    let filtered = r
        .story_groups(&GroupFilter {
            window: last(3600),
            kind: Some("slow".into()),
            service: None,
        })
        .await
        .unwrap();
    assert!(filtered.is_empty(), "kind filter applies");

    let detail = r
        .story_group(&g.group.fingerprint, last(3600))
        .await
        .unwrap()
        .expect("group exists");
    assert_eq!(detail.examples.len(), 2);
    assert!(r.story_group("1", last(3600)).await.unwrap().is_none());

    let story = r.story(&story_ids[0]).await.unwrap().expect("story exists");
    assert_eq!(story.fingerprint, g.group.fingerprint);
    assert_eq!(story.trace_id, trace_id);
    assert!(r.story(&"0".repeat(32)).await.unwrap().is_none());

    let trace = r.trace(&trace_id).await.unwrap();
    assert_eq!(trace.spans.len(), 2, "spans deduplicated");
    assert_eq!(trace.logs.len(), 1);
    assert_eq!(trace.logs[0].log_id, log_id.to_string());

    let edges = r.service_map(last(3600)).await.unwrap();
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
    let log = |log_id: u64, service: &str, ts: i64, trace: &str| LogRow {
        log_id,
        ts,
        observed_ts: ts,
        trace_id: trace.into(),
        span_id: "0000000000000002".into(),
        severity_number: 17,
        severity_text: "ERROR".into(),
        service_name: service.into(),
        body: "Payment request failed 42".into(),
        resource_attrs: vec![],
        log_attrs: vec![],
    };
    store
        .insert_logs(&[
            log(base, "payment", now - 2 * min, &trace_story),
            log(base + 3, "cart", now - min, &trace_other),
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
            baseline_day: Some(4.0),
            baseline_week: None,
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
    let af = |secs, kind: Option<&str>, service: Option<&str>| AlertFilter {
        window: last(secs),
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
    assert_eq!(
        (alerts[0].baseline_day, alerts[0].baseline_week),
        (Some(4.0), None),
        "seasonal comparators pass through"
    );
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
    let tf = |secs, service: Option<&str>, q: Option<&str>| TemplateFilter {
        window: last(secs),
        service: service.map(Into::into),
        q: q.map(Into::into),
    };
    // One window for the list and the detail, so their buckets are comparable.
    let hour = last(3600);
    let ts = r
        .log_templates(&TemplateFilter {
            window: hour,
            service: None,
            q: None,
        })
        .await
        .unwrap();
    assert_eq!(ts.len(), 2);
    assert_eq!(ts[0].template.template_id, tmpl_a.to_string());
    assert_eq!(ts[0].template.template, "Payment request failed <*>");
    assert_eq!(
        ts[0].template.count, 2,
        "replay deduplicated, old hit outside the window"
    );
    assert!(ts[0].template.alerting);
    assert!(!ts[1].template.alerting, "cart alert is old");
    assert_eq!(ts[1].template.count, 1);
    // Buckets come from one grouped query: they sum to the window count, stay ascending and
    // on the bucket grid, and match the detail endpoint's.
    assert_eq!(ts[0].bucket_secs, 60);
    for t in &ts {
        assert_eq!(t.buckets.iter().map(|b| b.1).sum::<u64>(), t.template.count);
        assert!(t.buckets.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(on_grid(hour, 60, t.buckets.iter().map(|b| b.0)));
    }
    let detail_a = r
        .log_template(&tmpl_a.to_string(), hour)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ts[0].buckets, detail_a.buckets);
    assert_eq!(
        r.log_templates(&tf(172_800, None, None)).await.unwrap()[0]
            .template
            .count,
        3
    );
    let by_service = r
        .log_templates(&tf(3600, Some("cart"), None))
        .await
        .unwrap();
    assert_eq!(by_service.len(), 1);
    assert_eq!(by_service[0].template.service, "cart");
    let by_q = r
        .log_templates(&tf(3600, None, Some("PAYMENT REQ")))
        .await
        .unwrap();
    assert_eq!(by_q.len(), 1);
    assert_eq!(by_q[0].template.template_id, tmpl_a.to_string());
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
    let day = last(86_400);
    let d = r
        .log_template(&tmpl_a.to_string(), day)
        .await
        .unwrap()
        .expect("exists");
    assert_eq!(d.template.template, "Payment request failed <*>");
    assert_eq!(d.sample, "Payment request failed 42");
    assert_eq!(d.template.count, 2);
    assert!(d.template.alerting);
    assert_eq!(d.bucket_secs, 720);
    assert_eq!(d.buckets.iter().map(|b| b.1).sum::<u64>(), 2);
    assert!(on_grid(day, 720, d.buckets.iter().map(|b| b.0)));
    assert_eq!(
        d.recent.len(),
        3,
        "recent ignores the window's start, one per log"
    );
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
    assert!(r.log_template("1", last(3600)).await.unwrap().is_none());

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

#[tokio::test]
#[ignore = "requires ClickHouse: make it, or TAYGA_IT_CLICKHOUSE against the live stack"]
async fn silence_settings_persist_and_silence_alerts_do_not_mark_templates_alerting() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let now = now_ns();
    let min = 60_000_000_000_i64;
    let (quiet, busy): (u64, u64) = (rand::random::<u32>().into(), rand::random::<u32>().into());
    let trace = hex32();
    let template = |id: u64, service: &str| LogTemplateRow {
        template_id: id,
        service: service.into(),
        template: format!("tmpl {id} <*>"),
        first_seen: now - 30 * min,
        last_seen: now,
        count: 1,
        max_severity: 9,
        sample: "s".into(),
        version: 1,
    };
    store
        .upsert_templates(&[template(quiet, "cart"), template(busy, "payment")])
        .await
        .unwrap();
    let hit = |log_id: u64, tmpl: u64, service: &str, trace: &str| LogHitRow {
        log_id,
        template_id: tmpl,
        service: service.into(),
        ts: now - min,
        severity_number: 9,
        trace_id: trace.into(),
        span_id: "0000000000000002".into(),
    };
    let base: u64 = rand::random::<u32>().into();
    store
        .insert_log_hits(&[
            hit(base, quiet, "cart", &trace),
            hit(base + 1, busy, "payment", ""),
        ])
        .await
        .unwrap();
    // Trace links need the trace's stored log.
    store
        .insert_logs(&[LogRow {
            log_id: base,
            ts: now - min,
            observed_ts: now - min,
            trace_id: trace.clone(),
            span_id: "0000000000000002".into(),
            severity_number: 9,
            severity_text: "INFO".into(),
            service_name: "cart".into(),
            body: "s".into(),
            resource_attrs: vec![],
            log_attrs: vec![],
        }])
        .await
        .unwrap();
    // A silence alert and a `new` alert, both active now, on different templates.
    let alert = |id: &str, kind: i8, tmpl: u64, service: &str| LogAlertRow {
        alert_id: id.into(),
        kind,
        template_id: tmpl,
        service: service.into(),
        template: format!("tmpl {tmpl} <*>"),
        started_at: now - 5 * min,
        last_at: now,
        window_count: 0,
        peak_count: 0,
        baseline_per_window: 1.0,
        example_trace_ids: vec![],
        version: 1,
        baseline_day: None,
        baseline_week: None,
    };
    let (silence_id, new_id) = (hex32(), hex32());
    store
        .insert_alerts(&[
            alert(&silence_id, 3, quiet, "cart"),
            alert(&new_id, 1, busy, "payment"),
        ])
        .await
        .unwrap();

    let r = ChRepo::new(&s);
    let hour = last(3600);
    let on = SilenceSetting {
        enabled: true,
        minutes: 15,
    };

    // Settings: unknown template refused and nothing written; known template persists.
    assert_eq!(r.silence(&quiet.to_string()).await.unwrap(), None);
    assert!(!r.put_silence("1", on).await.unwrap());
    assert_eq!(r.silence("1").await.unwrap(), None);
    assert!(r.put_silence(&quiet.to_string(), on).await.unwrap());
    assert_eq!(r.silence(&quiet.to_string()).await.unwrap(), Some(on));
    assert_eq!(r.silence(&busy.to_string()).await.unwrap(), None);

    // The list reflects it, and the silence alert does not make the template `alerting`
    // while the `new` alert does.
    let list = r
        .log_templates(&TemplateFilter {
            window: hour,
            service: None,
            q: None,
        })
        .await
        .unwrap();
    let find = |id: u64| {
        list.iter()
            .find(|t| t.template.template_id == id.to_string())
            .expect("listed")
    };
    assert!(find(quiet).silence_enabled);
    assert!(!find(busy).silence_enabled);
    assert!(!find(quiet).template.alerting, "silence is not alerting");
    assert!(find(busy).template.alerting);
    let detail = r
        .log_template(&quiet.to_string(), hour)
        .await
        .unwrap()
        .expect("exists");
    assert!(!detail.template.alerting);
    assert_eq!(detail.alerts.len(), 1, "the silence alert is still listed");
    assert_eq!(detail.alerts[0].kind, "silence");

    // Alert list: the silence kind filters and comes back as stored; the overview counts it.
    let silent = r
        .log_alerts(&AlertFilter {
            window: last(86_400),
            kind: Some("silence".into()),
            service: None,
        })
        .await
        .unwrap();
    assert_eq!(silent.len(), 1);
    assert_eq!(silent[0].alert_id, silence_id);
    assert_eq!(silent[0].window_count, 0);
    assert!(silent[0].example_traces.is_empty() && silent[0].active);
    assert_eq!(r.overview(last(3600)).await.unwrap().active_alerts, 2);

    // A trace hit proves the template was not silent: no alert label from a silence alert.
    let links = r.trace_log_templates(&trace).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].alert, None);

    // Switching off keeps the minutes and the latest write wins.
    let off = SilenceSetting {
        enabled: false,
        minutes: 15,
    };
    assert!(r.put_silence(&quiet.to_string(), off).await.unwrap());
    assert_eq!(r.silence(&quiet.to_string()).await.unwrap(), Some(off));
    let list = r
        .log_templates(&TemplateFilter {
            window: hour,
            service: None,
            q: None,
        })
        .await
        .unwrap();
    assert!(list.iter().all(|t| !t.silence_enabled));

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
        baseline_day: None,
        baseline_week: None,
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
    let hour = last(3600);
    let o = r.overview(hour).await.unwrap();
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
    assert!(on_grid(hour, 60, o.stories.error.iter().map(|b| b.0)));
    assert!(on_grid(hour, 60, o.spans.iter().map(|b| b.0)));

    // Stories series with kind and service filters.
    let gf = |kind: Option<&str>, service: Option<&str>| GroupFilter {
        window: last(3600),
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
        window: last(3600),
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
            window: last(3 * 3600),
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
    let err = r
        .service("errsvc", last(3600))
        .await
        .unwrap()
        .expect("exists");
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
    let ok = r.service("oksvc", last(3600)).await.unwrap().unwrap();
    assert_eq!(ok.calls, 5, "consumer spans count");
    let lg = r
        .service("loadgen", last(3600))
        .await
        .unwrap()
        .expect("client-only still exists");
    assert!(lg.buckets.is_empty() && lg.calls == 0);
    assert!(r.service("nosuch", last(3600)).await.unwrap().is_none());
    assert!(
        r.service("' OR 1=1 --", last(3600))
            .await
            .unwrap()
            .is_none()
    );

    // Service map nodes and health.
    let g = r.service_graph(last(3600)).await.unwrap();
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
        .metric_buckets(&SeriesQuery {
            window: last(3600),
            metric: "tayga_logminer_data_lag_seconds".into(),
            job: Some("tayga-logminer".into()),
            kind: SeriesKind::Gauge,
            labels: vec![],
        })
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

/// A window in the past (`until` set) returns only the rows inside it: every windowed read is
/// seeded with one row before the window, rows inside it and one after it.
#[tokio::test]
#[ignore = "requires ClickHouse: make it, or TAYGA_IT_CLICKHOUSE against the live stack"]
async fn a_past_window_returns_only_the_rows_inside_it() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let sec = 1_000_000_000_i64;
    let now_s = now_ns() / sec;
    // Two hours ending six hours ago, starting 7 s past a minute so the grid is not the epoch's.
    let end = (now_s - 6 * 3600) / 60 * 60 + 7;
    let w = Window {
        start: end - 7200,
        end,
        live: false,
    };
    let (before, inside, late, after) = (
        (w.start - 600) * sec,
        (w.start + 1800) * sec,
        (w.end - 30) * sec,
        (w.end + 600) * sec,
    );
    let times = [before, inside, late, after];

    // Spans of one service, one trace summary and one error story per moment.
    let ids: Vec<String> = times.iter().map(|_| hex32()).collect();
    let spans: Vec<SpanRow> = times
        .iter()
        .zip(&ids)
        .map(|(t, id)| {
            let mut sp = rspan("pastsvc", 2, *t, 5, *t == late);
            sp.trace_id = id.clone();
            sp
        })
        .collect();
    store.insert_rows("spans", &spans).await.unwrap();
    let summaries: Vec<TraceSummaryRow> = times
        .iter()
        .zip(&ids)
        .map(|(t, id)| summary(id, *t, "pastsvc", "GET /past", 5, *t == late))
        .collect();
    store
        .insert_rows("trace_summaries", &summaries)
        .await
        .unwrap();
    let fingerprint: u64 = rand::random();
    let stories: Vec<StoryRow> = times
        .iter()
        .zip(&ids)
        .map(|(t, id)| {
            let mut st = story_row(id, 1, *t, "pastsvc", "pastsvc Charge failed: boom");
            st.fingerprint = fingerprint;
            st
        })
        .collect();
    store.insert_rows("error_stories", &stories).await.unwrap();
    let edges: Vec<ServiceEdgeRow> = times
        .iter()
        .enumerate()
        .map(|(i, t)| ServiceEdgeRow {
            minute: u32::try_from(t / sec / 60 * 60).unwrap(),
            parent_service: "frontend".into(),
            child_service: "pastsvc".into(),
            calls: 10_u64.pow(u32::try_from(i).unwrap()),
            errors: 0,
            duration_ns_sum: 1,
        })
        .collect();
    store.insert_rows("service_edges", &edges).await.unwrap();

    // A template with one hit per moment, and alerts around the window.
    let tmpl: u64 = rand::random();
    store
        .upsert_templates(&[LogTemplateRow {
            template_id: tmpl,
            service: "pastsvc".into(),
            template: "Past failed <*>".into(),
            first_seen: before,
            last_seen: after,
            count: 4,
            max_severity: 17,
            sample: "Past failed 1".into(),
            version: 1,
        }])
        .await
        .unwrap();
    let hits: Vec<LogHitRow> = times
        .iter()
        .zip(&ids)
        .enumerate()
        .map(|(i, (t, id))| LogHitRow {
            log_id: u64::from(rand::random::<u32>()) * 8 + i as u64,
            template_id: tmpl,
            service: "pastsvc".into(),
            ts: *t,
            severity_number: 17,
            trace_id: id.clone(),
            span_id: "0000000000000002".into(),
        })
        .collect();
    store.insert_log_hits(&hits).await.unwrap();
    let alert = |started_at: i64, last_at: i64| LogAlertRow {
        alert_id: hex32(),
        kind: 2,
        template_id: tmpl,
        service: "pastsvc".into(),
        template: "Past failed <*>".into(),
        started_at,
        last_at,
        window_count: 1,
        peak_count: 1,
        baseline_per_window: 0.0,
        example_trace_ids: vec![],
        version: 1,
        baseline_day: None,
        baseline_week: None,
    };
    let (ended_before, overlapping, firing_at_end, started_after) = (
        alert(before - 60 * sec, before),
        alert(before, inside),
        alert(late - 60 * sec, after),
        alert(after, after + 60 * sec),
    );
    let alert_ids = |a: &[&LogAlertRow]| -> Vec<String> {
        let mut ids: Vec<String> = a.iter().map(|a| a.alert_id.clone()).collect();
        ids.sort();
        ids
    };
    let in_window = alert_ids(&[&overlapping, &firing_at_end]);
    let firing = firing_at_end.alert_id.clone();
    store
        .insert_alerts(&[ended_before, overlapping, firing_at_end, started_after])
        .await
        .unwrap();
    let sample = |t: i64, value: f64| MetricSampleRow {
        ts: t / 1_000_000,
        job: "tayga-logminer".into(),
        metric: "tayga_logminer_data_lag_seconds".into(),
        labels: vec![],
        value,
    };
    store
        .insert_metric_samples(&[
            sample(before, 1.0),
            sample(inside, 2.0),
            sample(late, 3.0),
            sample(after, 4.0),
        ])
        .await
        .unwrap();

    let r = ChRepo::new(&s);
    let step = w.step();
    assert_eq!(step, 60);
    let minute = |t: i64| t / 60 * 60;
    let gf = GroupFilter {
        window: w,
        kind: None,
        service: None,
    };

    // Story groups and their detail: two stories, in epoch-aligned buckets (the minutes of
    // `inside` and `late`), although the window starts 7 s past a minute.
    let groups = r.story_groups(&gf).await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].group.stories, 2);
    assert_eq!(groups[0].group.first_seen_ns, inside);
    assert_eq!(groups[0].group.last_seen_ns, late);
    let starts = |b: &[(u32, u64)]| b.iter().map(|b| i64::from(b.0)).collect::<Vec<_>>();
    assert_eq!(
        starts(&groups[0].buckets),
        [minute(w.start + 1800), minute(w.end - 30)],
        "buckets lie on the epoch grid"
    );
    let detail = r
        .story_group(&fingerprint.to_string(), w)
        .await
        .unwrap()
        .expect("in the window");
    assert_eq!(detail.group.group.stories, 2);
    let series = r.stories_series(&gf).await.unwrap();
    assert_eq!(series.error.iter().map(|b| b.1).sum::<u64>(), 2);
    assert!(on_grid(w, step, series.error.iter().map(|b| b.0)));
    // A window that ends before any of it is empty.
    let earlier = Window {
        start: w.start - 7200,
        end: w.start - 700,
        live: false,
    };
    assert!(
        r.story_groups(&GroupFilter {
            window: earlier,
            ..gf.clone()
        })
        .await
        .unwrap()
        .is_empty()
    );

    // Overview: stories, spans and the lag as of the window's end.
    let o = r.overview(w).await.unwrap();
    assert_eq!((o.error_stories, o.slow_stories), (2, 0));
    assert_eq!(o.active_alerts, 1, "only the alert firing at the end");
    assert_eq!(o.data_lag_secs, Some(3.0), "the last sample before the end");
    let spans_sum: f64 = o.spans.iter().map(|b| b.1 * f64::from(step)).sum();
    assert!((spans_sum - 2.0).abs() < 1e-6, "{spans_sum}");
    assert!((o.spans_per_sec - 2.0 / 7200.0).abs() < 1e-12);
    assert!(on_grid(w, step, o.spans.iter().map(|b| b.0)));
    // The same window 10 s later has the same bucket edges, so a live refresh does not shift
    // its bars.
    let later = Window {
        start: w.start + 10,
        end: w.end + 10,
        live: false,
    };
    let o2 = r.overview(later).await.unwrap();
    let edges = |v: &OverviewView| v.spans.iter().map(|b| b.0).collect::<Vec<_>>();
    assert_eq!(edges(&o), edges(&o2));
    assert_eq!(o.stories, o2.stories);

    // Trace search, by endpoint service and by touched service.
    let tf = TraceFilter {
        window: w,
        service: Some("pastsvc".into()),
        touched: false,
        endpoint: None,
        min_ns: 0,
        max_ns: u64::MAX,
        errors_only: false,
        limit: 500,
    };
    for touched in [false, true] {
        let hits = r
            .traces_search(&TraceFilter {
                touched,
                ..tf.clone()
            })
            .await
            .unwrap();
        assert_eq!(
            hits.iter().map(|h| h.trace_id.as_str()).collect::<Vec<_>>(),
            [ids[2].as_str(), ids[1].as_str()],
            "touched={touched}: newest first, only inside the window"
        );
    }

    // Service RED and the map.
    let svc = r.service("pastsvc", w).await.unwrap().expect("has spans");
    assert_eq!((svc.calls, svc.errors), (2, 1));
    assert!(on_grid(w, step, svc.buckets.iter().map(|b| b.bucket)));
    assert!(r.service("pastsvc", earlier).await.unwrap().is_none());
    let g = r.service_graph(w).await.unwrap();
    let node = g.nodes.iter().find(|n| n.service == "pastsvc").unwrap();
    assert_eq!((node.calls, node.error_ratio), (2, 0.5));
    assert_eq!(g.edges.len(), 1);
    assert_eq!(g.edges[0].calls, 110, "the two minutes inside the window");

    // Log alerts overlap the window; `active` is as of its end.
    let alerts = r
        .log_alerts(&AlertFilter {
            window: w,
            kind: None,
            service: Some("pastsvc".into()),
        })
        .await
        .unwrap();
    let mut got: Vec<String> = alerts.iter().map(|a| a.alert_id.clone()).collect();
    got.sort();
    assert_eq!(got, in_window);
    for a in &alerts {
        assert_eq!(a.active, a.alert_id == firing, "{a:?}");
    }

    // Templates: the window's hits, on its grid, in the list and the detail.
    let ts = r
        .log_templates(&TemplateFilter {
            window: w,
            service: Some("pastsvc".into()),
            q: None,
        })
        .await
        .unwrap();
    assert_eq!(ts.len(), 1);
    assert_eq!(ts[0].template.count, 2);
    assert!(ts[0].template.alerting, "an alert was firing at the end");
    assert_eq!(
        starts(&ts[0].buckets),
        [minute(w.start + 1800), minute(w.end - 30)]
    );
    let d = r.log_template(&tmpl.to_string(), w).await.unwrap().unwrap();
    assert_eq!(d.template.count, 2);
    assert_eq!(d.buckets, ts[0].buckets);
    assert_eq!(
        d.recent.iter().map(|h| h.ts_ns).collect::<Vec<_>>(),
        [late, inside, before],
        "recent hits up to the window's end"
    );
    assert_eq!(d.alerts.len(), 3, "every alert started by the window's end");
    assert!(
        r.log_templates(&TemplateFilter {
            window: earlier,
            service: Some("pastsvc".into()),
            q: None,
        })
        .await
        .unwrap()
        .is_empty()
    );

    // Pipeline series: the samples inside, in epoch-aligned buckets.
    let pts = r
        .metric_buckets(&SeriesQuery {
            window: w,
            metric: "tayga_logminer_data_lag_seconds".into(),
            job: None,
            kind: SeriesKind::Gauge,
            labels: vec![],
        })
        .await
        .unwrap();
    assert_eq!(
        pts.iter().map(|p| (p.ts_ms, p.value)).collect::<Vec<_>>(),
        [
            (minute(w.start + 1800) * 1000, 2.0),
            (minute(w.end - 30) * 1000, 3.0)
        ]
    );

    Store::new(&s)
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

/// A live window lists rows stamped up to 60 s ahead (a producer clock running fast), but its
/// bucketed and aggregated reads stop at the window's end: the overview's totals equal the sums
/// of its buckets, and spans/s divides only what the window holds.
#[tokio::test]
#[ignore = "requires ClickHouse: make it, or TAYGA_IT_CLICKHOUSE against the live stack"]
async fn a_live_window_lists_rows_ahead_but_totals_stop_at_its_end() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let sec = 1_000_000_000_i64;
    let w = last(3600);
    assert!(w.live);
    let (inside, ahead) = ((w.end - 120) * sec, (w.end + 30) * sec);
    let ids = [hex32(), hex32()];
    let spans: Vec<SpanRow> = [inside, ahead]
        .iter()
        .zip(&ids)
        .map(|(t, id)| {
            let mut sp = rspan("aheadsvc", 2, *t, 5, false);
            sp.trace_id = id.clone();
            sp
        })
        .collect();
    store.insert_rows("spans", &spans).await.unwrap();
    let summaries: Vec<TraceSummaryRow> = [inside, ahead]
        .iter()
        .zip(&ids)
        .map(|(t, id)| summary(id, *t, "aheadsvc", "GET /ahead", 5, true))
        .collect();
    store
        .insert_rows("trace_summaries", &summaries)
        .await
        .unwrap();
    let stories: Vec<StoryRow> = [inside, ahead]
        .iter()
        .zip(&ids)
        .map(|(t, id)| story_row(id, 1, *t, "aheadsvc", "aheadsvc Charge failed: boom"))
        .collect();
    store.insert_rows("error_stories", &stories).await.unwrap();

    let r = ChRepo::new(&s);
    let o = r.overview(w).await.unwrap();
    assert_eq!(
        o.error_stories, 1,
        "the story ahead of the end is not counted"
    );
    assert_eq!(
        o.error_stories,
        o.stories.error.iter().map(|b| b.1).sum::<u64>(),
        "the total equals the sum of its buckets"
    );
    let spans_sum: f64 = o.spans.iter().map(|b| b.1 * f64::from(o.bucket_secs)).sum();
    assert!((spans_sum - 1.0).abs() < 1e-9, "{spans_sum}");
    assert!((o.spans_per_sec - 1.0 / 3600.0).abs() < 1e-12);
    assert!(o.spans.iter().all(|b| i64::from(b.0) < w.end));
    let series = r
        .stories_series(&GroupFilter {
            window: w,
            kind: None,
            service: Some("aheadsvc".into()),
        })
        .await
        .unwrap();
    assert_eq!(series.error.iter().map(|b| b.1).sum::<u64>(), 1);
    let red = r.service("aheadsvc", w).await.unwrap().expect("has spans");
    assert_eq!(red.calls, 1);

    // The trace list, though, shows the trace stamped ahead too.
    let hits = r
        .traces_search(&TraceFilter {
            window: w,
            service: Some("aheadsvc".into()),
            touched: false,
            endpoint: None,
            min_ns: 0,
            max_ns: u64::MAX,
            errors_only: false,
            limit: 500,
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 2, "row lists keep the 60 s slack");

    Store::new(&s)
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

fn story_at(fingerprint: u64, ts: i64) -> StoryRow {
    let id = hex32();
    StoryRow {
        story_id: id.clone(),
        fingerprint,
        kind: 1,
        ts,
        trace_id: id,
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
    }
}

async fn drop_database(s: &ClickHouseSettings) {
    Store::new(s)
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires ClickHouse: make it, or TAYGA_IT_CLICKHOUSE against the live stack"]
async fn group_examples_stay_inside_the_window() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let now = now_ns();
    let fingerprint: u64 = rand::random();
    let inside = story_at(fingerprint, now - 10 * 60 * 1_000_000_000);
    let before = story_at(fingerprint, now - 2 * 3600 * 1_000_000_000);
    store
        .insert_rows("error_stories", &[inside.clone(), before])
        .await
        .unwrap();
    let detail = ChRepo::new(&s)
        .story_group(&fingerprint.to_string(), last(3600))
        .await
        .unwrap()
        .expect("the group has a story in the window");
    drop_database(&s).await;
    let ids: Vec<&str> = detail
        .examples
        .iter()
        .map(|e| e.story_id.as_str())
        .collect();
    assert_eq!(ids, [inside.story_id.as_str()]);
}

#[tokio::test]
#[ignore = "requires ClickHouse: make it, or TAYGA_IT_CLICKHOUSE against the live stack"]
async fn trace_links_need_the_traces_logs_and_stay_in_their_time_range() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let now = now_ns();
    let min = 60_000_000_000_i64;
    let trace = hex32();
    let tmpl: u64 = rand::random();
    store
        .upsert_templates(&[LogTemplateRow {
            template_id: tmpl,
            service: "payment".into(),
            template: "Payment request failed <*>".into(),
            first_seen: now - 30 * min,
            last_seen: now,
            count: 2,
            max_severity: 17,
            sample: "Payment request failed 42".into(),
            version: 1,
        }])
        .await
        .unwrap();
    let hit = |log_id: u64, ts: i64| LogHitRow {
        log_id,
        template_id: tmpl,
        service: "payment".into(),
        ts,
        severity_number: 17,
        trace_id: trace.clone(),
        span_id: "0000000000000002".into(),
    };
    let base: u64 = rand::random::<u32>().into();
    // The second hit lies a day before the trace's only log: outside its time range.
    store
        .insert_log_hits(&[hit(base, now - 2 * min), hit(base + 1, now - 24 * 60 * min)])
        .await
        .unwrap();
    let r = ChRepo::new(&s);
    let before_logs = r.trace_log_templates(&trace).await.unwrap();
    store
        .insert_logs(&[LogRow {
            log_id: base,
            ts: now - 2 * min,
            observed_ts: now - 2 * min,
            trace_id: trace.clone(),
            span_id: "0000000000000002".into(),
            severity_number: 17,
            severity_text: "ERROR".into(),
            service_name: "payment".into(),
            body: "Payment request failed 42".into(),
            resource_attrs: vec![],
            log_attrs: vec![],
        }])
        .await
        .unwrap();
    let links = r.trace_log_templates(&trace).await.unwrap();
    drop_database(&s).await;
    assert!(before_logs.is_empty(), "no stored log, no templates");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].log_id, base.to_string());
}

#[tokio::test]
#[ignore = "requires ClickHouse: make it, or TAYGA_IT_CLICKHOUSE against the live stack"]
async fn overview_data_lag_is_the_slowest_replica() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let now_ms = now_ns() / 1_000_000;
    let lag = |ts: i64, instance: &str, value: f64| MetricSampleRow {
        ts,
        job: "tayga-logminer".into(),
        metric: "tayga_logminer_data_lag_seconds".into(),
        labels: vec![("instance".into(), instance.into())],
        value,
    };
    let repo = ChRepo::new(&s);
    let inserted = store
        .insert_metric_samples(&[
            lag(now_ms - 20_000, "10.0.0.7:9100", 30.0),
            lag(now_ms - 5_000, "10.0.0.7:9100", 2.0),
            lag(now_ms - 5_000, "10.0.0.9:9100", 7.5),
        ])
        .await;
    let newest_tick = repo.overview(last(3600)).await;
    // A replica whose newest lag is older than the others' but still fresh counts; one whose
    // newest lag is older than the freshness bound (300 s) does not.
    let more = store
        .insert_metric_samples(&[
            lag(now_ms - 60_000, "10.0.0.11:9100", 12.0),
            lag(now_ms - 400_000, "10.0.0.12:9100", 99.0),
        ])
        .await;
    let per_replica = repo.overview(last(3600)).await;
    drop_database(&s).await;
    inserted.unwrap();
    more.unwrap();
    assert_eq!(newest_tick.unwrap().data_lag_secs, Some(7.5));
    assert_eq!(per_replica.unwrap().data_lag_secs, Some(12.0));
}

/// The map's 24 h health baseline is computed once per window-end minute; the window's own
/// counts never come from the cache (sub-project 4 spec §3.8).
#[tokio::test]
#[ignore = "requires ClickHouse: make it, or TAYGA_IT_CLICKHOUSE against the live stack"]
async fn the_map_baseline_is_cached_per_minute_and_the_window_is_not() {
    let s = settings();
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let r = ChRepo::new(&s);
    let sec = 1_000_000_000_i64;
    let minute = now_ns() / sec / 60 * 60;
    let at = |offset_secs: i64| (minute + offset_secs) * sec;
    // Baseline: 100 spans of 1 ms an hour ago; one of 10 ms two minutes ago, also in the window.
    let mut spans: Vec<SpanRow> = (0..100)
        .map(|_| rspan("basesvc", 2, at(-3_600), 1, false))
        .collect();
    spans.push(rspan("basesvc", 2, at(-120), 10, false));
    store.insert_spans(&spans).await.unwrap();
    let window = |end: i64| Window {
        start: end - 900,
        end,
        live: false,
    };
    let node = |g: anyhow::Result<tayga_api::model::ServiceMapView>| {
        g.ok().and_then(|g| {
            g.nodes
                .iter()
                .find(|n| n.service == "basesvc")
                .map(|n| (n.calls, n.baseline_p99_ns))
        })
    };
    let first = node(r.service_graph(window(minute - 30)).await);
    // 101 slow spans in the baseline and one more in the window.
    let mut more: Vec<SpanRow> = (0..101)
        .map(|_| rspan("basesvc", 2, at(-3_000), 10, false))
        .collect();
    more.push(rspan("basesvc", 2, at(-100), 10, false));
    store.insert_spans(&more).await.unwrap();
    // Same minute floor: the cached baseline, a fresh window.
    let cached = node(r.service_graph(window(minute - 5)).await);
    // The next minute recomputes it with the slow spans.
    let next = node(r.service_graph(window(minute + 1)).await);
    drop_database(&s).await;

    let (calls, first) = first.expect("basesvc node");
    assert_eq!(calls, 1);
    assert_eq!(cached, Some((2, first)));
    let (_, next) = next.expect("basesvc node");
    assert!(next > first, "{next} > {first}");
}
