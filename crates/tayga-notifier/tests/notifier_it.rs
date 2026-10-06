//! Dedup through `notifier_deliveries` against a real ClickHouse, with mock webhooks:
//! `TAYGA_IT_CLICKHOUSE=http://localhost:18123 cargo test -p tayga-notifier --test notifier_it -- --ignored`

use futures::future::join_all;
use std::net::SocketAddr;
use std::time::Duration;
use tayga_e2e::MockWebhook;
use tayga_notifier::config::{Target, TargetKind, WebhookUrl};
use tayga_notifier::deliver::{Breakers, Deliverer, Resolution, Sender};
use tayga_notifier::metrics::NotifierMetrics;
use tayga_notifier::payload::{AlertKind, AlertMsg, slack_payload, webhook_payload};
use tayga_store::ClickHouseSettings;
use tayga_store::migrate::migrate;
use tayga_store::notifier::{DeliveryRow, STATUS_DELIVERED, STATUS_PENDING};
use tayga_store::store::Store;
use tokio::sync::watch;

fn settings() -> ClickHouseSettings {
    let url =
        std::env::var("TAYGA_IT_CLICKHOUSE").unwrap_or_else(|_| "http://localhost:18123".into());
    let suffix: u32 = rand::random();
    ClickHouseSettings {
        url,
        database: format!("tayga_it_{suffix}"),
    }
}

fn alert(id: &str) -> AlertMsg {
    AlertMsg {
        alert_id: id.into(),
        kind: AlertKind::Spike,
        template_id: "42".into(),
        service: "checkout".into(),
        template: "payment <*> declined".into(),
        started_at_ns: 1_700_000_000_000_000_000,
        last_at_ns: 1_700_000_300_000_000_000,
        window_count: 12,
        peak_count: 12,
        baseline_per_window: 1.5,
        example_trace_ids: vec!["t1".into()],
    }
}

fn target(name: &str, kind: TargetKind, url: String) -> Target {
    Target {
        name: name.into(),
        kind,
        url: WebhookUrl::new(url),
    }
}

/// One notifier process: what a restart replaces.
struct Process {
    store: Store,
    sender: Sender,
    metrics: NotifierMetrics,
    /// In memory, like the binary's: a restart starts closed.
    breakers: Breakers,
}

impl Process {
    fn start(s: &ClickHouseSettings) -> Self {
        Self {
            store: Store::new(s),
            sender: Sender::new(Duration::from_secs(5)).unwrap(),
            metrics: NotifierMetrics::default(),
            breakers: Breakers::new(Duration::from_secs(300)),
        }
    }

    /// Delivers `a` to every target, as `main.rs` does for one record.
    async fn deliver(&self, a: &AlertMsg, targets: &[Target]) -> Vec<Resolution> {
        let d = Deliverer {
            log: &self.store,
            sender: &self.sender,
            metrics: &self.metrics,
            max_attempts: 8,
            breakers: &self.breakers,
        };
        let (_tx, rx) = watch::channel(false);
        let webhook = webhook_payload(a, "http://localhost:8090");
        let slack = slack_payload(a, "http://localhost:8090");
        join_all(targets.iter().map(|t| {
            let body = match t.kind {
                TargetKind::Webhook => &webhook,
                TargetKind::Slack => &slack,
            };
            d.deliver(&a.alert_id, t, body, rx.clone())
        }))
        .await
    }
}

#[tokio::test]
#[ignore = "requires ClickHouse: make it"]
async fn delivers_once_per_target_across_republishes_and_restarts() {
    let s = settings();
    migrate(&s).await.unwrap();
    let local = SocketAddr::from(([127, 0, 0, 1], 0));
    let hook = MockWebhook::start(local, &[503]).await.unwrap();
    let slack = MockWebhook::start(local, &[]).await.unwrap();
    let targets = [
        target("hook", TargetKind::Webhook, hook.url()),
        target("slack", TargetKind::Slack, slack.url()),
    ];
    let a = alert("a1");

    let p = Process::start(&s);
    assert_eq!(
        p.deliver(&a, &targets).await,
        [Resolution::Delivered, Resolution::Delivered]
    );
    assert_eq!(hook.received().len(), 2, "503, then 200");
    assert_eq!(hook.received()[1].body["alert_id"], "a1");
    assert!(slack.received()[0].body["blocks"].is_array());

    // The logminer re-publishes the alert on every update.
    for _ in 0..3 {
        assert_eq!(
            p.deliver(&a, &targets).await,
            [Resolution::AlreadyResolved, Resolution::AlreadyResolved]
        );
    }
    // A restart before the offset commit re-reads the record.
    drop(p);
    let p = Process::start(&s);
    assert_eq!(
        p.deliver(&a, &targets).await,
        [Resolution::AlreadyResolved, Resolution::AlreadyResolved]
    );
    assert_eq!((hook.received().len(), slack.received().len()), (2, 1));

    let state = |t: &'static str| {
        let store = &p.store;
        async move { store.delivery_get("a1", t).await.unwrap().unwrap() }
    };
    let h = state("hook").await;
    assert_eq!(
        (h.status, h.attempts, h.last_error.as_str()),
        (STATUS_DELIVERED, 2, "")
    );
    assert_eq!(state("slack").await.status, STATUS_DELIVERED);

    // A restart in the middle of retries resumes the attempt count from the stored row.
    p.store
        .delivery_put(&DeliveryRow {
            alert_id: "a2".into(),
            target: "slack".into(),
            status: STATUS_PENDING,
            attempts: 3,
            last_error: "HTTP 503".into(),
        })
        .await
        .unwrap();
    let p = Process::start(&s);
    assert_eq!(
        p.deliver(&alert("a2"), &targets[1..]).await,
        [Resolution::Delivered]
    );
    let r = p.store.delivery_get("a2", "slack").await.unwrap().unwrap();
    assert_eq!((r.status, r.attempts), (STATUS_DELIVERED, 4));
    assert_eq!(slack.received().len(), 2);

    p.store
        .client()
        .query(&format!("DROP DATABASE `{}`", s.database))
        .execute()
        .await
        .unwrap();
}
