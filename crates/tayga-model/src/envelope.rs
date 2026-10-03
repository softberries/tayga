//! Kafka record value for `tayga.signals`: OTLP data for exactly one routing key.

use crate::otlp::collector::logs::v1::ExportLogsServiceRequest;
use crate::otlp::collector::trace::v1::ExportTraceServiceRequest;

pub const SCHEMA_VERSION: &str = "1";
pub const HEADER_KIND: &str = "tayga-kind";
pub const HEADER_SCHEMA: &str = "tayga-schema";
/// Routing key kind: `trace` (16 raw trace id bytes) or `service` (UTF-8 service name).
pub const HEADER_KEY_KIND: &str = "tayga-key";

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
        Self {
            received_at_unix_nano,
            payload: Some(Payload::Traces(req)),
        }
    }

    pub fn logs(req: ExportLogsServiceRequest, received_at_unix_nano: u64) -> Self {
        Self {
            received_at_unix_nano,
            payload: Some(Payload::Logs(req)),
        }
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

/// Writes envelopes as `u32` little-endian length + protobuf bytes (fixture files).
pub fn write_framed<W: std::io::Write>(w: &mut W, envelopes: &[Envelope]) -> std::io::Result<()> {
    for env in envelopes {
        let bytes = env.encode();
        let len = u32::try_from(bytes.len()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "envelope larger than 4 GiB",
            )
        })?;
        w.write_all(&len.to_le_bytes())?;
        w.write_all(&bytes)?;
    }
    Ok(())
}

/// Reads envelopes written by [`write_framed`] until end of input.
pub fn read_framed<R: std::io::Read>(r: &mut R) -> std::io::Result<Vec<Envelope>> {
    use std::io::{Error, ErrorKind, Read};
    let mut out = Vec::new();
    loop {
        let mut len = [0u8; 4];
        // Zero bytes before the next header is a clean end; 1-3 bytes is a truncated header.
        let mut filled = 0;
        while filled < len.len() {
            match r.read(&mut len[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        match filled {
            0 => return Ok(out),
            4 => {}
            _ => {
                return Err(Error::new(
                    ErrorKind::UnexpectedEof,
                    "truncated frame header",
                ));
            }
        }
        let len = u32::from_le_bytes(len) as usize;
        // `take` bounds the read; memory grows only with bytes actually present.
        let mut buf = Vec::new();
        r.take(len as u64).read_to_end(&mut buf)?;
        if buf.len() != len {
            return Err(Error::new(ErrorKind::UnexpectedEof, "truncated frame body"));
        }
        out.push(Envelope::decode(&buf).map_err(|e| Error::new(ErrorKind::InvalidData, e))?);
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
                    spans: vec![Span {
                        trace_id: vec![7; 16],
                        name: "GET /".into(),
                        ..Default::default()
                    }],
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
        assert_eq!(
            Envelope::decode(&env.encode()).unwrap().kind(),
            Some(Kind::Logs)
        );
        assert_eq!(Kind::Logs.as_str(), "logs");
        assert_eq!(Kind::Traces.as_str(), "traces");
    }

    #[test]
    fn decode_garbage_fails() {
        assert!(Envelope::decode(&[0xff, 0xff, 0xff]).is_err());
    }

    #[test]
    fn wire_format_is_pinned() {
        // fixed64 field 1 (tag 0x09) + 8 LE bytes, then oneof field 3 (tag 0x1a) with length 0.
        let env = Envelope::logs(ExportLogsServiceRequest::default(), 1);
        assert_eq!(env.encode(), vec![0x09, 1, 0, 0, 0, 0, 0, 0, 0, 0x1a, 0x00]);
        // proto3 omits a zero fixed64; field 2 (tag 0x12) for traces.
        let env = Envelope::traces(ExportTraceServiceRequest::default(), 0);
        assert_eq!(env.encode(), vec![0x12, 0x00]);
    }

    #[test]
    fn framed_roundtrip() {
        let envs = vec![
            Envelope::traces(req(), 5),
            Envelope::logs(ExportLogsServiceRequest::default(), 6),
        ];
        let mut buf = Vec::new();
        write_framed(&mut buf, &envs).unwrap();
        assert_eq!(read_framed(&mut buf.as_slice()).unwrap(), envs);
        assert!(read_framed(&mut [].as_slice()).unwrap().is_empty());
    }

    #[test]
    fn framed_truncated_input_errors() {
        let mut buf = Vec::new();
        write_framed(&mut buf, &[Envelope::traces(req(), 5)]).unwrap();
        buf.pop();
        assert!(read_framed(&mut buf.as_slice()).is_err());
    }

    #[test]
    fn framed_huge_length_prefix_errors_without_allocating() {
        let mut buf = 0xFFFF_FFFFu32.to_le_bytes().to_vec();
        buf.extend_from_slice(&[1, 2, 3]);
        assert!(read_framed(&mut buf.as_slice()).is_err());
    }

    #[test]
    fn framed_partial_header_errors() {
        let mut buf = Vec::new();
        write_framed(&mut buf, &[Envelope::traces(req(), 5)]).unwrap();
        buf.extend_from_slice(&[7, 0]);
        assert!(read_framed(&mut buf.as_slice()).is_err());
    }
}
