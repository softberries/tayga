//! rdkafka configuration shared by Tayga producers and consumers.

use rdkafka::ClientConfig;
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::consumer::StreamConsumer;
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
    /// Byte budget for one record value; ingest splits larger groups. Must stay below
    /// `MAX_MESSAGE_BYTES` to leave room for the key, headers and record overhead.
    #[serde(default = "default_max_record_bytes")]
    pub max_record_bytes: usize,
}

/// Producer `message.max.bytes` and topic `max.message.bytes` (Redpanda's default batch limit).
pub const MAX_MESSAGE_BYTES: usize = 1_048_576;

fn default_topic() -> String {
    "tayga.signals".to_string()
}

fn default_partitions() -> i32 {
    12
}

fn default_max_record_bytes() -> usize {
    900_000
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

/// Manual commits only: callers commit after their side effects succeed.
pub fn consumer(s: &KafkaSettings, group: &str) -> KafkaResult<StreamConsumer> {
    ClientConfig::new()
        .set("bootstrap.servers", &s.brokers)
        .set("group.id", group)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("enable.partition.eof", "false")
        .set("session.timeout.ms", "30000")
        .set("max.poll.interval.ms", "600000")
        .create()
}

/// Creates the topic if missing (with `max.message.bytes` matching the producer);
/// an existing topic is left as is.
pub async fn ensure_topic(s: &KafkaSettings) -> anyhow::Result<()> {
    let admin: AdminClient<DefaultClientContext> =
        ClientConfig::new().set("bootstrap.servers", &s.brokers).create()?;
    let max_message_bytes = MAX_MESSAGE_BYTES.to_string();
    let topic = NewTopic::new(&s.topic, s.partitions, TopicReplication::Fixed(1))
        .set("max.message.bytes", &max_message_bytes);
    for result in admin.create_topics(&[topic], &AdminOptions::new()).await? {
        match result {
            Ok(_) | Err((_, RDKafkaErrorCode::TopicAlreadyExists)) => {}
            Err((name, code)) => anyhow::bail!("create topic {name}: {code}"),
        }
    }
    Ok(())
}

/// `key_kind` is `RoutingKey::kind_str()`: "trace" or "service".
pub fn headers(kind: Kind, key_kind: &str) -> OwnedHeaders {
    OwnedHeaders::new()
        .insert(Header { key: HEADER_KIND, value: Some(kind.as_str()) })
        .insert(Header { key: HEADER_SCHEMA, value: Some(SCHEMA_VERSION) })
        .insert(Header { key: HEADER_KEY_KIND, value: Some(key_kind) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdkafka::message::Headers;

    #[test]
    fn settings_defaults() {
        let s: KafkaSettings = serde_json::from_str(r#"{"brokers":"b:1"}"#).unwrap();
        assert_eq!(s.topic, "tayga.signals");
        assert_eq!(s.partitions, 12);
        assert_eq!(s.max_record_bytes, 900_000);
        assert!(s.max_record_bytes < MAX_MESSAGE_BYTES);
    }

    #[test]
    fn headers_carry_kind_schema_and_key_kind() {
        let h = headers(Kind::Logs, "service");
        let pairs: Vec<(String, Vec<u8>)> =
            h.iter().map(|x| (x.key.to_string(), x.value.unwrap_or_default().to_vec())).collect();
        assert_eq!(
            pairs,
            vec![
                ("tayga-kind".into(), b"logs".to_vec()),
                ("tayga-schema".into(), b"1".to_vec()),
                ("tayga-key".into(), b"service".to_vec()),
            ]
        );
    }
}
