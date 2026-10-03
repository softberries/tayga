//! Captures complete traces from `tayga.signals` into fixture files for golden tests.

use rdkafka::ClientConfig;
use rdkafka::Message;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::message::Headers;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tayga_analysis::model::TraceBundle;
use tayga_analysis::rootcause::error_flags;
use tayga_analysis::tree::SpanTree;
use tayga_model::envelope::{Envelope, HEADER_KEY_KIND};
use tayga_model::ids::TraceId;

/// After the collection window, keep reading this long to complete traces already seen.
const SETTLE: Duration = Duration::from_secs(15);

pub fn has_error(envelopes: &[Envelope]) -> bool {
    let mut bundle = TraceBundle::new("");
    for env in envelopes {
        bundle.add_envelope(env);
    }
    SpanTree::build(&bundle).is_some_and(|t| error_flags(&t).into_iter().any(|e| e))
}

pub fn has_service(envelopes: &[Envelope], service: &str) -> bool {
    let mut bundle = TraceBundle::new("");
    for env in envelopes {
        bundle.add_envelope(env);
    }
    bundle.spans.iter().any(|s| s.service == service)
}

/// Reads new records for `seconds`, then `SETTLE` more for traces already seen; returns the
/// envelopes of up to `max_traces` traces (smallest trace ids first), optionally only error traces and/or traces containing a span of
/// `require_service`.
pub async fn capture(
    brokers: &str,
    topic: &str,
    seconds: u64,
    max_traces: usize,
    only_errors: bool,
    require_service: Option<&str>,
) -> anyhow::Result<Vec<Envelope>> {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set(
            "group.id",
            format!("tayga-capture-{}", rand::random::<u32>()),
        )
        .set("auto.offset.reset", "latest")
        .set("enable.auto.commit", "false")
        .create()?;
    consumer.subscribe(&[topic])?;
    let collect_until = Instant::now() + Duration::from_secs(seconds);
    let settle_until = collect_until + SETTLE;
    let mut traces: BTreeMap<String, Vec<Envelope>> = BTreeMap::new();
    while Instant::now() < settle_until {
        let Ok(next) = tokio::time::timeout(Duration::from_millis(500), consumer.recv()).await
        else {
            continue;
        };
        let m = match next {
            Ok(m) => m,
            // Transport errors are transient (e.g. an unreachable IPv6 `localhost` address
            // being tried first); librdkafka reconnects on its own.
            Err(e) => {
                eprintln!("consumer error (continuing): {e}");
                continue;
            }
        };
        let service_keyed = m
            .headers()
            .and_then(|h| h.iter().find(|h| h.key == HEADER_KEY_KIND))
            .is_some_and(|h| h.value == Some(b"service".as_slice()));
        let Some(id) = m
            .key()
            .and_then(TraceId::from_slice)
            .filter(|_| !service_keyed)
            .map(|t| t.to_hex())
        else {
            continue;
        };
        if Instant::now() >= collect_until && !traces.contains_key(&id) {
            continue;
        }
        let Some(payload) = m.payload() else { continue };
        traces
            .entry(id)
            .or_default()
            .push(Envelope::decode(payload)?);
    }
    Ok(traces
        .into_values()
        .filter(|envs| !only_errors || has_error(envs))
        .filter(|envs| require_service.is_none_or(|svc| has_service(envs, svc)))
        .take(max_traces)
        .flatten()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tayga_model::otlp::collector::trace::v1::ExportTraceServiceRequest;
    use tayga_model::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
    use tayga_model::otlp::resource::v1::Resource;
    use tayga_model::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};

    fn env(svc: &str, id: u8) -> Envelope {
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![KeyValue {
                        key: "service.name".into(),
                        value: Some(AnyValue {
                            value: Some(Value::StringValue(svc.into())),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                scope_spans: vec![ScopeSpans {
                    spans: vec![Span {
                        trace_id: vec![7; 16],
                        span_id: vec![id; 8],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        Envelope::traces(req, 0)
    }

    #[test]
    fn has_service_matches_any_span_service() {
        let envs = vec![env("checkout", 1), env("shipping", 2)];
        assert!(has_service(&envs, "checkout"));
        assert!(has_service(&envs, "shipping"));
        assert!(!has_service(&envs, "payment"));
    }
}
