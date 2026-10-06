//! Notifier metrics on `:9100` (spec 7b §4). Labels carry the target `name`, never its URL.

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;

/// `result` values of `tayga_notifier_deliveries_total`.
pub const DELIVERED: &str = "delivered";
/// Gave up: a permanent error, or `max_attempts` reached.
pub const FAILED: &str = "failed";
/// One attempt failed with a retryable error; another follows.
pub const RETRY: &str = "retry";
/// Already delivered or given up earlier (a re-published alert, or a re-read record): counts
/// re-publishes, not deliveries.
pub const DUPLICATE: &str = "duplicate";
/// Skipped as older than `max_age_secs` (a retained backlog); nothing was sent.
pub const STALE: &str = "stale";
/// One attempt made while the target's breaker was open (no backoff ladder follows). The
/// attempt's resolution is counted as well, as `delivered` or `failed`.
pub const BREAKER: &str = "breaker";

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct DeliveryLabels {
    pub target: String,
    pub result: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TargetLabel {
    pub target: String,
}

#[derive(Clone)]
pub struct NotifierMetrics {
    pub deliveries: Family<DeliveryLabels, Counter>,
    /// Seconds per HTTP attempt, whatever its result.
    pub delivery_seconds: Histogram,
    /// Deliveries (alert × target) started and not yet resolved.
    pub pending: Gauge,
    /// 1 while the target's circuit breaker is open, else 0.
    pub breaker_open: Family<TargetLabel, Gauge>,
}

impl Default for NotifierMetrics {
    fn default() -> Self {
        Self {
            deliveries: Family::default(),
            // 10 ms .. ~20 s, past the 10 s request timeout.
            delivery_seconds: Histogram::new(exponential_buckets(0.01, 2.0, 12)),
            pending: Gauge::default(),
            breaker_open: Family::default(),
        }
    }
}

impl NotifierMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let m = Self::default();
        registry.register(
            "tayga_notifier_deliveries",
            "Delivery results per target. delivered and failed are deliveries; retry counts failed \
             attempts that are retried; duplicate counts re-publishes of an alert already \
             resolved, not deliveries; stale counts alerts skipped as older than max_age_secs; \
             breaker counts attempts made while the target's breaker was open",
            m.deliveries.clone(),
        );
        registry.register(
            "tayga_notifier_delivery_seconds",
            "Duration of one delivery attempt",
            m.delivery_seconds.clone(),
        );
        registry.register(
            "tayga_notifier_pending",
            "Deliveries started and not yet resolved",
            m.pending.clone(),
        );
        registry.register(
            "tayga_notifier_breaker_open",
            "1 while the target's circuit breaker is open: each new alert gets one attempt to it, \
             without the backoff ladder",
            m.breaker_open.clone(),
        );
        m
    }

    pub fn set_breaker_open(&self, target: &str, open: bool) {
        self.breaker_open
            .get_or_create(&TargetLabel {
                target: target.to_string(),
            })
            .set(i64::from(open));
    }

    pub fn count(&self, target: &str, result: &str) {
        self.deliveries
            .get_or_create(&DeliveryLabels {
                target: target.to_string(),
                result: result.to_string(),
            })
            .inc();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_every_series() {
        let mut registry = Registry::default();
        let m = NotifierMetrics::register(&mut registry);
        m.count("ops-slack", DELIVERED);
        m.delivery_seconds.observe(0.03);
        m.pending.inc();
        m.set_breaker_open("ops-slack", true);
        let text = tayga_common::metrics::render(&registry);
        assert!(
            text.contains(
                "tayga_notifier_deliveries_total{target=\"ops-slack\",result=\"delivered\"} 1"
            ),
            "{text}"
        );
        assert!(text.contains("# TYPE tayga_notifier_delivery_seconds histogram"));
        assert!(text.contains("tayga_notifier_delivery_seconds_count 1"));
        assert!(text.contains("tayga_notifier_pending 1"));
        assert!(
            text.contains("tayga_notifier_breaker_open{target=\"ops-slack\"} 1"),
            "{text}"
        );
        assert!(
            text.contains(
                "duplicate counts re-publishes of an alert already resolved, not deliveries"
            ),
            "{text}"
        );
    }
}
