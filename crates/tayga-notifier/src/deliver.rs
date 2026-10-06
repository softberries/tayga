//! Delivery of one alert to one target: dedup against `notifier_deliveries`, the HTTP attempt,
//! the retry classifier and backoff, and the state written after every attempt.
//!
//! Exactly-once is per `(alert_id, target)`: a `delivered` or `failed` row means the target is
//! resolved and is never sent again, whatever re-reads the record (a logminer re-publish, a
//! restart before the offset commit). A shutdown never cancels an attempt in flight; it is
//! finished and recorded first, so a restart cannot resend what was already delivered. The one
//! window left is a hard kill between a 2xx and its row being written.

use crate::config::{REDACTED, Target, WebhookUrl};
use crate::metrics::{DELIVERED, DUPLICATE, FAILED, NotifierMetrics, RETRY};
use serde_json::Value;
use std::future::Future;
use std::time::{Duration, Instant};
use tayga_common::retry::retry_until;
use tayga_store::notifier::{DeliveryRow, STATUS_DELIVERED, STATUS_FAILED, STATUS_PENDING};
use tayga_store::store::Store;
use tokio::sync::watch;

pub const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// After shutdown, how long a final delivery state may keep retrying its write.
const RECORD_GRACE: Duration = Duration::from_secs(15);
/// `last_error` is cut to this many characters.
const ERROR_MAX: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Delivered,
    /// Retry after at least this long (`Retry-After`; zero when absent).
    Retry(Duration),
    /// Give up; the reason is recorded.
    Permanent(String),
}

/// 2xx is delivered; 429, 5xx and no response at all (`None`: a network error or timeout)
/// retry; any other status is permanent. `Retry-After` is read as delta-seconds.
pub fn classify(status: Option<u16>, retry_after: Option<&str>) -> Outcome {
    match status {
        Some(200..=299) => Outcome::Delivered,
        None | Some(429 | 500..=599) => Outcome::Retry(
            retry_after
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map_or(Duration::ZERO, |s| Duration::from_secs(s).min(MAX_BACKOFF)),
        ),
        Some(s) => Outcome::Permanent(format!("HTTP {s}")),
    }
}

/// 1 s × 2^`attempt`, capped at [`MAX_BACKOFF`].
pub fn backoff(attempt: u32) -> Duration {
    let secs = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    Duration::from_secs(secs).min(MAX_BACKOFF)
}

/// The wait after the `attempts`-th failed attempt: the backoff, or `Retry-After` when longer,
/// capped at [`MAX_BACKOFF`].
pub fn retry_wait(attempts: u32, retry_after: Duration) -> Duration {
    backoff(attempts.saturating_sub(1))
        .max(retry_after)
        .min(MAX_BACKOFF)
}

/// Replaces every `http://…` / `https://…` run (up to whitespace, a quote, `<`, `>` or `)`)
/// with [`REDACTED`]. Applied to every error text before it is logged or stored.
pub fn redact_urls(text: &str) -> String {
    // ASCII lowercasing keeps byte offsets, so indices into `lower` are valid in `text`.
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(start) = ["http://", "https://"]
        .iter()
        .filter_map(|scheme| lower[at..].find(scheme))
        .min()
        .map(|i| at + i)
    {
        out.push_str(&text[at..start]);
        out.push_str(REDACTED);
        at = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')'))
            .map_or(text.len(), |end| start + end);
    }
    out.push_str(&text[at..]);
    out
}

/// A transport error as stored text: the error and its sources, without the URL, redacted, and
/// at most [`ERROR_MAX`] characters.
fn describe(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut text = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(s) = source {
        text.push_str(": ");
        text.push_str(&s.to_string());
        source = s.source();
    }
    redact_urls(&text).chars().take(ERROR_MAX).collect()
}

/// What to do with a target given its recorded state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Delivered or given up earlier: send nothing.
    Skip,
    /// Send, `done` attempts having been made already (a restart resumes the count).
    Attempt { done: u32 },
}

pub fn next_step(prior: Option<&DeliveryRow>) -> Next {
    match prior {
        None => Next::Attempt { done: 0 },
        Some(r) if r.status == STATUS_DELIVERED || r.status == STATUS_FAILED => Next::Skip,
        Some(r) => Next::Attempt { done: r.attempts },
    }
}

/// The result of one HTTP attempt. `error` is empty on 2xx, else `HTTP <status>` or the
/// redacted transport error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: Option<u16>,
    pub retry_after: Option<String>,
    pub error: String,
}

pub struct Sender {
    client: reqwest::Client,
}

impl Sender {
    /// No redirects: a webhook that answers 3xx is misconfigured (permanent).
    pub fn new(timeout: Duration) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { client })
    }

    /// `POST` `body` as JSON. Never fails: transport errors come back with `status: None`.
    pub async fn post(&self, url: &WebhookUrl, body: &Value) -> Response {
        match self.client.post(url.expose()).json(body).send().await {
            Ok(r) => {
                let status = r.status().as_u16();
                Response {
                    status: Some(status),
                    retry_after: r
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string),
                    error: if r.status().is_success() {
                        String::new()
                    } else {
                        format!("HTTP {status}")
                    },
                }
            }
            Err(e) => Response {
                status: None,
                retry_after: None,
                error: describe(e),
            },
        }
    }
}

/// Where delivery states are kept: `notifier_deliveries` in production.
pub trait DeliveryLog: Sync {
    fn get(
        &self,
        alert_id: &str,
        target: &str,
    ) -> impl Future<Output = anyhow::Result<Option<DeliveryRow>>> + Send;
    fn put(&self, row: &DeliveryRow) -> impl Future<Output = anyhow::Result<()>> + Send;
}

impl DeliveryLog for Store {
    async fn get(&self, alert_id: &str, target: &str) -> anyhow::Result<Option<DeliveryRow>> {
        Ok(self.delivery_get(alert_id, target).await?)
    }

    async fn put(&self, row: &DeliveryRow) -> anyhow::Result<()> {
        Ok(self.delivery_put(row).await?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Delivered,
    Failed,
    /// Resolved by an earlier delivery: nothing was sent.
    AlreadyResolved,
    /// Shutdown before the target was resolved: the record must not be committed.
    Interrupted,
}

pub struct Deliverer<'a, L> {
    pub log: &'a L,
    pub sender: &'a Sender,
    pub metrics: &'a NotifierMetrics,
    pub max_attempts: u32,
}

impl<L: DeliveryLog> Deliverer<'_, L> {
    /// Delivers `body` for `alert_id` to `target` once: until delivered, a permanent error, or
    /// `max_attempts`, recording the state after every attempt.
    pub async fn deliver(
        &self,
        alert_id: &str,
        target: &Target,
        body: &Value,
        mut stop: watch::Receiver<bool>,
    ) -> Resolution {
        let name = target.name.as_str();
        let Some(prior) = retry_until(
            "read delivery state",
            || self.log.get(alert_id, name),
            &mut stop,
        )
        .await
        else {
            return Resolution::Interrupted;
        };
        let mut done = match next_step(prior.as_ref()) {
            Next::Skip => {
                self.metrics.count(name, DUPLICATE);
                return Resolution::AlreadyResolved;
            }
            Next::Attempt { done } => done,
        };
        let _pending = PendingGuard::new(self.metrics);
        let mut last_error = prior.map(|p| p.last_error).unwrap_or_default();
        let row = |status, attempts, last_error: &str| DeliveryRow {
            alert_id: alert_id.to_string(),
            target: name.to_string(),
            status,
            attempts,
            last_error: last_error.to_string(),
        };
        loop {
            if done >= self.max_attempts {
                // Only after a restart that found a pending row at the limit.
                return self
                    .give_up(row(STATUS_FAILED, done, &last_error), &stop)
                    .await;
            }
            // Not raced against `stop`: an attempt in flight is finished and recorded.
            let started = Instant::now();
            let resp = self.sender.post(&target.url, body).await;
            self.metrics
                .delivery_seconds
                .observe(started.elapsed().as_secs_f64());
            done += 1;
            match classify(resp.status, resp.retry_after.as_deref()) {
                Outcome::Delivered => {
                    self.record(&row(STATUS_DELIVERED, done, ""), &stop).await;
                    self.metrics.count(name, DELIVERED);
                    tracing::info!(alert_id, target = name, attempts = done, "alert delivered");
                    return Resolution::Delivered;
                }
                Outcome::Permanent(error) => {
                    return self.give_up(row(STATUS_FAILED, done, &error), &stop).await;
                }
                Outcome::Retry(retry_after) => {
                    last_error = resp.error;
                    if done >= self.max_attempts {
                        return self
                            .give_up(row(STATUS_FAILED, done, &last_error), &stop)
                            .await;
                    }
                    if !self
                        .record(&row(STATUS_PENDING, done, &last_error), &stop)
                        .await
                    {
                        return Resolution::Interrupted;
                    }
                    self.metrics.count(name, RETRY);
                    let wait = retry_wait(done, retry_after);
                    tracing::warn!(
                        alert_id,
                        target = name,
                        attempts = done,
                        error = %last_error,
                        retry_in_s = wait.as_secs(),
                        "delivery failed; retrying"
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = stop.wait_for(|s| *s) => return Resolution::Interrupted,
                    }
                }
            }
        }
    }

    async fn give_up(&self, row: DeliveryRow, stop: &watch::Receiver<bool>) -> Resolution {
        self.record(&row, stop).await;
        self.metrics.count(&row.target, FAILED);
        tracing::warn!(
            alert_id = %row.alert_id,
            target = %row.target,
            attempts = row.attempts,
            error = %row.last_error,
            "delivery failed; giving up"
        );
        Resolution::Failed
    }

    /// Writes a delivery state, retrying until it is stored. Shutdown does not stop it at once:
    /// a state not written means a resend after restart, so it keeps trying for
    /// [`RECORD_GRACE`] after shutdown. Returns whether the row was stored.
    async fn record(&self, row: &DeliveryRow, stop: &watch::Receiver<bool>) -> bool {
        let mut wait = Duration::from_millis(100);
        let mut deadline: Option<Instant> = None;
        loop {
            match self.log.put(row).await {
                Ok(()) => return true,
                Err(e) => tracing::warn!(
                    alert_id = %row.alert_id,
                    target = %row.target,
                    error = %redact_urls(&e.to_string()),
                    "delivery state write failed; retrying"
                ),
            }
            if *stop.borrow() {
                let deadline = *deadline.get_or_insert_with(|| Instant::now() + RECORD_GRACE);
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    tracing::error!(
                        alert_id = %row.alert_id,
                        target = %row.target,
                        status = row.status,
                        "delivery state not stored before shutdown"
                    );
                    return false;
                }
                wait = wait.min(left);
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(30));
        }
    }
}

/// `tayga_notifier_pending` for one delivery, decremented however it ends.
struct PendingGuard<'a>(&'a NotifierMetrics);

impl<'a> PendingGuard<'a> {
    fn new(metrics: &'a NotifierMetrics) -> Self {
        metrics.pending.inc();
        Self(metrics)
    }
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.0.pending.dec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TargetKind;
    use std::net::SocketAddr;
    use std::sync::Mutex;
    use tayga_e2e::MockWebhook;

    const SECRET_URL: &str = "http://127.0.0.1:9/services/T000/B000/XXXXSECRETXXXX";

    #[test]
    fn classifies_statuses() {
        assert_eq!(classify(Some(200), None), Outcome::Delivered);
        assert_eq!(classify(Some(204), None), Outcome::Delivered);
        assert_eq!(
            classify(Some(400), None),
            Outcome::Permanent("HTTP 400".into())
        );
        assert_eq!(
            classify(Some(404), None),
            Outcome::Permanent("HTTP 404".into())
        );
        assert_eq!(
            classify(Some(302), None),
            Outcome::Permanent("HTTP 302".into())
        );
        assert_eq!(
            classify(Some(429), Some("7")),
            Outcome::Retry(Duration::from_secs(7))
        );
        assert_eq!(classify(Some(429), None), Outcome::Retry(Duration::ZERO));
        assert_eq!(classify(Some(500), None), Outcome::Retry(Duration::ZERO));
        assert_eq!(
            classify(Some(503), Some(" 30 ")),
            Outcome::Retry(Duration::from_secs(30))
        );
        assert_eq!(
            classify(Some(503), Some("Wed, 21 Oct 2015 07:28:00 GMT")),
            Outcome::Retry(Duration::ZERO),
            "an HTTP date falls back to the backoff"
        );
        assert_eq!(
            classify(Some(429), Some("99999")),
            Outcome::Retry(MAX_BACKOFF)
        );
        assert_eq!(classify(None, None), Outcome::Retry(Duration::ZERO));
    }

    #[test]
    fn backoff_doubles_from_one_second_and_caps_at_five_minutes() {
        let secs: Vec<u64> = (0..10).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 32, 64, 128, 256, 300]);
        assert_eq!(backoff(64), MAX_BACKOFF);
        assert_eq!(backoff(u32::MAX), MAX_BACKOFF);
        assert_eq!(retry_wait(1, Duration::ZERO), Duration::from_secs(1));
        assert_eq!(retry_wait(3, Duration::ZERO), Duration::from_secs(4));
        assert_eq!(
            retry_wait(1, Duration::from_secs(30)),
            Duration::from_secs(30)
        );
        assert_eq!(retry_wait(20, Duration::ZERO), MAX_BACKOFF);
    }

    #[test]
    fn redacts_every_url() {
        assert_eq!(
            redact_urls("error sending request for url (https://hooks.slack.com/services/A/B/C)"),
            "error sending request for url (<redacted>)"
        );
        assert_eq!(
            redact_urls("HTTP://x/y and http://a/b?c=d\tend 'https://q'"),
            "<redacted> and <redacted>\tend '<redacted>'"
        );
        assert_eq!(redact_urls("HTTP 503"), "HTTP 503");
    }

    fn row(status: i8, attempts: u32) -> DeliveryRow {
        DeliveryRow {
            alert_id: "a".into(),
            target: "t".into(),
            status,
            attempts,
            last_error: String::new(),
        }
    }

    #[test]
    fn dedup_decision() {
        assert_eq!(next_step(None), Next::Attempt { done: 0 });
        assert_eq!(
            next_step(Some(&row(STATUS_PENDING, 3))),
            Next::Attempt { done: 3 }
        );
        assert_eq!(next_step(Some(&row(STATUS_DELIVERED, 1))), Next::Skip);
        assert_eq!(next_step(Some(&row(STATUS_FAILED, 8))), Next::Skip);
    }

    /// Every row written, in order; `get` returns the latest per key, as `FINAL` does.
    #[derive(Default)]
    struct MemLog(Mutex<Vec<DeliveryRow>>);

    impl MemLog {
        fn rows(&self) -> Vec<DeliveryRow> {
            self.0.lock().unwrap().clone()
        }
    }

    impl DeliveryLog for MemLog {
        async fn get(&self, alert_id: &str, target: &str) -> anyhow::Result<Option<DeliveryRow>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|r| r.alert_id == alert_id && r.target == target)
                .cloned())
        }

        async fn put(&self, row: &DeliveryRow) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(row.clone());
            Ok(())
        }
    }

    fn hook(name: &str, url: &str) -> Target {
        Target {
            name: name.into(),
            kind: TargetKind::Webhook,
            url: WebhookUrl::new(url),
        }
    }

    async fn mock(script: &[u16]) -> MockWebhook {
        MockWebhook::start(SocketAddr::from(([127, 0, 0, 1], 0)), script)
            .await
            .unwrap()
    }

    struct Fixture {
        log: MemLog,
        sender: Sender,
        metrics: NotifierMetrics,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                log: MemLog::default(),
                sender: Sender::new(Duration::from_secs(5)).unwrap(),
                metrics: NotifierMetrics::default(),
            }
        }

        fn deliverer(&self, max_attempts: u32) -> Deliverer<'_, MemLog> {
            Deliverer {
                log: &self.log,
                sender: &self.sender,
                metrics: &self.metrics,
                max_attempts,
            }
        }

        fn count(&self, target: &str, result: &str) -> u64 {
            self.metrics
                .deliveries
                .get_or_create(&crate::metrics::DeliveryLabels {
                    target: target.into(),
                    result: result.into(),
                })
                .get()
        }
    }

    /// A stop flag that never flips (its sender stays alive: a closed channel reads as stop).
    fn running() -> watch::Receiver<bool> {
        static TX: std::sync::OnceLock<watch::Sender<bool>> = std::sync::OnceLock::new();
        TX.get_or_init(|| watch::channel(false).0).subscribe()
    }

    #[tokio::test]
    async fn posts_json_once_and_a_republished_alert_sends_nothing() {
        let m = mock(&[]).await;
        let f = Fixture::new();
        let target = hook("hook", &m.url());
        let body = serde_json::json!({ "alert_id": "a1" });
        let d = f.deliverer(8);
        assert_eq!(
            d.deliver("a1", &target, &body, running()).await,
            Resolution::Delivered
        );
        // The logminer re-publishes the same alert id on every update.
        for _ in 0..3 {
            assert_eq!(
                d.deliver("a1", &target, &body, running()).await,
                Resolution::AlreadyResolved
            );
        }
        let got = m.received();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].content_type.as_deref(), Some("application/json"));
        assert_eq!(got[0].body, body);
        assert_eq!(
            f.log.rows(),
            [DeliveryRow {
                alert_id: "a1".into(),
                target: "hook".into(),
                status: STATUS_DELIVERED,
                attempts: 1,
                last_error: String::new(),
            }]
        );
        assert_eq!(
            (f.count("hook", DELIVERED), f.count("hook", DUPLICATE)),
            (1, 3)
        );
        assert_eq!(f.metrics.pending.get(), 0);
    }

    #[tokio::test]
    async fn retries_a_5xx_and_records_every_attempt() {
        let m = mock(&[503]).await;
        let f = Fixture::new();
        let target = hook("hook", &m.url());
        let started = Instant::now();
        let r = f
            .deliverer(8)
            .deliver("a2", &target, &serde_json::json!({}), running())
            .await;
        assert_eq!(r, Resolution::Delivered);
        assert!(
            started.elapsed() >= Duration::from_secs(1),
            "waited the backoff"
        );
        assert_eq!(m.received().len(), 2);
        let rows = f.log.rows();
        assert_eq!(
            rows.iter()
                .map(|r| (r.status, r.attempts, r.last_error.as_str()))
                .collect::<Vec<_>>(),
            [(STATUS_PENDING, 1, "HTTP 503"), (STATUS_DELIVERED, 2, "")]
        );
        assert_eq!((f.count("hook", RETRY), f.count("hook", DELIVERED)), (1, 1));
    }

    #[tokio::test]
    async fn a_4xx_gives_up_at_once_and_is_never_retried() {
        let m = mock(&[404]).await;
        let f = Fixture::new();
        let target = hook("hook", &m.url());
        let d = f.deliverer(8);
        let body = serde_json::json!({});
        assert_eq!(
            d.deliver("a3", &target, &body, running()).await,
            Resolution::Failed
        );
        assert_eq!(
            d.deliver("a3", &target, &body, running()).await,
            Resolution::AlreadyResolved
        );
        assert_eq!(m.received().len(), 1);
        let last = f.log.rows().pop().unwrap();
        assert_eq!((last.status, last.attempts), (STATUS_FAILED, 1));
        assert_eq!(last.last_error, "HTTP 404");
        assert_eq!(f.count("hook", FAILED), 1);
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() {
        let m = mock(&[500, 500, 500]).await;
        let f = Fixture::new();
        let target = hook("hook", &m.url());
        let r = f
            .deliverer(2)
            .deliver("a4", &target, &serde_json::json!({}), running())
            .await;
        assert_eq!(r, Resolution::Failed);
        assert_eq!(m.received().len(), 2);
        let last = f.log.rows().pop().unwrap();
        assert_eq!(
            (last.status, last.attempts, last.last_error.as_str()),
            (STATUS_FAILED, 2, "HTTP 500")
        );
    }

    #[tokio::test]
    async fn a_restart_resumes_the_attempt_count() {
        let m = mock(&[]).await;
        let f = Fixture::new();
        f.log
            .put(&DeliveryRow {
                alert_id: "a5".into(),
                target: "hook".into(),
                status: STATUS_PENDING,
                attempts: 3,
                last_error: "HTTP 503".into(),
            })
            .await
            .unwrap();
        let target = hook("hook", &m.url());
        let r = f
            .deliverer(8)
            .deliver("a5", &target, &serde_json::json!({}), running())
            .await;
        assert_eq!(r, Resolution::Delivered);
        assert_eq!(f.log.rows().pop().unwrap().attempts, 4);
        // A pending row already at the limit (max_attempts lowered) gives up without sending.
        let r = f
            .deliverer(2)
            .deliver("a6", &target, &serde_json::json!({}), running())
            .await;
        assert_eq!(r, Resolution::Delivered, "a fresh alert is still sent");
        f.log
            .put(&DeliveryRow {
                alert_id: "a7".into(),
                target: "hook".into(),
                status: STATUS_PENDING,
                attempts: 2,
                last_error: "HTTP 503".into(),
            })
            .await
            .unwrap();
        let r = f
            .deliverer(2)
            .deliver("a7", &target, &serde_json::json!({}), running())
            .await;
        assert_eq!(r, Resolution::Failed);
        assert_eq!(m.received().len(), 2);
        let last = f.log.rows().pop().unwrap();
        assert_eq!((last.status, last.attempts), (STATUS_FAILED, 2));
    }

    #[tokio::test]
    async fn shutdown_during_backoff_leaves_the_target_unresolved() {
        let m = mock(&[503]).await;
        let f = Fixture::new();
        let target = hook("hook", &m.url());
        let (tx, rx) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            tx.send(true).unwrap();
        });
        let started = Instant::now();
        let r = f
            .deliverer(8)
            .deliver("a8", &target, &serde_json::json!({}), rx)
            .await;
        assert_eq!(r, Resolution::Interrupted);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(m.received().len(), 1);
        let last = f.log.rows().pop().unwrap();
        assert_eq!((last.status, last.attempts), (STATUS_PENDING, 1));
        assert_eq!(f.metrics.pending.get(), 0);
    }

    #[tokio::test]
    async fn transport_errors_never_carry_the_url() {
        // Port 9 (discard) on loopback: nothing listens, so the connect is refused.
        let f = Fixture::new();
        let target = hook("ops", SECRET_URL);
        let resp = f.sender.post(&target.url, &serde_json::json!({})).await;
        assert_eq!(resp.status, None);
        assert!(!resp.error.is_empty());
        let r = f
            .deliverer(1)
            .deliver("a9", &target, &serde_json::json!({}), running())
            .await;
        assert_eq!(r, Resolution::Failed);
        let last = f.log.rows().pop().unwrap();
        for text in [
            resp.error.as_str(),
            last.last_error.as_str(),
            &format!("{target:?}"),
            &format!("{target}"),
        ] {
            assert!(!text.contains("SECRET"), "{text}");
            assert!(!text.contains("/services/"), "{text}");
        }
        assert!(last.last_error.len() <= ERROR_MAX);
    }
}
