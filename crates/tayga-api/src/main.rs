use prometheus_client::registry::Registry;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tayga_api::repo::ChRepo;
use tayga_api::routes::{ApiMetrics, api_router};
use tayga_api::ui::{UiLinks, ui_router};
use tayga_store::ClickHouseSettings;

#[derive(Deserialize)]
struct Settings {
    clickhouse: ClickHouseSettings,
    #[serde(default = "default_http")]
    http_addr: SocketAddr,
    #[serde(default = "default_jaeger")]
    jaeger_url: String,
    #[serde(default = "default_grafana")]
    grafana_url: String,
}

fn default_http() -> SocketAddr {
    "0.0.0.0:8090".parse().expect("valid default")
}

fn default_jaeger() -> String {
    "http://localhost:8080/jaeger/ui".to_string()
}

fn default_grafana() -> String {
    "http://localhost:3001".to_string()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tayga_common::init_logging();
    let settings: Settings = tayga_common::load_settings()?;
    let repo = Arc::new(ChRepo::new(&settings.clickhouse));
    let mut registry = Registry::default();
    let metrics = ApiMetrics::register(&mut registry);
    let app = api_router(repo.clone(), metrics.clone())
        .merge(ui_router(
            repo,
            metrics,
            UiLinks {
                jaeger_url: settings.jaeger_url,
                grafana_url: settings.grafana_url,
            },
        ))
        .merge(tayga_common::metrics::router(Arc::new(registry)));
    let listener = tokio::net::TcpListener::bind(settings.http_addr).await?;
    tracing::info!(addr = %settings.http_addr, "tayga-api listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(tayga_common::shutdown_signal())
        .await?;
    tracing::info!("tayga-api stopped");
    Ok(())
}
