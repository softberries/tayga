use prometheus_client::registry::Registry;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::{BorrowedMessage, Headers};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::{Message, Offset, TopicPartitionList};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tayga_common::metrics::KindLabel;
use tayga_common::retry::retry_until;
use tayga_drain::detect::{
    Alert, BaselineMode, DetectConfig, NewCandidate, SpikeSkip, SpikeTracker, TemplateWindow,
    initial_watermark, is_new, is_silent, new_alert, new_template_since, seasonal_decision,
    silence_alert, spike_baseline, template_coverage,
};
use tayga_drain::drain::DrainConfig;
use tayga_drain::preprocess::masking_version;
use tayga_kafka::KafkaSettings;
use tayga_logminer::metrics::{LogminerMetrics, PRE_EPOCH_MATCH, ReasonLabel};
use tayga_logminer::miner::{Miner, alert_from_row, alert_json, alert_row};
use tayga_model::envelope::{Envelope, HEADER_KIND, Kind};
use tayga_store::ClickHouseSettings;
use tayga_store::flatten::rows_from_envelope;
use tayga_store::logs::{LogHitRow, SeasonalWindow, SilenceInput};
use tayga_store::store::Store;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;

const GROUP: &str = "tayga-logminer";
const EXAMPLES: u32 = 5;
const ALERTS_PARTITIONS: i32 = 3;
const MIN_NS: i64 = 60_000_000_000;
/// Seasonal comparators: the spike window 1 day and 7 days earlier (spec 7a §2.3).
const SEASONAL_SHIFTS_SECS: [u32; 2] = [86_400, 7 * 86_400];
const KEY_WATERMARK: &str = "new_template_watermark_ns";
/// Liveness stamp for `tayga-devtools remine`: written after every detection pass.
const KEY_HEARTBEAT: &str = "logminer_heartbeat_ns";
const KEY_MASKING_VERSION: &str = "masking_version";
const KEY_EPOCH_START: &str = "masking_epoch_start_ns";
/// Timeout of one `fetch_watermarks` call per detection tick.
const WATERMARK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
struct Settings {
    kafka: KafkaSettings,
    clickhouse: ClickHouseSettings,
    #[serde(default)]
    logminer: LogminerSettings,
}

/// Spec §7. Missing keys take the values of [`LogminerSettings::default`].
#[derive(Deserialize, Debug)]
#[serde(default)]
struct LogminerSettings {
    sim_threshold: f64,
    max_clusters_per_service: usize,
    keep_http_status: bool,
    max_batch: usize,
    flush_ms: u64,
    detect_secs: u64,
    spike_window_min: u32,
    baseline_window_min: u32,
    spike_factor: f64,
    spike_min_count: u64,
    new_template_warmup_min: u32,
    alert_active_min: u32,
    /// `"flat"` or `"seasonal"`.
    baseline_mode: String,
    alerts_topic: String,
    metrics_addr: SocketAddr,
}

impl Default for LogminerSettings {
    fn default() -> Self {
        let drain = DrainConfig::default();
        let detect = DetectConfig::default();
        Self {
            sim_threshold: drain.sim_threshold,
            max_clusters_per_service: drain.max_clusters_per_service,
            keep_http_status: drain.keep_http_status,
            max_batch: 5_000,
            flush_ms: 1_000,
            detect_secs: 60,
            spike_window_min: detect.spike_window_min,
            baseline_window_min: detect.baseline_window_min,
            spike_factor: detect.spike_factor,
            spike_min_count: detect.spike_min_count,
            new_template_warmup_min: detect.new_template_warmup_min,
            alert_active_min: detect.alert_active_min,
            baseline_mode: "flat".to_string(),
            alerts_topic: "tayga.alerts".to_string(),
            metrics_addr: SocketAddr::from(([0, 0, 0, 0], 9100)),
        }
    }
}

impl LogminerSettings {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.max_batch > 0, "logminer.max_batch must be positive");
        anyhow::ensure!(
            self.detect_secs > 0,
            "logminer.detect_secs must be positive"
        );
        anyhow::ensure!(
            self.spike_window_min > 0,
            "logminer.spike_window_min must be positive"
        );
        anyhow::ensure!(
            self.max_clusters_per_service > 0,
            "logminer.max_clusters_per_service must be positive"
        );
        self.baseline_mode
            .parse::<BaselineMode>()
            .map_err(|e| anyhow::anyhow!("logminer.baseline_mode: {e}"))?;
        Ok(())
    }

    fn drain(&self) -> DrainConfig {
        DrainConfig {
            sim_threshold: self.sim_threshold,
            max_clusters_per_service: self.max_clusters_per_service,
            keep_http_status: self.keep_http_status,
            ..DrainConfig::default()
        }
    }

    fn detect(&self) -> DetectConfig {
        DetectConfig {
            spike_window_min: self.spike_window_min,
            baseline_window_min: self.baseline_window_min,
            spike_factor: self.spike_factor,
            spike_min_count: self.spike_min_count,
            new_template_warmup_min: self.new_template_warmup_min,
            alert_active_min: self.alert_active_min,
            // Checked by `validate`.
            baseline_mode: self.baseline_mode.parse().unwrap_or_default(),
            ..DetectConfig::default()
        }
    }
}

/// Hits mined since the last flush, and the highest offset seen per partition.
#[derive(Default)]
struct Pending {
    hits: Vec<LogHitRow>,
    offsets: HashMap<i32, i64>,
    /// Newest hit `ts` mined per partition.
    max_ts: HashMap<i32, i64>,
    /// Kafka timestamp (ns) of the newest record per partition, of any kind.
    record_ts: HashMap<i32, i64>,
    since: Option<Instant>,
}

impl Pending {
    fn record(&mut self, partition: i32, offset: i64, record_ts_ns: i64, now: Instant) {
        let o = self.offsets.entry(partition).or_insert(offset);
        *o = (*o).max(offset);
        let t = self.record_ts.entry(partition).or_insert(record_ts_ns);
        *t = (*t).max(record_ts_ns);
        self.since.get_or_insert(now);
    }

    fn record_ts(&mut self, partition: i32, ts: i64) {
        let t = self.max_ts.entry(partition).or_insert(ts);
        *t = (*t).max(ts);
    }

    fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    fn should_flush(&self, now: Instant, max_batch: usize, max_age: Duration) -> bool {
        self.hits.len() >= max_batch
            || self
                .since
                .is_some_and(|since| now.duration_since(since) >= max_age)
    }

    /// Next offset to consume per partition.
    fn commit_offsets(&self) -> Vec<(i32, i64)> {
        let mut v: Vec<(i32, i64)> = self.offsets.iter().map(|(&p, &o)| (p, o + 1)).collect();
        v.sort_unstable();
        v
    }
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tayga_common::init_logging();
    let settings: Settings = tayga_common::load_settings()?;
    run(settings).await
}

async fn run(settings: Settings) -> anyhow::Result<()> {
    settings.kafka.validate()?;
    let cfg = &settings.logminer;
    cfg.validate()?;
    let detect_cfg = cfg.detect();
    let store = Store::new(&settings.clickhouse);
    tayga_kafka::ensure_topic(&settings.kafka).await?;
    tayga_kafka::ensure_topic(&KafkaSettings {
        topic: cfg.alerts_topic.clone(),
        partitions: ALERTS_PARTITIONS,
        ..settings.kafka.clone()
    })
    .await?;
    let producer = tayga_kafka::producer(&settings.kafka)?;
    let mut stop_rx = tayga_common::shutdown_flag();

    let mut registry = Registry::default();
    let metrics = LogminerMetrics::register(&mut registry);

    let mut miner = Miner::new(cfg.drain());
    let Some(templates) =
        retry_until("load templates", || store.load_templates(), &mut stop_rx).await
    else {
        return Ok(());
    };
    let restored = templates.len();
    miner.restore(templates);
    metrics.templates.set(miner.len() as i64);
    let mut tracker = Trackers::default();
    let Some(active) = retry_until(
        "load active spike alerts",
        || store.active_spike_alerts(cfg.alert_active_min),
        &mut stop_rx,
    )
    .await
    else {
        return Ok(());
    };
    let active: Vec<Alert> = active.iter().filter_map(alert_from_row).collect();
    let active_spikes = active.len();
    tracker.spikes.restore(active);
    let Some(data_now) = retry_until("load data clock", || store.data_now_ns(), &mut stop_rx).await
    else {
        return Ok(());
    };
    // Data time up to which new templates have been checked; advanced after each detection pass.
    let now = now_ns();
    let Some(stored_watermark) = retry_until(
        "load watermark",
        || store.state_get(KEY_WATERMARK),
        &mut stop_rx,
    )
    .await
    else {
        return Ok(());
    };
    // A partial wipe of log_templates needs the new_template_watermark_ns key deleted too.
    let new_watermark = stored_watermark
        .map(|w| w.min(now))
        .unwrap_or_else(|| initial_watermark(&detect_cfg, data_clock(data_now, now), now));
    let Some(stored_version) = retry_until(
        "load masking version",
        || store.state_get(KEY_MASKING_VERSION),
        &mut stop_rx,
    )
    .await
    else {
        return Ok(());
    };
    let Some(stored_epoch) = retry_until(
        "load masking epoch",
        || store.state_get(KEY_EPOCH_START),
        &mut stop_rx,
    )
    .await
    else {
        return Ok(());
    };
    let current_version = masking_version(cfg.keep_http_status);
    let (epoch_start, must_store) = startup_epoch(
        stored_version,
        stored_epoch,
        current_version,
        now,
        restored > 0,
    );
    if must_store {
        // The epoch start goes first: a crash between the two writes re-detects the change.
        if retry_until(
            "store masking epoch",
            || store.state_put(KEY_EPOCH_START, epoch_start),
            &mut stop_rx,
        )
        .await
        .is_none()
            || retry_until(
                "store masking version",
                || store.state_put(KEY_MASKING_VERSION, i64::from(current_version)),
                &mut stop_rx,
            )
            .await
            .is_none()
        {
            return Ok(());
        }
    }

    let consumer = Arc::new(tayga_kafka::consumer(&settings.kafka, GROUP)?);
    consumer.subscribe(&[&settings.kafka.topic])?;

    let metrics_addr = cfg.metrics_addr;
    let metrics_stop = stop_rx.clone();
    tokio::spawn(async move {
        if let Err(e) =
            tayga_common::metrics::serve(metrics_addr, Arc::new(registry), metrics_stop).await
        {
            tracing::warn!(error = %e, "metrics server stopped");
        }
    });
    let mut clock = NewTemplateClock {
        watermark: new_watermark,
        epoch_start,
        partitions: None,
    };
    let mut seen = PartitionClocks::default();
    tracing::info!(
        topic = %settings.kafka.topic,
        alerts = %cfg.alerts_topic,
        restored,
        active_spikes,
        new_watermark,
        epoch_start,
        masking_version = current_version,
        "tayga-logminer consuming"
    );

    let flush_age = Duration::from_millis(cfg.flush_ms);
    let mut detect_tick = tokio::time::interval(Duration::from_secs(cfg.detect_secs));
    detect_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    detect_tick.tick().await; // the first tick fires immediately

    let ctx = Ctx {
        store: &store,
        consumer: &consumer,
        topic: &settings.kafka.topic,
        metrics: &metrics,
    };
    let mut pending = Pending::default();
    // Set when shutdown interrupted a flush: nothing was committed, records are re-read on restart.
    let mut interrupted = false;
    let mut main_stop = stop_rx.clone();
    loop {
        let mut detect_due = false;
        tokio::select! {
            _ = main_stop.wait_for(|stop| *stop) => break,
            _ = detect_tick.tick() => detect_due = true,
            next = tokio::time::timeout(Duration::from_millis(200), consumer.recv()) => match next {
                Ok(Ok(msg)) => on_message(&msg, &mut miner, &mut pending, &metrics),
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
        // Detection flushes first so it counts every hit mined so far.
        let flush_now = pending.should_flush(Instant::now(), cfg.max_batch, flush_age)
            || (detect_due && !pending.is_empty());
        if flush_now {
            let batch = std::mem::take(&mut pending);
            if !flush(&ctx, &mut miner, batch, &mut seen, Some(&mut stop_rx)).await? {
                interrupted = true;
                break;
            }
        }
        if detect_due {
            let mut detect_stop = stop_rx.clone();
            tokio::select! {
                snapshot = partition_snapshot(&consumer, &settings.kafka.topic, &mut seen, now_ns(), clock.watermark) => {
                    clock.partitions = snapshot;
                }
                _ = detect_stop.wait_for(|stop| *stop) => break,
            }
            let mut detect_stop = stop_rx.clone();
            tokio::select! {
                _ = detect(&store, &producer, cfg, &miner, &mut tracker, &mut clock, &metrics) => {}
                _ = detect_stop.wait_for(|stop| *stop) => break,
            }
        }
    }
    if !interrupted && !pending.is_empty() {
        // Single attempt: on failure exit without committing.
        flush(&ctx, &mut miner, pending, &mut seen, None).await?;
    }
    tracing::info!("tayga-logminer stopped");
    Ok(())
}

/// Mines the logs of one record. Every record's offset is recorded, including skipped ones.
fn on_message(
    msg: &BorrowedMessage<'_>,
    miner: &mut Miner,
    pending: &mut Pending,
    metrics: &LogminerMetrics,
) {
    // Without a broker timestamp the record counts as fresh (consumed now).
    let record_ts = msg
        .timestamp()
        .to_millis()
        .map_or_else(now_ns, |ms| ms.saturating_mul(1_000_000));
    pending.record(msg.partition(), msg.offset(), record_ts, Instant::now());
    if !is_logs_record(msg) {
        return;
    }
    let env = match msg.payload().map(Envelope::decode) {
        Some(Ok(env)) => env,
        Some(Err(e)) => {
            tracing::warn!(partition = msg.partition(), offset = msg.offset(), error = %e, "skipping undecodable envelope");
            return;
        }
        None => return,
    };
    let (_, logs) = rows_from_envelope(&env);
    for log in &logs {
        let (hit, a) = miner.mine(log);
        pending.record_ts(msg.partition(), hit.ts);
        pending.hits.push(hit);
        metrics.logs_mined.inc();
        if a.created {
            metrics.templates_created.inc();
        }
        if a.overflow {
            metrics.cluster_cap_hits.inc();
        }
    }
}

/// Records with a `tayga-kind` header other than `logs` are skipped undecoded. A record without
/// the header is decoded; `rows_from_envelope` yields no logs for a traces envelope.
fn is_logs_record(msg: &BorrowedMessage<'_>) -> bool {
    let Some(headers) = msg.headers() else {
        return true;
    };
    match headers.iter().find(|h| h.key == HEADER_KIND) {
        Some(h) => h.value == Some(Kind::Logs.as_str().as_bytes()),
        None => true,
    }
}

struct Ctx<'a> {
    store: &'a Store,
    consumer: &'a StreamConsumer,
    topic: &'a str,
    metrics: &'a LogminerMetrics,
}

/// Inserts hits, then the templates changed since the last flush, then commits offsets.
/// Offsets are never committed for records whose hits and templates are not both stored.
/// With `shutdown` each insert retries until it succeeds or shutdown fires; without it, one
/// attempt each. Returns `Ok(true)` if the batch was stored and committed.
async fn flush(
    ctx: &Ctx<'_>,
    miner: &mut Miner,
    batch: Pending,
    seen: &mut PartitionClocks,
    mut shutdown: Option<&mut watch::Receiver<bool>>,
) -> anyhow::Result<bool> {
    let templates = miner.dirty_templates(now_ns());
    let insert_hits = || async {
        ctx.store
            .insert_log_hits(&batch.hits)
            .await
            .inspect_err(|_| {
                ctx.metrics.write_failures.inc();
            })
    };
    let upsert_templates = || async {
        ctx.store
            .upsert_templates(&templates)
            .await
            .inspect_err(|_| {
                ctx.metrics.write_failures.inc();
            })
    };
    let stored = attempt("insert log hits", insert_hits, shutdown.as_deref_mut()).await
        && attempt("upsert templates", upsert_templates, shutdown).await;
    if !stored {
        tracing::warn!(
            hits = batch.hits.len(),
            templates = templates.len(),
            "records remain uncommitted and will be re-read on restart"
        );
        return Ok(false);
    }
    seen.record_flush(&batch);
    let mut tpl = TopicPartitionList::new();
    for (partition, offset) in batch.commit_offsets() {
        tpl.add_partition_offset(ctx.topic, partition, Offset::Offset(offset))?;
    }
    if let Err(e) = ctx.consumer.commit(&tpl, CommitMode::Sync) {
        // Typically a revoked partition after rebalance; its records are re-read and the hits
        // deduplicated by `log_id`.
        tracing::warn!(error = %e, "offset commit failed");
    }
    ctx.metrics.templates.set(miner.len() as i64);
    tracing::debug!(
        hits = batch.hits.len(),
        templates = templates.len(),
        "flushed"
    );
    Ok(true)
}

/// Retries `op` until shutdown when given a shutdown receiver, otherwise runs it once.
async fn attempt<E, F, Fut>(
    what: &str,
    mut op: F,
    shutdown: Option<&mut watch::Receiver<bool>>,
) -> bool
where
    E: std::fmt::Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    match shutdown {
        Some(rx) => retry_until(what, op, rx).await.is_some(),
        None => match op().await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, what, "final attempt failed");
                false
            }
        },
    }
}

/// What new-template detection needs between passes: the data time checked so far (advanced
/// after each pass) and the start of the current masking epoch.
struct NewTemplateClock {
    watermark: i64,
    epoch_start: i64,
    /// Per assigned partition: newest flushed hit `ts` and whether the partition is caught up.
    /// Refreshed before each detection pass. `None` when the consumer has no usable assignment
    /// (empty during a rebalance or rejoin, or the lookup failed): the pass then holds at the
    /// watermark.
    partitions: Option<BTreeMap<i32, (i64, bool)>>,
}

/// A partition whose latest consumed record is older than this is replaying a backlog.
const STALE_RECORD_NS: i64 = 60_000_000_000;

/// What the logminer has flushed per partition, for records of any kind.
#[derive(Default)]
struct PartitionClocks {
    by_partition: BTreeMap<i32, Flushed>,
}

#[derive(Clone, Copy, Default)]
struct Flushed {
    /// Newest flushed log hit `ts` in ns; 0 when the partition has not yielded a hit.
    max_ts: i64,
    /// Kafka timestamp (ns) of the newest consumed record of any kind.
    last_record_ts: i64,
    /// Next offset to consume (last flushed offset + 1).
    next_offset: i64,
}

impl PartitionClocks {
    /// Called once a batch's hits and templates are stored.
    fn record_flush(&mut self, batch: &Pending) {
        for (partition, next) in batch.commit_offsets() {
            let f = self.by_partition.entry(partition).or_default();
            f.next_offset = f.next_offset.max(next);
        }
        for (&partition, &ts) in &batch.record_ts {
            let f = self.by_partition.entry(partition).or_default();
            f.last_record_ts = f.last_record_ts.max(ts);
        }
        for (&partition, &ts) in &batch.max_ts {
            let f = self.by_partition.entry(partition).or_default();
            f.max_ts = f.max_ts.max(ts);
        }
    }

    /// Drops partitions that are no longer assigned (the consumer has no rebalance callbacks).
    fn retain_assigned(&mut self, assigned: &HashSet<i32>) {
        self.by_partition.retain(|p, _| assigned.contains(p));
    }
}

/// One assigned partition's contribution to the detection clock, as `(ts, caught_up)`, or
/// `None` when it contributes nothing.
///
/// A partition is *behind* when records remain (`next_offset < high`) AND its newest consumed
/// record is older than `STALE_RECORD_NS`: a busy partition always has records ahead of the
/// consumer but its newest record is fresh, so it is not behind. A behind partition contributes
/// `min(newest log hit ts if any, newest record ts)`, holding the clock back. Others contribute
/// their newest log hit ts (if any) to the maximum.
/// - Never consumed since start (`flushed` is `None`): behind when `position` (the consumer's
///   position, else the low watermark) is below `high`, or either is unknown; it then holds the
///   clock at `hold_ns` (the current watermark: hold, do not advance).
/// - Unknown `high` (watermark fetch failed): behind only when the newest record is stale.
fn partition_state(
    flushed: Option<Flushed>,
    position: Option<i64>,
    high: Option<i64>,
    now_ns: i64,
    hold_ns: i64,
) -> Option<(i64, bool)> {
    let Some(f) = flushed else {
        let behind = match (position, high) {
            (Some(n), Some(h)) => n < h,
            _ => true,
        };
        return behind.then_some((hold_ns, false));
    };
    let stale = now_ns.saturating_sub(f.last_record_ts) > STALE_RECORD_NS;
    let behind = stale && high.is_none_or(|h| f.next_offset < h);
    if behind {
        let ts = [f.max_ts, f.last_record_ts]
            .into_iter()
            .filter(|&t| t > 0)
            .min()
            .unwrap_or(hold_ns);
        Some((ts, false))
    } else {
        (f.max_ts > 0).then_some((f.max_ts, true))
    }
}

/// Offsets of `topic` partitions in `tpl` that hold a concrete offset (not invalid/beginning).
fn offsets_of(
    tpl: rdkafka::error::KafkaResult<TopicPartitionList>,
    topic: &str,
) -> HashMap<i32, i64> {
    tpl.map(|tpl| {
        tpl.elements()
            .iter()
            .filter(|e| e.topic() == topic)
            .filter_map(|e| match e.offset() {
                Offset::Offset(o) => Some((e.partition(), o)),
                _ => None,
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Where an unconsumed partition will read next: the consumer position, else the committed
/// offset, else (last resort) the low watermark.
fn resolve_position(
    position: Option<i64>,
    committed: Option<i64>,
    low: Option<i64>,
) -> Option<i64> {
    position.or(committed).or(low)
}

/// The assigned partitions of `topic`, pruning `seen` to them; `None` (and `seen` untouched) when
/// the assignment is empty or could not be read. An empty assignment is a rebalance or a rejoin
/// (e.g. after `max.poll.interval.ms` during a long ClickHouse outage), not "nothing to wait
/// for": pruning there would forget every partition's progress.
fn assigned_partitions(
    assignment: rdkafka::error::KafkaResult<TopicPartitionList>,
    topic: &str,
    seen: &mut PartitionClocks,
) -> Option<Vec<i32>> {
    let assigned: Vec<i32> = match assignment {
        Ok(tpl) => tpl
            .elements()
            .iter()
            .filter(|e| e.topic() == topic)
            .map(|e| e.partition())
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "reading the consumer assignment failed");
            return None;
        }
    };
    if assigned.is_empty() {
        return None;
    }
    seen.retain_assigned(&assigned.iter().copied().collect());
    Some(assigned)
}

/// Builds the per-partition view for one detection pass (`None`: no usable assignment, see
/// [`assigned_partitions`]): prunes revoked partitions using the consumer's current assignment,
/// then fetches the watermarks of every assigned partition
/// (blocking librdkafka calls, run on the blocking pool, `WATERMARK_TIMEOUT` each; a failed
/// partition is skipped and the others still asked) and classifies it with [`partition_state`].
async fn partition_snapshot(
    consumer: &Arc<StreamConsumer>,
    topic: &str,
    seen: &mut PartitionClocks,
    now_ns: i64,
    hold_ns: i64,
) -> Option<BTreeMap<i32, (i64, bool)>> {
    let assigned = assigned_partitions(consumer.assignment(), topic, seen)?;
    let positions = offsets_of(consumer.position(), topic);
    let c = Arc::clone(consumer);
    let t = topic.to_string();
    let ids = assigned.clone();
    let (marks, committed) = tokio::task::spawn_blocking(move || {
        let mut marks = HashMap::new();
        for p in ids {
            match c.fetch_watermarks(&t, p, WATERMARK_TIMEOUT) {
                Ok(lh) => {
                    marks.insert(p, lh);
                }
                Err(e) => tracing::warn!(partition = p, error = %e, "fetching watermarks failed"),
            }
        }
        // `position()` is invalid until a message is delivered after a restart; the committed
        // offset is where consumption resumes.
        let committed = offsets_of(c.committed(WATERMARK_TIMEOUT), &t);
        (marks, committed)
    })
    .await
    .unwrap_or_default();
    assigned
        .into_iter()
        .filter_map(|p| {
            let (low, high) = marks
                .get(&p)
                .map_or((None, None), |&(l, h)| (Some(l), Some(h)));
            let position =
                resolve_position(positions.get(&p).copied(), committed.get(&p).copied(), low);
            partition_state(
                seen.by_partition.get(&p).copied(),
                position,
                high,
                now_ns,
                hold_ns,
            )
            .map(|st| (p, st))
        })
        .collect::<BTreeMap<_, _>>()
        .into()
}

/// Data clock for new-template detection (spec 7a §5.1): the minimum newest-hit `ts` over
/// partitions that are not caught up, so a lagging partition holds the clock back. When every
/// partition is caught up (an idle partition must not hold it back), the maximum over all.
/// With no partition data (assigned, but every partition caught up with no hit, or restart with
/// nothing consumed yet), `fallback_ns`.
fn detection_clock(per_partition: &BTreeMap<i32, (i64, bool)>, fallback_ns: i64) -> i64 {
    per_partition
        .values()
        .filter(|(_, caught_up)| !caught_up)
        .map(|(ts, _)| *ts)
        .min()
        .or_else(|| per_partition.values().map(|(ts, _)| *ts).max())
        .unwrap_or(fallback_ns)
}

/// The clock of one pass: with no usable assignment, hold at `hold_ns` (the watermark), since
/// nothing says the backlog was read; otherwise [`detection_clock`] with `fallback_ns`.
fn pass_clock(
    partitions: Option<&BTreeMap<i32, (i64, bool)>>,
    fallback_ns: i64,
    hold_ns: i64,
) -> i64 {
    partitions.map_or(hold_ns, |m| detection_clock(m, fallback_ns))
}

/// One detection pass (spec §6). Failures are logged; the loop continues. The new-template
/// watermark advances to this pass's data clock only when the pass found and stored its alerts,
/// so a failed pass is retried over the same range.
async fn detect(
    store: &Store,
    producer: &FutureProducer,
    cfg: &LogminerSettings,
    miner: &Miner,
    tracker: &mut Trackers,
    clock: &mut NewTemplateClock,
    metrics: &LogminerMetrics,
) {
    let started = Instant::now();
    let now = now_ns();
    let detect_cfg = &cfg.detect();
    match find_alerts(store, detect_cfg, miner, tracker, clock, now, metrics).await {
        Ok((alerts, data_now)) => {
            if publish_alerts(store, producer, &cfg.alerts_topic, &alerts, now, metrics).await {
                clock.watermark = clock.watermark.max(data_now);
                if let Err(e) = store.state_put(KEY_WATERMARK, clock.watermark).await {
                    metrics.state_save_failures.inc();
                    tracing::warn!(error = %e, "saving the new-template watermark failed");
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "detection failed"),
    }
    if let Err(e) = store.state_put(KEY_HEARTBEAT, now).await {
        metrics.state_save_failures.inc();
        tracing::warn!(error = %e, "saving the heartbeat failed");
    }
    metrics
        .detect_seconds
        .observe(started.elapsed().as_secs_f64());
    tracker.spikes.expire(detect_cfg, now);
}

/// Masking epoch at startup: `(epoch start, whether to store the version and start)`.
/// No stored version but existing templates means an upgrade from a pre-7a install, whose
/// templates were mined with masking version 1. A truly fresh install has no epoch (start 0, so
/// no extra warmup); a changed masking version starts a new epoch now; an unchanged one keeps
/// the stored start (and stores the version if it was inferred).
fn startup_epoch(
    stored_version: Option<i64>,
    stored_start: Option<i64>,
    current_version: u32,
    now_ns: i64,
    has_templates: bool,
) -> (i64, bool) {
    match stored_version.or(has_templates.then_some(1)) {
        None => (0, true),
        Some(v) if v == i64::from(current_version) => {
            (stored_start.unwrap_or(0), stored_version.is_none())
        }
        Some(_) => (now_ns, true),
    }
}

/// The data clock never runs ahead of the wall clock: a log stamped slightly in the future
/// (within the query's 1-minute allowance) must not push the watermark past real time.
fn data_clock(raw_ns: i64, now_ns: i64) -> i64 {
    raw_ns.min(now_ns)
}

/// Seconds from the newest mined log to the wall clock; `None` before any log was mined.
fn data_lag_secs(data_now_ns: i64, now_ns: i64) -> Option<f64> {
    (data_now_ns > 0).then(|| (now_ns - data_now_ns) as f64 / 1e9)
}

/// Minutes back from `now_ns` that cover `first_seen_ns`, at least `floor_min`: example traces
/// of a template found after a lag are older than the usual window.
fn minutes_covering(first_seen_ns: i64, now_ns: i64, floor_min: u32) -> u32 {
    let age_min = now_ns.saturating_sub(first_seen_ns).max(0) / MIN_NS + 1;
    u32::try_from(age_min).unwrap_or(u32::MAX).max(floor_min)
}

/// Alert state kept between detection passes.
#[derive(Debug, Default)]
struct Trackers {
    spikes: SpikeTracker,
    /// Ids of the silence alerts raised so far, to tell a new silence period (counted) from a
    /// refresh. Pruned to the currently silent templates each pass. Silence alerts need no restore:
    /// their id and `started_at` are recomputed from `t_last`, so a restart continues the same
    /// alert; the only effect is that a silence alert still running is counted once more.
    silent: HashSet<String>,
}

/// Silence alerts of the silent templates among `inputs` (log time, spec 7b §2.2), `settings`
/// being `(template_id, minutes)` of the enabled templates. The template text comes from the
/// miner; a template it does not know is left out. `baseline_per_window` is 0.0: the spec marks
/// it informational, and no query for the hour before `t_last` exists.
fn silent_alerts(
    settings: &[(u64, u32)],
    inputs: &[SilenceInput],
    miner: &Miner,
    now: i64,
) -> Vec<Alert> {
    inputs
        .iter()
        .filter_map(|i| {
            let &(_, minutes) = settings.iter().find(|(id, _)| *id == i.template_id)?;
            is_silent(minutes, i.first_seen_ns, i.t_last_ns, i.s_last_ns).then_some(())?;
            let template = miner.template(i.template_id)?;
            Some(silence_alert(i, &template, minutes, 0.0, now))
        })
        .collect()
}

/// Alerts to write, each with whether it was created (as opposed to an active spike updated),
/// and the data clock the new-template check ran against.
async fn find_alerts(
    store: &Store,
    cfg: &DetectConfig,
    miner: &Miner,
    tracker: &mut Trackers,
    clock: &NewTemplateClock,
    now: i64,
    metrics: &LogminerMetrics,
) -> anyhow::Result<(Vec<(Alert, bool)>, i64)> {
    let stored_now = store.data_now_ns().await?;
    if let Some(lag) = data_lag_secs(data_clock(stored_now, now), now) {
        metrics.data_lag_seconds.set(lag);
        if lag > f64::from(cfg.new_template_recent_min) * 60.0 {
            tracing::warn!(
                lag_secs = lag,
                "logminer is behind the logs: new templates are still found, spikes in the lag are not"
            );
        }
    }
    // Never ahead of the wall clock; the store's latest hit stands in when every assigned
    // partition is caught up with no hit; with no usable assignment the watermark holds.
    let data_now = data_clock(
        pass_clock(clock.partitions.as_ref(), stored_now, clock.watermark),
        now,
    );
    let mut out = Vec::new();
    // Once per pass; each template's coverage is derived from these buckets.
    let buckets = store
        .covered_minute_buckets(cfg.spike_window_min, cfg.baseline_window_min)
        .await?;
    let windows = store
        .template_windows(
            cfg.spike_window_min,
            cfg.baseline_window_min,
            cfg.spike_min_count,
        )
        .await?;
    let mut spiking = Vec::new();
    for r in windows {
        let w = TemplateWindow {
            template_id: r.template_id,
            service: r.service,
            template: r.template,
            first_seen_ns: r.first_seen_ns,
            current: r.current,
            baseline_total: r.baseline_total,
        };
        let cov = template_coverage(cfg, &buckets, w.first_seen_ns, now);
        match spike_baseline(cfg, &w, cov, now) {
            Ok(Some(b)) => spiking.push((w, b)),
            Ok(None) => {}
            Err(SpikeSkip::Coverage) => {
                metrics
                    .spike_skipped
                    .get_or_create(&ReasonLabel::new("coverage"))
                    .inc();
            }
        }
    }
    // Seasonal comparators are only looked up for templates the flat rule already flags.
    let seasonal = if cfg.baseline_mode == BaselineMode::Seasonal && !spiking.is_empty() {
        let ids: Vec<u64> = spiking.iter().map(|(w, _)| w.template_id).collect();
        seasonal_or_flat(
            store
                .seasonal_counts(&ids, cfg.spike_window_min, &SEASONAL_SHIFTS_SECS)
                .await,
            metrics,
        )
    } else {
        None
    };
    for (w, baseline) in spiking {
        let (day, week) = match &seasonal {
            Some(windows) => {
                let pick = |i: usize| {
                    windows[i].covered.then(|| {
                        windows[i]
                            .counts
                            .iter()
                            .find(|(id, _)| *id == w.template_id)
                            .map_or(0, |&(_, n)| n)
                    })
                };
                (pick(0), pick(1))
            }
            None => (None, None),
        };
        if !seasonal_decision(cfg, w.current, baseline, day, week) {
            continue;
        }
        let examples = examples(store, w.template_id, cfg.spike_window_min).await;
        let comparators = (day.map(|n| n as f64), week.map(|n| n as f64));
        out.push(
            tracker
                .spikes
                .observe(cfg, &w, baseline, comparators, examples, now),
        );
    }
    out.extend(silence_pass(store, miner, tracker, now, metrics).await);
    let since = new_template_since(clock.watermark);
    let candidates = store.new_template_candidates(since).await?;
    for r in candidates {
        let c = NewCandidate {
            template_id: r.template_id,
            service: r.service,
            template: r.template,
            first_seen_ns: r.first_seen_ns,
            service_oldest_ns: r.service_oldest_ns,
        };
        if !is_new(cfg, &c, since, clock.epoch_start)
            || suppressed_pre_epoch(miner, &c, clock.epoch_start, metrics)
        {
            continue;
        }
        let window = minutes_covering(c.first_seen_ns, now, cfg.new_template_recent_min);
        let examples = examples(store, c.template_id, window).await;
        out.push((new_alert(&c, examples, now), true));
    }
    Ok((out, data_now))
}

/// Silence alerts of this pass. A failed lookup is logged and skips silence only, so it cannot
/// drop the pass's spike and new-template alerts; the gauge then keeps its last value.
async fn silence_pass(
    store: &Store,
    miner: &Miner,
    tracker: &mut Trackers,
    now: i64,
    metrics: &LogminerMetrics,
) -> Vec<(Alert, bool)> {
    let lookup = async {
        let settings = store.silence_enabled().await?;
        let ids: Vec<u64> = settings.iter().map(|&(id, _)| id).collect();
        let inputs = store.silence_inputs(&ids).await?;
        anyhow::Ok((settings, inputs))
    };
    let (settings, inputs) = match lookup.await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "silence lookup failed, silence alerts skipped this pass");
            return Vec::new();
        }
    };
    let alerts = silent_alerts(&settings, &inputs, miner, now);
    metrics.silence_alerts.set(alerts.len() as i64);
    mark_created(&mut tracker.silent, alerts)
}

/// Pairs each alert with whether its id is new, and keeps only the current ids.
fn mark_created(seen: &mut HashSet<String>, alerts: Vec<Alert>) -> Vec<(Alert, bool)> {
    let out: Vec<_> = alerts
        .into_iter()
        .map(|a| {
            let created = !seen.contains(&a.alert_id);
            (a, created)
        })
        .collect();
    *seen = out.iter().map(|(a, _)| a.alert_id.clone()).collect();
    out
}

/// A `new` candidate whose kept status code merely split it out of a template that existed before
/// the masking epoch is not new behaviour: no alert, counted as `pre_epoch_match` (final review
/// I1). Only templates with a protected token can match; others are never suppressed.
fn suppressed_pre_epoch(
    miner: &Miner,
    c: &NewCandidate,
    epoch_start: i64,
    metrics: &LogminerMetrics,
) -> bool {
    let hit = miner.would_have_matched_pre_epoch(&c.service, &c.template, epoch_start);
    if hit {
        metrics
            .new_suppressed
            .get_or_create(&ReasonLabel::new(PRE_EPOCH_MATCH))
            .inc();
        tracing::info!(
            template_id = c.template_id,
            service = %c.service,
            template = %c.template,
            "new template matches a pre-epoch template under the old masking: no alert"
        );
    }
    hit
}

/// A failed comparator lookup must not drop the tick's alerts: log it, count it and judge by the
/// flat rule alone (spec 7a §2.3 fallback).
fn seasonal_or_flat<E: std::fmt::Display>(
    res: Result<Vec<SeasonalWindow>, E>,
    metrics: &LogminerMetrics,
) -> Option<Vec<SeasonalWindow>> {
    res.inspect_err(|e| {
        metrics.seasonal_failures.inc();
        tracing::warn!(error = %e, "seasonal comparator lookup failed, using the flat rule");
    })
    .ok()
}

/// Example traces are best effort: a failed lookup yields none rather than dropping the alert.
async fn examples(store: &Store, template_id: u64, since_min: u32) -> Vec<String> {
    store
        .example_traces(template_id, since_min, EXAMPLES)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, template_id, "example trace lookup failed");
            Vec::new()
        })
}

/// One attempt each: insert all alerts, then publish each one. Alerts are published only once
/// stored; a new-template alert that failed to store is found again on the next pass, and an
/// active spike is rewritten on its next update. Returns whether the alerts were stored.
async fn publish_alerts(
    store: &Store,
    producer: &FutureProducer,
    topic: &str,
    alerts: &[(Alert, bool)],
    now: i64,
    metrics: &LogminerMetrics,
) -> bool {
    if alerts.is_empty() {
        return true;
    }
    let version = u64::try_from(now).unwrap_or(0);
    let rows: Vec<_> = alerts.iter().map(|(a, _)| alert_row(a, version)).collect();
    if let Err(e) = store.insert_alerts(&rows).await {
        tracing::warn!(error = %e, alerts = rows.len(), "alert insert failed");
        return false;
    }
    for (alert, created) in alerts {
        if *created {
            metrics
                .alerts
                .get_or_create(&KindLabel::new(alert.kind.as_str()))
                .inc();
        }
        let key = alert.template_id.to_string();
        let payload = alert_json(alert).to_string();
        if let Err((e, _)) = producer
            .send(
                FutureRecord::to(topic).key(&key).payload(&payload),
                Duration::from_secs(5),
            )
            .await
        {
            tracing::warn!(error = %e, alert_id = %alert.alert_id, "alert publish failed");
        }
        tracing::info!(
            alert_id = %alert.alert_id,
            kind = alert.kind.as_str(),
            created,
            service = %alert.service,
            template = %alert.template,
            "log alert"
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tayga_store::logs::LogTemplateRow;

    #[test]
    fn settings_default_when_section_missing() {
        let s = LogminerSettings::default();
        assert_eq!(s.max_batch, 5_000);
        assert_eq!(s.flush_ms, 1_000);
        assert_eq!(s.detect_secs, 60);
        assert_eq!(s.alerts_topic, "tayga.alerts");
        assert_eq!(s.metrics_addr, SocketAddr::from(([0, 0, 0, 0], 9100)));
        assert_eq!(s.detect(), DetectConfig::default());
        assert_eq!(s.detect().baseline_mode, BaselineMode::Flat);
        assert_eq!(s.drain(), DrainConfig::default());
        s.validate().unwrap();
    }

    #[test]
    fn a_failed_seasonal_lookup_falls_back_to_flat_and_is_counted() {
        let m = LogminerMetrics::default();
        let ok = vec![SeasonalWindow {
            shift_secs: 86_400,
            covered: true,
            counts: vec![(1, 3)],
        }];
        assert_eq!(seasonal_or_flat(Ok::<_, String>(ok.clone()), &m), Some(ok));
        assert_eq!(m.seasonal_failures.get(), 0);
        assert_eq!(seasonal_or_flat(Err("boom".to_string()), &m), None);
        assert_eq!(m.seasonal_failures.get(), 1);
    }

    #[test]
    fn baseline_mode_setting_is_validated() {
        let s: LogminerSettings = serde_json::from_str(r#"{"baseline_mode": "seasonal"}"#).unwrap();
        s.validate().unwrap();
        assert_eq!(s.detect().baseline_mode, BaselineMode::Seasonal);
        let bad: LogminerSettings = serde_json::from_str(r#"{"baseline_mode": "daily"}"#).unwrap();
        assert!(bad.validate().is_err());
    }

    #[test]
    fn partial_section_keeps_other_defaults() {
        let s: LogminerSettings =
            serde_json::from_str(r#"{"detect_secs": 5, "alerts_topic": "x"}"#).unwrap();
        assert_eq!((s.detect_secs, s.alerts_topic.as_str()), (5, "x"));
        assert_eq!(s.max_batch, 5_000);
    }

    #[test]
    fn startup_epoch_tracks_the_masking_version() {
        let now = 100 * MIN_NS;
        let keep = masking_version(true);
        assert_eq!(keep, 3);
        assert_eq!(startup_epoch(None, None, keep, now, false), (0, true));
        assert_eq!(startup_epoch(None, None, keep, now, true), (now, true));
        assert_eq!(startup_epoch(None, None, 1, now, true), (0, true));
        assert_eq!(startup_epoch(Some(3), Some(7), keep, now, true), (7, false));
        assert_eq!(startup_epoch(Some(3), None, keep, now, true), (0, false));
        assert_eq!(
            startup_epoch(Some(1), Some(7), keep, now, true),
            (now, true)
        );
        // The retired v2 (statuses kept but generalisable) starts a new epoch too.
        assert_eq!(
            startup_epoch(Some(2), Some(7), keep, now, true),
            (now, true)
        );
    }

    fn template_row(id: u64, service: &str, template: &str, first_seen: i64) -> LogTemplateRow {
        LogTemplateRow {
            template_id: id,
            service: service.into(),
            template: template.into(),
            first_seen,
            last_seen: first_seen,
            count: 13,
            max_severity: 9,
            sample: String::new(),
            version: 1,
        }
    }

    fn candidate(id: u64, service: &str, template: &str, first_seen_ns: i64) -> NewCandidate {
        NewCandidate {
            template_id: id,
            service: service.into(),
            template: template.into(),
            first_seen_ns,
            service_oldest_ns: 0,
        }
    }

    fn silence_input(id: u64, t_last: Option<i64>, s_last: Option<i64>) -> SilenceInput {
        SilenceInput {
            template_id: id,
            service: "svc".into(),
            first_seen_ns: MIN_NS,
            t_last_ns: t_last,
            s_last_ns: s_last,
        }
    }

    fn silence_miner(id: u64) -> Miner {
        let mut miner = Miner::new(DrainConfig::default());
        miner.restore(vec![template_row(id, "svc", "db down", MIN_NS)]);
        miner
    }

    #[test]
    fn a_silent_template_keeps_one_alert_id_and_refreshes_last_at() {
        let miner = silence_miner(7);
        let settings = [(7, 10)];
        let t = 100 * MIN_NS;
        let inputs = [silence_input(7, Some(t), Some(t + 12 * MIN_NS))];
        let mut seen = HashSet::new();
        let first = mark_created(&mut seen, silent_alerts(&settings, &inputs, &miner, 5));
        assert_eq!(first.len(), 1);
        assert!(first[0].1, "the first pass creates the alert");
        assert_eq!(first[0].0.kind, tayga_drain::detect::AlertKind::Silence);
        assert_eq!(first[0].0.template, "db down");
        // A later pass, and a restart (fresh `seen`), keep the id and move `last_at`.
        let later = [silence_input(7, Some(t), Some(t + 30 * MIN_NS))];
        let second = mark_created(&mut seen, silent_alerts(&settings, &later, &miner, 9));
        assert!(!second[0].1);
        assert_eq!(second[0].0.alert_id, first[0].0.alert_id);
        assert_eq!(second[0].0.last_at_ns, 9);
        assert_eq!(second[0].0.started_at_ns, first[0].0.started_at_ns);
        let restarted = silent_alerts(&settings, &later, &miner, 11);
        assert_eq!(restarted[0].alert_id, first[0].0.alert_id);
    }

    #[test]
    fn silence_stops_with_a_hit_and_starts_a_new_period_afterwards() {
        let miner = silence_miner(7);
        let settings = [(7, 10)];
        let t = 100 * MIN_NS;
        let mut seen = HashSet::new();
        let a = silent_alerts(
            &settings,
            &[silence_input(7, Some(t), Some(t + 20 * MIN_NS))],
            &miner,
            1,
        );
        mark_created(&mut seen, a.clone());
        // A hit arrives: not silent, nothing is refreshed.
        let t2 = t + 25 * MIN_NS;
        let hit = [silence_input(7, Some(t2), Some(t2 + 2 * MIN_NS))];
        assert!(mark_created(&mut seen, silent_alerts(&settings, &hit, &miner, 2)).is_empty());
        // Silent again: a new id, counted as created.
        let again = [silence_input(7, Some(t2), Some(t2 + 15 * MIN_NS))];
        let b = mark_created(&mut seen, silent_alerts(&settings, &again, &miner, 3));
        assert!(b[0].1);
        assert_ne!(b[0].0.alert_id, a[0].alert_id);
    }

    #[test]
    fn a_service_without_hits_raises_no_silence_alert() {
        let miner = silence_miner(7);
        let inputs = [silence_input(7, None, None)];
        assert!(silent_alerts(&[(7, 1)], &inputs, &miner, 1).is_empty());
    }

    #[test]
    fn new_alerts_for_status_splits_of_pre_epoch_templates_are_suppressed() {
        let epoch = 100 * MIN_NS;
        let cfg = DetectConfig::default();
        let after = epoch + 30 * MIN_NS; // past the epoch warmup
        let since = after - MIN_NS;
        let mut miner = Miner::new(DrainConfig::default());
        miner.restore(vec![
            template_row(
                1,
                "frontend-proxy",
                r#"<*> "GET <*> <*> <*> <*> upstream_reset_before_response_started{connection_termination} <*> <*> <*> <*> <*> <*> <*> <*> <*> <*>"#,
                epoch - 2 * 86_400 * 1_000_000_000,
            ),
            template_row(2, "payment", "Payment failed for order <*>", epoch - MIN_NS),
        ]);
        let m = LogminerMetrics::default();
        let suppressed = |c: &NewCandidate| {
            is_new(&cfg, c, since, epoch) && suppressed_pre_epoch(&miner, c, epoch, &m)
        };
        // The 18:49:39 alert: a `503 UC` access-log shape that existed (under `<*>`) before.
        let split = candidate(
            10,
            "frontend-proxy",
            r#"<*> "GET <*> <*> 503 UC upstream_reset_before_response_started{connection_termination} <*> <*> <*> <*> <*> <*> <*> <*> <*> <*>"#,
            after,
        );
        assert!(suppressed(&split));
        assert_eq!(
            m.new_suppressed
                .get_or_create(&ReasonLabel::new(PRE_EPOCH_MATCH))
                .get(),
            1
        );
        // A genuinely new shape that carries a status code still alerts.
        let fresh = candidate(
            11,
            "frontend-proxy",
            r#"<*> "DELETE <*> <*> 410 - gone"#,
            after,
        );
        assert!(is_new(&cfg, &fresh, since, epoch) && !suppressed(&fresh));
        // A non-HTTP template is judged exactly as before, even if it matches an old template.
        let plain = candidate(12, "payment", "Payment failed for order <*>", after);
        assert!(is_new(&cfg, &plain, since, epoch) && !suppressed(&plain));
        assert_eq!(
            m.new_suppressed
                .get_or_create(&ReasonLabel::new(PRE_EPOCH_MATCH))
                .get(),
            1
        );
    }

    #[test]
    fn data_clock_never_runs_ahead_of_the_wall_clock() {
        assert_eq!(data_clock(5 * MIN_NS, 10 * MIN_NS), 5 * MIN_NS);
        assert_eq!(data_clock(11 * MIN_NS, 10 * MIN_NS), 10 * MIN_NS);
    }

    fn parts(v: &[(i32, i64, bool)]) -> BTreeMap<i32, (i64, bool)> {
        v.iter().map(|&(p, ts, c)| (p, (ts, c))).collect()
    }

    #[test]
    fn a_lagging_partition_holds_the_detection_clock() {
        let m = parts(&[(0, 50, false), (1, 90, true), (2, 70, false)]);
        assert_eq!(detection_clock(&m, 999), 50);
    }

    #[test]
    fn caught_up_partitions_do_not_hold_the_clock() {
        let m = parts(&[(0, 10, true), (1, 90, true), (2, 70, true)]);
        assert_eq!(detection_clock(&m, 999), 90, "idle partition 0 is ignored");
        assert_eq!(
            detection_clock(&parts(&[(0, 10, true), (1, 90, false)]), 999),
            90
        );
    }

    #[test]
    fn no_partition_data_falls_back() {
        assert_eq!(detection_clock(&BTreeMap::new(), 999), 999);
    }

    const NOW_T: i64 = 1_000 * MIN_NS;

    fn fl(max_ts: i64, last_record_ts: i64, next_offset: i64) -> Flushed {
        Flushed {
            max_ts,
            last_record_ts,
            next_offset,
        }
    }

    #[test]
    fn a_spans_only_lagging_partition_holds_the_clock() {
        // No log hit, stale records, records remaining: contributes its record ts.
        let st = partition_state(
            Some(fl(0, NOW_T - 10 * MIN_NS, 5)),
            None,
            Some(50),
            NOW_T,
            7,
        );
        assert_eq!(st, Some((NOW_T - 10 * MIN_NS, false)));
        let m = BTreeMap::from([(0, st.unwrap()), (1, (NOW_T - MIN_NS, true))]);
        assert_eq!(detection_clock(&m, 0), NOW_T - 10 * MIN_NS);
    }

    #[test]
    fn a_behind_partition_contributes_the_older_of_hit_and_record_ts() {
        let st = partition_state(
            Some(fl(NOW_T - 20 * MIN_NS, NOW_T - 10 * MIN_NS, 5)),
            None,
            Some(50),
            NOW_T,
            7,
        );
        assert_eq!(st, Some((NOW_T - 20 * MIN_NS, false)));
    }

    #[test]
    fn a_busy_partition_with_a_stale_log_ts_but_fresh_records_does_not_hold() {
        let st = partition_state(Some(fl(5, NOW_T - 1_000, 5)), None, Some(500), NOW_T, 7);
        assert_eq!(st, Some((5, true)));
    }

    #[test]
    fn a_stale_partition_that_reached_the_high_watermark_is_caught_up() {
        let st = partition_state(
            Some(fl(100, NOW_T - 10 * MIN_NS, 50)),
            None,
            Some(50),
            NOW_T,
            7,
        );
        assert_eq!(st, Some((100, true)));
        let idle = partition_state(
            Some(fl(0, NOW_T - 10 * MIN_NS, 50)),
            None,
            Some(50),
            NOW_T,
            7,
        );
        assert_eq!(idle, None);
    }

    #[test]
    fn an_unpolled_assigned_behind_partition_holds_at_the_watermark() {
        assert_eq!(
            partition_state(None, Some(3), Some(9), NOW_T, 777),
            Some((777, false))
        );
        assert_eq!(partition_state(None, Some(9), Some(9), NOW_T, 777), None);
        assert_eq!(
            partition_state(None, None, None, NOW_T, 777),
            Some((777, false))
        );
    }

    #[test]
    fn an_idle_partition_with_a_committed_offset_at_the_high_watermark_is_caught_up() {
        let pos = resolve_position(None, Some(9), Some(0));
        assert_eq!(pos, Some(9), "committed beats the low watermark");
        assert_eq!(partition_state(None, pos, Some(9), NOW_T, 777), None);
        let behind = resolve_position(None, Some(4), Some(0));
        assert_eq!(
            partition_state(None, behind, Some(9), NOW_T, 777),
            Some((777, false))
        );
        assert_eq!(resolve_position(Some(2), Some(9), Some(0)), Some(2));
        assert_eq!(
            resolve_position(None, None, Some(0)),
            Some(0),
            "last resort"
        );
    }

    #[test]
    fn a_failed_watermark_fetch_only_matters_for_stale_partitions() {
        let fresh = partition_state(Some(fl(100, NOW_T - 1_000, 5)), None, None, NOW_T, 7);
        assert_eq!(fresh, Some((100, true)));
        let stale = partition_state(Some(fl(100, NOW_T - 10 * MIN_NS, 5)), None, None, NOW_T, 7);
        assert_eq!(stale, Some((100.min(NOW_T - 10 * MIN_NS), false)));
        // One partition's failure leaves the others' states unchanged.
        let ok = partition_state(
            Some(fl(100, NOW_T - 10 * MIN_NS, 50)),
            None,
            Some(50),
            NOW_T,
            7,
        );
        assert_eq!(ok, Some((100, true)));
    }

    #[test]
    fn an_empty_or_failed_assignment_holds_at_the_watermark_and_keeps_partition_state() {
        let mut seen = PartitionClocks::default();
        let mut b = Pending::default();
        b.record(0, 4, 10, Instant::now());
        b.record(1, 9, 20, Instant::now());
        seen.record_flush(&b);
        let before: Vec<_> = seen.by_partition.keys().copied().collect();
        assert_eq!(
            assigned_partitions(Ok(TopicPartitionList::new()), "t", &mut seen),
            None
        );
        let err = Err(rdkafka::error::KafkaError::ClientCreation("x".into()));
        assert_eq!(assigned_partitions(err, "t", &mut seen), None);
        let mut other_topic = TopicPartitionList::new();
        other_topic.add_partition("u", 0);
        assert_eq!(assigned_partitions(Ok(other_topic), "t", &mut seen), None);
        let after: Vec<_> = seen.by_partition.keys().copied().collect();
        assert_eq!(after, before, "nothing pruned");
        // No usable assignment: the pass holds at the watermark, not the store's latest hit.
        assert_eq!(pass_clock(None, 999, 40), 40);
        // Assigned, every partition caught up with no hits: the store's latest hit.
        assert_eq!(pass_clock(Some(&BTreeMap::new()), 999, 40), 999);
        assert_eq!(pass_clock(Some(&parts(&[(0, 50, false)])), 999, 40), 50);
        // A real assignment still prunes revoked partitions.
        let mut tpl = TopicPartitionList::new();
        tpl.add_partition("t", 1);
        assert_eq!(assigned_partitions(Ok(tpl), "t", &mut seen), Some(vec![1]));
        assert_eq!(seen.by_partition.keys().copied().collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn revoked_partitions_are_pruned() {
        let mut seen = PartitionClocks::default();
        let mut b = Pending::default();
        b.record(0, 4, 10, Instant::now());
        b.record(1, 9, 20, Instant::now());
        b.record(2, 1, 30, Instant::now());
        b.record_ts(0, 100);
        seen.record_flush(&b);
        assert_eq!(seen.by_partition[&1].next_offset, 10);
        assert_eq!(seen.by_partition[&1].last_record_ts, 20);
        assert_eq!(seen.by_partition[&0].max_ts, 100);
        seen.retain_assigned(&HashSet::from([1, 2]));
        let left: Vec<_> = seen.by_partition.keys().copied().collect();
        assert_eq!(left, vec![1, 2]);
    }

    #[test]
    fn data_lag_is_unknown_before_any_log() {
        assert_eq!(data_lag_secs(0, 5 * MIN_NS), None);
        assert_eq!(data_lag_secs(MIN_NS, 3 * MIN_NS), Some(120.0));
    }

    #[test]
    fn example_window_covers_the_template_age() {
        let now = 1_000 * MIN_NS;
        assert_eq!(minutes_covering(now - MIN_NS, now, 10), 10, "floor");
        assert_eq!(minutes_covering(now - 12 * MIN_NS, now, 10), 13);
        assert_eq!(
            minutes_covering(now + MIN_NS, now, 10),
            10,
            "future first_seen"
        );
    }

    #[test]
    fn pending_tracks_max_offset_and_flush_triggers() {
        let t0 = Instant::now();
        let mut p = Pending::default();
        assert!(p.is_empty());
        assert!(!p.should_flush(t0 + Duration::from_secs(10), 1, Duration::from_secs(1)));
        p.record(0, 7, 0, t0);
        p.record(0, 5, 0, t0);
        p.record(3, 1, 0, t0);
        assert_eq!(p.commit_offsets(), vec![(0, 8), (3, 2)]);
        assert!(!p.should_flush(t0, 10, Duration::from_secs(1)));
        assert!(p.should_flush(t0 + Duration::from_secs(1), 10, Duration::from_secs(1)));
        p.hits.push(LogHitRow {
            log_id: 1,
            template_id: 1,
            service: "s".into(),
            ts: 0,
            severity_number: 0,
            trace_id: String::new(),
            span_id: String::new(),
        });
        assert!(p.should_flush(t0, 1, Duration::from_secs(1)));
    }
}
