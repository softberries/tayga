use crate::records::{Converted, OutRecord, Topic};
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::registry::Registry;
use tayga_common::metrics::KindLabel;
use tayga_model::envelope::Kind;

/// `topic`: the Kafka topic name.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TopicLabel {
    pub topic: String,
}

/// `kind`: `traces` | `logs`.
#[derive(Clone)]
pub struct IngestMetrics {
    signals_topic: String,
    logs_topic: String,
    pub records_published: Family<KindLabel, Counter>,
    pub log_records_published: Family<TopicLabel, Counter>,
    pub service_routed_items: Counter,
    pub oversized_dropped: Family<TopicLabel, Counter>,
    pub publish_failures: Counter,
    pub rejected_requests: Counter,
}

impl Default for IngestMetrics {
    fn default() -> Self {
        Self::new("tayga.signals", "tayga.logs")
    }
}

impl IngestMetrics {
    /// `signals_topic` / `logs_topic`: the Kafka topic names used as metric label values.
    pub fn new(signals_topic: &str, logs_topic: &str) -> Self {
        Self {
            signals_topic: signals_topic.to_string(),
            logs_topic: logs_topic.to_string(),
            records_published: Family::default(),
            log_records_published: Family::default(),
            service_routed_items: Counter::default(),
            oversized_dropped: Family::default(),
            publish_failures: Counter::default(),
            rejected_requests: Counter::default(),
        }
    }

    fn topic_label(&self, topic: Topic) -> TopicLabel {
        TopicLabel {
            topic: match topic {
                Topic::Signals => self.signals_topic.clone(),
                Topic::Logs => self.logs_topic.clone(),
            },
        }
    }

    pub fn register(registry: &mut Registry, signals_topic: &str, logs_topic: &str) -> Self {
        let m = Self::new(signals_topic, logs_topic);
        registry.register(
            "tayga_ingest_records_published",
            "Kafka records published, one logical copy: logs count their signals-topic records",
            m.records_published.clone(),
        );
        registry.register(
            "tayga_ingest_log_records_published",
            "Kafka records carrying logs, by destination topic",
            m.log_records_published.clone(),
        );
        registry.register(
            "tayga_ingest_service_routed_items",
            "Spans/logs without a valid trace id, routed by service, in requests Kafka accepted",
            m.service_routed_items.clone(),
        );
        registry.register(
            "tayga_ingest_oversized_dropped",
            "Single spans/logs dropped for exceeding the record budget, per topic, in accepted requests",
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

    /// Count the conversion outcome of one request, once Kafka accepted it.
    pub fn record_conversion(&self, converted: &Converted) {
        self.service_routed_items
            .inc_by(converted.routed_by_service as u64);
        for (topic, dropped) in [
            (Topic::Signals, converted.dropped_oversized),
            (Topic::Logs, converted.dropped_oversized_logs_topic),
        ] {
            if dropped > 0 {
                self.oversized_dropped
                    .get_or_create(&self.topic_label(topic))
                    .inc_by(dropped as u64);
            }
        }
    }

    /// Count records that were accepted by Kafka. `kind` is `traces` | `logs`.
    pub fn record_published(&self, kind: &str, published: &Published) {
        self.records_published
            .get_or_create(&KindLabel::new(kind))
            .inc_by(published.signals_topic as u64);
        for (topic, count) in [
            (Topic::Signals, published.signals_topic_logs),
            (Topic::Logs, published.logs_topic_logs),
        ] {
            if count > 0 {
                self.log_records_published
                    .get_or_create(&self.topic_label(topic))
                    .inc_by(count as u64);
            }
        }
    }
}

/// Record counts of one request, taken before the records are handed to the sink.
pub struct Published {
    /// Records on the signals topic: one logical copy of the request.
    signals_topic: usize,
    signals_topic_logs: usize,
    logs_topic_logs: usize,
}

impl Published {
    pub fn of(records: &[OutRecord]) -> Self {
        let logs_on = |t| {
            records
                .iter()
                .filter(|r| r.topic == t && r.kind == Kind::Logs)
                .count()
        };
        Self {
            signals_topic: records.iter().filter(|r| r.topic == Topic::Signals).count(),
            signals_topic_logs: logs_on(Topic::Signals),
            logs_topic_logs: logs_on(Topic::Logs),
        }
    }
}
