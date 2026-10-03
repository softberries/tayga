//! Retry with exponential backoff that gives up as soon as shutdown is signalled.

use std::fmt::Display;
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;

/// Runs `op` until it succeeds. Returns `None` if shutdown fires first
/// (checked before each attempt, while an attempt runs, and during backoff).
pub async fn retry_until<T, E, F, Fut>(
    what: &str,
    mut op: F,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<T>
where
    E: Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut backoff = Duration::from_millis(100);
    loop {
        if *shutdown.borrow() {
            return None;
        }
        let result = tokio::select! {
            r = op() => r,
            _ = shutdown.wait_for(|stop| *stop) => return None,
        };
        match result {
            Ok(v) => return Some(v),
            Err(e) => {
                tracing::warn!(error = %e, what, retry_in_ms = backoff.as_millis() as u64, "operation failed; retrying");
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown.wait_for(|stop| *stop) => return None,
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn returns_none_promptly_when_shutdown_already_set() {
        let (_tx, mut rx) = watch::channel(true);
        let start = Instant::now();
        let out: Option<()> = retry_until("test", || async { Err::<(), _>("down") }, &mut rx).await;
        assert!(out.is_none());
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn interrupts_backoff_when_shutdown_fires() {
        let (tx, mut rx) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            tx.send(true).unwrap();
        });
        let start = Instant::now();
        let out: Option<()> = retry_until("test", || async { Err::<(), _>("down") }, &mut rx).await;
        assert!(out.is_none());
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn returns_value_after_transient_failure() {
        let (_tx, mut rx) = watch::channel(false);
        let mut n = 0;
        let out = retry_until(
            "test",
            || {
                n += 1;
                let ok = n >= 2;
                async move { if ok { Ok(7) } else { Err("x") } }
            },
            &mut rx,
        )
        .await;
        assert_eq!(out, Some(7));
    }
}
