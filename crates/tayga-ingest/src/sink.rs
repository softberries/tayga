use crate::records::OutRecord;
use std::future::Future;

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("producer queue full")]
    QueueFull,
    #[error("delivery failed: {0}")]
    Delivery(String),
}

/// Destination for ingest records. Resolves only when every record is durably accepted.
pub trait Sink: Send + Sync + 'static {
    fn publish(&self, records: Vec<OutRecord>) -> impl Future<Output = Result<(), SinkError>> + Send;
}
