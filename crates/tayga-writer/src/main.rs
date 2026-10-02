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
use tayga_writer::retry::retry_until;
use tokio::sync::watch;

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
    let (stop_tx, mut stop_rx) = watch::channel(false);
    tokio::spawn(async move {
        tayga_common::shutdown_signal().await;
        let _ = stop_tx.send(true);
    });
    tracing::info!(topic = %settings.kafka.topic, "tayga-writer consuming");

    // Set when shutdown interrupted a flush: nothing was committed, rows are re-read on restart.
    let mut interrupted = false;
    let mut main_stop = stop_rx.clone();
    loop {
        tokio::select! {
            _ = main_stop.wait_for(|stop| *stop) => break,
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
        if batch.should_flush(Instant::now(), settings.writer.max_rows, max_age) {
            let pending = std::mem::take(&mut batch);
            if !flush(&store, &consumer, &settings.kafka.topic, pending, Some(&mut stop_rx)).await? {
                interrupted = true;
                break;
            }
        }
    }
    if !interrupted && !batch.commit_offsets().is_empty() {
        // Single attempt: on failure exit without committing.
        flush(&store, &consumer, &settings.kafka.topic, batch, None).await?;
    }
    tracing::info!("tayga-writer stopped");
    Ok(())
}

/// Inserts, then commits. Offsets are never committed for rows not yet stored.
/// With `shutdown` it retries until the insert succeeds or shutdown fires; without it, one attempt.
/// Returns `Ok(true)` if the batch was stored and committed, `Ok(false)` if it was not stored.
async fn flush(
    store: &Store,
    consumer: &StreamConsumer,
    topic: &str,
    batch: Batch,
    shutdown: Option<&mut watch::Receiver<bool>>,
) -> anyhow::Result<bool> {
    let insert = || async {
        store.insert_spans(&batch.spans).await?;
        store.insert_logs(&batch.logs).await
    };
    let stored = match shutdown {
        Some(rx) => retry_until(insert, rx).await.is_some(),
        None => match insert().await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, "final clickhouse insert failed");
                false
            }
        },
    };
    if !stored {
        tracing::warn!(rows = batch.rows(), "rows remain uncommitted and will be re-read on restart");
        return Ok(false);
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
    Ok(true)
}
