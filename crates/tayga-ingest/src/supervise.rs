//! Runs the gRPC and HTTP servers together: whichever ends first (or a shutdown signal) stops the other.

use std::future::Future;
use tokio::sync::watch;

/// Drives both server futures to completion. Each one, when it finishes (ok or error), sends `stop`;
/// `signal` resolving also sends `stop`. Returns the first server error, gRPC before HTTP.
pub async fn supervise<G, H, E1, E2>(
    grpc: G,
    http: H,
    signal: impl Future<Output = ()>,
    stop: watch::Sender<()>,
) -> anyhow::Result<()>
where
    G: Future<Output = Result<(), E1>>,
    H: Future<Output = Result<(), E2>>,
    E1: Into<anyhow::Error>,
    E2: Into<anyhow::Error>,
{
    let grpc = async {
        let r = grpc.await.map_err(Into::into);
        let _ = stop.send(());
        r
    };
    let http = async {
        let r = http.await.map_err(Into::into);
        let _ = stop.send(());
        r
    };
    let servers = async { tokio::join!(grpc, http) };
    tokio::pin!(servers, signal);
    let (g, h): (anyhow::Result<()>, anyhow::Result<()>) = tokio::select! {
        done = &mut servers => done,
        () = &mut signal => {
            let _ = stop.send(());
            servers.await
        }
    };
    g.and(h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn until_stopped(mut rx: watch::Receiver<()>) -> Result<(), std::io::Error> {
        let _ = rx.changed().await;
        Ok(())
    }

    #[tokio::test]
    async fn server_error_stops_the_other_and_returns_without_signal() {
        let (tx, rx) = watch::channel(());
        let grpc = async { Err::<(), _>(std::io::Error::other("bind")) };
        let out = tokio::time::timeout(
            Duration::from_secs(5),
            supervise(grpc, until_stopped(rx), std::future::pending(), tx),
        )
        .await
        .expect("must not hang");
        assert_eq!(out.unwrap_err().to_string(), "bind");
    }

    #[tokio::test]
    async fn signal_stops_both_servers() {
        let (tx, rx) = watch::channel(());
        let out = tokio::time::timeout(
            Duration::from_secs(5),
            supervise(until_stopped(rx.clone()), until_stopped(rx), async {}, tx),
        )
        .await
        .expect("must not hang");
        assert!(out.is_ok());
    }
}
