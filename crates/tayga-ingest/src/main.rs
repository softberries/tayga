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
use tokio_stream::wrappers::TcpListenerStream;
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

    // Bind both listeners eagerly so a bind failure aborts startup before anything serves.
    let grpc_listener = tokio::net::TcpListener::bind(settings.grpc_addr).await?;
    let http_listener = tokio::net::TcpListener::bind(settings.http_addr).await?;

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
        .serve_with_incoming_shutdown(TcpListenerStream::new(grpc_listener), async move {
            let _ = grpc_stop.changed().await;
        });

    let mut http_stop = stop_rx;
    let http_server = axum::serve(http_listener, tayga_ingest::http::router(sink.clone()))
        .with_graceful_shutdown(async move {
            let _ = http_stop.changed().await;
        });

    tracing::info!(grpc = %settings.grpc_addr, http = %settings.http_addr, topic = %settings.kafka.topic, "tayga-ingest listening");

    // Whichever finishes first (signal, error, or exit) stops the other; the first error wins.
    let grpc_task = async {
        let r = grpc_server.await.map_err(anyhow::Error::from);
        let _ = stop_tx.send(());
        r
    };
    let http_task = async {
        let r = http_server.await.map_err(anyhow::Error::from);
        let _ = stop_tx.send(());
        r
    };
    let signal_task = async {
        tayga_common::shutdown_signal().await;
        let _ = stop_tx.send(());
    };
    let (g, h, ()) = tokio::join!(grpc_task, http_task, signal_task);

    sink.producer().flush(Duration::from_secs(10))?;
    g.and(h)?;
    tracing::info!("tayga-ingest stopped");
    Ok(())
}
