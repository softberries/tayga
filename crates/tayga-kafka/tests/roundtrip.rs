use rdkafka::Message;
use rdkafka::consumer::{CommitMode, Consumer};
use rdkafka::producer::FutureRecord;
use std::time::Duration;
use tayga_kafka::{KafkaSettings, consumer, ensure_topic, headers, producer};
use tayga_model::envelope::Kind;

fn settings() -> KafkaSettings {
    let brokers = std::env::var("TAYGA_IT_KAFKA").unwrap_or_else(|_| "localhost:19092".into());
    let suffix: u32 = rand::random();
    KafkaSettings { brokers, topic: format!("tayga-it-{suffix}"), partitions: 3 }
}

#[tokio::test]
#[ignore = "requires Redpanda: make it"]
async fn produce_consume_commit_roundtrip() {
    let s = settings();
    ensure_topic(&s).await.unwrap();
    ensure_topic(&s).await.unwrap(); // idempotent

    let p = producer(&s).unwrap();
    let key = [9u8; 16];
    p.send(
        FutureRecord::to(&s.topic).key(&key[..]).payload(&b"hello"[..]).headers(headers(Kind::Traces)),
        Duration::from_secs(5),
    )
    .await
    .map_err(|(e, _)| e)
    .unwrap();

    let c = consumer(&s, "tayga-it").unwrap();
    c.subscribe(&[&s.topic]).unwrap();
    let m = tokio::time::timeout(Duration::from_secs(30), c.recv()).await.unwrap().unwrap();
    assert_eq!(m.key(), Some(&key[..]));
    assert_eq!(m.payload(), Some(&b"hello"[..]));
    c.commit_message(&m, CommitMode::Sync).unwrap();
}
