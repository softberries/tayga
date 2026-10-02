//! Kafka record value for `tayga.signals`: OTLP data for exactly one routing key.

use crate::otlp::collector::logs::v1::ExportLogsServiceRequest;
use crate::otlp::collector::trace::v1::ExportTraceServiceRequest;

pub const SCHEMA_VERSION: &str = "1";
pub const HEADER_KIND: &str = "tayga-kind";
pub const HEADER_SCHEMA: &str = "tayga-schema";

#[derive(Clone, PartialEq, prost::Message)]
pub struct Envelope {
    #[prost(fixed64, tag = "1")]
    pub received_at_unix_nano: u64,
    #[prost(oneof = "Payload", tags = "2, 3")]
    pub payload: Option<Payload>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum Payload {
    #[prost(message, tag = "2")]
    Traces(ExportTraceServiceRequest),
    #[prost(message, tag = "3")]
    Logs(ExportLogsServiceRequest),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Traces,
    Logs,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Traces => "traces",
            Kind::Logs => "logs",
        }
    }
}

impl Envelope {
    pub fn traces(req: ExportTraceServiceRequest, received_at_unix_nano: u64) -> Self {
        Self { received_at_unix_nano, payload: Some(Payload::Traces(req)) }
    }

    pub fn logs(req: ExportLogsServiceRequest, received_at_unix_nano: u64) -> Self {
        Self { received_at_unix_nano, payload: Some(Payload::Logs(req)) }
    }

    pub fn kind(&self) -> Option<Kind> {
        match self.payload {
            Some(Payload::Traces(_)) => Some(Kind::Traces),
            Some(Payload::Logs(_)) => Some(Kind::Logs),
            None => None,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        prost::Message::encode_to_vec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, prost::DecodeError> {
        prost::Message::decode(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};

    fn req() -> ExportTraceServiceRequest {
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    spans: vec![Span { trace_id: vec![7; 16], name: "GET /".into(), ..Default::default() }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn roundtrip_traces() {
        let env = Envelope::traces(req(), 42);
        let back = Envelope::decode(&env.encode()).unwrap();
        assert_eq!(back, env);
        assert_eq!(back.kind(), Some(Kind::Traces));
        assert_eq!(back.received_at_unix_nano, 42);
    }

    #[test]
    fn roundtrip_logs_kind() {
        let env = Envelope::logs(ExportLogsServiceRequest::default(), 1);
        assert_eq!(Envelope::decode(&env.encode()).unwrap().kind(), Some(Kind::Logs));
        assert_eq!(Kind::Logs.as_str(), "logs");
        assert_eq!(Kind::Traces.as_str(), "traces");
    }

    #[test]
    fn decode_garbage_fails() {
        assert!(Envelope::decode(&[0xff, 0xff, 0xff]).is_err());
    }
}
