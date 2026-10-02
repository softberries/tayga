use tayga_model::envelope::{Envelope, Kind};
use tayga_model::otlp::collector::logs::v1::ExportLogsServiceRequest;
use tayga_model::otlp::collector::trace::v1::ExportTraceServiceRequest;
use tayga_model::split::{split_logs, split_traces};

#[derive(Debug, Clone, PartialEq)]
pub struct OutRecord {
    pub key: Vec<u8>,
    /// `tayga-key` header value: "trace" or "service".
    pub key_kind: &'static str,
    pub kind: Kind,
    pub payload: Vec<u8>,
}

pub fn now_unix_nano() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

pub fn trace_records(req: ExportTraceServiceRequest, now_unix_nano: u64) -> Vec<OutRecord> {
    split_traces(req)
        .into_iter()
        .map(|r| OutRecord {
            key: r.key.to_bytes(),
            key_kind: r.key.kind_str(),
            kind: Kind::Traces,
            payload: Envelope::traces(r.request, now_unix_nano).encode(),
        })
        .collect()
}

pub fn log_records(req: ExportLogsServiceRequest, now_unix_nano: u64) -> Vec<OutRecord> {
    split_logs(req)
        .into_iter()
        .map(|r| OutRecord {
            key: r.key.to_bytes(),
            key_kind: r.key.kind_str(),
            kind: Kind::Logs,
            payload: Envelope::logs(r.request, now_unix_nano).encode(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tayga_model::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};

    #[test]
    fn one_record_per_trace_with_decodable_envelope() {
        let spans = vec![
            Span { trace_id: vec![1; 16], ..Default::default() },
            Span { trace_id: vec![2; 16], ..Default::default() },
        ];
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans { spans, ..Default::default() }],
                ..Default::default()
            }],
        };
        let out = trace_records(req, 5);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key, vec![1; 16]);
        assert_eq!(out[0].key_kind, "trace");
        assert_eq!(out[0].kind, Kind::Traces);
        let env = Envelope::decode(&out[1].payload).unwrap();
        assert_eq!(env.received_at_unix_nano, 5);
        assert_eq!(env.kind(), Some(Kind::Traces));
    }

    #[test]
    fn empty_logs_request_yields_no_records() {
        assert!(log_records(ExportLogsServiceRequest::default(), 1).is_empty());
    }
}
