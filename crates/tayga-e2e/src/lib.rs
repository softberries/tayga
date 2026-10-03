//! Black-box helpers for end-to-end tests against the running demo stack.

use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const API: &str = "http://localhost:8090";
pub const SCENARIO_TIMEOUT: Duration = Duration::from_secs(180);
pub const POLL_EVERY: Duration = Duration::from_secs(5);
/// Spec §15 target for flag-to-story latency; reported, not asserted.
pub const TARGET_LATENCY: Duration = Duration::from_secs(60);

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn now_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
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

    pub async fn story(&self, id: &str) -> anyhow::Result<Value> {
        self.get(&format!("/api/v1/stories/{id}")).await
    }

    pub async fn service_map(&self, since: &str) -> anyhow::Result<Vec<Value>> {
        Ok(self
            .get(&format!("/api/v1/service-map?since={since}"))
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }
}

fn story_count(g: &Value) -> u64 {
    g["stories"].as_u64().unwrap_or(0)
}

/// Story counts per fingerprint for `query`; take it just before flipping the flag.
pub async fn snapshot(api: &Api, query: &str) -> anyhow::Result<HashMap<String, u64>> {
    let groups = api.groups(query).await?;
    Ok(groups
        .iter()
        .map(|g| {
            (
                g["fingerprint"].as_str().unwrap_or_default().to_string(),
                story_count(g),
            )
        })
        .collect())
}

/// Polls story groups until one seen after `after_ns` satisfies `pred` and has gained at least
/// `min_new` stories since `before` (a group absent from `before` counts from 0). Returns it and the wait.
pub async fn wait_for_group(
    api: &Api,
    query: &str,
    before: &HashMap<String, u64>,
    after_ns: i64,
    min_new: u64,
    timeout: Duration,
    pred: impl Fn(&Value) -> bool,
) -> anyhow::Result<(Value, Duration)> {
    let start = Instant::now();
    let mut last_seen: Vec<String> = Vec::new();
    let mut last_err: Option<String> = None;
    while start.elapsed() < timeout {
        match api.groups(query).await {
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
                    let prior = before
                        .get(g["fingerprint"].as_str().unwrap_or_default())
                        .copied()
                        .unwrap_or(0);
                    g["last_seen_ns"].as_i64().unwrap_or(0) > after_ns
                        && story_count(g).saturating_sub(prior) >= min_new
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
