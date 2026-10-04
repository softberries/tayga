//! Black-box helpers for end-to-end tests against the running demo stack.

use serde_json::Value;
use std::path::PathBuf;
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
                last_seen = groups
                    .iter()
                    .map(|g| {
                        format!(
                            "{} | {} | stories={}",
                            g["rc_service"],
                            g["summary"],
                            story_count(g)
                        )
                    })
                    .collect();
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
}
