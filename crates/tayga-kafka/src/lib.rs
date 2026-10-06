//! rdkafka configuration shared by Tayga producers and consumers.

use rdkafka::ClientConfig;
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::consumer::{ConsumerContext, StreamConsumer};
use rdkafka::error::KafkaResult;
use rdkafka::message::{Header, OwnedHeaders};
use rdkafka::producer::FutureProducer;
use rdkafka::types::RDKafkaErrorCode;
use serde::Deserialize;
use tayga_model::envelope::{HEADER_KEY_KIND, HEADER_KIND, HEADER_SCHEMA, Kind, SCHEMA_VERSION};

#[derive(Deserialize, Clone, Debug)]
pub struct KafkaSettings {
    pub brokers: String,
    #[serde(default = "default_topic")]
    pub topic: String,
    #[serde(default = "default_partitions")]
    pub partitions: i32,
    /// Logs keyed by service name, for the logminer replicas.
    #[serde(default = "default_logs_topic")]
    pub logs_topic: String,
    /// Byte budget for one record value; ingest splits larger groups. Must stay below
    /// `MAX_MESSAGE_BYTES` to leave room for the key, headers and record overhead.
    #[serde(default = "default_max_record_bytes")]
    pub max_record_bytes: usize,
    /// `retention.ms` set when a topic is created (24 h by default; `-1` is unlimited). A writer
    /// or assembler outage longer than this loses the records it has not consumed yet. Applies
    /// only when a topic is created: an existing topic keeps its own retention.
    #[serde(default = "default_retention_ms")]
    pub retention_ms: i64,
}

/// Producer `message.max.bytes` and topic `max.message.bytes` (Redpanda's default batch limit).
pub const MAX_MESSAGE_BYTES: usize = 1_048_576;

fn default_topic() -> String {
    "tayga.signals".to_string()
}

fn default_logs_topic() -> String {
    "tayga.logs".to_string()
}

fn default_partitions() -> i32 {
    12
}

fn default_max_record_bytes() -> usize {
    900_000
}

fn default_retention_ms() -> i64 {
    86_400_000
}

impl KafkaSettings {
    /// Rejects settings that would let ingest produce records the broker refuses.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_record_bytes > 0 && self.max_record_bytes < MAX_MESSAGE_BYTES,
            "kafka.max_record_bytes must be between 1 and {} (got {})",
            MAX_MESSAGE_BYTES - 1,
            self.max_record_bytes
        );
        anyhow::ensure!(
            self.logs_topic != self.topic,
            "kafka.logs_topic must differ from kafka.topic (both are {:?})",
            self.topic
        );
        anyhow::ensure!(
            self.partitions > 0,
            "kafka.partitions must be positive (got {})",
            self.partitions
        );
        anyhow::ensure!(
            self.retention_ms == -1 || self.retention_ms > 0,
            "kafka.retention_ms must be -1 (unlimited) or positive (got {})",
            self.retention_ms
        );
        Ok(())
    }
}

pub fn producer(s: &KafkaSettings) -> KafkaResult<FutureProducer> {
    ClientConfig::new()
        .set("bootstrap.servers", &s.brokers)
        .set("acks", "all")
        .set("enable.idempotence", "true")
        .set("compression.type", "lz4")
        .set("linger.ms", "5")
        .set("queue.buffering.max.messages", "200000")
        .set("message.max.bytes", MAX_MESSAGE_BYTES.to_string())
        // Fail deliveries before the collector's 30 s export timeout, so a broker outage
        // surfaces as UNAVAILABLE instead of a client-side timeout plus a late duplicate.
        .set("message.timeout.ms", "25000")
        .create()
}

fn consumer_config(s: &KafkaSettings, group: &str) -> ClientConfig {
    let mut config = ClientConfig::new();
    config
        .set("bootstrap.servers", &s.brokers)
        .set("group.id", group)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("enable.partition.eof", "false")
        .set("session.timeout.ms", "30000")
        .set("max.poll.interval.ms", "600000");
    config
}

/// Manual commits only: callers commit after their side effects succeed.
pub fn consumer(s: &KafkaSettings, group: &str) -> KafkaResult<StreamConsumer> {
    consumer_config(s, group).create()
}

/// Like [`consumer`], with some properties overridden for one service.
pub fn consumer_with_overrides(
    s: &KafkaSettings,
    group: &str,
    overrides: &[(&str, &str)],
) -> KafkaResult<StreamConsumer> {
    config_with_overrides(s, group, overrides).create()
}

fn config_with_overrides(
    s: &KafkaSettings,
    group: &str,
    overrides: &[(&str, &str)],
) -> ClientConfig {
    let mut config = consumer_config(s, group);
    for (key, value) in overrides {
        config.set(*key, *value);
    }
    config
}

/// Like [`consumer`], with a context that receives rebalance callbacks.
pub fn consumer_with_context<C: ConsumerContext + 'static>(
    s: &KafkaSettings,
    group: &str,
    ctx: C,
) -> KafkaResult<StreamConsumer<C>> {
    consumer_config(s, group).create_with_context(ctx)
}

/// Creates the topic if missing (with `max.message.bytes` matching the producer);
/// an existing topic is left as is.
pub async fn ensure_topic(s: &KafkaSettings) -> anyhow::Result<()> {
    create_topics(s, &[&s.topic]).await
}

/// [`ensure_topic`] for the signals topic and the logs topic.
pub async fn ensure_topics(s: &KafkaSettings) -> anyhow::Result<()> {
    create_topics(s, &[&s.topic, &s.logs_topic]).await
}

async fn create_topics(s: &KafkaSettings, names: &[&str]) -> anyhow::Result<()> {
    let admin: AdminClient<DefaultClientContext> = ClientConfig::new()
        .set("bootstrap.servers", &s.brokers)
        .create()?;
    let max_message_bytes = MAX_MESSAGE_BYTES.to_string();
    let retention_ms = s.retention_ms.to_string();
    let topics: Vec<NewTopic> = names
        .iter()
        .map(|name| new_topic(name, s.partitions, &max_message_bytes, &retention_ms))
        .collect();
    for result in admin.create_topics(&topics, &AdminOptions::new()).await? {
        match result {
            Ok(_) | Err((_, RDKafkaErrorCode::TopicAlreadyExists)) => {}
            Err((name, code)) => anyhow::bail!("create topic {name}: {code}"),
        }
    }
    Ok(())
}

/// Config applies only on creation; `create_topics` never alters an existing topic.
fn new_topic<'a>(
    name: &'a str,
    partitions: i32,
    max_message_bytes: &'a str,
    retention_ms: &'a str,
) -> NewTopic<'a> {
    NewTopic::new(name, partitions, TopicReplication::Fixed(1))
        .set("max.message.bytes", max_message_bytes)
        .set("retention.ms", retention_ms)
}

/// `key_kind` is `RoutingKey::kind_str()`: "trace" or "service".
pub fn headers(kind: Kind, key_kind: &str) -> OwnedHeaders {
    OwnedHeaders::new()
        .insert(Header {
            key: HEADER_KIND,
            value: Some(kind.as_str()),
        })
        .insert(Header {
            key: HEADER_SCHEMA,
            value: Some(SCHEMA_VERSION),
        })
        .insert(Header {
            key: HEADER_KEY_KIND,
            value: Some(key_kind),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdkafka::message::Headers;

    #[test]
    fn settings_defaults() {
        let s: KafkaSettings = serde_json::from_str(r#"{"brokers":"b:1"}"#).unwrap();
        assert_eq!(s.topic, "tayga.signals");
        assert_eq!(s.logs_topic, "tayga.logs");
        assert_eq!(s.partitions, 12);
        assert_eq!(s.max_record_bytes, 900_000);
        assert_eq!(s.retention_ms, 86_400_000);
        assert!(s.max_record_bytes < MAX_MESSAGE_BYTES);
    }

    #[test]
    fn headers_carry_kind_schema_and_key_kind() {
        let h = headers(Kind::Logs, "service");
        let pairs: Vec<(String, Vec<u8>)> = h
            .iter()
            .map(|x| (x.key.to_string(), x.value.unwrap_or_default().to_vec()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("tayga-kind".into(), b"logs".to_vec()),
                ("tayga-schema".into(), b"1".to_vec()),
                ("tayga-key".into(), b"service".to_vec()),
            ]
        );
    }

    #[test]
    fn overrides_replace_only_the_named_properties() {
        let s = settings(900_000);
        let c = config_with_overrides(&s, "g", &[("max.poll.interval.ms", "2400000")]);
        assert_eq!(c.get("max.poll.interval.ms"), Some("2400000"));
        assert_eq!(c.get("enable.auto.commit"), Some("false"));
        assert_eq!(
            consumer_config(&s, "g").get("max.poll.interval.ms"),
            Some("600000"),
            "the shared default is unchanged"
        );
    }

    fn settings(max_record_bytes: usize) -> KafkaSettings {
        KafkaSettings {
            brokers: "b:1".into(),
            topic: "t".into(),
            logs_topic: "l".into(),
            partitions: 3,
            max_record_bytes,
            retention_ms: 1000,
        }
    }

    #[test]
    fn new_topic_sets_retention_and_max_message_bytes() {
        let t = new_topic("t", 3, "1048576", "86400000");
        assert!(t.config.contains(&("retention.ms", "86400000")));
        assert!(t.config.contains(&("max.message.bytes", "1048576")));
    }

    #[test]
    fn validate_accepts_defaults() {
        let s: KafkaSettings = serde_json::from_str(r#"{"brokers":"b:1"}"#).unwrap();
        s.validate().unwrap();
    }

    #[test]
    fn validate_rejects_record_budget_at_or_above_message_limit() {
        assert!(settings(MAX_MESSAGE_BYTES).validate().is_err());
        assert!(settings(0).validate().is_err());
        settings(MAX_MESSAGE_BYTES - 1).validate().unwrap();
    }

    #[test]
    fn validate_rejects_non_positive_partitions() {
        let mut s = settings(1000);
        s.partitions = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_logs_topic_equal_to_topic() {
        let mut s = settings(1000);
        s.logs_topic = s.topic.clone();
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_checks_retention() {
        for ok in [-1, 1, default_retention_ms()] {
            let mut s = settings(1000);
            s.retention_ms = ok;
            s.validate().unwrap();
        }
        for bad in [0, -2] {
            let mut s = settings(1000);
            s.retention_ms = bad;
            assert!(s.validate().is_err(), "{bad}");
        }
    }
}
