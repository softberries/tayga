//! OTLP/HTTP log probe: sends one log record to a Tayga ingest endpoint.

use anyhow::{Context, ensure};
use prost::Message;
use std::time::{SystemTime, UNIX_EPOCH};
use tayga_model::otlp::collector::logs::v1::ExportLogsServiceRequest;
use tayga_model::otlp::common::v1::{AnyValue, KeyValue, any_value};
use tayga_model::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use tayga_model::otlp::resource::v1::Resource;

pub fn random_trace_id() -> [u8; 16] {
    let mut id = [0u8; 16];
    rand::fill(&mut id);
    id
}

/// Random lowercase a-z word of `len` characters.
pub fn random_word(len: usize) -> String {
    (0..len)
        .map(|_| (b'a' + rand::random::<u8>() % 26) as char)
        .collect()
}

/// Service the e2e new-template probe logs under, kept apart from the demo services.
pub const PROBE_SERVICE: &str = "tayga-e2e-probe";
/// Constant log that keeps the probe service's oldest template alive (warmup, spec §6).
pub const PROBE_SEED: &str = "tayga e2e probe seed";
/// Number of `probe` tokens in a probe body: bodies have 4 to 58 tokens.
pub const PROBE_REPEATS: std::ops::RangeInclusive<usize> = 2..=56;

/// `{word} probe … probe marker` with `repeats` `probe`s. Drain routes on the token count and
/// then on `word`, so random words and lengths spread probes over ~55 length nodes of 100
/// children each before a node fills (spec §12.7). No token has a digit, so nothing is masked.
pub fn probe_body(word: &str, repeats: usize) -> String {
    let mut body = String::from(word);
    for _ in 0..repeats {
        body.push_str(" probe");
    }
    body.push_str(" marker");
    body
}

pub fn random_probe_repeats() -> usize {
    rand::random_range(PROBE_REPEATS)
}

fn severity_text(severity: i32) -> &'static str {
    match severity {
        9 => "INFO",
        13 => "WARN",
        17 => "ERROR",
        _ => "",
    }
}

/// Pure request construction (no I/O besides the supplied clock value).
pub fn build_request(
    service: &str,
    body: &str,
    trace_id: &[u8; 16],
    span_id: [u8; 8],
    severity: i32,
    now_unix_nano: u64,
) -> ExportLogsServiceRequest {
    let record = LogRecord {
        time_unix_nano: now_unix_nano,
        observed_time_unix_nano: now_unix_nano,
        severity_number: severity,
        severity_text: severity_text(severity).to_string(),
        body: Some(AnyValue {
            value: Some(any_value::Value::StringValue(body.to_string())),
        }),
        trace_id: trace_id.to_vec(),
        span_id: span_id.to_vec(),
        ..Default::default()
    };
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: Some(Resource {
                attributes: vec![KeyValue {
                    key: "service.name".to_string(),
                    value: Some(AnyValue {
                        value: Some(any_value::Value::StringValue(service.to_string())),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            scope_logs: vec![ScopeLogs {
                log_records: vec![record],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

pub async fn emit_log(
    endpoint: &str,
    service: &str,
    body: &str,
    trace_id: &[u8; 16],
    severity: i32,
) -> anyhow::Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u64;
    let mut span_id = [0u8; 8];
    rand::fill(&mut span_id);
    let req = build_request(service, body, trace_id, span_id, severity, now);
    let url = format!("{}/v1/logs", endpoint.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/x-protobuf")
        .body(req.encode_to_vec())
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    ensure!(status.is_success(), "ingest returned {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_expected_request() {
        let tid = [7u8; 16];
        let req = build_request("checkout", "hello world", &tid, [1; 8], 13, 42);
        let rl = &req.resource_logs[0];
        let attr = &rl.resource.as_ref().unwrap().attributes[0];
        assert_eq!(attr.key, "service.name");
        assert_eq!(
            attr.value.as_ref().unwrap().value,
            Some(any_value::Value::StringValue("checkout".into()))
        );
        let rec = &rl.scope_logs[0].log_records[0];
        assert_eq!(rec.trace_id, tid.to_vec());
        assert_eq!(rec.span_id, vec![1u8; 8]);
        assert_eq!(rec.severity_text, "WARN");
        assert_eq!(rec.time_unix_nano, 42);
        assert_eq!(
            rec.body.as_ref().unwrap().value,
            Some(any_value::Value::StringValue("hello world".into()))
        );
    }

    #[test]
    fn probe_body_has_the_planned_shape_and_is_sent_verbatim() {
        let body = probe_body("qwertyuiopas", 2);
        assert_eq!(body, "qwertyuiopas probe probe marker");
        for repeats in [*PROBE_REPEATS.start(), *PROBE_REPEATS.end()] {
            let body = probe_body(&random_word(12), repeats);
            let tokens: Vec<&str> = body.split(' ').collect();
            assert_eq!(tokens.len(), repeats + 2);
            assert!((4..=58).contains(&tokens.len()));
            assert!(!body.bytes().any(|b| b.is_ascii_digit()), "{body}");
            let req = build_request(PROBE_SERVICE, &body, &[1; 16], [1; 8], 9, 1);
            let rec = &req.resource_logs[0].scope_logs[0].log_records[0];
            assert_eq!(
                rec.body.as_ref().unwrap().value,
                Some(any_value::Value::StringValue(body.clone()))
            );
        }
        assert!(PROBE_REPEATS.contains(&random_probe_repeats()));
    }

    #[test]
    fn random_helpers_shape() {
        let w = random_word(12);
        assert_eq!(w.len(), 12);
        assert!(w.bytes().all(|b| b.is_ascii_lowercase()));
        assert_ne!(random_trace_id(), random_trace_id());
    }
}
