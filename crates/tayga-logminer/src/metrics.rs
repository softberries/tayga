use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use std::sync::atomic::AtomicU64;
use tayga_common::metrics::KindLabel;
use tayga_drain::detect::AlertKind;
use tayga_drain::drain::{Assignment, CacheReset, Lookup};

/// `reason` label on `spike_skipped` (`coverage`) and `new_suppressed` (`pre_epoch_match`).
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ReasonLabel {
    pub reason: String,
}

impl ReasonLabel {
    pub fn new(reason: &str) -> Self {
        Self {
            reason: reason.to_string(),
        }
    }
}

/// `backend` label on `fingerprinter`: `off`, `scalar`, `parallel` or `gpu`.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct BackendLabel {
    pub backend: String,
}

/// `reason` of a new-template candidate suppressed because a pre-epoch template would have
/// matched it.
pub const PRE_EPOCH_MATCH: &str = "pre_epoch_match";

/// `kind` label on `alerts`: `new` | `spike`.
#[derive(Clone)]
pub struct LogminerMetrics {
    pub logs_mined: Counter,
    pub templates_created: Counter,
    pub cluster_cap_hits: Counter,
    pub write_failures: Counter,
    /// Offset commits Kafka refused; the records are read again, nothing is lost.
    pub commit_failures: Counter,
    /// Stored alerts published again because no publication was recorded.
    pub alerts_republished: Counter,
    pub templates: Gauge,
    pub alerts: Family<KindLabel, Counter>,
    pub detect_seconds: Histogram,
    /// Wall clock minus this replica's data clock (its slowest partition still behind), set each
    /// detection tick.
    pub data_lag_seconds: Gauge<f64, AtomicU64>,
    /// Failed saves of the new-template watermark to `logminer_state`.
    pub state_save_failures: Counter,
    /// Template windows not judged for a spike, by reason.
    /// Templates currently silent (silence alerts being kept), set each pass.
    pub silence_alerts: Gauge,
    pub spike_skipped: Family<ReasonLabel, Counter>,
    /// Failed seasonal comparator lookups (the tick fell back to flat).
    pub seasonal_failures: Counter,
    /// New-template candidates not alerted, by reason.
    pub new_suppressed: Family<ReasonLabel, Counter>,
    /// Lines assigned from the fingerprint cache (sub-project 4 spec §3.7).
    pub fingerprint_cache_hits: Counter,
    /// Lines through the Drain tree: cache misses, collisions and lines without a fingerprint.
    pub fingerprint_cache_misses: Counter,
    pub fingerprint_collisions: Counter,
    /// Service caches emptied, by reason (`generalised`, `full`).
    pub fingerprint_cache_resets: Family<ReasonLabel, Counter>,
    /// Wall time of mining one Kafka record's logs.
    pub mine_batch_seconds: Histogram,
    /// 1 for the backend in use.
    pub fingerprinter: Family<BackendLabel, Gauge>,
}

impl Default for LogminerMetrics {
    fn default() -> Self {
        Self {
            logs_mined: Counter::default(),
            templates_created: Counter::default(),
            cluster_cap_hits: Counter::default(),
            write_failures: Counter::default(),
            commit_failures: Counter::default(),
            alerts_republished: Counter::default(),
            templates: Gauge::default(),
            alerts: Family::default(),
            // 10 ms .. ~20 s.
            detect_seconds: Histogram::new(exponential_buckets(0.01, 2.0, 12)),
            data_lag_seconds: Gauge::default(),
            state_save_failures: Counter::default(),
            silence_alerts: Gauge::default(),
            spike_skipped: Family::default(),
            seasonal_failures: Counter::default(),
            new_suppressed: Family::default(),
            fingerprint_cache_hits: Counter::default(),
            fingerprint_cache_misses: Counter::default(),
            fingerprint_collisions: Counter::default(),
            fingerprint_cache_resets: Family::default(),
            // 1 µs .. ~262 ms.
            mine_batch_seconds: Histogram::new(exponential_buckets(1e-6, 4.0, 10)),
            fingerprinter: Family::default(),
        }
    }
}

impl LogminerMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let m = Self::default();
        registry.register(
            "tayga_logminer_logs_mined",
            "Log records assigned to a template",
            m.logs_mined.clone(),
        );
        registry.register(
            "tayga_logminer_templates_created",
            "Templates created by Drain",
            m.templates_created.clone(),
        );
        registry.register(
            "tayga_logminer_cluster_cap_hits",
            "Logs sent to a service's overflow template because its cluster cap was reached",
            m.cluster_cap_hits.clone(),
        );
        registry.register(
            "tayga_logminer_write_failures",
            "Failed ClickHouse insert attempts for hits and templates",
            m.write_failures.clone(),
        );
        registry.register(
            "tayga_logminer_commit_failures",
            "Offset commits Kafka refused (the records are read again; nothing is lost)",
            m.commit_failures.clone(),
        );
        registry.register(
            "tayga_logminer_alerts_republished",
            "Stored alerts published again because no publication was recorded",
            m.alerts_republished.clone(),
        );
        registry.register(
            "tayga_logminer_templates",
            "Templates held in memory",
            m.templates.clone(),
        );
        registry.register(
            "tayga_logminer_alerts",
            "Alerts created (updates of active spike alerts are not counted)",
            m.alerts.clone(),
        );
        registry.register(
            "tayga_logminer_detect_seconds",
            "Duration of one detection pass",
            m.detect_seconds.clone(),
        );
        registry.register(
            "tayga_logminer_data_lag_seconds",
            "Wall clock minus this replica's data clock (its slowest partition still behind), at the last detection pass",
            m.data_lag_seconds.clone(),
        );
        registry.register(
            "tayga_logminer_state_save_failures",
            "Failed saves of the new-template watermark to logminer_state",
            m.state_save_failures.clone(),
        );
        registry.register(
            "tayga_logminer_silence_alerts",
            "Templates currently silent (log time) with silence alerts enabled",
            m.silence_alerts.clone(),
        );
        registry.register(
            "tayga_logminer_spike_skipped",
            "Spike candidates not judged, by reason (coverage: under half the baseline minutes had logs)",
            m.spike_skipped.clone(),
        );
        registry.register(
            "tayga_logminer_seasonal_failures",
            "Failed seasonal comparator lookups; the pass fell back to the flat rule",
            m.seasonal_failures.clone(),
        );
        registry.register(
            "tayga_logminer_new_suppressed",
            "New-template candidates not alerted, by reason (pre_epoch_match: a kept status code \
             split out of a template that existed before the masking epoch)",
            m.new_suppressed.clone(),
        );
        registry.register(
            "tayga_logminer_fingerprint_cache_hits",
            "Log lines assigned from the fingerprint cache, without the Drain tree",
            m.fingerprint_cache_hits.clone(),
        );
        registry.register(
            "tayga_logminer_fingerprint_cache_misses",
            "Log lines assigned by the Drain tree (cache miss, collision or no fingerprint)",
            m.fingerprint_cache_misses.clone(),
        );
        registry.register(
            "tayga_logminer_fingerprint_collisions",
            "Fingerprint keys found in the cache with a different check hash",
            m.fingerprint_collisions.clone(),
        );
        registry.register(
            "tayga_logminer_fingerprint_cache_resets",
            "Service fingerprint caches emptied, by reason (generalised: a template generalised; \
             full: the cache held its maximum)",
            m.fingerprint_cache_resets.clone(),
        );
        registry.register(
            "tayga_logminer_mine_batch_seconds",
            "Wall time of mining the logs of one Kafka record",
            m.mine_batch_seconds.clone(),
        );
        registry.register(
            "tayga_logminer_fingerprinter",
            "The fingerprint backend in use (1), by backend",
            m.fingerprinter.clone(),
        );
        for r in [CacheReset::Generalised, CacheReset::Full] {
            drop(
                m.fingerprint_cache_resets
                    .get_or_create(&ReasonLabel::new(r.as_str())),
            );
        }
        drop(m.spike_skipped.get_or_create(&ReasonLabel::new("coverage")));
        drop(
            m.new_suppressed
                .get_or_create(&ReasonLabel::new(PRE_EPOCH_MATCH)),
        );
        // Export both series at 0 so the family is visible before the first alert.
        for kind in [AlertKind::New, AlertKind::Spike, AlertKind::Silence] {
            drop(m.alerts.get_or_create(&KindLabel::new(kind.as_str())));
        }
        m
    }
}

impl LogminerMetrics {
    /// Counts how one line found its template.
    pub fn record_lookup(&self, a: &Assignment) {
        match a.lookup {
            Lookup::Hit => {
                self.fingerprint_cache_hits.inc();
            }
            Lookup::Collision => {
                self.fingerprint_collisions.inc();
                self.fingerprint_cache_misses.inc();
            }
            Lookup::Miss | Lookup::Unfingerprinted => {
                self.fingerprint_cache_misses.inc();
            }
        }
        if let Some(r) = a.reset {
            self.fingerprint_cache_resets
                .get_or_create(&ReasonLabel::new(r.as_str()))
                .inc();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_are_exported() {
        let mut registry = Registry::default();
        let m = LogminerMetrics::register(&mut registry);
        m.logs_mined.inc();
        m.templates.set(3);
        m.silence_alerts.set(2);
        m.alerts.get_or_create(&KindLabel::new("new")).inc();
        m.detect_seconds.observe(0.015);
        m.data_lag_seconds.set(2.5);
        m.spike_skipped
            .get_or_create(&ReasonLabel::new("coverage"))
            .inc();
        let out = tayga_common::metrics::render(&registry);
        for line in [
            "tayga_logminer_logs_mined_total 1",
            "tayga_logminer_templates_created_total 0",
            "tayga_logminer_cluster_cap_hits_total 0",
            "tayga_logminer_write_failures_total 0",
            "tayga_logminer_commit_failures_total 0",
            "tayga_logminer_alerts_republished_total 0",
            "tayga_logminer_templates 3",
            "tayga_logminer_silence_alerts 2",
            "tayga_logminer_alerts_total{kind=\"new\"} 1",
            "tayga_logminer_alerts_total{kind=\"spike\"} 0",
            "# TYPE tayga_logminer_detect_seconds histogram",
            "tayga_logminer_detect_seconds_bucket{le=\"0.02\"} 1",
            "# TYPE tayga_logminer_data_lag_seconds gauge",
            "tayga_logminer_data_lag_seconds 2.5",
            "tayga_logminer_state_save_failures_total 0",
            "tayga_logminer_seasonal_failures_total 0",
            "tayga_logminer_spike_skipped_total{reason=\"coverage\"} 1",
            "tayga_logminer_new_suppressed_total{reason=\"pre_epoch_match\"} 0",
            "tayga_logminer_fingerprint_cache_hits_total 0",
            "tayga_logminer_fingerprint_cache_misses_total 0",
            "tayga_logminer_fingerprint_collisions_total 0",
            "tayga_logminer_fingerprint_cache_resets_total{reason=\"generalised\"} 0",
            "tayga_logminer_fingerprint_cache_resets_total{reason=\"full\"} 0",
            "# TYPE tayga_logminer_mine_batch_seconds histogram",
        ] {
            assert!(out.contains(line), "missing {line:?} in\n{out}");
        }
    }

    #[test]
    fn lookups_are_counted_by_outcome() {
        let mut registry = Registry::default();
        let m = LogminerMetrics::register(&mut registry);
        let a = |lookup, reset| Assignment {
            template_id: 1,
            created: false,
            overflow: false,
            lookup,
            reset,
        };
        m.record_lookup(&a(Lookup::Hit, None));
        m.record_lookup(&a(Lookup::Hit, None));
        m.record_lookup(&a(Lookup::Miss, Some(CacheReset::Generalised)));
        m.record_lookup(&a(Lookup::Collision, Some(CacheReset::Full)));
        m.record_lookup(&a(Lookup::Unfingerprinted, None));
        m.fingerprinter
            .get_or_create(&BackendLabel {
                backend: "scalar".into(),
            })
            .set(1);
        m.mine_batch_seconds.observe(0.000_002);
        let out = tayga_common::metrics::render(&registry);
        for line in [
            "tayga_logminer_fingerprint_cache_hits_total 2",
            "tayga_logminer_fingerprint_cache_misses_total 3",
            "tayga_logminer_fingerprint_collisions_total 1",
            "tayga_logminer_fingerprint_cache_resets_total{reason=\"generalised\"} 1",
            "tayga_logminer_fingerprint_cache_resets_total{reason=\"full\"} 1",
            "tayga_logminer_fingerprinter{backend=\"scalar\"} 1",
            "tayga_logminer_mine_batch_seconds_bucket{le=\"0.000004\"} 1",
        ] {
            assert!(out.contains(line), "missing {line:?} in\n{out}");
        }
    }
}
