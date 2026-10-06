//! Black-box helpers for end-to-end tests against the running demo stack.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const API: &str = "http://localhost:8090";
pub const SCENARIO_TIMEOUT: Duration = Duration::from_secs(180);
/// `intlShippingSlowdown` only delays orders shipped outside the US, a small share of the
/// load generator's ~3 orders/min (15 of 286 orders in earlier flag-on periods), so the first
/// slow story can take several minutes. One clean-baseline run saw 10 orders, none
/// international, in 180 s.
pub const SHIPPING_TIMEOUT: Duration = Duration::from_secs(600);
/// Log detection runs every 60 s and the spike rule needs 10 hits in 5 min.
pub const LOG_SPIKE_TIMEOUT: Duration = Duration::from_secs(600);
pub const NEW_TEMPLATE_TIMEOUT: Duration = Duration::from_secs(180);
/// The logminer's per-service warmup (`new_template_warmup_min`): a service's templates are
/// reported as new only once it has had a template for this long.
pub const PROBE_WARMUP: Duration = Duration::from_secs(15 * 60);
/// First run only: how long to wait for the probe service's seed template to age past the warmup.
pub const PROBE_WARMUP_TIMEOUT: Duration = Duration::from_secs(17 * 60);
/// How long a fresh probe body may take to show up as a template (ingest, Kafka, logminer flush).
pub const TEMPLATE_TIMEOUT: Duration = Duration::from_secs(120);
/// The silence threshold the silence scenario sets on its probe template.
pub const SILENCE_MINUTES: u32 = 2;
/// The silence scenario keeps the probe service logging at this pace, so its log time moves on.
pub const SILENCE_KEEPALIVE: Duration = Duration::from_secs(20);
/// `SILENCE_MINUTES` of quiet, plus up to a 60 s detection pass, plus slack for the
/// per-partition clock that holds detection back to the slowest assigned partition.
pub const SILENCE_TIMEOUT: Duration = Duration::from_secs(6 * 60);
/// Set by `make e2e-notifier`: the silence scenario then also checks the notifier's delivery.
pub const NOTIFIER_CHECK_ENV: &str = "TAYGA_E2E_NOTIFIER";
/// Host port of the notifier check's mock webhook; `deploy/tayga-notifier.e2e.toml` points the
/// `e2e-mock` target at `http://host.docker.internal:18099/hook`.
pub const NOTIFIER_MOCK_PORT: u16 = 18099;
/// Target name of the mock in `deploy/tayga-notifier.e2e.toml`.
pub const NOTIFIER_MOCK_TARGET: &str = "e2e-mock";
/// How long the notifier may take to deliver an alert to the mock once it exists.
pub const DELIVERY_TIMEOUT: Duration = Duration::from_secs(120);
/// After `docker restart tayga-notifier`, how long to watch for a resend. The logminer
/// re-publishes a silence alert on every 60 s pass, so this covers at least two re-publishes.
pub const RESEND_WATCH: Duration = Duration::from_secs(150);
pub const POLL_EVERY: Duration = Duration::from_secs(5);
/// Spec §15 target for flag-to-story latency; reported, not asserted.
pub const TARGET_LATENCY: Duration = Duration::from_secs(60);

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn now_ns() -> i64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the Unix epoch");
    i64::try_from(d.as_nanos()).expect("nanoseconds since epoch fit in i64")
}

/// Sets a demo flag; restores the upstream flag file when dropped (also on panic).
pub struct FlagGuard {
    live: PathBuf,
    upstream: PathBuf,
}

impl FlagGuard {
    pub fn set(name: &str, variant: &str) -> anyhow::Result<Self> {
        let root = repo_root();
        // Built before any mutation so Drop restores the flags even if a step below fails.
        let guard = Self {
            live: root.join("deploy/flagd/demo.flagd.json"),
            upstream: root.join("vendor/opentelemetry-demo/src/flagd/demo.flagd.json"),
        };
        std::fs::copy(&guard.upstream, &guard.live)?;
        tayga_devtools::flags::set_flag(&guard.live, name, variant)?;
        Ok(guard)
    }
}

impl Drop for FlagGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::copy(&self.upstream, &self.live) {
            eprintln!("failed to reset flags: {e}");
        }
    }
}

pub struct Api {
    base: String,
    http: reqwest::Client,
}

impl Api {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("client builds");
        Self {
            base: base.trim_end_matches('/').to_string(),
            http,
        }
    }

    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        let res = self
            .http
            .get(format!("{}{path}", self.base))
            .send()
            .await?
            .error_for_status()?;
        Ok(res.json().await?)
    }

    pub async fn groups(&self, query: &str) -> anyhow::Result<Vec<Value>> {
        Ok(self
            .get(&format!("/api/v1/story-groups?{query}"))
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// One group with its latest example stories (`{"group": {...}, "examples": [...]}`).
    pub async fn group_detail(&self, fingerprint: &str, since: &str) -> anyhow::Result<Value> {
        self.get(&format!("/api/v1/story-groups/{fingerprint}?since={since}"))
            .await
    }

    pub async fn log_alerts(&self, query: &str) -> anyhow::Result<Vec<Value>> {
        Ok(self
            .get(&format!("/api/v1/log-alerts?{query}"))
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    pub async fn log_templates(&self, query: &str) -> anyhow::Result<Vec<Value>> {
        Ok(self
            .get(&format!("/api/v1/log-templates?{query}"))
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// `PUT /log-templates/{id}/silence`; returns the stored setting.
    pub async fn put_silence(
        &self,
        template_id: &str,
        enabled: bool,
        minutes: u32,
    ) -> anyhow::Result<Value> {
        let res = self
            .http
            .put(format!(
                "{}/api/v1/log-templates/{template_id}/silence",
                self.base
            ))
            .json(&serde_json::json!({ "enabled": enabled, "minutes": minutes }))
            .send()
            .await?
            .error_for_status()?;
        Ok(res.json().await?)
    }

    pub async fn story(&self, id: &str) -> anyhow::Result<Value> {
        self.get(&format!("/api/v1/stories/{id}")).await
    }

    /// The map's edges (`{"edges": [...], "nodes": [...]}`).
    pub async fn service_map(&self, since: &str) -> anyhow::Result<Vec<Value>> {
        Ok(self
            .get(&format!("/api/v1/service-map?since={since}"))
            .await?["edges"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }
}

fn story_count(g: &Value) -> u64 {
    g["stories"].as_u64().unwrap_or(0)
}

/// `since` covering only the time after `after_ns` (whole seconds, rounded up, at least 1 s), so
/// stories from before the flip, such as an earlier run of the same scenario, are not counted.
pub fn since_flip(after_ns: i64) -> String {
    let elapsed_ns = now_ns().saturating_sub(after_ns).max(0) as u64;
    format!("{}s", elapsed_ns.div_ceil(1_000_000_000).max(1))
}

/// Polls story groups matching `filter` (query string without `since`) until one seen after
/// `after_ns` satisfies `pred` and has at least `min_new` stories since the flip. Returns it and the wait.
pub async fn wait_for_group(
    api: &Api,
    filter: &str,
    after_ns: i64,
    min_new: u64,
    timeout: Duration,
    pred: impl Fn(&Value) -> bool,
) -> anyhow::Result<(Value, Duration)> {
    let start = Instant::now();
    let mut last_seen: Vec<String> = Vec::new();
    let mut last_err: Option<String> = None;
    while start.elapsed() < timeout {
        let query = format!("since={}&{filter}", since_flip(after_ns));
        match api.groups(&query).await {
            Ok(groups) => {
                last_seen = describe_groups(&groups);
                let found = groups.into_iter().find(|g| {
                    g["last_seen_ns"].as_i64().unwrap_or(0) > after_ns
                        && story_count(g) >= min_new
                        && pred(g)
                });
                if let Some(g) = found {
                    return Ok((g, start.elapsed()));
                }
            }
            Err(e) => {
                eprintln!("[e2e] poll error (continuing): {e}");
                last_err = Some(e.to_string());
            }
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
    anyhow::bail!(
        "no matching story group within {timeout:?} (last poll error: {last_err:?}); last groups seen:\n{}",
        last_seen.join("\n")
    )
}

/// Sums `stories` over the groups seen after `after_ns` that satisfy `pred`. Returns the sum and
/// the matching group with the most stories; `None` when no group matches.
pub fn sum_matching_groups(
    groups: &[Value],
    after_ns: i64,
    pred: impl Fn(&Value) -> bool,
) -> Option<(u64, Value)> {
    let matching: Vec<&Value> = groups
        .iter()
        .filter(|g| g["last_seen_ns"].as_i64().unwrap_or(0) > after_ns && pred(g))
        .collect();
    let top = matching.iter().max_by_key(|g| story_count(g))?;
    Some((
        matching.iter().map(|g| story_count(g)).sum(),
        (*top).clone(),
    ))
}

/// Like `wait_for_group`, but for outcomes that split across several groups (one fingerprint per
/// endpoint, identical summaries): succeeds when the matching groups together have at least
/// `min_new` stories since the flip. Returns the matching group with the most stories.
pub async fn wait_for_group_sum(
    api: &Api,
    filter: &str,
    after_ns: i64,
    min_new: u64,
    timeout: Duration,
    pred: impl Fn(&Value) -> bool,
) -> anyhow::Result<(Value, Duration)> {
    let start = Instant::now();
    let mut last_seen: Vec<String> = Vec::new();
    let mut last_err: Option<String> = None;
    while start.elapsed() < timeout {
        let query = format!("since={}&{filter}", since_flip(after_ns));
        match api.groups(&query).await {
            Ok(groups) => {
                last_seen = describe_groups(&groups);
                if let Some((sum, top)) = sum_matching_groups(&groups, after_ns, &pred)
                    && sum >= min_new
                {
                    return Ok((top, start.elapsed()));
                }
            }
            Err(e) => {
                eprintln!("[e2e] poll error (continuing): {e}");
                last_err = Some(e.to_string());
            }
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
    anyhow::bail!(
        "matching story groups never reached {min_new} stories within {timeout:?} (last poll error: {last_err:?}); last groups seen:\n{}",
        last_seen.join("\n")
    )
}

/// The longest example story in a group detail (`GET /story-groups/{fp}`) newer than `after_ns`
/// whose duration is at least `min_ns`, as `(story_id, duration_ns)`.
pub fn slow_example_after(detail: &Value, after_ns: i64, min_ns: u64) -> Option<(String, u64)> {
    detail["examples"]
        .as_array()?
        .iter()
        .filter(|e| e["ts_ns"].as_i64().unwrap_or(0) > after_ns)
        .filter_map(|e| {
            Some((
                e["story_id"].as_str()?.to_string(),
                e["duration_ns"].as_u64()?,
            ))
        })
        .filter(|(_, d)| *d >= min_ns)
        .max_by_key(|(_, d)| *d)
}

/// Polls story groups matching `filter` until one satisfying `pred` has an example story after
/// `after_ns` lasting at least `min_ns`. A group's sample story is only its latest, which can be
/// a spontaneous short one, so the examples are checked instead. Returns the group, that story's
/// id and the wait.
pub async fn wait_for_slow_story(
    api: &Api,
    filter: &str,
    after_ns: i64,
    min_ns: u64,
    timeout: Duration,
    pred: impl Fn(&Value) -> bool,
) -> anyhow::Result<(Value, String, Duration)> {
    let start = Instant::now();
    let mut last_seen: Vec<String> = Vec::new();
    let mut last_err: Option<String> = None;
    while start.elapsed() < timeout {
        let since = since_flip(after_ns);
        match api.groups(&format!("since={since}&{filter}")).await {
            Ok(groups) => {
                last_seen = describe_groups(&groups);
                for g in groups
                    .into_iter()
                    .filter(|g| g["last_seen_ns"].as_i64().unwrap_or(0) > after_ns && pred(g))
                {
                    let fp = g["fingerprint"].as_str().unwrap_or_default().to_string();
                    match api.group_detail(&fp, &since).await {
                        Ok(d) => {
                            if let Some((id, _)) = slow_example_after(&d, after_ns, min_ns) {
                                return Ok((g, id, start.elapsed()));
                            }
                        }
                        Err(e) => {
                            eprintln!("[e2e] poll error (continuing): {e}");
                            last_err = Some(e.to_string());
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("[e2e] poll error (continuing): {e}");
                last_err = Some(e.to_string());
            }
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
    anyhow::bail!(
        "no story lasting >= {min_ns} ns in a matching group within {timeout:?} (last poll error: {last_err:?}); last groups seen:\n{}",
        last_seen.join("\n")
    )
}

fn describe_groups(groups: &[Value]) -> Vec<String> {
    groups
        .iter()
        .map(|g| {
            format!(
                "{} | {} | stories={}",
                g["rc_service"],
                g["summary"],
                story_count(g)
            )
        })
        .collect()
}

/// Polls log alerts matching `query` until one with `last_at_ns > after_ns` satisfies `pred`.
/// Returns it and the wait.
pub async fn wait_for_alert(
    api: &Api,
    query: &str,
    after_ns: i64,
    timeout: Duration,
    pred: impl Fn(&Value) -> bool,
) -> anyhow::Result<(Value, Duration)> {
    let start = Instant::now();
    let mut last_seen: Vec<String> = Vec::new();
    let mut last_err: Option<String> = None;
    while start.elapsed() < timeout {
        match api.log_alerts(query).await {
            Ok(alerts) => {
                last_seen = alerts
                    .iter()
                    .map(|a| {
                        format!(
                            "{} | {} | {} | last_at_ns={}",
                            a["kind"], a["service"], a["template"], a["last_at_ns"]
                        )
                    })
                    .collect();
                let found = alerts
                    .into_iter()
                    .find(|a| a["last_at_ns"].as_i64().is_some_and(|n| n > after_ns) && pred(a));
                if let Some(a) = found {
                    return Ok((a, start.elapsed()));
                }
            }
            Err(e) => {
                eprintln!("[e2e] poll error (continuing): {e}");
                last_err = Some(e.to_string());
            }
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
    anyhow::bail!(
        "no matching log alert within {timeout:?} (last poll error: {last_err:?}); last alerts seen:\n{}",
        last_seen.join("\n")
    )
}

/// Whether any template was first seen at least `warmup` before `now_ns`.
pub fn has_template_older_than(templates: &[Value], now_ns: i64, warmup: Duration) -> bool {
    let cutoff = now_ns.saturating_sub(i64::try_from(warmup.as_nanos()).unwrap_or(i64::MAX));
    templates
        .iter()
        .any(|t| t["first_seen_ns"].as_i64().is_some_and(|f| f <= cutoff))
}

/// Polls `service`'s templates until one is at least `PROBE_WARMUP` old. Returns the wait.
pub async fn wait_for_service_warmup(
    api: &Api,
    service: &str,
    timeout: Duration,
) -> anyhow::Result<Duration> {
    let start = Instant::now();
    let query = format!("service={service}&since=7d");
    let mut announced = false;
    loop {
        match api.log_templates(&query).await {
            Ok(t) if has_template_older_than(&t, now_ns(), PROBE_WARMUP) => {
                return Ok(start.elapsed());
            }
            Ok(_) => {}
            Err(e) => eprintln!("[e2e] poll error (continuing): {e}"),
        }
        if !announced {
            println!("[e2e] first run: waiting for the probe service warmup, up to 16 min");
            announced = true;
        }
        anyhow::ensure!(
            start.elapsed() < timeout,
            "service {service} has no template older than {PROBE_WARMUP:?} after {timeout:?}"
        );
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}

/// The id of the template whose text is exactly `body`.
pub fn template_id_of(templates: &[Value], body: &str) -> Option<String> {
    templates
        .iter()
        .find(|t| t["template"].as_str() == Some(body))
        .and_then(|t| t["template_id"].as_str().map(str::to_string))
}

/// Polls `service`'s templates matching `word` until one's text is exactly `body`. Returns its id.
pub async fn wait_for_template(
    api: &Api,
    service: &str,
    word: &str,
    body: &str,
    timeout: Duration,
) -> anyhow::Result<String> {
    let start = Instant::now();
    let query = format!("service={service}&q={word}&since=1h");
    loop {
        match api.log_templates(&query).await {
            Ok(t) => {
                if let Some(id) = template_id_of(&t, body) {
                    return Ok(id);
                }
            }
            Err(e) => eprintln!("[e2e] poll error (continuing): {e}"),
        }
        anyhow::ensure!(
            start.elapsed() < timeout,
            "no template {body:?} in {service} after {timeout:?}"
        );
        tokio::time::sleep(POLL_EVERY).await;
    }
}

/// The requests the mock received for `alert_id`.
pub fn deliveries_of<'a>(received: &'a [Received], alert_id: &str) -> Vec<&'a Received> {
    received
        .iter()
        .filter(|r| r.body["alert_id"].as_str() == Some(alert_id))
        .collect()
}

/// Fails fast when no checkout endpoint can flag a `delay_s` trace as slow: the assembler needs
/// ≥ 50 baseline traces and a duration above max(1.5 × p99, p99 + 100 ms) over the last 60 min.
pub async fn ensure_checkout_baseline_detects(
    clickhouse: &str,
    delay_s: f64,
) -> anyhow::Result<()> {
    let sql = format!(
        "SELECT endpoint_name, count() AS n, quantile(0.99)(duration_ns) / 1e9 AS p99, \
         n >= 50 AND {delay_s} > greatest(p99 * 1.5, p99 + 0.1) AS ok FROM tayga.trace_summaries FINAL \
         WHERE ts > now() - INTERVAL 60 MINUTE AND is_error = 0 AND endpoint_service = 'load-generator' \
         AND endpoint_name LIKE 'user_checkout%' AND trace_id NOT IN (SELECT trace_id FROM \
         tayga.error_stories WHERE kind = 'slow' AND ts > now() - INTERVAL 70 MINUTE) \
         GROUP BY endpoint_name FORMAT TSVWithNames"
    );
    let res = reqwest::Client::new()
        .post(clickhouse)
        .body(sql)
        .send()
        .await?;
    let rows = res.error_for_status()?.text().await?;
    anyhow::ensure!(
        rows.lines().any(|l| l.ends_with("\t1")),
        "checkout baselines cannot flag a {delay_s} s trace as slow (degraded stack, or recent \
         slowdown runs in the last 60 min):\n{rows}"
    );
    Ok(())
}

/// One request the mock webhook received, and the status it answered with.
#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    pub path: String,
    pub content_type: Option<String>,
    /// The body as JSON; `Value::Null` when it was not JSON.
    pub body: Value,
    pub status: u16,
}

#[derive(Default)]
struct MockState {
    script: VecDeque<u16>,
    received: Vec<Received>,
    delay: Duration,
}

/// A local webhook receiver for notifier tests: every request on any path is recorded and
/// answered with the next scripted status, then 200 once the script is used up. A 429 carries
/// `Retry-After: 1`.
pub struct MockWebhook {
    pub addr: SocketAddr,
    state: Arc<Mutex<MockState>>,
    server: tokio::task::JoinHandle<()>,
}

impl MockWebhook {
    /// Binds `addr` (port 0 for an ephemeral one); `127.0.0.1` for in-process tests, `0.0.0.0`
    /// to be reachable from containers through `host.docker.internal`.
    pub async fn start(addr: SocketAddr, script: &[u16]) -> anyhow::Result<Self> {
        let state = Arc::new(Mutex::new(MockState {
            script: script.iter().copied().collect(),
            received: Vec::new(),
            delay: Duration::ZERO,
        }));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let app = Router::new()
            .fallback(mock_receive)
            .with_state(state.clone());
        let server = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("mock webhook stopped: {e}");
            }
        });
        Ok(Self {
            addr,
            state,
            server,
        })
    }

    /// `http://<addr>/hook`.
    pub fn url(&self) -> String {
        format!("http://{}/hook", self.addr)
    }

    /// Delays every later response by `delay`; the request is recorded on arrival.
    pub fn set_delay(&self, delay: Duration) {
        self.state.lock().expect("mock state lock").delay = delay;
    }

    pub fn received(&self) -> Vec<Received> {
        self.state.lock().expect("mock state lock").received.clone()
    }
}

impl Drop for MockWebhook {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn mock_receive(
    State(state): State<Arc<Mutex<MockState>>>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let (status, delay) = {
        let mut s = state.lock().expect("mock state lock");
        let status = s.script.pop_front().unwrap_or(200);
        s.received.push(Received {
            path: uri.path().to_string(),
            content_type: headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
            body: serde_json::from_str(&body).unwrap_or(Value::Null),
            status,
        });
        (status, s.delay)
    };
    tokio::time::sleep(delay).await;
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    if status == 429 {
        (code, [(header::RETRY_AFTER, "1")]).into_response()
    } else {
        code.into_response()
    }
}

pub fn report(name: &str, waited: Duration) {
    let verdict = if waited <= TARGET_LATENCY {
        "within"
    } else {
        "OVER"
    };
    println!(
        "[e2e] {name}: first matching story after {:.0}s ({verdict} the 60s target)",
        waited.as_secs_f64()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_webhook_records_requests_and_follows_its_script() {
        let mock = MockWebhook::start(SocketAddr::from(([127, 0, 0, 1], 0)), &[503, 429])
            .await
            .unwrap();
        let http = reqwest::Client::new();
        let post = || {
            http.post(mock.url())
                .json(&serde_json::json!({ "n": 1 }))
                .send()
        };
        assert_eq!(post().await.unwrap().status(), 503);
        let r = post().await.unwrap();
        assert_eq!(r.status(), 429);
        assert_eq!(r.headers()[header::RETRY_AFTER], "1");
        assert_eq!(post().await.unwrap().status(), 200);
        let got = mock.received();
        assert_eq!(
            got.iter().map(|r| r.status).collect::<Vec<_>>(),
            [503, 429, 200]
        );
        assert_eq!(got[0].path, "/hook");
        assert_eq!(got[0].content_type.as_deref(), Some("application/json"));
        assert_eq!(got[0].body, serde_json::json!({ "n": 1 }));
        mock.set_delay(Duration::from_millis(300));
        let started = Instant::now();
        assert_eq!(post().await.unwrap().status(), 200);
        assert!(started.elapsed() >= Duration::from_millis(300));
    }

    #[test]
    fn template_id_needs_the_exact_text() {
        let t = [
            serde_json::json!({ "template_id": "1", "template": "abc probe marker" }),
            serde_json::json!({ "template_id": "2", "template": "abc probe probe marker" }),
        ];
        assert_eq!(
            template_id_of(&t, "abc probe probe marker").as_deref(),
            Some("2")
        );
        assert!(template_id_of(&t, "abc marker").is_none());
    }

    #[test]
    fn deliveries_are_counted_per_alert_id() {
        let r = |id: &str| Received {
            path: "/hook".into(),
            content_type: None,
            body: serde_json::json!({ "alert_id": id }),
            status: 200,
        };
        let got = [r("a"), r("b"), r("a")];
        assert_eq!(deliveries_of(&got, "a").len(), 2);
        assert_eq!(deliveries_of(&got, "c").len(), 0);
    }

    #[test]
    fn since_flip_covers_only_the_time_after_the_flip() {
        assert_eq!(since_flip(now_ns()), "1s");
        assert_eq!(since_flip(now_ns() + 5_000_000_000), "1s", "future flip");
        let s = since_flip(now_ns() - 90_500_000_000);
        assert!(s == "91s" || s == "92s", "{s}");
    }

    #[test]
    fn warmup_needs_a_template_at_least_that_old() {
        let now = 100 * 60 * 1_000_000_000_i64;
        let warmup = Duration::from_secs(15 * 60);
        let at =
            |mins_ago: i64| serde_json::json!({ "first_seen_ns": now - mins_ago * 60_000_000_000 });
        assert!(has_template_older_than(&[at(1), at(15)], now, warmup));
        assert!(!has_template_older_than(&[at(1), at(14)], now, warmup));
        assert!(!has_template_older_than(&[], now, warmup));
    }

    fn group(fp: &str, stories: u64, last_seen_ns: i64, summary: &str) -> Value {
        serde_json::json!({
            "fingerprint": fp, "stories": stories,
            "last_seen_ns": last_seen_ns, "summary": summary
        })
    }

    #[test]
    fn sums_split_groups_and_returns_the_largest() {
        let groups = [
            group("1", 2, 100, "checkout could not reach A"),
            group("2", 3, 100, "checkout could not reach A"),
            group("3", 9, 100, "unrelated"),
            group("4", 7, 50, "checkout could not reach A"), // before the flip
        ];
        let (sum, top) = sum_matching_groups(&groups, 60, |g| {
            g["summary"].as_str().unwrap().contains("reach")
        })
        .unwrap();
        assert_eq!(sum, 5);
        assert_eq!(top["fingerprint"], "2");
    }

    #[test]
    fn sum_is_none_without_a_match() {
        let groups = [group("1", 2, 10, "x")];
        assert!(sum_matching_groups(&groups, 60, |_| true).is_none());
        assert!(sum_matching_groups(&[], 0, |_| true).is_none());
    }

    #[test]
    fn slow_example_must_be_after_the_flip_and_long_enough() {
        let detail = serde_json::json!({ "examples": [
            { "story_id": "latest", "ts_ns": 300, "duration_ns": 579_000_000_u64 },
            { "story_id": "slow", "ts_ns": 200, "duration_ns": 5_100_000_000_u64 },
            { "story_id": "old", "ts_ns": 10, "duration_ns": 9_000_000_000_u64 },
        ]});
        let (id, d) = slow_example_after(&detail, 100, 4_500_000_000).unwrap();
        assert_eq!((id.as_str(), d), ("slow", 5_100_000_000));
        assert!(slow_example_after(&detail, 250, 4_500_000_000).is_none());
        assert!(slow_example_after(&serde_json::json!({}), 0, 1).is_none());
    }
}
