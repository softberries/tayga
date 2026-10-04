//! Metric history recorder (spec §7): every `record_secs` it scrapes each target's `/metrics`,
//! adds the API's own registry, and stores one batch of samples in `metric_samples`.

use crate::openmetrics;
use crate::routes::{ApiMetrics, JobLabel};
use serde::Deserialize;
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tayga_store::metrics_store::MetricSampleRow;
use tayga_store::store::Store;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// The job name of the API's own registry, recorded without HTTP.
pub const API_JOB: &str = "tayga-api";
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Target {
    pub job: String,
    pub url: String,
}

/// The compose services' metric endpoints.
/// Keep in sync with `deploy/tayga-api.toml`.
pub fn default_targets() -> Vec<Target> {
    let t = |job: &str, url: &str| Target {
        job: job.to_string(),
        url: url.to_string(),
    };
    vec![
        t("tayga-ingest", "http://tayga-ingest:4318/metrics"),
        t("tayga-writer", "http://tayga-writer:9100/metrics"),
        t("tayga-assembler", "http://tayga-assembler:9100/metrics"),
        t("tayga-logminer", "http://tayga-logminer:9100/metrics"),
    ]
}

/// Fetches one exposition. A trait so the tick logic is testable without HTTP.
pub trait Fetch: Send + Sync + 'static {
    fn fetch(&self, url: &str) -> impl Future<Output = anyhow::Result<String>> + Send;
}

pub struct HttpFetch(reqwest::Client);

impl HttpFetch {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self(
            reqwest::Client::builder().timeout(FETCH_TIMEOUT).build()?,
        ))
    }
}

impl Fetch for HttpFetch {
    async fn fetch(&self, url: &str) -> anyhow::Result<String> {
        Ok(self
            .0
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?)
    }
}

fn up(job: &str, ts: i64, value: f64) -> MetricSampleRow {
    MetricSampleRow {
        ts,
        job: job.to_string(),
        metric: "up".to_string(),
        labels: Vec::new(),
        value,
    }
}

fn push_exposition(rows: &mut Vec<MetricSampleRow>, job: &str, text: &str, ts: i64) {
    let parsed = openmetrics::parse_counted(text);
    if parsed.malformed > 0 {
        tracing::warn!(
            job,
            malformed = parsed.malformed,
            "skipped malformed metric lines"
        );
    }
    rows.extend(parsed.samples.into_iter().map(|s| MetricSampleRow {
        ts,
        job: job.to_string(),
        metric: s.name,
        labels: s.labels,
        value: s.value,
    }));
    rows.push(up(job, ts, 1.0));
}

/// One tick's rows: every target fetched concurrently, plus the API's own exposition. A failed
/// target contributes only `up` = 0 and increments `scrape_failures{job}`.
pub async fn collect<F: Fetch>(
    fetch: &F,
    targets: &[Target],
    api_text: &str,
    metrics: &ApiMetrics,
    ts: i64,
) -> Vec<MetricSampleRow> {
    let results = futures::future::join_all(targets.iter().map(|t| fetch.fetch(&t.url))).await;
    let mut rows = Vec::new();
    for (t, result) in targets.iter().zip(results) {
        match result {
            Ok(text) => push_exposition(&mut rows, &t.job, &text, ts),
            Err(e) => {
                tracing::warn!(job = %t.job, url = %t.url, error = format!("{e:#}"), "metric scrape failed");
                metrics
                    .scrape_failures
                    .get_or_create(&JobLabel { job: t.job.clone() })
                    .inc();
                rows.push(up(&t.job, ts, 0.0));
            }
        }
    }
    push_exposition(&mut rows, API_JOB, api_text, ts);
    rows
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Runs the recorder until `stop` flips to true. One insert per tick; a failed insert is
/// logged and that tick's samples are dropped.
pub fn spawn<T>(
    store: Store,
    targets: Vec<Target>,
    registry_text: T,
    metrics: ApiMetrics,
    every: Duration,
    mut stop: watch::Receiver<bool>,
) -> JoinHandle<()>
where
    T: Fn() -> String + Send + 'static,
{
    tokio::spawn(async move {
        let fetch = match HttpFetch::new() {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(error = %e, "metric recorder disabled: HTTP client failed");
                return;
            }
        };
        let mut tick = tokio::time::interval(every.max(Duration::from_secs(1)));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = stop.wait_for(|s| *s) => break,
                _ = tick.tick() => {}
            }
            let ts = now_ms();
            let rows = collect(&fetch, &targets, &registry_text(), &metrics, ts).await;
            if let Err(e) = store.insert_metric_samples(&rows).await {
                tracing::warn!(error = %e, rows = rows.len(), "metric samples insert failed; tick dropped");
            }
        }
        tracing::info!("metric recorder stopped");
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeFetch(HashMap<String, Result<String, String>>);

    impl Fetch for FakeFetch {
        async fn fetch(&self, url: &str) -> anyhow::Result<String> {
            match self.0.get(url) {
                Some(Ok(text)) => Ok(text.clone()),
                Some(Err(e)) => Err(anyhow::anyhow!(e.clone())),
                None => Err(anyhow::anyhow!("no such target")),
            }
        }
    }

    fn rows_of<'a>(rows: &'a [MetricSampleRow], job: &str) -> Vec<&'a MetricSampleRow> {
        rows.iter().filter(|r| r.job == job).collect()
    }

    #[tokio::test]
    async fn failed_target_records_up_zero_and_counts_the_failure() {
        let targets = vec![
            Target {
                job: "tayga-writer".into(),
                url: "http://w/metrics".into(),
            },
            Target {
                job: "tayga-logminer".into(),
                url: "http://l/metrics".into(),
            },
        ];
        let fetch = FakeFetch(HashMap::from([
            (
                "http://w/metrics".to_string(),
                Ok(include_str!("../fixtures/metrics/writer.txt").to_string()),
            ),
            ("http://l/metrics".to_string(), Err("timed out".to_string())),
        ]));
        let metrics = ApiMetrics::default();
        let api_text = "# TYPE x counter\nx_total 3\n# EOF\n";

        let rows = collect(&fetch, &targets, api_text, &metrics, 1_700_000_000_000).await;

        assert!(rows.iter().all(|r| r.ts == 1_700_000_000_000));
        let logminer = rows_of(&rows, "tayga-logminer");
        assert_eq!(logminer.len(), 1);
        assert_eq!(
            (logminer[0].metric.as_str(), logminer[0].value),
            ("up", 0.0)
        );
        let failed = JobLabel {
            job: "tayga-logminer".into(),
        };
        assert_eq!(metrics.scrape_failures.get_or_create(&failed).get(), 1);
        let ok = JobLabel {
            job: "tayga-writer".into(),
        };
        assert_eq!(metrics.scrape_failures.get_or_create(&ok).get(), 0);

        let writer = rows_of(&rows, "tayga-writer");
        assert_eq!(writer.len(), 21 + 1, "21 samples plus up");
        assert!(
            writer
                .iter()
                .any(|r| r.metric == "up" && r.value == 1.0 && r.labels.is_empty())
        );
        assert!(
            writer
                .iter()
                .any(|r| r.metric == "tayga_writer_batch_seconds_bucket"
                    && r.labels == vec![("le".to_string(), "+Inf".to_string())])
        );

        let api = rows_of(&rows, API_JOB);
        assert_eq!(api.len(), 2);
        assert!(api.iter().any(|r| r.metric == "x_total" && r.value == 3.0));
        assert!(api.iter().any(|r| r.metric == "up" && r.value == 1.0));
    }

    #[test]
    fn default_targets_cover_the_four_services() {
        let jobs: Vec<String> = default_targets().into_iter().map(|t| t.job).collect();
        assert_eq!(
            jobs,
            [
                "tayga-ingest",
                "tayga-writer",
                "tayga-assembler",
                "tayga-logminer"
            ]
        );
    }
}
