use futures::future::join_all;
use prometheus_client::registry::Registry;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::BorrowedMessage;
use rdkafka::{Message, Offset, TopicPartitionList};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tayga_kafka::KafkaSettings;
use tayga_notifier::config::{NotifierSettings, TargetKind};
use tayga_notifier::deliver::{Deliverer, Resolution, Sender};
use tayga_notifier::metrics::NotifierMetrics;
use tayga_notifier::payload::{AlertMsg, slack_payload, webhook_payload};
use tayga_store::ClickHouseSettings;
use tayga_store::store::Store;
use tokio::sync::watch;

const GROUP: &str = "tayga-notifier";
/// Matches the logminer, which normally creates the topic first.
const ALERTS_PARTITIONS: i32 = 3;

#[derive(Deserialize)]
struct Settings {
    kafka: KafkaSettings,
    clickhouse: ClickHouseSettings,
    #[serde(default)]
    notifier: NotifierSettings,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tayga_common::init_logging();
    let settings: Settings = tayga_common::load_settings()?;
    run(settings).await
}

async fn run(settings: Settings) -> anyhow::Result<()> {
    settings.kafka.validate()?;
    let cfg = &settings.notifier;
    cfg.validate()?;
    let store = Store::new(&settings.clickhouse);
    let sender = Sender::new(Duration::from_secs(cfg.timeout_secs))?;
    let alerts = KafkaSettings {
        topic: cfg.alerts_topic.clone(),
        partitions: ALERTS_PARTITIONS,
        ..settings.kafka.clone()
    };
    tayga_kafka::ensure_topic(&alerts).await?;
    let consumer = tayga_kafka::consumer(&alerts, GROUP)?;
    consumer.subscribe(&[&alerts.topic])?;

    let stop_rx = tayga_common::shutdown_flag();
    let mut registry = Registry::default();
    let metrics = NotifierMetrics::register(&mut registry);
    let metrics_addr = cfg.metrics_addr;
    let metrics_stop = stop_rx.clone();
    tokio::spawn(async move {
        if let Err(e) =
            tayga_common::metrics::serve(metrics_addr, Arc::new(registry), metrics_stop).await
        {
            tracing::warn!(error = %e, "metrics server stopped");
        }
    });

    let targets: Vec<&str> = cfg.targets.iter().map(|t| t.name.as_str()).collect();
    let kinds: Vec<&str> = cfg.kinds.iter().map(|k| k.as_str()).collect();
    if targets.is_empty() {
        tracing::info!("delivery disabled: no targets");
    }
    tracing::info!(
        topic = %alerts.topic,
        ?targets,
        ?kinds,
        max_attempts = cfg.max_attempts,
        "tayga-notifier consuming"
    );

    let deliverer = Deliverer {
        log: &store,
        sender: &sender,
        metrics: &metrics,
        max_attempts: cfg.max_attempts,
    };
    let mut main_stop = stop_rx.clone();
    loop {
        tokio::select! {
            _ = main_stop.wait_for(|stop| *stop) => break,
            next = tokio::time::timeout(Duration::from_millis(200), consumer.recv()) => match next {
                Ok(Ok(msg)) => {
                    if !handle(&msg, cfg, &deliverer, &stop_rx).await {
                        // Not committed: the record is re-read on restart and deduplicated.
                        break;
                    }
                    commit(&consumer, &msg);
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "kafka receive error");
                    let mut backoff_stop = stop_rx.clone();
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                        _ = backoff_stop.wait_for(|stop| *stop) => break,
                    }
                }
                Err(_) => {}
            },
        }
    }
    tracing::info!("tayga-notifier stopped");
    Ok(())
}

/// Delivers one record to every target. Returns whether every target is resolved (delivered,
/// given up, or resolved earlier), i.e. whether the offset may be committed. Undecodable
/// records, kinds not in `kinds`, and every record when no target is configured are resolved
/// at once.
async fn handle(
    msg: &BorrowedMessage<'_>,
    cfg: &NotifierSettings,
    deliverer: &Deliverer<'_, Store>,
    stop: &watch::Receiver<bool>,
) -> bool {
    let alert: AlertMsg = match msg.payload().map(serde_json::from_slice) {
        Some(Ok(a)) => a,
        Some(Err(e)) => {
            tracing::warn!(partition = msg.partition(), offset = msg.offset(), error = %e, "skipping undecodable alert");
            return true;
        }
        None => return true,
    };
    if cfg.targets.is_empty() || !cfg.delivers(alert.kind) {
        return true;
    }
    let webhook = webhook_payload(&alert, &cfg.public_url);
    let slack = slack_payload(&alert, &cfg.public_url);
    let results = join_all(cfg.targets.iter().map(|target| {
        let body = match target.kind {
            TargetKind::Webhook => &webhook,
            TargetKind::Slack => &slack,
        };
        deliverer.deliver(&alert.alert_id, target, body, stop.clone())
    }))
    .await;
    results.iter().all(|r| *r != Resolution::Interrupted)
}

fn commit(consumer: &StreamConsumer, msg: &BorrowedMessage<'_>) {
    let mut tpl = TopicPartitionList::new();
    let added = tpl.add_partition_offset(
        msg.topic(),
        msg.partition(),
        Offset::Offset(msg.offset() + 1),
    );
    if let Err(e) = added.and_then(|()| consumer.commit(&tpl, CommitMode::Sync)) {
        // Typically a revoked partition after a rebalance; the record is re-read and deduplicated.
        tracing::warn!(error = %e, partition = msg.partition(), offset = msg.offset(), "offset commit failed");
    }
}
