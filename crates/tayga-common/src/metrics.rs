//! Prometheus text exposition for every Tayga binary.

use anyhow::Context;
use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::routing::get;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::registry::Registry;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::watch;

pub const CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// Label for counters split by signal or story kind.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct KindLabel {
    pub kind: String,
}

impl KindLabel {
    pub fn new(kind: &str) -> Self {
        Self {
            kind: kind.to_string(),
        }
    }
}

pub fn render(registry: &Registry) -> String {
    let mut out = String::new();
    prometheus_client::encoding::text::encode(&mut out, registry)
        .expect("encoding into a String cannot fail");
    out
}

async fn metrics(
    State(registry): State<Arc<Registry>>,
) -> ([(header::HeaderName, &'static str); 1], String) {
    ([(header::CONTENT_TYPE, CONTENT_TYPE)], render(&registry))
}

/// `GET /metrics` router, to merge into an existing axum app.
pub fn router(registry: Arc<Registry>) -> Router {
    Router::new()
        .route("/metrics", get(metrics))
        .with_state(registry)
}

/// Binds `addr` first, so a port that is taken fails the caller's startup, then serves
/// `GET /metrics` in a task until `stop` flips to true. Returns the bound address (the OS picks
/// the port for port 0).
pub async fn spawn_server(
    addr: SocketAddr,
    registry: Arc<Registry>,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind metrics server on {addr}"))?;
    let bound = listener.local_addr()?;
    tokio::spawn(async move {
        let served = axum::serve(listener, router(registry))
            .with_graceful_shutdown(async move {
                let _ = stop.wait_for(|s| *s).await;
            })
            .await;
        if let Err(e) = served {
            tracing::warn!(error = %e, "metrics server stopped");
        }
    });
    Ok(bound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus_client::metrics::counter::Counter;
    use prometheus_client::metrics::family::Family;

    #[test]
    fn renders_counters_with_total_suffix_and_labels() {
        let mut registry = Registry::default();
        let stories: Family<KindLabel, Counter> = Family::default();
        registry.register("tayga_stories", "Stories produced", stories.clone());
        stories.get_or_create(&KindLabel::new("error")).inc();
        let text = render(&registry);
        assert!(
            text.contains("tayga_stories_total{kind=\"error\"} 1"),
            "{text}"
        );
        assert!(text.ends_with("# EOF\n"));
    }

    #[tokio::test]
    async fn a_taken_metrics_port_is_an_error_and_a_free_one_binds() {
        let taken = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = taken.local_addr().unwrap();
        let (_tx, stop) = watch::channel(false);
        let err = spawn_server(addr, Arc::new(Registry::default()), stop.clone())
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains(&format!("bind metrics server on {addr}")),
            "{err:#}"
        );
        let bound = spawn_server(
            "127.0.0.1:0".parse().unwrap(),
            Arc::new(Registry::default()),
            stop,
        )
        .await
        .unwrap();
        assert_ne!(bound.port(), 0);
        tokio::net::TcpStream::connect(bound).await.unwrap();
    }
}
