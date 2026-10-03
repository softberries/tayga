//! Process plumbing shared by every Tayga binary.

use serde::de::DeserializeOwned;
use tracing_subscriber::EnvFilter;

pub mod metrics;
pub mod retry;

/// A flag that flips to `true` on SIGINT/SIGTERM. Must be called inside a tokio runtime;
/// clone the receiver into every loop that has to stop.
pub fn shutdown_flag() -> tokio::sync::watch::Receiver<bool> {
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = tx.send(true);
    });
    rx
}

/// Loads settings from the optional TOML file at `$TAYGA_CONFIG`, then
/// overrides from `TAYGA__SECTION__KEY` environment variables.
pub fn load_settings<T: DeserializeOwned>() -> anyhow::Result<T> {
    let mut builder = config::Config::builder();
    if let Ok(path) = std::env::var("TAYGA_CONFIG") {
        builder = builder.add_source(config::File::with_name(&path));
    }
    let settings = builder
        .add_source(
            config::Environment::with_prefix("TAYGA")
                .prefix_separator("__")
                .separator("__"),
        )
        .build()?
        .try_deserialize()?;
    Ok(settings)
}

/// JSON logs, filtered by `RUST_LOG` (default `info`).
pub fn init_logging() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

/// Resolves on SIGINT or SIGTERM.
pub async fn shutdown_signal() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Kafka {
        brokers: String,
    }

    #[derive(serde::Deserialize)]
    struct Settings {
        kafka: Kafka,
    }

    #[test]
    fn env_overrides_nested_keys() {
        // SAFETY: test-only, single-threaded access to this variable name.
        unsafe { std::env::set_var("TAYGA__KAFKA__BROKERS", "b:1") };
        let s: Settings = load_settings().unwrap();
        assert_eq!(s.kafka.brokers, "b:1");
    }
}
