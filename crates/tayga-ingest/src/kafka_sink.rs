use crate::records::{OutRecord, Topic};
use crate::sink::{Sink, SinkError};
use futures::future::join_all;
use rdkafka::error::KafkaError;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::types::RDKafkaErrorCode;
use std::time::Duration;

pub struct KafkaSink {
    producer: FutureProducer,
    topic: String,
    logs_topic: String,
}

impl KafkaSink {
    pub fn new(producer: FutureProducer, topic: String, logs_topic: String) -> Self {
        Self {
            producer,
            topic,
            logs_topic,
        }
    }

    pub fn producer(&self) -> &FutureProducer {
        &self.producer
    }
}

impl Sink for KafkaSink {
    async fn publish(&self, records: Vec<OutRecord>) -> Result<(), SinkError> {
        let sends = records.iter().map(|r| {
            let topic = match r.topic {
                Topic::Signals => &self.topic,
                Topic::Logs => &self.logs_topic,
            };
            let rec = FutureRecord::to(topic)
                .key(&r.key)
                .payload(&r.payload)
                .headers(tayga_kafka::headers(r.kind, r.key_kind));
            // Zero queue timeout: a full local queue fails fast and becomes backpressure.
            self.producer.send(rec, Duration::ZERO)
        });
        for result in join_all(sends).await {
            if let Err((e, _)) = result {
                return Err(match e {
                    KafkaError::MessageProduction(RDKafkaErrorCode::QueueFull) => {
                        SinkError::QueueFull
                    }
                    other => SinkError::Delivery(other.to_string()),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdkafka::ClientConfig;
    use tayga_model::envelope::Kind;

    #[tokio::test]
    async fn a_full_local_queue_is_queue_full_backpressure() {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", "127.0.0.1:1")
            .set("queue.buffering.max.messages", "1")
            .create()
            .unwrap();
        // Fill the one-message queue; the broker is unreachable, so the message stays queued.
        let _queued = producer
            .send_result(FutureRecord::<(), [u8]>::to("t").payload(&b"x"[..]))
            .map_err(|(e, _)| e)
            .unwrap();
        let sink = KafkaSink::new(producer, "t".into(), "l".into());
        let err = sink
            .publish(vec![OutRecord {
                topic: Topic::Signals,
                key: vec![1; 16],
                key_kind: "trace",
                kind: Kind::Traces,
                payload: vec![1, 2, 3],
            }])
            .await
            .unwrap_err();
        assert!(matches!(err, SinkError::QueueFull), "{err}");
    }
}
