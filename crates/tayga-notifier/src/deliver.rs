//! Delivery of one alert to one target: dedup against `notifier_deliveries`, the HTTP attempt,
//! the retry classifier and backoff, and the state written after every attempt.
//!
//! Exactly-once is per `(alert_id, target)`: a `delivered` or `failed` row means the target is
//! resolved and is never sent again, whatever re-reads the record (a logminer re-publish, a
//! restart before the offset commit). A shutdown never cancels an attempt in flight; it is
//! finished and recorded first, so a restart cannot resend what was already delivered.
//!
//! Known resend windows (accepted; documented in the README):
//! - a hard kill (SIGKILL, OOM) between a 2xx and its row being written;
//! - a final row that could not be written before shutdown (ClickHouse down for the whole
//!   [`RECORD_GRACE`]): the record is still committed, but a later re-publish of the same alert
//!   finds no resolved row and is sent again;
//! - `notifier_deliveries` rows expire after 30 days (TTL), so an alert id re-published after
//!   that is delivered again.
//!
//! Records are handled one at a time, so a target that keeps failing would make every record
//! wait out its whole backoff ladder, and the backlog would pass `max_age_secs` for every
//! target. A per-target circuit breaker ([`BreakerState`]) bounds that: after a target gives up
//! on a retryable error it is open for `breaker_cooldown_secs`, and each new alert gets one
//! attempt to it and no ladder. Breaker state is in memory; a restart starts every target closed.

use crate::config::{REDACTED, Target, WebhookUrl};
use crate::metrics::{BREAKER, DELIVERED, DUPLICATE, FAILED, NotifierMetrics, RETRY};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};
use tayga_common::retry::retry_until;
use tayga_store::notifier::{DeliveryRow, STATUS_DELIVERED, STATUS_FAILED, STATUS_PENDING};
use tayga_store::store::Store;
use tokio::sync::watch;

pub const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// After shutdown, how long a delivery state may keep retrying its write.
const RECORD_GRACE: Duration = Duration::from_secs(10);
/// Bound of one state write. Worst case from SIGTERM to exit: an attempt in flight
/// (`timeout_secs` ≤ 15 s), a failed write (5 s), then writes until [`RECORD_GRACE`] ends, the
/// last one starting just before it (5 s): 35 s, inside the compose `stop_grace_period` of 40 s.
const PUT_TIMEOUT: Duration = Duration::from_secs(5);
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
/// retry; any other status is permanent. `Retry-After` is read by [`retry_after_delay`].
pub fn classify(status: Option<u16>, retry_after: Option<&str>) -> Outcome {
    match status {
        Some(200..=299) => Outcome::Delivered,
        None | Some(429 | 500..=599) => Outcome::Retry(
            retry_after.map_or(Duration::ZERO, |v| retry_after_delay(v, SystemTime::now())),
        ),
        Some(s) => Outcome::Permanent(format!("HTTP {s}")),
    }
}

/// A `Retry-After` value as a delay from `now`: delta-seconds or an HTTP date (IMF-fixdate; the
/// obsolete RFC 850 and asctime forms are accepted too), capped at [`MAX_BACKOFF`]. A past date
/// or an unreadable value is zero, which leaves the backoff in charge.
pub fn retry_after_delay(value: &str, now: SystemTime) -> Duration {
    let value = value.trim();
    let delay = match value.parse::<u64>() {
        Ok(secs) => Duration::from_secs(secs),
        Err(_) => httpdate::parse_http_date(value)
            .ok()
            .and_then(|at| at.duration_since(now).ok())
            .unwrap_or(Duration::ZERO),
    };
    delay.min(MAX_BACKOFF)
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

/// What a resolved delivery tells a target's breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerEvent {
    /// A 2xx: the target works again.
    Delivered,
    /// Gave up on a retryable error (429, 5xx, network, timeout): the ladder was exhausted, or
    /// the one attempt made while open failed.
    RetryableGiveUp,
    /// Gave up on a permanent error (a 4xx): the target answered, so it says nothing about the
    /// target's health; one bad payload must not cut the next alert's ladder short.
    PermanentGiveUp,
}

/// One target's circuit breaker, a pure state machine over an injected clock.
///
/// - closed: a delivery runs the whole backoff ladder (up to `max_attempts`);
/// - open (until `open_until`): a delivery gets exactly one attempt.
///
/// A retryable give-up opens it, or keeps it open, for `cooldown` from now; a delivery closes
/// it; a permanent give-up leaves it as it is. Once the cooldown has passed with no new
/// failure it reads as closed, and the next delivery runs the ladder again.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BreakerState {
    open_until: Option<Instant>,
}

impl BreakerState {
    pub fn is_open(&self, now: Instant) -> bool {
        self.open_until.is_some_and(|until| now < until)
    }

    pub fn on(&mut self, event: BreakerEvent, now: Instant, cooldown: Duration) {
        match event {
            BreakerEvent::Delivered => self.open_until = None,
            BreakerEvent::RetryableGiveUp => self.open_until = Some(now + cooldown),
            BreakerEvent::PermanentGiveUp => {}
        }
    }
}

/// Breaker state of every target, in memory: a restart starts every target closed.
pub struct Breakers {
    cooldown: Duration,
    states: Mutex<HashMap<String, BreakerState>>,
}

impl Breakers {
    pub fn new(cooldown: Duration) -> Self {
        Self {
            cooldown,
            states: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, BreakerState>> {
        // The state is a plain value; a panic elsewhere cannot leave it half-written.
        self.states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn is_open(&self, target: &str, now: Instant) -> bool {
        self.lock().get(target).is_some_and(|b| b.is_open(now))
    }

    /// Applies `event`; returns whether the breaker is open afterwards.
    pub fn on(&self, target: &str, event: BreakerEvent, now: Instant) -> bool {
        let mut states = self.lock();
        let b = states.entry(target.to_string()).or_default();
        b.on(event, now, self.cooldown);
        b.is_open(now)
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
    pub breakers: &'a Breakers,
}

impl<L: DeliveryLog> Deliverer<'_, L> {
    /// Delivers `body` for `alert_id` to `target` once: until delivered, a permanent error, or
    /// `max_attempts` (one attempt while the target's breaker is open), recording the state
    /// after every attempt.
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
        // Read once: a breaker that opens or closes meanwhile applies from the next alert.
        let open = self.breakers.is_open(name, Instant::now());
        self.metrics.set_breaker_open(name, open);
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
            if open {
                self.metrics.count(name, BREAKER);
            }
            match classify(resp.status, resp.retry_after.as_deref()) {
                Outcome::Delivered => {
                    self.record(&row(STATUS_DELIVERED, done, ""), &stop).await;
                    self.metrics.count(name, DELIVERED);
                    self.breaker(name, BreakerEvent::Delivered);
                    tracing::info!(alert_id, target = name, attempts = done, "alert delivered");
                    return Resolution::Delivered;
                }
                Outcome::Permanent(error) => {
                    self.breaker(name, BreakerEvent::PermanentGiveUp);
                    return self.give_up(row(STATUS_FAILED, done, &error), &stop).await;
                }
                Outcome::Retry(retry_after) => {
                    last_error = resp.error;
                    if open || done >= self.max_attempts {
                        self.breaker(name, BreakerEvent::RetryableGiveUp);
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

    /// Applies `event` to the target's breaker and mirrors it in `tayga_notifier_breaker_open`.
    fn breaker(&self, name: &str, event: BreakerEvent) {
        let was_open = self.breakers.is_open(name, Instant::now());
        let open = self.breakers.on(name, event, Instant::now());
        self.metrics.set_breaker_open(name, open);
        if open && !was_open {
            tracing::warn!(
                target = name,
                cooldown_s = self.breakers.cooldown.as_secs(),
                "breaker open: new alerts get one attempt to this target, without backoff"
            );
        } else if was_open && !open {
            tracing::info!(target = name, "breaker closed");
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
            let error = match tokio::time::timeout(PUT_TIMEOUT, self.log.put(row)).await {
                Ok(Ok(())) => return true,
                Ok(Err(e)) => redact_urls(&e.to_string()),
                Err(_) => format!("timed out after {} s", PUT_TIMEOUT.as_secs()),
            };
            tracing::warn!(
                alert_id = %row.alert_id,
                target = %row.target,
                %error,
                "delivery state write failed; retrying"
            );
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
            "a past HTTP date leaves the backoff in charge"
        );
        assert_eq!(
            classify(Some(429), Some("Fri, 31 Dec 9999 23:59:59 GMT")),
            Outcome::Retry(MAX_BACKOFF),
            "a far HTTP date is capped"
        );
        assert_eq!(
            classify(Some(429), Some("99999")),
            Outcome::Retry(MAX_BACKOFF)
        );
        assert_eq!(classify(None, None), Outcome::Retry(Duration::ZERO));
    }

    #[test]
    fn retry_after_reads_seconds_and_http_dates() {
        // Wed, 21 Oct 2015 07:28:00 GMT
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_445_412_480);
        let d = |v: &str| retry_after_delay(v, now);
        assert_eq!(d("7"), Duration::from_secs(7));
        assert_eq!(d("Wed, 21 Oct 2015 07:28:30 GMT"), Duration::from_secs(30));
        assert_eq!(d("Wed, 21 Oct 2015 08:28:00 GMT"), MAX_BACKOFF, "capped");
        assert_eq!(d("Wed, 21 Oct 2015 07:27:00 GMT"), Duration::ZERO, "past");
        assert_eq!(d("Wed, 21 Oct 2015 07:28:00 GMT"), Duration::ZERO, "now");
        assert_eq!(d("soon"), Duration::ZERO);
        assert_eq!(d("-5"), Duration::ZERO);
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
        breakers: Breakers,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_cooldown(Duration::from_secs(300))
        }

        fn with_cooldown(cooldown: Duration) -> Self {
            Self {
                log: MemLog::default(),
                sender: Sender::new(Duration::from_secs(5)).unwrap(),
                metrics: NotifierMetrics::default(),
                breakers: Breakers::new(cooldown),
            }
        }

        fn deliverer(&self, max_attempts: u32) -> Deliverer<'_, MemLog> {
            Deliverer {
                log: &self.log,
                sender: &self.sender,
                metrics: &self.metrics,
                max_attempts,
                breakers: &self.breakers,
            }
        }

        fn breaker_gauge(&self, target: &str) -> i64 {
            self.metrics
                .breaker_open
                .get_or_create(&crate::metrics::TargetLabel {
                    target: target.into(),
                })
                .get()
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
    async fn a_429_waits_its_retry_after_then_delivers() {
        let m = mock(&[429]).await;
        let f = Fixture::new();
        let target = hook("hook", &m.url());
        let started = Instant::now();
        let r = f
            .deliverer(8)
            .deliver("a10", &target, &serde_json::json!({}), running())
            .await;
        assert_eq!(r, Resolution::Delivered);
        assert!(
            started.elapsed() >= Duration::from_secs(1),
            "Retry-After: 1"
        );
        assert_eq!(
            m.received().iter().map(|r| r.status).collect::<Vec<_>>(),
            [429, 200]
        );
        assert_eq!(
            f.log
                .rows()
                .iter()
                .map(|r| (r.status, r.attempts, r.last_error.as_str()))
                .collect::<Vec<_>>(),
            [(STATUS_PENDING, 1, "HTTP 429"), (STATUS_DELIVERED, 2, "")]
        );
    }

    #[tokio::test]
    async fn a_stop_during_the_post_still_records_the_delivery() {
        let m = mock(&[]).await;
        m.set_delay(Duration::from_millis(500));
        let f = Fixture::new();
        let target = hook("hook", &m.url());
        let (tx, rx) = watch::channel(false);
        let flip = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            tx.send(true).unwrap();
            tx // kept alive: a dropped sender would read as stop anyway
        });
        let d = f.deliverer(8);
        let body = serde_json::json!({ "alert_id": "a11" });
        assert_eq!(
            d.deliver("a11", &target, &body, rx).await,
            Resolution::Delivered
        );
        let tx = flip.await.unwrap();
        assert!(*tx.borrow(), "stop flipped while the POST was in flight");
        assert_eq!(
            f.log.rows(),
            [DeliveryRow {
                alert_id: "a11".into(),
                target: "hook".into(),
                status: STATUS_DELIVERED,
                attempts: 1,
                last_error: String::new(),
            }]
        );
        // After the restart the record is re-read: nothing is sent again.
        assert_eq!(
            d.deliver("a11", &target, &body, running()).await,
            Resolution::AlreadyResolved
        );
        assert_eq!(m.received().len(), 1);
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

    #[test]
    fn breaker_state_machine() {
        use BreakerEvent::*;
        let cooldown = Duration::from_secs(300);
        let t0 = Instant::now();
        let at = |secs: u64| t0 + Duration::from_secs(secs);
        let mut b = BreakerState::default();
        assert!(!b.is_open(t0), "starts closed");
        b.on(PermanentGiveUp, t0, cooldown);
        assert!(!b.is_open(t0), "a 4xx does not open it");
        b.on(Delivered, t0, cooldown);
        assert!(!b.is_open(t0));

        b.on(RetryableGiveUp, at(10), cooldown);
        assert!(b.is_open(at(10)) && b.is_open(at(309)));
        assert!(!b.is_open(at(310)), "closed once the cooldown has passed");
        // A failed one-shot attempt while open keeps it open for another cooldown.
        b.on(RetryableGiveUp, at(200), cooldown);
        assert!(b.is_open(at(499)) && !b.is_open(at(500)));
        b.on(PermanentGiveUp, at(250), cooldown);
        assert!(b.is_open(at(499)), "a 4xx while open changes nothing");
        b.on(Delivered, at(260), cooldown);
        assert!(!b.is_open(at(260)), "a delivery closes it");

        let all = Breakers::new(cooldown);
        assert!(all.on("dead", RetryableGiveUp, t0));
        assert!(
            all.is_open("dead", at(1)) && !all.is_open("ok", at(1)),
            "per target"
        );
        assert!(!all.on("dead", Delivered, at(2)));
        assert!(!all.is_open("dead", at(2)));
    }

    /// Delivers one record to every target concurrently, as `main.rs` does.
    async fn deliver_record(
        d: &Deliverer<'_, MemLog>,
        alert_id: &str,
        targets: &[Target],
    ) -> Vec<Resolution> {
        let body = serde_json::json!({ "alert_id": alert_id });
        futures::future::join_all(
            targets
                .iter()
                .map(|t| d.deliver(alert_id, t, &body, running())),
        )
        .await
    }

    /// One target answers 503 forever: the first record runs its ladder and opens the breaker,
    /// every later record gets one attempt to it, so the healthy target keeps pace.
    #[tokio::test]
    async fn a_dead_target_does_not_hold_back_a_healthy_one() {
        const RECORDS: usize = 30;
        // Short timings through the config: max_attempts 3 makes a ladder 1 s + 2 s of backoff.
        let cfg = crate::config::NotifierSettings {
            max_attempts: 3,
            breaker_cooldown_secs: 300,
            ..crate::config::NotifierSettings::default()
        };
        let dead = mock(&[503; 200]).await;
        let ok = mock(&[]).await;
        let f = Fixture::with_cooldown(Duration::from_secs(cfg.breaker_cooldown_secs));
        let d = f.deliverer(cfg.max_attempts);
        let targets = [hook("dead", &dead.url()), hook("ok", &ok.url())];
        let ladder: Duration = (1..cfg.max_attempts)
            .map(|n| retry_wait(n, Duration::ZERO))
            .sum();
        assert_eq!(ladder, Duration::from_secs(3));

        let started = Instant::now();
        let mut slowest_after_first = Duration::ZERO;
        for i in 0..RECORDS {
            let record = Instant::now();
            let r = deliver_record(&d, &format!("r{i}"), &targets).await;
            assert_eq!(r, [Resolution::Failed, Resolution::Delivered], "record {i}");
            if i > 0 {
                slowest_after_first = slowest_after_first.max(record.elapsed());
            }
        }
        let took = started.elapsed();

        assert_eq!(
            ok.received().len(),
            RECORDS,
            "the healthy target got every record"
        );
        assert_eq!(
            dead.received().len(),
            cfg.max_attempts as usize + RECORDS - 1,
            "one ladder, then one attempt per record"
        );
        assert!(
            took < ladder * 2,
            "{took:?}: one ladder per record would take {:?}",
            ladder * RECORDS as u32
        );
        assert!(
            slowest_after_first < Duration::from_millis(500),
            "{slowest_after_first:?}"
        );

        // While open: the (alert, target) is failed with its one attempt recorded.
        let rows = f.log.rows();
        let last = rows.iter().rev().find(|r| r.target == "dead").unwrap();
        assert_eq!(
            (last.status, last.attempts, last.last_error.as_str()),
            (STATUS_FAILED, 1, "HTTP 503")
        );
        assert_eq!(f.count("dead", BREAKER), RECORDS as u64 - 1);
        assert_eq!(f.count("dead", FAILED), RECORDS as u64);
        assert_eq!(f.count("dead", RETRY), u64::from(cfg.max_attempts) - 1);
        assert_eq!(f.count("ok", DELIVERED), RECORDS as u64);
        assert_eq!(f.count("ok", BREAKER), 0);
        assert_eq!((f.breaker_gauge("dead"), f.breaker_gauge("ok")), (1, 0));
        assert_eq!(f.metrics.pending.get(), 0);
    }

    #[tokio::test]
    async fn a_delivery_while_open_closes_the_breaker() {
        // Ladder of 2 fails, the one attempt while open fails, then the target recovers.
        let m = mock(&[503, 503, 503]).await;
        let f = Fixture::new();
        let d = f.deliverer(2);
        let target = [hook("hook", &m.url())];
        assert_eq!(
            deliver_record(&d, "b1", &target).await,
            [Resolution::Failed]
        );
        assert_eq!(f.breaker_gauge("hook"), 1);
        assert_eq!(
            deliver_record(&d, "b2", &target).await,
            [Resolution::Failed]
        );
        assert_eq!(
            f.breaker_gauge("hook"),
            1,
            "a failure while open keeps it open"
        );
        assert_eq!(
            deliver_record(&d, "b3", &target).await,
            [Resolution::Delivered]
        );
        assert_eq!(f.breaker_gauge("hook"), 0, "a delivery closes it");
        assert_eq!(
            f.count("hook", BREAKER),
            2,
            "b2 and b3 were sent while open"
        );
        // Closed again: a 503 is retried on the ladder.
        let m2 = mock(&[503]).await;
        let again = [hook("hook", &m2.url())];
        assert_eq!(
            deliver_record(&d, "b4", &again).await,
            [Resolution::Delivered]
        );
        assert_eq!(m2.received().len(), 2);
        assert_eq!(f.count("hook", BREAKER), 2);
    }

    #[tokio::test]
    async fn a_permanent_error_does_not_open_the_breaker() {
        let m = mock(&[400, 503]).await;
        let f = Fixture::new();
        let d = f.deliverer(8);
        let target = [hook("hook", &m.url())];
        assert_eq!(
            deliver_record(&d, "p1", &target).await,
            [Resolution::Failed]
        );
        assert_eq!(f.breaker_gauge("hook"), 0);
        // The next alert still gets its ladder: the 503 is retried.
        assert_eq!(
            deliver_record(&d, "p2", &target).await,
            [Resolution::Delivered]
        );
        assert_eq!(m.received().len(), 3);
    }
}
