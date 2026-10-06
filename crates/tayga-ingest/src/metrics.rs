use crate::records::{Converted, OutRecord, Topic};
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::registry::Registry;
use tayga_common::metrics::KindLabel;

/// `topic`: `signals` | `logs`.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TopicLabel {
    pub topic: String,
}

/// `kind`: `traces` | `logs`.
#[derive(Clone, Default)]
pub struct IngestMetrics {
    pub records_published: Family<KindLabel, Counter>,
    pub log_records_published: Family<TopicLabel, Counter>,
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
            "tayga_ingest_log_records_published",
            "Kafka records carrying logs, by destination topic",
            m.log_records_published.clone(),
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

    /// Count records that were accepted by Kafka. `kind` is `traces` | `logs`.
    pub fn record_published(&self, kind: &str, published: &Published) {
        self.records_published
            .get_or_create(&KindLabel::new(kind))
            .inc_by(published.total as u64);
        for (topic, count) in [
            (Topic::Signals, published.signals_logs),
            (Topic::Logs, published.logs_topic),
        ] {
            if count > 0 {
                self.log_records_published
                    .get_or_create(&TopicLabel {
                        topic: topic.as_str().to_string(),
                    })
                    .inc_by(count as u64);
            }
        }
    }
}

/// Record counts of one request, taken before the records are handed to the sink.
pub struct Published {
    total: usize,
    signals_logs: usize,
    logs_topic: usize,
}

impl Published {
    /// `logs`: whether the records carry logs (trace requests never reach the logs topic).
    pub fn of(records: &[OutRecord], logs: bool) -> Self {
        let on = |t| {
            if logs {
                records.iter().filter(|r| r.topic == t).count()
            } else {
                0
            }
        };
        Self {
            total: records.len(),
            signals_logs: on(Topic::Signals),
            logs_topic: on(Topic::Logs),
        }
    }
}
