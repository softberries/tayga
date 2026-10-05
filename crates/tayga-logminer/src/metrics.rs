use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use std::sync::atomic::AtomicU64;
use tayga_common::metrics::KindLabel;
use tayga_drain::detect::AlertKind;

/// `reason` label on `spike_skipped`: `coverage`.
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

/// `kind` label on `alerts`: `new` | `spike`.
#[derive(Clone)]
pub struct LogminerMetrics {
    pub logs_mined: Counter,
    pub templates_created: Counter,
    pub cluster_cap_hits: Counter,
    pub write_failures: Counter,
    pub templates: Gauge,
    pub alerts: Family<KindLabel, Counter>,
    pub detect_seconds: Histogram,
    /// Wall clock minus the latest mined log's `ts`, set each detection tick.
    pub data_lag_seconds: Gauge<f64, AtomicU64>,
    /// Failed saves of the new-template watermark to `logminer_state`.
    pub state_save_failures: Counter,
    /// Template windows not judged for a spike, by reason.
    pub spike_skipped: Family<ReasonLabel, Counter>,
}

impl Default for LogminerMetrics {
    fn default() -> Self {
        Self {
            logs_mined: Counter::default(),
            templates_created: Counter::default(),
            cluster_cap_hits: Counter::default(),
            write_failures: Counter::default(),
            templates: Gauge::default(),
            alerts: Family::default(),
            // 10 ms .. ~20 s.
            detect_seconds: Histogram::new(exponential_buckets(0.01, 2.0, 12)),
            data_lag_seconds: Gauge::default(),
            state_save_failures: Counter::default(),
            spike_skipped: Family::default(),
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
            "Wall clock minus the newest mined log timestamp, at the last detection pass",
            m.data_lag_seconds.clone(),
        );
        registry.register(
            "tayga_logminer_state_save_failures",
            "Failed saves of the new-template watermark to logminer_state",
            m.state_save_failures.clone(),
        );
        registry.register(
            "tayga_logminer_spike_skipped",
            "Spike candidates not judged, by reason (coverage: under half the baseline minutes had logs)",
            m.spike_skipped.clone(),
        );
        drop(m.spike_skipped.get_or_create(&ReasonLabel::new("coverage")));
        // Export both series at 0 so the family is visible before the first alert.
        for kind in [AlertKind::New, AlertKind::Spike] {
            drop(m.alerts.get_or_create(&KindLabel::new(kind.as_str())));
        }
        m
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
            "tayga_logminer_templates 3",
            "tayga_logminer_alerts_total{kind=\"new\"} 1",
            "tayga_logminer_alerts_total{kind=\"spike\"} 0",
            "# TYPE tayga_logminer_detect_seconds histogram",
            "tayga_logminer_detect_seconds_bucket{le=\"0.02\"} 1",
            "# TYPE tayga_logminer_data_lag_seconds gauge",
            "tayga_logminer_data_lag_seconds 2.5",
            "tayga_logminer_state_save_failures_total 0",
            "tayga_logminer_spike_skipped_total{reason=\"coverage\"} 1",
        ] {
            assert!(out.contains(line), "missing {line:?} in\n{out}");
        }
    }
}
