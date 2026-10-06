use prometheus_client::registry::Registry;
use rdkafka::consumer::{
    BaseConsumer, CommitMode, Consumer, ConsumerContext, Rebalance, StreamConsumer,
};
use rdkafka::message::{BorrowedMessage, Headers};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::{ClientContext, Message, Offset, TopicPartitionList};
use serde::Deserialize;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tayga_analysis::baseline::Thresholds;
use tayga_assembler::baselines::Refresh;
use tayga_assembler::metrics::AssemblerMetrics;
use tayga_assembler::pipeline::{Outputs, process};
use tayga_assembler::window::{ClosedTrace, WindowConfig, Windows};
use tayga_common::metrics::KindLabel;
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
const FINAL_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

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
    metrics_addr: SocketAddr,
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
            metrics_addr: SocketAddr::from(([0, 0, 0, 0], 9100)),
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
    let mut registry = Registry::default();
    let metrics = AssemblerMetrics::register(&mut registry);
    let mut stop = tayga_common::shutdown_flag();
    tayga_common::metrics::spawn_server(a.metrics_addr, Arc::new(registry), stop.clone()).await?;
    let clock = Instant::now();
    let mut baselines = Refresh::default();
    load_baselines(
        &store,
        a.baseline_window_minutes,
        &mut baselines,
        clock.elapsed().as_secs(),
        &settings.thresholds,
        &metrics,
    )
    .await;
    let mut main_stop = stop.clone();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut refresh = tokio::time::interval(Duration::from_secs(a.baseline_refresh_secs.max(1)));
    refresh.set_missed_tick_behavior(MissedTickBehavior::Delay);
    refresh.tick().await; // the first tick fires immediately; baselines were just loaded
    let mut pending: Vec<ClosedTrace> = Vec::new();
    let mut ticks: u64 = 0;
    let mut late_items_reported: u64 = 0;
    tracing::info!(topic = %settings.kafka.topic, stories = %a.stories_topic, "tayga-assembler consuming");

    loop {
        tokio::select! {
            _ = main_stop.wait_for(|s| *s) => break,
            _ = tick.tick() => {
                drop_partitions(&mut windows, &mut pending, &revoked.take());
                pending.extend(windows.close_due(Instant::now()));
                let started = Instant::now();
                let outputs = process(&pending, &baselines.baselines, &settings.thresholds);
                let written = write_outputs(
                    &store,
                    &producer,
                    &a.stories_topic,
                    &outputs,
                    &metrics,
                    &mut stop,
                )
                .await;
                // Records were not read while analysing and writing; that time is not trace inactivity.
                windows.shift(started.elapsed());
                if !written {
                    break; // shutdown during retries: nothing committed, records are re-read on restart
                }
                record_outputs(&metrics, pending.len(), &outputs);
                pending.clear();
                commit(&consumer, &settings.kafka.topic, &mut windows, &mut pending, CommitMode::Async);
                metrics.open_traces.set(windows.open_traces() as i64);
                metrics.buffered_bytes.set(windows.buffered_bytes() as i64);
                metrics.record_late_items(&mut late_items_reported, windows.late_items());
                ticks += 1;
                if ticks.is_multiple_of(STATS_EVERY_TICKS) {
                    tracing::info!(
                        open_traces = windows.open_traces(),
                        buffered_bytes = windows.buffered_bytes(),
                        late_items = windows.late_items(),
                        endpoints_with_baseline = baselines.baselines.len(),
                        "window stats"
                    );
                }
                if outputs.failed > 0 {
                    tracing::warn!(failed = outputs.failed, "traces skipped after analysis panic");
                }
            }
            _ = refresh.tick() => {
                let started = Instant::now();
                load_baselines(
                    &store,
                    a.baseline_window_minutes,
                    &mut baselines,
                    clock.elapsed().as_secs(),
                    &settings.thresholds,
                    &metrics,
                )
                .await;
                windows.shift(started.elapsed());
            }
            msg = consumer.recv() => match msg {
                Ok(m) => {
                    drop_partitions(&mut windows, &mut pending, &revoked.take());
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
    if !pending.is_empty() {
        let outputs = process(&pending, &baselines.baselines, &settings.thresholds);
        let write = write_once(&store, &producer, &a.stories_topic, &outputs, &metrics);
        match tokio::time::timeout(FINAL_WRITE_TIMEOUT, write).await {
            Ok(Ok(())) => {
                tracing::info!(traces = pending.len(), "wrote closed traces at shutdown");
                pending.clear();
                commit(
                    &consumer,
                    &settings.kafka.topic,
                    &mut windows,
                    &mut pending,
                    CommitMode::Sync,
                );
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, traces = pending.len(), "final write failed; traces are re-read on restart")
            }
            Err(_) => tracing::warn!(
                traces = pending.len(),
                "final write timed out; traces are re-read on restart"
            ),
        }
    }
    tracing::info!(
        open_traces = windows.open_traces(),
        "tayga-assembler stopped; open traces are re-read on restart"
    );
    Ok(())
}

/// Refreshes `state` in place; a failed refresh keeps the previous baselines and carries.
async fn load_baselines(
    store: &Store,
    window_minutes: u32,
    state: &mut Refresh,
    now_secs: u64,
    thresholds: &Thresholds,
    metrics: &AssemblerMetrics,
) {
    match tayga_assembler::baselines::load(
        store,
        window_minutes,
        &state.baselines,
        &state.carried,
        now_secs,
        thresholds,
    )
    .await
    {
        Ok(r) => {
            metrics.baseline_excluded_traces.set(r.excluded as i64);
            metrics
                .baseline_carried_endpoints
                .set(r.carried.len() as i64);
            metrics.baseline_endpoints.set(r.baselines.len() as i64);
            tracing::debug!(
                endpoints = r.baselines.len(),
                excluded = r.excluded,
                carried = r.carried.len(),
                "baselines refreshed"
            );
            *state = r;
        }
        Err(e) => {
            tracing::warn!(error = %e, "baseline refresh failed; keeping previous baselines");
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

/// Story kind as stored in `error_stories.kind`: 1 = error, 2 = slow.
fn kind_label(kind: i8) -> &'static str {
    if kind == 1 { "error" } else { "slow" }
}

/// Counters for one successfully written tick.
fn record_outputs(metrics: &AssemblerMetrics, closed: usize, outputs: &Outputs) {
    metrics.closed_traces.inc_by(closed as u64);
    for story in &outputs.stories {
        metrics
            .stories
            .get_or_create(&KindLabel::new(kind_label(story.kind)))
            .inc();
    }
    metrics.analysis_panics.inc_by(outputs.failed as u64);
    metrics
        .serialization_failures
        .inc_by(outputs.serialization_failures as u64);
}

/// Inserts the analysis rows. Edges go last: they are aggregated by a SummingMergeTree, so a
/// retry after a failed summary/story insert must not have inserted them already.
async fn insert_outputs(store: &Store, out: &Outputs) -> anyhow::Result<()> {
    store.insert_rows("trace_summaries", &out.summaries).await?;
    store.insert_rows("error_stories", &out.stories).await?;
    store.insert_rows("service_edges", &out.edges).await?;
    Ok(())
}

async fn publish_stories(
    producer: &FutureProducer,
    topic: &str,
    out: &Outputs,
) -> Result<(), rdkafka::error::KafkaError> {
    let sends = out.story_messages.iter().map(|(key, json)| {
        producer.send(
            FutureRecord::to(topic).key(key).payload(json),
            Duration::from_secs(5),
        )
    });
    for result in futures::future::join_all(sends).await {
        result.map_err(|(e, _)| e)?;
    }
    Ok(())
}

/// Returns false if shutdown interrupted the writes (nothing may be committed then).
async fn write_outputs(
    store: &Store,
    producer: &FutureProducer,
    topic: &str,
    out: &Outputs,
    metrics: &AssemblerMetrics,
    stop: &mut watch::Receiver<bool>,
) -> bool {
    if out.is_empty() {
        return true;
    }
    let insert = || async {
        insert_outputs(store, out).await.inspect_err(|_| {
            metrics.write_failures.inc();
        })
    };
    if retry_until("clickhouse insert", insert, stop)
        .await
        .is_none()
    {
        return false;
    }
    let publish = || async {
        publish_stories(producer, topic, out)
            .await
            .inspect_err(|_| {
                metrics.write_failures.inc();
            })
    };
    retry_until("story publish", publish, stop).await.is_some()
}

/// One write attempt for traces closed but not yet written at shutdown.
async fn write_once(
    store: &Store,
    producer: &FutureProducer,
    topic: &str,
    out: &Outputs,
    metrics: &AssemblerMetrics,
) -> anyhow::Result<()> {
    let result = async {
        insert_outputs(store, out).await?;
        publish_stories(producer, topic, out).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        metrics.write_failures.inc();
    }
    result
}

/// Drops all state of partitions this consumer no longer owns, including closed traces not yet
/// written: the new owner re-reads them from the last committed offset.
fn drop_partitions(windows: &mut Windows, pending: &mut Vec<ClosedTrace>, partitions: &[i32]) {
    if partitions.is_empty() {
        return;
    }
    windows.revoke(partitions);
    pending.retain(|t| !partitions.contains(&t.partition));
}

/// Commits the windows' offsets for partitions still assigned; state of partitions that are
/// held but no longer assigned (a missed revoke) is dropped instead.
fn commit(
    consumer: &StreamConsumer<Ctx>,
    topic: &str,
    windows: &mut Windows,
    pending: &mut Vec<ClosedTrace>,
    mode: CommitMode,
) {
    let assigned: HashSet<i32> = match consumer.assignment() {
        Ok(tpl) => tpl
            .elements()
            .iter()
            .filter(|e| e.topic() == topic)
            .map(|e| e.partition())
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "cannot read assignment; skipping commit");
            return;
        }
    };
    let (offsets, stale): (Vec<_>, Vec<_>) = windows
        .commit_offsets()
        .into_iter()
        .partition(|(p, _)| assigned.contains(p));
    if !stale.is_empty() {
        let stale: Vec<i32> = stale.into_iter().map(|(p, _)| p).collect();
        tracing::info!(partitions = ?stale, "dropping state of unassigned partitions");
        drop_partitions(windows, pending, &stale);
    }
    if offsets.is_empty() {
        return;
    }
    let mut tpl = TopicPartitionList::new();
    for &(partition, offset) in &offsets {
        if let Err(e) = tpl.add_partition_offset(topic, partition, Offset::Offset(offset)) {
            tracing::warn!(error = %e, partition, "cannot build commit list");
            return;
        }
    }
    if let Err(e) = consumer.commit(&tpl, mode) {
        // Typically a partition revoked by a concurrent rebalance; the next tick retries.
        tracing::warn!(error = %e, "offset commit failed");
    }
}
