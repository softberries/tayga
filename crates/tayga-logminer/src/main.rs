use prometheus_client::registry::Registry;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::{BorrowedMessage, Headers};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::{Message, Offset, TopicPartitionList};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tayga_common::metrics::KindLabel;
use tayga_common::retry::retry_until;
use tayga_drain::detect::{
    Alert, DetectConfig, NewCandidate, SpikeTracker, TemplateWindow, is_new, new_alert,
    spike_baseline,
};
use tayga_drain::drain::DrainConfig;
use tayga_kafka::KafkaSettings;
use tayga_logminer::metrics::LogminerMetrics;
use tayga_logminer::miner::{Miner, alert_from_row, alert_json, alert_row};
use tayga_model::envelope::{Envelope, HEADER_KIND, Kind};
use tayga_store::ClickHouseSettings;
use tayga_store::flatten::rows_from_envelope;
use tayga_store::logs::LogHitRow;
use tayga_store::store::Store;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;

const GROUP: &str = "tayga-logminer";
const EXAMPLES: u32 = 5;
const ALERTS_PARTITIONS: i32 = 3;

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
    max_batch: usize,
    flush_ms: u64,
    detect_secs: u64,
    spike_window_min: u32,
    baseline_window_min: u32,
    spike_factor: f64,
    spike_min_count: u64,
    new_template_warmup_min: u32,
    alert_active_min: u32,
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
            max_batch: 5_000,
            flush_ms: 1_000,
            detect_secs: 60,
            spike_window_min: detect.spike_window_min,
            baseline_window_min: detect.baseline_window_min,
            spike_factor: detect.spike_factor,
            spike_min_count: detect.spike_min_count,
            new_template_warmup_min: detect.new_template_warmup_min,
            alert_active_min: detect.alert_active_min,
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
        Ok(())
    }

    fn drain(&self) -> DrainConfig {
        DrainConfig {
            sim_threshold: self.sim_threshold,
            max_clusters_per_service: self.max_clusters_per_service,
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
            ..DetectConfig::default()
        }
    }
}

/// Hits mined since the last flush, and the highest offset seen per partition.
#[derive(Default)]
struct Pending {
    hits: Vec<LogHitRow>,
    offsets: HashMap<i32, i64>,
    since: Option<Instant>,
}

impl Pending {
    fn record(&mut self, partition: i32, offset: i64, now: Instant) {
        let o = self.offsets.entry(partition).or_insert(offset);
        *o = (*o).max(offset);
        self.since.get_or_insert(now);
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
    let mut tracker = SpikeTracker::default();
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
    tracker.restore(active);

    let consumer = tayga_kafka::consumer(&settings.kafka, GROUP)?;
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
    tracing::info!(
        topic = %settings.kafka.topic,
        alerts = %cfg.alerts_topic,
        restored,
        active_spikes,
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
            if !flush(&ctx, &mut miner, batch, Some(&mut stop_rx)).await? {
                interrupted = true;
                break;
            }
        }
        if detect_due {
            let mut detect_stop = stop_rx.clone();
            tokio::select! {
                _ = detect(&store, &producer, cfg, &detect_cfg, &mut tracker, &metrics) => {}
                _ = detect_stop.wait_for(|stop| *stop) => break,
            }
        }
    }
    if !interrupted && !pending.is_empty() {
        // Single attempt: on failure exit without committing.
        flush(&ctx, &mut miner, pending, None).await?;
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
    pending.record(msg.partition(), msg.offset(), Instant::now());
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

/// One detection pass (spec §6). Failures are logged; the loop continues.
async fn detect(
    store: &Store,
    producer: &FutureProducer,
    cfg: &LogminerSettings,
    detect_cfg: &DetectConfig,
    tracker: &mut SpikeTracker,
    metrics: &LogminerMetrics,
) {
    let started = Instant::now();
    let now = now_ns();
    match find_alerts(store, detect_cfg, tracker, now).await {
        Ok(alerts) => {
            publish_alerts(store, producer, &cfg.alerts_topic, &alerts, now, metrics).await
        }
        Err(e) => tracing::warn!(error = %e, "detection failed"),
    }
    metrics
        .detect_seconds
        .observe(started.elapsed().as_secs_f64());
    tracker.expire(detect_cfg, now);
}

/// Alerts to write, each with whether it was created (as opposed to an active spike updated).
async fn find_alerts(
    store: &Store,
    cfg: &DetectConfig,
    tracker: &mut SpikeTracker,
    now: i64,
) -> anyhow::Result<Vec<(Alert, bool)>> {
    let mut out = Vec::new();
    let windows = store
        .template_windows(
            cfg.spike_window_min,
            cfg.baseline_window_min,
            cfg.spike_min_count,
        )
        .await?;
    for r in windows {
        let w = TemplateWindow {
            template_id: r.template_id,
            service: r.service,
            template: r.template,
            first_seen_ns: r.first_seen_ns,
            current: r.current,
            baseline_total: r.baseline_total,
        };
        let Some(baseline) = spike_baseline(cfg, &w, now) else {
            continue;
        };
        let examples = examples(store, w.template_id, cfg.spike_window_min).await;
        out.push(tracker.observe(cfg, &w, baseline, examples, now));
    }
    let candidates = store
        .new_template_candidates(cfg.new_template_recent_min)
        .await?;
    for r in candidates {
        let c = NewCandidate {
            template_id: r.template_id,
            service: r.service,
            template: r.template,
            first_seen_ns: r.first_seen_ns,
            service_oldest_ns: r.service_oldest_ns,
        };
        if !is_new(cfg, &c, now) {
            continue;
        }
        let examples = examples(store, c.template_id, cfg.new_template_recent_min).await;
        out.push((new_alert(&c, examples, now), true));
    }
    Ok(out)
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
/// active spike is rewritten on its next update.
async fn publish_alerts(
    store: &Store,
    producer: &FutureProducer,
    topic: &str,
    alerts: &[(Alert, bool)],
    now: i64,
    metrics: &LogminerMetrics,
) {
    if alerts.is_empty() {
        return;
    }
    let version = u64::try_from(now).unwrap_or(0);
    let rows: Vec<_> = alerts.iter().map(|(a, _)| alert_row(a, version)).collect();
    if let Err(e) = store.insert_alerts(&rows).await {
        tracing::warn!(error = %e, alerts = rows.len(), "alert insert failed");
        return;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_when_section_missing() {
        let s = LogminerSettings::default();
        assert_eq!(s.max_batch, 5_000);
        assert_eq!(s.flush_ms, 1_000);
        assert_eq!(s.detect_secs, 60);
        assert_eq!(s.alerts_topic, "tayga.alerts");
        assert_eq!(s.metrics_addr, SocketAddr::from(([0, 0, 0, 0], 9100)));
        assert_eq!(s.detect(), DetectConfig::default());
        assert_eq!(s.drain(), DrainConfig::default());
        s.validate().unwrap();
    }

    #[test]
    fn partial_section_keeps_other_defaults() {
        let s: LogminerSettings =
            serde_json::from_str(r#"{"detect_secs": 5, "alerts_topic": "x"}"#).unwrap();
        assert_eq!((s.detect_secs, s.alerts_topic.as_str()), (5, "x"));
        assert_eq!(s.max_batch, 5_000);
    }

    #[test]
    fn pending_tracks_max_offset_and_flush_triggers() {
        let t0 = Instant::now();
        let mut p = Pending::default();
        assert!(p.is_empty());
        assert!(!p.should_flush(t0 + Duration::from_secs(10), 1, Duration::from_secs(1)));
        p.record(0, 7, t0);
        p.record(0, 5, t0);
        p.record(3, 1, t0);
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
