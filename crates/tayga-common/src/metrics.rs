//! Prometheus text exposition for every Tayga binary.

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

/// Standalone metrics server until `stop` flips to true.
pub async fn serve(
    addr: SocketAddr,
    registry: Arc<Registry>,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(registry))
        .with_graceful_shutdown(async move {
            let _ = stop.wait_for(|s| *s).await;
        })
        .await?;
    Ok(())
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
}
