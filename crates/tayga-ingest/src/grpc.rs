use crate::records::{OutRecord, log_records, now_unix_nano, trace_records};
use crate::sink::{Sink, SinkError};
use std::sync::Arc;
use tayga_model::otlp::collector::logs::v1::logs_service_server::LogsService;
use tayga_model::otlp::collector::logs::v1::{ExportLogsServiceRequest, ExportLogsServiceResponse};
use tayga_model::otlp::collector::trace::v1::trace_service_server::TraceService;
use tayga_model::otlp::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use tonic::{Request, Response, Status};

pub struct OtlpGrpc<S> {
    sink: Arc<S>,
    max_record_bytes: usize,
}

impl<S> OtlpGrpc<S> {
    pub fn new(sink: Arc<S>, max_record_bytes: usize) -> Self {
        Self {
            sink,
            max_record_bytes,
        }
    }
}

impl<S> Clone for OtlpGrpc<S> {
    fn clone(&self) -> Self {
        Self {
            sink: self.sink.clone(),
            max_record_bytes: self.max_record_bytes,
        }
    }
}

pub fn status_from(e: SinkError) -> Status {
    // UNAVAILABLE is retryable for OTLP exporters, so the collector retries/queues.
    Status::unavailable(e.to_string())
}

async fn publish<S: Sink>(
    sink: &S,
    records: Vec<OutRecord>,
    signal: &'static str,
) -> Result<(), Status> {
    if records.is_empty() {
        return Ok(());
    }
    let count = records.len();
    sink.publish(records).await.map_err(|e| {
        tracing::warn!(signal, records = count, error = %e, "otlp/grpc export failed: kafka publish");
        status_from(e)
    })
}

#[tonic::async_trait]
impl<S: Sink> TraceService for OtlpGrpc<S> {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let records =
            trace_records(request.into_inner(), now_unix_nano(), self.max_record_bytes).records;
        publish(&*self.sink, records, "traces").await?;
        Ok(Response::new(ExportTraceServiceResponse::default()))
    }
}

#[tonic::async_trait]
impl<S: Sink> LogsService for OtlpGrpc<S> {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let records =
            log_records(request.into_inner(), now_unix_nano(), self.max_record_bytes).records;
        publish(&*self.sink, records, "logs").await?;
        Ok(Response::new(ExportLogsServiceResponse::default()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;
    use tayga_model::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};

    pub(crate) const TEST_MAX_RECORD_BYTES: usize = 1 << 20;

    #[derive(Default)]
    pub(crate) struct FakeSink {
        pub published: Mutex<Vec<OutRecord>>,
        pub fail: bool,
    }

    impl Sink for FakeSink {
        async fn publish(&self, records: Vec<OutRecord>) -> Result<(), SinkError> {
            if self.fail {
                return Err(SinkError::QueueFull);
            }
            self.published.lock().unwrap().extend(records);
            Ok(())
        }
    }

    pub(crate) fn two_trace_request() -> ExportTraceServiceRequest {
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    spans: vec![
                        Span {
                            trace_id: vec![1; 16],
                            ..Default::default()
                        },
                        Span {
                            trace_id: vec![2; 16],
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    #[tokio::test]
    async fn traces_are_published_per_trace() {
        let sink = Arc::new(FakeSink::default());
        let svc = OtlpGrpc::new(sink.clone(), TEST_MAX_RECORD_BYTES);
        TraceService::export(&svc, Request::new(two_trace_request()))
            .await
            .unwrap();
        assert_eq!(sink.published.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn oversized_span_is_dropped_without_failing_the_request() {
        let sink = Arc::new(FakeSink::default());
        let svc = OtlpGrpc::new(sink.clone(), 2_000);
        let mut req = two_trace_request();
        req.resource_spans[0].scope_spans[0].spans[0].name = "x".repeat(10_000);
        TraceService::export(&svc, Request::new(req)).await.unwrap();
        let published = sink.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].key, vec![2; 16]);
    }

    #[tokio::test]
    async fn sink_failure_maps_to_unavailable() {
        let sink = Arc::new(FakeSink {
            fail: true,
            ..Default::default()
        });
        let svc = OtlpGrpc::new(sink, TEST_MAX_RECORD_BYTES);
        let err = TraceService::export(&svc, Request::new(two_trace_request()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unavailable);
    }

    #[tokio::test]
    async fn empty_logs_export_succeeds_without_publishing() {
        let sink = Arc::new(FakeSink::default());
        let svc = OtlpGrpc::new(sink.clone(), TEST_MAX_RECORD_BYTES);
        LogsService::export(&svc, Request::new(ExportLogsServiceRequest::default()))
            .await
            .unwrap();
        assert!(sink.published.lock().unwrap().is_empty());
    }
}
