use crate::records::Converted;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::registry::Registry;
use tayga_common::metrics::KindLabel;

/// `kind`: `traces` | `logs`.
#[derive(Clone, Default)]
pub struct IngestMetrics {
    pub records_published: Family<KindLabel, Counter>,
    pub service_routed_items: Counter,
    pub oversized_dropped: Counter,
    pub publish_failures: Counter,
    pub rejected_requests: Counter,
}

impl IngestMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let m = Self::default();
        registry.register(
            "tayga_ingest_records_published",
            "Kafka records published",
            m.records_published.clone(),
        );
        registry.register(
            "tayga_ingest_service_routed_items",
            "Spans/logs without a valid trace id, routed by service",
            m.service_routed_items.clone(),
        );
        registry.register(
            "tayga_ingest_oversized_dropped",
            "Single spans/logs dropped for exceeding the record budget",
            m.oversized_dropped.clone(),
        );
        registry.register(
            "tayga_ingest_publish_failures",
            "Requests answered UNAVAILABLE/503 because Kafka delivery failed",
            m.publish_failures.clone(),
        );
        registry.register(
            "tayga_ingest_rejected_requests",
            "Requests rejected as undecodable (HTTP 400)",
            m.rejected_requests.clone(),
        );
        m
    }

    /// Count the conversion outcome of one request.
    pub fn record_conversion(&self, converted: &Converted) {
        self.service_routed_items
            .inc_by(converted.routed_by_service as u64);
        self.oversized_dropped
            .inc_by(converted.dropped_oversized as u64);
    }

    pub fn record_published(&self, kind: &str, count: usize) {
        self.records_published
            .get_or_create(&KindLabel::new(kind))
            .inc_by(count as u64);
    }
}
