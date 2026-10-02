use rdkafka::producer::Producer;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tayga_ingest::grpc::OtlpGrpc;
use tayga_ingest::kafka_sink::KafkaSink;
use tayga_kafka::KafkaSettings;
use tayga_model::otlp::collector::logs::v1::logs_service_server::LogsServiceServer;
use tayga_model::otlp::collector::trace::v1::trace_service_server::TraceServiceServer;
use tokio::sync::watch;
use tonic::codec::CompressionEncoding;

const MAX_GRPC_MESSAGE: usize = 64 << 20;

#[derive(Deserialize)]
struct Settings {
    kafka: KafkaSettings,
    #[serde(default = "default_grpc")]
    grpc_addr: SocketAddr,
    #[serde(default = "default_http")]
    http_addr: SocketAddr,
}

fn default_grpc() -> SocketAddr {
    "0.0.0.0:4317".parse().expect("valid default")
}

fn default_http() -> SocketAddr {
    "0.0.0.0:4318".parse().expect("valid default")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tayga_common::init_logging();
    let settings: Settings = tayga_common::load_settings()?;
    tayga_kafka::ensure_topic(&settings.kafka).await?;
    let sink = Arc::new(KafkaSink::new(tayga_kafka::producer(&settings.kafka)?, settings.kafka.topic.clone()));

    let (stop_tx, stop_rx) = watch::channel(());
    let grpc = OtlpGrpc::new(sink.clone());
    let traces = TraceServiceServer::new(grpc.clone())
        .accept_compressed(CompressionEncoding::Gzip)
        .max_decoding_message_size(MAX_GRPC_MESSAGE);
    let logs = LogsServiceServer::new(grpc)
        .accept_compressed(CompressionEncoding::Gzip)
        .max_decoding_message_size(MAX_GRPC_MESSAGE);
    let mut grpc_stop = stop_rx.clone();
    let grpc_server = tonic::transport::Server::builder()
        .add_service(traces)
        .add_service(logs)
        .serve_with_shutdown(settings.grpc_addr, async move {
            let _ = grpc_stop.changed().await;
        });

    let listener = tokio::net::TcpListener::bind(settings.http_addr).await?;
    let mut http_stop = stop_rx;
    let http_server = axum::serve(listener, tayga_ingest::http::router(sink.clone()))
        .with_graceful_shutdown(async move {
            let _ = http_stop.changed().await;
        });

    tracing::info!(grpc = %settings.grpc_addr, http = %settings.http_addr, topic = %settings.kafka.topic, "tayga-ingest listening");
    tokio::spawn(async move {
        tayga_common::shutdown_signal().await;
        let _ = stop_tx.send(());
    });
    let (g, h) = tokio::join!(grpc_server, http_server);
    g?;
    h?;
    sink.producer().flush(Duration::from_secs(10))?;
    tracing::info!("tayga-ingest stopped");
    Ok(())
}
