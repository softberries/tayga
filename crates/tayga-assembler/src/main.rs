use rdkafka::consumer::{
    BaseConsumer, CommitMode, Consumer, ConsumerContext, Rebalance, StreamConsumer,
};
use rdkafka::message::{BorrowedMessage, Headers};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::{ClientContext, Message, Offset, TopicPartitionList};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tayga_analysis::baseline::{Baseline, Thresholds};
use tayga_analysis::model::Endpoint;
use tayga_assembler::pipeline::{Outputs, process};
use tayga_assembler::window::{ClosedTrace, WindowConfig, Windows};
use tayga_common::retry::retry_until;
use tayga_kafka::KafkaSettings;
use tayga_model::envelope::{Envelope, HEADER_KEY_KIND};
use tayga_model::ids::TraceId;
use tayga_store::ClickHouseSettings;
use tayga_store::store::Store;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;

const GROUP: &str = "tayga-assembler";
const STATS_EVERY_TICKS: u64 = 30;

#[derive(Deserialize)]
struct Settings {
    kafka: KafkaSettings,
    clickhouse: ClickHouseSettings,
    #[serde(default)]
    assembler: AssemblerSettings,
    #[serde(default)]
    thresholds: Thresholds,
}

#[derive(Deserialize)]
#[serde(default)]
struct AssemblerSettings {
    stories_topic: String,
    stories_partitions: i32,
    gap_ms: u64,
    max_age_ms: u64,
    max_spans: usize,
    max_buffer_bytes: usize,
    recent_per_partition: usize,
    baseline_window_minutes: u32,
    baseline_refresh_secs: u64,
}

impl Default for AssemblerSettings {
    fn default() -> Self {
        Self {
            stories_topic: "tayga.stories".to_string(),
            stories_partitions: 3,
            gap_ms: 10_000,
            max_age_ms: 60_000,
            max_spans: 10_000,
            max_buffer_bytes: 512 * 1024 * 1024,
            recent_per_partition: 100_000,
            baseline_window_minutes: 60,
            baseline_refresh_secs: 60,
        }
    }
}

/// Partitions revoked by the last rebalance; drained by the main loop before it touches state.
#[derive(Clone, Default)]
struct Revoked(Arc<Mutex<Vec<i32>>>);

impl Revoked {
    fn take(&self) -> Vec<i32> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

struct Ctx {
    revoked: Revoked,
}

impl ClientContext for Ctx {}

impl ConsumerContext for Ctx {
    fn pre_rebalance(&self, _consumer: &BaseConsumer<Self>, rebalance: &Rebalance<'_>) {
        if let Rebalance::Revoke(tpl) = rebalance {
            let partitions = tpl
                .elements()
                .iter()
                .map(|e| e.partition())
                .collect::<Vec<_>>();
            tracing::info!(?partitions, "partitions revoked");
            self.revoked
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(partitions);
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tayga_common::init_logging();
    let settings: Settings = tayga_common::load_settings()?;
    settings.kafka.validate()?;
    let a = &settings.assembler;
    let store = Store::new(&settings.clickhouse);
    tayga_kafka::ensure_topic(&settings.kafka).await?;
    let stories = KafkaSettings {
        topic: a.stories_topic.clone(),
        partitions: a.stories_partitions,
        ..settings.kafka.clone()
    };
    tayga_kafka::ensure_topic(&stories).await?;
    let producer = tayga_kafka::producer(&settings.kafka)?;
    let revoked = Revoked::default();
    let consumer: StreamConsumer<Ctx> = tayga_kafka::consumer_with_context(
        &settings.kafka,
        GROUP,
        Ctx {
            revoked: revoked.clone(),
        },
    )?;
    consumer.subscribe(&[&settings.kafka.topic])?;

    let mut windows = Windows::new(WindowConfig {
        gap: Duration::from_millis(a.gap_ms),
        max_age: Duration::from_millis(a.max_age_ms),
        max_spans: a.max_spans,
        max_bytes: a.max_buffer_bytes,
        recent_per_partition: a.recent_per_partition,
    });
    let mut baselines = load_baselines(&store, a.baseline_window_minutes, HashMap::new()).await;
    let mut stop = tayga_common::shutdown_flag();
    let mut main_stop = stop.clone();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut refresh = tokio::time::interval(Duration::from_secs(a.baseline_refresh_secs.max(1)));
    refresh.set_missed_tick_behavior(MissedTickBehavior::Delay);
    refresh.tick().await; // the first tick fires immediately; baselines were just loaded
    let mut pending: Vec<ClosedTrace> = Vec::new();
    let mut ticks: u64 = 0;
    tracing::info!(topic = %settings.kafka.topic, stories = %a.stories_topic, "tayga-assembler consuming");

    loop {
        tokio::select! {
            _ = main_stop.wait_for(|s| *s) => break,
            _ = tick.tick() => {
                windows.revoke(&revoked.take());
                pending.extend(windows.close_due(Instant::now()));
                let started = Instant::now();
                let outputs = process(&pending, &baselines, &settings.thresholds);
                let written = write_outputs(&store, &producer, &a.stories_topic, &outputs, &mut stop).await;
                // Records were not read while analysing and writing; that time is not trace inactivity.
                windows.shift(started.elapsed());
                if !written {
                    break; // shutdown during retries: nothing committed, records are re-read on restart
                }
                pending.clear();
                commit(&consumer, &settings.kafka.topic, &windows.commit_offsets());
                ticks += 1;
                if ticks.is_multiple_of(STATS_EVERY_TICKS) {
                    tracing::info!(
                        open_traces = windows.open_traces(),
                        buffered_bytes = windows.buffered_bytes(),
                        late_items = windows.late_items(),
                        endpoints_with_baseline = baselines.len(),
                        "window stats"
                    );
                }
                if outputs.failed > 0 {
                    tracing::warn!(failed = outputs.failed, "traces skipped after analysis panic");
                }
            }
            _ = refresh.tick() => {
                let started = Instant::now();
                baselines = load_baselines(&store, a.baseline_window_minutes, baselines).await;
                windows.shift(started.elapsed());
            }
            msg = consumer.recv() => match msg {
                Ok(m) => {
                    windows.revoke(&revoked.take());
                    pending.extend(ingest(&mut windows, &m));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "kafka receive error");
                    let mut backoff_stop = stop.clone();
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                        _ = backoff_stop.wait_for(|s| *s) => break,
                    }
                }
            },
        }
    }
    tracing::info!(
        open_traces = windows.open_traces(),
        "tayga-assembler stopped; open traces are re-read on restart"
    );
    Ok(())
}

async fn load_baselines(
    store: &Store,
    window_minutes: u32,
    previous: HashMap<Endpoint, Baseline>,
) -> HashMap<Endpoint, Baseline> {
    match tayga_assembler::baselines::load(store, window_minutes).await {
        Ok(b) => {
            tracing::debug!(endpoints = b.len(), "baselines refreshed");
            b
        }
        Err(e) => {
            tracing::warn!(error = %e, "baseline refresh failed; keeping previous baselines");
            previous
        }
    }
}

/// Trace id from the record key, unless the `tayga-key` header marks a service key.
fn trace_key(m: &BorrowedMessage<'_>) -> Option<String> {
    let service_keyed = m
        .headers()
        .and_then(|h| h.iter().find(|h| h.key == HEADER_KEY_KIND))
        .is_some_and(|h| h.value == Some(b"service".as_slice()));
    if service_keyed {
        return None;
    }
    m.key().and_then(TraceId::from_slice).map(|t| t.to_hex())
}

fn ingest(windows: &mut Windows, m: &BorrowedMessage<'_>) -> Vec<ClosedTrace> {
    let now = Instant::now();
    let empty = Envelope::default();
    let Some(payload) = m.payload() else {
        return windows.ingest(m.partition(), m.offset(), None, &empty, 0, now);
    };
    match Envelope::decode(payload) {
        Ok(env) => windows.ingest(
            m.partition(),
            m.offset(),
            trace_key(m).as_deref(),
            &env,
            payload.len(),
            now,
        ),
        Err(e) => {
            tracing::warn!(partition = m.partition(), offset = m.offset(), error = %e, "skipping undecodable envelope");
            windows.ingest(m.partition(), m.offset(), None, &empty, 0, now)
        }
    }
}

/// Returns false if shutdown interrupted the writes (nothing may be committed then).
async fn write_outputs(
    store: &Store,
    producer: &FutureProducer,
    topic: &str,
    out: &Outputs,
    stop: &mut watch::Receiver<bool>,
) -> bool {
    if out.is_empty() {
        return true;
    }
    let insert = || async {
        store.insert_rows("trace_summaries", &out.summaries).await?;
        store.insert_rows("service_edges", &out.edges).await?;
        store.insert_rows("error_stories", &out.stories).await
    };
    if retry_until("clickhouse insert", insert, stop)
        .await
        .is_none()
    {
        return false;
    }
    let publish = || async {
        let sends = out.story_messages.iter().map(|(key, json)| {
            producer.send(
                FutureRecord::to(topic).key(key).payload(json),
                Duration::from_secs(5),
            )
        });
        for result in futures::future::join_all(sends).await {
            result.map_err(|(e, _)| e)?;
        }
        Ok::<(), rdkafka::error::KafkaError>(())
    };
    retry_until("story publish", publish, stop).await.is_some()
}

fn commit(consumer: &StreamConsumer<Ctx>, topic: &str, offsets: &[(i32, i64)]) {
    if offsets.is_empty() {
        return;
    }
    let mut tpl = TopicPartitionList::new();
    for &(partition, offset) in offsets {
        if let Err(e) = tpl.add_partition_offset(topic, partition, Offset::Offset(offset)) {
            tracing::warn!(error = %e, partition, "cannot build commit list");
            return;
        }
    }
    if let Err(e) = consumer.commit(&tpl, CommitMode::Async) {
        // Typically a partition revoked by a concurrent rebalance; the next tick retries.
        tracing::warn!(error = %e, "offset commit failed");
    }
}
