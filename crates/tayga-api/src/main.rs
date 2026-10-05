use prometheus_client::registry::Registry;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tayga_api::recorder::{self, Target};
use tayga_api::repo::ChRepo;
use tayga_api::routes::{ApiMetrics, api_router};
use tayga_api::routes_v2::{self, ClientConfig, LagCache};
use tayga_api::spa;
use tayga_kafka::KafkaSettings;
use tayga_store::ClickHouseSettings;
use tayga_store::store::Store;

#[derive(Deserialize)]
struct Settings {
    clickhouse: ClickHouseSettings,
    kafka: KafkaSettings,
    #[serde(default = "default_http")]
    http_addr: SocketAddr,
    /// Optional external links; empty hides them in the app.
    #[serde(default)]
    jaeger_url: String,
    #[serde(default)]
    grafana_url: String,
    #[serde(default = "recorder::default_targets")]
    metric_targets: Vec<Target>,
    #[serde(default = "default_record_secs")]
    record_secs: u64,
}

fn default_record_secs() -> u64 {
    15
}

fn default_http() -> SocketAddr {
    "0.0.0.0:8090".parse().expect("valid default")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tayga_common::init_logging();
    let settings: Settings = tayga_common::load_settings()?;
    tracing::info!(brokers = %settings.kafka.brokers, topic = %settings.kafka.topic, "kafka for consumer-group lag");
    let repo = Arc::new(ChRepo::new(&settings.clickhouse));
    let mut registry = Registry::default();
    let metrics = ApiMetrics::register(&mut registry);
    let registry = Arc::new(registry);
    let stop = tayga_common::shutdown_flag();
    let recorder_registry = registry.clone();
    let recorder = recorder::spawn(
        Store::new(&settings.clickhouse),
        settings.metric_targets,
        move || tayga_common::metrics::render(&recorder_registry),
        metrics.clone(),
        Duration::from_secs(settings.record_secs),
        stop.clone(),
    );
    let v2 = routes_v2::router(
        repo.clone(),
        metrics.clone(),
        LagCache::kafka(settings.kafka.brokers.clone(), settings.kafka.topic.clone()),
        ClientConfig::new(&settings.jaeger_url, &settings.grafana_url),
    );
    let app = api_router(repo, metrics)
        .merge(v2)
        .merge(tayga_common::metrics::router(registry))
        // Last: only paths no other route matched fall through to the app.
        .merge(spa::router());
    let listener = tokio::net::TcpListener::bind(settings.http_addr).await?;
    tracing::info!(addr = %settings.http_addr, "tayga-api listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let mut stop = stop;
            let _ = stop.wait_for(|s| *s).await;
        })
        .await?;
    recorder.await?;
    tracing::info!("tayga-api stopped");
    Ok(())
}
