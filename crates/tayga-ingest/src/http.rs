//! OTLP/HTTP: protobuf or JSON bodies, optionally gzip-compressed.

use crate::records::{Converted, log_records, now_unix_nano, trace_records};
use crate::sink::Sink;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use std::io::Read;
use std::sync::Arc;
use tayga_model::otlp::collector::logs::v1::ExportLogsServiceResponse;
use tayga_model::otlp::collector::trace::v1::ExportTraceServiceResponse;

const MAX_BODY: usize = 64 << 20;

pub fn router<S: Sink>(sink: Arc<S>, max_record_bytes: usize) -> Router {
    Router::new()
        .route("/v1/traces", post(traces::<S>))
        .route("/v1/logs", post(logs::<S>))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(Ingest { sink, max_record_bytes })
}

struct Ingest<S> {
    sink: Arc<S>,
    max_record_bytes: usize,
}

impl<S> Clone for Ingest<S> {
    fn clone(&self) -> Self {
        Self { sink: self.sink.clone(), max_record_bytes: self.max_record_bytes }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Protobuf,
    Json,
}

fn format(headers: &HeaderMap) -> Format {
    let ctype = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if ctype.starts_with("application/json") { Format::Json } else { Format::Protobuf }
}

fn body_bytes(headers: &HeaderMap, body: Bytes) -> Result<Vec<u8>, String> {
    let gzip = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("gzip"));
    if !gzip {
        return Ok(body.to_vec());
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&body[..])
        .take(MAX_BODY as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("gzip: {e}"))?;
    if out.len() > MAX_BODY {
        return Err("decompressed body too large".into());
    }
    Ok(out)
}

fn decode<T>(format: Format, bytes: &[u8]) -> Result<T, String>
where
    T: prost::Message + Default + serde::de::DeserializeOwned,
{
    match format {
        Format::Protobuf => T::decode(bytes).map_err(|e| e.to_string()),
        Format::Json => serde_json::from_slice(bytes).map_err(|e| e.to_string()),
    }
}

fn encode<T: prost::Message + serde::Serialize>(format: Format, msg: &T) -> Response {
    match format {
        Format::Protobuf => {
            ([(header::CONTENT_TYPE, "application/x-protobuf")], msg.encode_to_vec()).into_response()
        }
        Format::Json => (
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::to_vec(msg).unwrap_or_default(),
        )
            .into_response(),
    }
}

async fn traces<S: Sink>(State(ingest): State<Ingest<S>>, headers: HeaderMap, body: Bytes) -> Response {
    handle(&ingest, &headers, body, trace_records, ExportTraceServiceResponse::default()).await
}

async fn logs<S: Sink>(State(ingest): State<Ingest<S>>, headers: HeaderMap, body: Bytes) -> Response {
    handle(&ingest, &headers, body, log_records, ExportLogsServiceResponse::default()).await
}

/// Shared flow: decode (400 on failure), convert to records, publish (503 on failure), encode the response.
async fn handle<S, Req, Resp>(
    ingest: &Ingest<S>,
    headers: &HeaderMap,
    body: Bytes,
    to_records: fn(Req, u64, usize) -> Converted,
    response: Resp,
) -> Response
where
    S: Sink,
    Req: prost::Message + Default + serde::de::DeserializeOwned,
    Resp: prost::Message + serde::Serialize,
{
    let fmt = format(headers);
    let req: Req = match body_bytes(headers, body).and_then(|b| decode(fmt, &b)) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "otlp/http rejected undecodable body");
            return (StatusCode::BAD_REQUEST, e).into_response();
        }
    };
    let records = to_records(req, now_unix_nano(), ingest.max_record_bytes).records;
    let count = records.len();
    if count > 0
        && let Err(e) = ingest.sink.publish(records).await
    {
        tracing::warn!(records = count, error = %e, "otlp/http export failed: kafka publish");
        return (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response();
    }
    encode(fmt, &response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc::tests::{FakeSink, TEST_MAX_RECORD_BYTES, two_trace_request};
    use axum::body::Body;
    use axum::http::Request;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    use tower::ServiceExt;

    async fn post_to(sink: Arc<FakeSink>, path: &str, ctype: &str, gzip: bool, body: Vec<u8>) -> StatusCode {
        let mut req = Request::post(path).header(header::CONTENT_TYPE, ctype);
        if gzip {
            req = req.header(header::CONTENT_ENCODING, "gzip");
        }
        router(sink, TEST_MAX_RECORD_BYTES).oneshot(req.body(Body::from(body)).unwrap()).await.unwrap().status()
    }

    #[tokio::test]
    async fn http_accepts_protobuf() {
        let sink = Arc::new(FakeSink::default());
        let body = prost::Message::encode_to_vec(&two_trace_request());
        assert_eq!(post_to(sink.clone(), "/v1/traces", "application/x-protobuf", false, body).await, StatusCode::OK);
        assert_eq!(sink.published.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn http_accepts_json() {
        let sink = Arc::new(FakeSink::default());
        let body = serde_json::to_vec(&two_trace_request()).unwrap();
        assert_eq!(post_to(sink.clone(), "/v1/traces", "application/json", false, body).await, StatusCode::OK);
        assert_eq!(sink.published.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn http_accepts_gzip_protobuf() {
        let sink = Arc::new(FakeSink::default());
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&prost::Message::encode_to_vec(&two_trace_request())).unwrap();
        let body = enc.finish().unwrap();
        assert_eq!(post_to(sink.clone(), "/v1/traces", "application/x-protobuf", true, body).await, StatusCode::OK);
        assert_eq!(sink.published.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn http_accepts_logs_protobuf() {
        use tayga_model::otlp::collector::logs::v1::ExportLogsServiceRequest;
        use tayga_model::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
        let sink = Arc::new(FakeSink::default());
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord { trace_id: vec![3; 16], ..Default::default() }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let body = prost::Message::encode_to_vec(&req);
        assert_eq!(post_to(sink.clone(), "/v1/logs", "application/x-protobuf", false, body).await, StatusCode::OK);
        assert_eq!(sink.published.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn http_rejects_garbage_with_400() {
        let sink = Arc::new(FakeSink::default());
        let status = post_to(sink, "/v1/logs", "application/x-protobuf", false, vec![0xff, 0xff, 0xff]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn http_sink_failure_is_503() {
        let sink = Arc::new(FakeSink { fail: true, ..Default::default() });
        let body = prost::Message::encode_to_vec(&two_trace_request());
        let status = post_to(sink, "/v1/traces", "application/x-protobuf", false, body).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
