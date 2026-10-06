//! `remine` against live ClickHouse, in a self-seeded unique database.
//! Run: `TAYGA_IT_CLICKHOUSE=http://localhost:18123 cargo test -p tayga-devtools --test remine_it -- --ignored`

use std::collections::BTreeSet;
use tayga_devtools::remine::{Options, remine, remine_paged};
use tayga_drain::drain::DrainConfig;
use tayga_drain::preprocess::masking_version;
use tayga_logminer::config::{
    KEY_EPOCH_START, KEY_HEARTBEAT, KEY_MASKING_VERSION, KEY_WATERMARK, heartbeat_key,
    watermark_key,
};
use tayga_logminer::miner::Miner;
use tayga_store::ClickHouseSettings;
use tayga_store::migrate::migrate;
use tayga_store::rows::LogRow;
use tayga_store::store::Store;

const MIN_NS: i64 = 60_000_000_000;

fn now_ns() -> i64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    i64::try_from(d.as_nanos()).unwrap()
}

async fn seeded_store() -> (ClickHouseSettings, Store, Vec<LogRow>) {
    let s = ClickHouseSettings {
        url: std::env::var("TAYGA_IT_CLICKHOUSE")
            .unwrap_or_else(|_| "http://localhost:18123".into()),
        database: format!("tayga_it_{}", rand::random::<u32>()),
    };
    migrate(&s).await.unwrap();
    let store = Store::new(&s);
    let base = now_ns() - 60 * MIN_NS;
    let mut logs = Vec::new();
    let mut id = 1u64;
    for i in 0..30i64 {
        for (service, body) in [
            ("api", format!("user {i} logged in from 10.0.0.{i}")),
            ("api", format!("payment failed for order {}", 1000 + i)),
            ("db", format!("connection pool exhausted after {i} retries")),
        ] {
            logs.push(LogRow {
                log_id: id,
                // Three logs share each timestamp, so keyset paging must break ties by id.
                ts: base + (i / 3) * 1_000_000_000,
                observed_ts: 0,
                trace_id: String::new(),
                span_id: String::new(),
                severity_number: 9,
                severity_text: String::new(),
                service_name: service.into(),
                body,
                resource_attrs: vec![],
                log_attrs: vec![],
            });
            id += 1;
        }
    }
    // A log stored twice (unmerged parts) must be mined once.
    store.insert_logs(&logs).await.unwrap();
    store.insert_logs(&logs[..3]).await.unwrap();
    (s, store, logs)
}

async fn count(store: &Store, table: &str) -> u64 {
    store
        .client()
        .query(&format!("SELECT count() FROM {table}"))
        .fetch_one()
        .await
        .unwrap()
}

async fn drop_db(s: &ClickHouseSettings, store: &Store) {
    store
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}

fn direct_ids(logs: &[LogRow]) -> BTreeSet<u64> {
    let mut sorted: Vec<&LogRow> = logs.iter().collect();
    sorted.sort_by_key(|l| (l.ts, l.log_id));
    let mut miner = Miner::new(DrainConfig::default());
    for l in sorted {
        miner.mine(l);
    }
    miner
        .dirty_templates(1)
        .iter()
        .map(|t| t.template_id)
        .collect()
}

#[tokio::test]
#[ignore = "requires ClickHouse"]
async fn real_run_matches_the_miner_and_stores_state() {
    let (s, store, logs) = seeded_store().await;
    let drain = DrainConfig::default();
    let now = now_ns();
    store
        .state_put(KEY_HEARTBEAT, now - 10 * MIN_NS)
        .await
        .unwrap();

    let sum = remine(&store, &drain, Options::default(), now)
        .await
        .unwrap();

    let expected = direct_ids(&logs);
    let stored: BTreeSet<u64> = store
        .load_templates()
        .await
        .unwrap()
        .iter()
        .map(|t| t.template_id)
        .collect();
    assert_eq!(stored, expected);
    assert!(expected.len() >= 3);
    assert_eq!(sum.logs_read, 90);
    assert_eq!(sum.hits, 90);
    assert_eq!(sum.templates_after, expected.len());
    assert_eq!(sum.added, expected.len());
    assert_eq!(count(&store, "log_template_hits FINAL").await, 90);

    let max_ts = logs.iter().map(|l| l.ts).max().unwrap();
    assert_eq!(store.state_get(KEY_WATERMARK).await.unwrap(), Some(max_ts));
    assert_eq!(store.state_get(KEY_EPOCH_START).await.unwrap(), Some(now));
    assert_eq!(
        store.state_get(KEY_MASKING_VERSION).await.unwrap(),
        Some(i64::from(masking_version(drain.keep_http_status)))
    );

    // A second run replaces the tables with the same ids and no leftovers.
    let again = remine(&store, &drain, Options::default(), now_ns())
        .await
        .unwrap();
    assert_eq!(again.templates_before, expected.len());
    assert_eq!((again.added, again.removed), (0, 0));
    assert_eq!(again.unchanged, expected.len());
    assert_eq!(count(&store, "log_template_hits FINAL").await, 90);
    drop_db(&s, &store).await;
}

/// Seven logs per page: 13 pages, and each of the three templates changes on every page. The
/// stored row must be the one from the last page (full `count` and `last_seen`), which the
/// strictly increasing per-page version guarantees without relying on an equal-version tie.
#[tokio::test]
#[ignore = "requires ClickHouse"]
async fn templates_spanning_many_pages_keep_the_last_page() {
    const PAGE: u32 = 7;
    let (s, store, logs) = seeded_store().await;
    let drain = DrainConfig::default();
    let now = now_ns();
    let sum = remine_paged(&store, &drain, Options::default(), now, PAGE)
        .await
        .unwrap();
    assert_eq!(sum.logs_read, 90);
    let pages = 90u64.div_ceil(u64::from(PAGE));
    assert_eq!(pages, 13);

    // Expected per template, from mining the same logs directly in one pass.
    let mut sorted: Vec<&LogRow> = logs.iter().collect();
    sorted.sort_by_key(|l| (l.ts, l.log_id));
    let mut miner = Miner::new(DrainConfig::default());
    for l in &sorted {
        miner.mine(l);
    }
    let mut expected = miner.dirty_templates(1);
    expected.sort_by_key(|t| t.template_id);
    assert_eq!(expected.len(), 3);

    let mut stored = store.load_templates().await.unwrap();
    stored.sort_by_key(|t| t.template_id);
    assert_eq!(stored.len(), expected.len());
    for (got, want) in stored.iter().zip(&expected) {
        assert_eq!(got.template_id, want.template_id);
        assert_eq!(got.count, 30, "{}", got.template);
        assert_eq!(
            (got.count, got.first_seen, got.last_seen, &got.template),
            (want.count, want.first_seen, want.last_seen, &want.template)
        );
        // Each template changed on every page, so its winning row is the last page's.
        assert_eq!(got.version, u64::try_from(now).unwrap() + pages - 1);
    }
    // Every template was written once per page, each time with a new version.
    let per_template: Vec<(u64, u64, u64)> = store
        .client()
        .query(
            "SELECT template_id, count(), uniqExact(version) FROM log_templates \
             GROUP BY template_id ORDER BY template_id",
        )
        .fetch_all()
        .await
        .unwrap();
    for (id, rows, versions) in per_template {
        assert_eq!(
            rows, versions,
            "template {id}: a version repeats across pages"
        );
    }
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse"]
async fn dry_run_writes_nothing() {
    let (s, store, logs) = seeded_store().await;
    let drain = DrainConfig::default();
    // Existing derived data and state that a real run would replace.
    store
        .upsert_templates(&[tayga_store::logs::LogTemplateRow {
            template_id: 42,
            service: "api".into(),
            template: "old <*>".into(),
            first_seen: now_ns() - MIN_NS,
            last_seen: now_ns() - MIN_NS,
            count: 1,
            max_severity: 9,
            sample: "old 1".into(),
            version: 1,
        }])
        .await
        .unwrap();
    store.silence_put(42, true, 10).await.unwrap();
    store.state_put(KEY_WATERMARK, 7).await.unwrap();
    let tables = ["log_templates", "log_template_hits", "log_template_minutes"];
    let mut before = Vec::new();
    for t in tables {
        before.push(count(&store, t).await);
    }
    let state_before = count(&store, "logminer_state").await;

    let sum = remine(
        &store,
        &drain,
        Options {
            dry_run: true,
            force: false,
        },
        now_ns(),
    )
    .await
    .unwrap();

    let expected = direct_ids(&logs);
    assert_eq!(sum.templates_after, expected.len());
    assert_eq!(sum.removed, 1);
    assert_eq!(sum.orphaned_silence, vec![42]);
    assert_eq!(sum.hits, 90);
    for (t, n) in tables.iter().zip(before) {
        assert_eq!(count(&store, t).await, n, "{t}");
    }
    assert_eq!(count(&store, "logminer_state").await, state_before);
    assert_eq!(store.state_get(KEY_WATERMARK).await.unwrap(), Some(7));
    assert_eq!(store.state_get(KEY_EPOCH_START).await.unwrap(), None);
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse"]
async fn a_fresh_heartbeat_refuses_a_real_run_but_not_a_dry_run() {
    let (s, store, _) = seeded_store().await;
    let drain = DrainConfig::default();
    let now = now_ns();
    store
        .state_put(KEY_HEARTBEAT, now - 5_000_000_000)
        .await
        .unwrap();
    let err = remine(&store, &drain, Options::default(), now)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("looks alive"), "{err}");
    assert_eq!(count(&store, "log_template_hits").await, 0);
    assert_eq!(store.state_get(KEY_EPOCH_START).await.unwrap(), None);

    // A dry run is read-only, so a live logminer does not stop it.
    let dry = remine(
        &store,
        &drain,
        Options {
            dry_run: true,
            force: false,
        },
        now,
    )
    .await
    .unwrap();
    assert_eq!(dry.logs_read, 90);
    assert_eq!(count(&store, "log_templates").await, 0);
    assert_eq!(count(&store, "log_template_hits").await, 0);
    assert_eq!(store.state_get(KEY_EPOCH_START).await.unwrap(), None);

    let forced = remine(
        &store,
        &drain,
        Options {
            dry_run: false,
            force: true,
        },
        now,
    )
    .await
    .unwrap();
    assert_eq!(forced.logs_read, 90);
    assert_eq!(store.state_get(KEY_EPOCH_START).await.unwrap(), Some(now));
    drop_db(&s, &store).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse"]
async fn a_fresh_replica_heartbeat_refuses_and_partition_watermarks_follow_the_remine() {
    let (s, store, logs) = seeded_store().await;
    let drain = DrainConfig::default();
    let now = now_ns();
    // A stale legacy key and a stale replica, but one replica is alive.
    store
        .state_put(KEY_HEARTBEAT, now - 60 * MIN_NS)
        .await
        .unwrap();
    store
        .state_put(&heartbeat_key("a"), now - 10 * MIN_NS)
        .await
        .unwrap();
    store
        .state_put(&heartbeat_key("b"), now - 5_000_000_000)
        .await
        .unwrap();
    let err = remine(&store, &drain, Options::default(), now)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("looks alive"), "{err}");
    assert_eq!(count(&store, "log_template_hits").await, 0);

    // Every replica stopped: the run proceeds and moves the per-partition watermarks too.
    store
        .state_put(&heartbeat_key("b"), now - 10 * MIN_NS)
        .await
        .unwrap();
    store.state_put(&watermark_key(0), 3).await.unwrap();
    store.state_put(&watermark_key(5), 4).await.unwrap();
    remine(&store, &drain, Options::default(), now)
        .await
        .unwrap();
    let max_ts = logs.iter().map(|l| l.ts).max().unwrap();
    assert_eq!(
        store.state_get_prefix(KEY_WATERMARK).await.unwrap(),
        [
            (KEY_WATERMARK.to_string(), max_ts),
            (watermark_key(0), max_ts),
            (watermark_key(5), max_ts)
        ]
    );
    drop_db(&s, &store).await;
}
