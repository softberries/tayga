use axum::middleware;
use prometheus_client::registry::Registry;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tayga_api::auth::{self, AuthSettings};
use tayga_api::recorder::{self, Target};
use tayga_api::repo::ChRepo;
use tayga_api::routes::{ApiMetrics, api_router};
use tayga_api::routes_v2::{self, ClientConfig, LagCache};
use tayga_api::spa;
use tayga_api::timeout;
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
    /// Seconds a ClickHouse read may run (`max_execution_time`); an `/api/` request still running
    /// `timeout::SLACK_SECS` later is answered 504. `0` turns both off.
    #[serde(default = "default_query_timeout_secs")]
    query_timeout_secs: u64,
    #[serde(default)]
    auth: AuthSettings,
    #[serde(default)]
    map: MapSettings,
}

/// Service map options.
#[derive(Deserialize)]
struct MapSettings {
    /// Services the map hides unless "Show infrastructure" is on.
    #[serde(default = "default_infra")]
    infra_services: Vec<String>,
}

impl Default for MapSettings {
    fn default() -> Self {
        Self {
            infra_services: default_infra(),
        }
    }
}

fn default_infra() -> Vec<String> {
    vec!["flagd".into()]
}

fn default_record_secs() -> u64 {
    15
}

fn default_query_timeout_secs() -> u64 {
    15
}

fn default_http() -> SocketAddr {
    "0.0.0.0:8090".parse().expect("valid default")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tayga_common::init_logging();
    let settings: Settings = tayga_common::load_settings()?;
    let auth = auth::Auth::from_settings(&settings.auth)?;
    tracing::info!(enabled = auth.is_some(), "auth");
    tracing::info!(brokers = %settings.kafka.brokers, topic = %settings.kafka.topic, "kafka for consumer-group lag");
    if (1..5).contains(&settings.record_secs) {
        tracing::warn!(
            record_secs = settings.record_secs,
            "record_secs is small: small parts; use >= 5 or 0 to turn off"
        );
    }
    let repo = Arc::new(
        ChRepo::new(&settings.clickhouse).with_max_execution_time(settings.query_timeout_secs),
    );
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
        LagCache::kafka(
            settings.kafka.brokers.clone(),
            settings.kafka.topic.clone(),
            settings.kafka.logs_topic.clone(),
        ),
        ClientConfig::new(
            &settings.jaeger_url,
            &settings.grafana_url,
            auth.is_some(),
            settings.map.infra_services,
        ),
    );
    let mut app = api_router(repo, metrics)
        .merge(v2)
        .merge(tayga_common::metrics::router(registry))
        .merge(auth::routes(auth.clone()))
        // Only paths no other route matched fall through to the app.
        .merge(spa::router());
    if settings.query_timeout_secs > 0 {
        let limit = Duration::from_secs(
            settings
                .query_timeout_secs
                .saturating_add(timeout::SLACK_SECS),
        );
        app = app.layer(middleware::from_fn_with_state(limit, timeout::api_timeout));
    }
    if let Some(auth) = auth {
        app = app.layer(middleware::from_fn_with_state(auth, auth::require));
    }
    let listener = tokio::net::TcpListener::bind(settings.http_addr).await?;
    tracing::info!(addr = %settings.http_addr, "tayga-api listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let mut stop = stop;
        let _ = stop.wait_for(|s| *s).await;
    })
    .await?;
    recorder.await?;
    tracing::info!("tayga-api stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(extra: serde_json::Value) -> Settings {
        let mut v = serde_json::json!({
            "clickhouse": {"url": "http://ch"},
            "kafka": {"brokers": "k:9092"},
        });
        v.as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        serde_json::from_value(v).expect("settings")
    }

    #[test]
    fn query_timeout_defaults_to_15_s_and_0_turns_it_off() {
        assert_eq!(settings(serde_json::json!({})).query_timeout_secs, 15);
        assert_eq!(
            settings(serde_json::json!({"query_timeout_secs": 0})).query_timeout_secs,
            0
        );
    }

    #[test]
    fn infra_services_default_to_flagd_and_can_be_overridden() {
        assert_eq!(
            settings(serde_json::json!({})).map.infra_services,
            ["flagd"]
        );
        assert_eq!(
            settings(serde_json::json!({"map": {}})).map.infra_services,
            ["flagd"]
        );
        assert_eq!(
            settings(serde_json::json!({"map": {"infra_services": ["flagd", "otel-collector"]}}))
                .map
                .infra_services,
            ["flagd", "otel-collector"]
        );
        assert!(
            settings(serde_json::json!({"map": {"infra_services": []}}))
                .map
                .infra_services
                .is_empty()
        );
    }
}
