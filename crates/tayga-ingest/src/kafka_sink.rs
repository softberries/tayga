use crate::records::OutRecord;
use crate::sink::{Sink, SinkError};
use futures::future::join_all;
use rdkafka::error::KafkaError;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::types::RDKafkaErrorCode;
use std::time::Duration;

pub struct KafkaSink {
    producer: FutureProducer,
    topic: String,
}

impl KafkaSink {
    pub fn new(producer: FutureProducer, topic: String) -> Self {
        Self { producer, topic }
    }

    pub fn producer(&self) -> &FutureProducer {
        &self.producer
    }
}

impl Sink for KafkaSink {
    async fn publish(&self, records: Vec<OutRecord>) -> Result<(), SinkError> {
        let sends = records.iter().map(|r| {
            let rec = FutureRecord::to(&self.topic)
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
