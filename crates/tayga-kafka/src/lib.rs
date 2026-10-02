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
}

fn default_topic() -> String {
    "tayga.signals".to_string()
}

fn default_partitions() -> i32 {
    12
}

pub fn producer(s: &KafkaSettings) -> KafkaResult<FutureProducer> {
    ClientConfig::new()
        .set("bootstrap.servers", &s.brokers)
        .set("acks", "all")
        .set("enable.idempotence", "true")
        .set("compression.type", "lz4")
        .set("linger.ms", "5")
        .set("queue.buffering.max.messages", "200000")
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

/// Creates the topic if missing; an existing topic is left as is.
pub async fn ensure_topic(s: &KafkaSettings) -> anyhow::Result<()> {
    let admin: AdminClient<DefaultClientContext> =
        ClientConfig::new().set("bootstrap.servers", &s.brokers).create()?;
    let topic = NewTopic::new(&s.topic, s.partitions, TopicReplication::Fixed(1));
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
