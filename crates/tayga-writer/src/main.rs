use clap::{Parser, Subcommand};
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::{Message, Offset, TopicPartitionList};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tayga_kafka::KafkaSettings;
use tayga_model::envelope::Envelope;
use tayga_store::ClickHouseSettings;
use tayga_store::flatten::rows_from_envelope;
use tayga_store::store::Store;
use tayga_writer::batch::Batch;

const GROUP: &str = "tayga-writer";

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Consume tayga.signals and write raw rows (default).
    Run,
    /// Apply ClickHouse schema migrations and exit.
    Migrate,
}

#[derive(Deserialize)]
struct Settings {
    kafka: KafkaSettings,
    clickhouse: ClickHouseSettings,
    #[serde(default)]
    writer: WriterSettings,
}

#[derive(Deserialize)]
struct WriterSettings {
    #[serde(default = "default_max_rows")]
    max_rows: usize,
    #[serde(default = "default_max_age_ms")]
    max_age_ms: u64,
}

impl Default for WriterSettings {
    fn default() -> Self {
        Self { max_rows: default_max_rows(), max_age_ms: default_max_age_ms() }
    }
}

fn default_max_rows() -> usize {
    10_000
}

fn default_max_age_ms() -> u64 {
    1_000
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cmd = Cli::parse().cmd.unwrap_or(Cmd::Run);
    tayga_common::init_logging();
    let settings: Settings = tayga_common::load_settings()?;
    match cmd {
        Cmd::Migrate => {
            let applied = tayga_store::migrate::migrate(&settings.clickhouse).await?;
            tracing::info!(?applied, "migrations complete");
            Ok(())
        }
        Cmd::Run => run(settings).await,
    }
}

async fn run(settings: Settings) -> anyhow::Result<()> {
    let store = Store::new(&settings.clickhouse);
    let consumer = tayga_kafka::consumer(&settings.kafka, GROUP)?;
    consumer.subscribe(&[&settings.kafka.topic])?;
    let max_age = Duration::from_millis(settings.writer.max_age_ms);
    let mut batch = Batch::default();
    let shutdown = tayga_common::shutdown_signal();
    tokio::pin!(shutdown);
    tracing::info!(topic = %settings.kafka.topic, "tayga-writer consuming");

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            next = tokio::time::timeout(Duration::from_millis(200), consumer.recv()) => match next {
                Ok(Ok(msg)) => {
                    let (spans, logs) = match msg.payload().map(Envelope::decode) {
                        Some(Ok(env)) => rows_from_envelope(&env),
                        Some(Err(e)) => {
                            tracing::warn!(partition = msg.partition(), offset = msg.offset(), error = %e, "skipping undecodable envelope");
                            (Vec::new(), Vec::new())
                        }
                        None => (Vec::new(), Vec::new()),
                    };
                    batch.add(msg.partition(), msg.offset(), spans, logs, Instant::now());
                }
                Ok(Err(e)) => tracing::warn!(error = %e, "kafka receive error"),
                Err(_) => {}
            },
        }
        if batch.should_flush(Instant::now(), settings.writer.max_rows, max_age) {
            flush(&store, &consumer, &settings.kafka.topic, std::mem::take(&mut batch)).await?;
        }
    }
    if !batch.commit_offsets().is_empty() {
        flush(&store, &consumer, &settings.kafka.topic, batch).await?;
    }
    tracing::info!("tayga-writer stopped");
    Ok(())
}

/// Inserts with retry, then commits. Offsets are never committed for rows not yet stored.
async fn flush(store: &Store, consumer: &StreamConsumer, topic: &str, batch: Batch) -> anyhow::Result<()> {
    let mut backoff = Duration::from_millis(100);
    loop {
        let result = async {
            store.insert_spans(&batch.spans).await?;
            store.insert_logs(&batch.logs).await
        }
        .await;
        match result {
            Ok(()) => break,
            Err(e) => {
                tracing::warn!(error = %e, retry_in_ms = backoff.as_millis() as u64, "clickhouse insert failed");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
    let mut tpl = TopicPartitionList::new();
    for (partition, offset) in batch.commit_offsets() {
        tpl.add_partition_offset(topic, partition, Offset::Offset(offset))?;
    }
    if let Err(e) = consumer.commit(&tpl, CommitMode::Sync) {
        // Typically a revoked partition after rebalance; its rows are re-read and deduplicated.
        tracing::warn!(error = %e, "offset commit failed");
    }
    tracing::debug!(spans = batch.spans.len(), logs = batch.logs.len(), "flushed");
    Ok(())
}
