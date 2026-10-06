//! Metric history recorder (spec §7): every `record_secs` it scrapes each target's `/metrics`,
//! adds the API's own registry, and stores one batch of samples in `metric_samples`.

use crate::openmetrics;
use crate::routes::{ApiMetrics, JobLabel};
use serde::Deserialize;
use std::collections::HashSet;
use std::future::Future;
use std::net::SocketAddr;
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
        t("tayga-notifier", "http://tayga-notifier:9100/metrics"),
    ]
}

/// Fetches one exposition. A trait so the tick logic is testable without HTTP.
pub trait Fetch: Send + Sync + 'static {
    fn fetch(&self, url: &str) -> impl Future<Output = anyhow::Result<String>> + Send;
    /// The socket addresses `url`'s host resolves to; empty when it does not resolve.
    fn resolve(&self, url: &str) -> impl Future<Output = Vec<SocketAddr>> + Send;
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

    async fn resolve(&self, url: &str) -> Vec<SocketAddr> {
        let Ok(u) = reqwest::Url::parse(url) else {
            return Vec::new();
        };
        let (Some(host), Some(port)) = (u.host_str(), u.port_or_known_default()) else {
            return Vec::new();
        };
        match tokio::net::lookup_host((host, port)).await {
            Ok(found) => found.collect(),
            Err(e) => {
                tracing::debug!(url, error = %e, "metric target did not resolve");
                Vec::new()
            }
        }
    }
}

/// One scrape of a target: its URL, and the `instance` label when the target's host resolves to
/// several addresses (one per replica).
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub url: String,
    pub instance: Option<String>,
}

/// The addresses a target is scraped at: IPv4 only when there is any (a dual-stack name would
/// otherwise count each replica twice), else IPv6; sorted, without duplicates.
fn distinct(addrs: &[SocketAddr]) -> Vec<SocketAddr> {
    let any_v4 = addrs.iter().any(SocketAddr::is_ipv4);
    let mut out: Vec<SocketAddr> = addrs
        .iter()
        .copied()
        .filter(|a| a.is_ipv4() == any_v4)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The scrapes of `url` given its host's resolved `addrs` (see [`distinct`]). At most one
/// address and not `sticky`: the URL as configured, unlabelled, so one replica records exactly
/// what it did before. Several, or one of a `sticky` target (one that has resolved to several
/// before, so its series keep their `instance` label when it scales down): one URL per address,
/// labelled `instance="<ip>:<port>"`. Unresolved: the URL as configured. Scraping by address
/// also keeps the HTTP client from reusing one pooled connection to one replica.
pub fn endpoints(url: &str, addrs: &[SocketAddr], sticky: bool) -> Vec<Endpoint> {
    let addrs = distinct(addrs);
    let as_configured = || {
        vec![Endpoint {
            url: url.to_string(),
            instance: None,
        }]
    };
    if addrs.is_empty() || (addrs.len() == 1 && !sticky) {
        return as_configured();
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return as_configured();
    };
    let labelled: Vec<Endpoint> = addrs
        .into_iter()
        .filter_map(|a| {
            let mut u = parsed.clone();
            u.set_ip_host(a.ip()).ok()?;
            Some(Endpoint {
                url: u.to_string(),
                instance: Some(a.to_string()),
            })
        })
        .collect();
    if labelled.is_empty() {
        return as_configured();
    }
    labelled
}

fn instance_labels(instance: Option<&str>) -> Vec<(String, String)> {
    instance
        .map(|i| vec![("instance".to_string(), i.to_string())])
        .unwrap_or_default()
}

fn up(job: &str, instance: Option<&str>, ts: i64, value: f64) -> MetricSampleRow {
    MetricSampleRow {
        ts,
        job: job.to_string(),
        metric: "up".to_string(),
        labels: instance_labels(instance),
        value,
    }
}

fn push_exposition(
    rows: &mut Vec<MetricSampleRow>,
    job: &str,
    instance: Option<&str>,
    text: &str,
    ts: i64,
) {
    let parsed = openmetrics::parse_counted(text);
    if parsed.malformed > 0 {
        tracing::warn!(
            job,
            malformed = parsed.malformed,
            "skipped malformed metric lines"
        );
    }
    rows.extend(parsed.samples.into_iter().map(|s| {
        let mut labels = s.labels;
        labels.extend(instance_labels(instance));
        MetricSampleRow {
            ts,
            job: job.to_string(),
            metric: s.name,
            labels,
            value: s.value,
        }
    }));
    rows.push(up(job, instance, ts, 1.0));
}

/// `fetch.resolve(url)`, bounded by [`FETCH_TIMEOUT`]; a lookup that does not finish in time
/// resolves to nothing, so the target is scraped at its configured URL.
async fn resolve_bounded<F: Fetch>(fetch: &F, url: &str) -> Vec<SocketAddr> {
    tokio::time::timeout(FETCH_TIMEOUT, fetch.resolve(url))
        .await
        .unwrap_or_else(|_| {
            tracing::debug!(url, "metric target lookup timed out");
            Vec::new()
        })
}

/// One tick's rows: every target resolved, every resolved address fetched concurrently (see
/// [`endpoints`]), plus the API's own exposition. A failed scrape contributes only `up` = 0 and
/// increments `scrape_failures{job}`. `multi` holds the target URLs that have resolved to
/// several addresses in this process; it grows here and keeps their labelling sticky.
pub async fn collect<F: Fetch>(
    fetch: &F,
    targets: &[Target],
    api_text: &str,
    metrics: &ApiMetrics,
    ts: i64,
    multi: &mut HashSet<String>,
) -> Vec<MetricSampleRow> {
    let resolved =
        futures::future::join_all(targets.iter().map(|t| resolve_bounded(fetch, &t.url))).await;
    let mut scrapes: Vec<(&Target, Endpoint)> = Vec::new();
    for (t, addrs) in targets.iter().zip(resolved) {
        let eps = endpoints(&t.url, &addrs, multi.contains(&t.url));
        if eps.len() > 1 {
            multi.insert(t.url.clone());
        }
        scrapes.extend(eps.into_iter().map(|e| (t, e)));
    }
    let results = futures::future::join_all(scrapes.iter().map(|(_, e)| fetch.fetch(&e.url))).await;
    let mut rows = Vec::new();
    for ((t, e), result) in scrapes.iter().zip(results) {
        let instance = e.instance.as_deref();
        match result {
            Ok(text) => push_exposition(&mut rows, &t.job, instance, &text, ts),
            Err(err) => {
                tracing::warn!(job = %t.job, url = %e.url, error = format!("{err:#}"), "metric scrape failed");
                metrics
                    .scrape_failures
                    .get_or_create(&JobLabel { job: t.job.clone() })
                    .inc();
                rows.push(up(&t.job, instance, ts, 0.0));
            }
        }
    }
    push_exposition(&mut rows, API_JOB, None, api_text, ts);
    rows
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Runs the recorder until `stop` flips to true. One insert per tick; a failed insert is
/// logged and that tick's samples are dropped. With `every` = 0 (`record_secs = 0`, for runs on
/// the host) it records nothing and returns a finished task.
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
    if every.is_zero() {
        tracing::info!("metric recorder disabled (record_secs = 0)");
        return tokio::spawn(async {});
    }
    tokio::spawn(async move {
        let fetch = match HttpFetch::new() {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(error = %e, "metric recorder disabled: HTTP client failed");
                return;
            }
        };
        let mut multi = HashSet::new();
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = stop.wait_for(|s| *s) => break,
                _ = tick.tick() => {}
            }
            let ts = now_ms();
            let rows = collect(&fetch, &targets, &registry_text(), &metrics, ts, &mut multi).await;
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
    use std::collections::{HashMap, HashSet};

    #[tokio::test]
    async fn record_secs_zero_turns_the_recorder_off() {
        let store = Store::new(&tayga_store::ClickHouseSettings {
            url: "http://127.0.0.1:1".into(),
            database: "tayga".into(),
        });
        let (_tx, stop) = tokio::sync::watch::channel(false);
        let task = spawn(
            store,
            default_targets(),
            String::new,
            ApiMetrics::default(),
            Duration::ZERO,
            stop,
        );
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the task ends at once")
            .unwrap();
    }

    #[derive(Default)]
    struct FakeFetch {
        bodies: HashMap<String, Result<String, String>>,
        addrs: HashMap<String, Vec<SocketAddr>>,
        /// `resolve` never completes, as a DNS lookup that hangs.
        hang: bool,
    }

    impl Fetch for FakeFetch {
        async fn fetch(&self, url: &str) -> anyhow::Result<String> {
            match self.bodies.get(url) {
                Some(Ok(text)) => Ok(text.clone()),
                Some(Err(e)) => Err(anyhow::anyhow!(e.clone())),
                None => Err(anyhow::anyhow!("no such target")),
            }
        }

        async fn resolve(&self, url: &str) -> Vec<SocketAddr> {
            if self.hang {
                std::future::pending::<()>().await;
            }
            self.addrs.get(url).cloned().unwrap_or_default()
        }
    }

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
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
        let fetch = FakeFetch {
            bodies: HashMap::from([
                (
                    "http://w/metrics".to_string(),
                    Ok(include_str!("../fixtures/metrics/writer.txt").to_string()),
                ),
                ("http://l/metrics".to_string(), Err("timed out".to_string())),
            ]),
            ..Default::default()
        };
        let metrics = ApiMetrics::default();
        let api_text = "# TYPE x counter\nx_total 3\n# EOF\n";

        let rows = collect(
            &fetch,
            &targets,
            api_text,
            &metrics,
            1_700_000_000_000,
            &mut HashSet::new(),
        )
        .await;

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
    fn one_address_keeps_the_url_and_several_get_an_instance_each() {
        let url = "http://tayga-logminer:9100/metrics";
        let one = endpoints(url, &[addr("10.0.0.7:9100")], false);
        assert_eq!(
            one,
            [Endpoint {
                url: url.into(),
                instance: None
            }]
        );
        assert_eq!(endpoints(url, &[], false), one, "unresolved: as configured");
        let two = endpoints(
            url,
            &[
                addr("10.0.0.9:9100"),
                addr("10.0.0.7:9100"),
                addr("10.0.0.9:9100"),
            ],
            false,
        );
        assert_eq!(
            two,
            [
                Endpoint {
                    url: "http://10.0.0.7:9100/metrics".into(),
                    instance: Some("10.0.0.7:9100".into())
                },
                Endpoint {
                    url: "http://10.0.0.9:9100/metrics".into(),
                    instance: Some("10.0.0.9:9100".into())
                },
            ]
        );
    }

    #[tokio::test]
    async fn every_replica_is_scraped_with_its_instance_label() {
        let url = "http://tayga-logminer:9100/metrics";
        let targets = vec![Target {
            job: "tayga-logminer".into(),
            url: url.into(),
        }];
        let fetch = FakeFetch {
            bodies: HashMap::from([
                (
                    "http://10.0.0.7:9100/metrics".to_string(),
                    Ok("# TYPE x counter\nx_total 3\n# EOF\n".to_string()),
                ),
                (
                    "http://10.0.0.9:9100/metrics".to_string(),
                    Err("refused".to_string()),
                ),
            ]),
            addrs: HashMap::from([(
                url.to_string(),
                vec![addr("10.0.0.7:9100"), addr("10.0.0.9:9100")],
            )]),
            ..Default::default()
        };
        let metrics = ApiMetrics::default();
        let rows = collect(
            &fetch,
            &targets,
            "# EOF\n",
            &metrics,
            1,
            &mut HashSet::new(),
        )
        .await;
        let inst = |i: &str| vec![("instance".to_string(), i.to_string())];
        let logminer = rows_of(&rows, "tayga-logminer");
        assert_eq!(logminer.len(), 3, "{logminer:?}");
        assert!(
            logminer.iter().any(|r| r.metric == "x_total"
                && r.value == 3.0
                && r.labels == inst("10.0.0.7:9100"))
        );
        assert!(
            logminer
                .iter()
                .any(|r| r.metric == "up" && r.value == 1.0 && r.labels == inst("10.0.0.7:9100"))
        );
        assert!(
            logminer
                .iter()
                .any(|r| r.metric == "up" && r.value == 0.0 && r.labels == inst("10.0.0.9:9100"))
        );
        let job = JobLabel {
            job: "tayga-logminer".into(),
        };
        assert_eq!(metrics.scrape_failures.get_or_create(&job).get(), 1);
    }

    #[test]
    fn ipv4_addresses_win_over_ipv6() {
        let url = "http://localhost:9100/metrics";
        assert_eq!(
            endpoints(url, &[addr("127.0.0.1:9100"), addr("[::1]:9100")], false),
            [Endpoint {
                url: url.into(),
                instance: None
            }]
        );
        let v6 = endpoints(url, &[addr("[::2]:9100"), addr("[::1]:9100")], false);
        assert_eq!(
            v6.iter().map(|e| e.url.as_str()).collect::<Vec<_>>(),
            ["http://[::1]:9100/metrics", "http://[::2]:9100/metrics"]
        );
        assert_eq!(v6[0].instance.as_deref(), Some("[::1]:9100"));
    }

    #[test]
    fn a_sticky_target_labels_its_single_address() {
        let url = "http://tayga-logminer:9100/metrics";
        assert_eq!(
            endpoints(url, &[addr("10.0.0.7:9100")], true),
            [Endpoint {
                url: "http://10.0.0.7:9100/metrics".into(),
                instance: Some("10.0.0.7:9100".into())
            }]
        );
    }

    /// One logminer target resolving to `addrs`; the configured URL and every address answer.
    fn logminer_fetch(addrs: &[&str]) -> (Vec<Target>, FakeFetch) {
        let url = "http://tayga-logminer:9100/metrics";
        let body = || Ok("# TYPE x counter\nx_total 3\n# EOF\n".to_string());
        let mut bodies = HashMap::from([(url.to_string(), body())]);
        for a in addrs {
            bodies.insert(format!("http://{a}/metrics"), body());
        }
        let fetch = FakeFetch {
            bodies,
            addrs: HashMap::from([(url.to_string(), addrs.iter().map(|a| addr(a)).collect())]),
            ..Default::default()
        };
        let targets = vec![Target {
            job: "tayga-logminer".into(),
            url: url.into(),
        }];
        (targets, fetch)
    }

    fn logminer_labels(rows: &[MetricSampleRow]) -> Vec<Vec<(String, String)>> {
        rows_of(rows, "tayga-logminer")
            .into_iter()
            .map(|r| r.labels.clone())
            .collect()
    }

    #[tokio::test]
    async fn once_several_replicas_are_seen_the_target_stays_labelled() {
        let metrics = ApiMetrics::default();
        let mut multi = HashSet::new();
        let (targets, one) = logminer_fetch(&["10.0.0.7:9100"]);
        let rows = collect(&one, &targets, "# EOF\n", &metrics, 1, &mut multi).await;
        assert_eq!(
            logminer_labels(&rows),
            [vec![], vec![]],
            "only one address seen: unlabelled"
        );
        let (_, two) = logminer_fetch(&["10.0.0.7:9100", "10.0.0.9:9100"]);
        collect(&two, &targets, "# EOF\n", &metrics, 2, &mut multi).await;
        let rows = collect(&one, &targets, "# EOF\n", &metrics, 3, &mut multi).await;
        let inst = vec![("instance".to_string(), "10.0.0.7:9100".to_string())];
        assert_eq!(logminer_labels(&rows), [inst.clone(), inst]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_lookup_falls_back_to_the_configured_url() {
        let (targets, mut fetch) = logminer_fetch(&[]);
        fetch.hang = true;
        let metrics = ApiMetrics::default();
        let rows = collect(
            &fetch,
            &targets,
            "# EOF\n",
            &metrics,
            1,
            &mut HashSet::new(),
        )
        .await;
        let logminer = rows_of(&rows, "tayga-logminer");
        assert_eq!(logminer.len(), 2, "{logminer:?}");
        assert!(logminer.iter().all(|r| r.labels.is_empty()));
        assert!(logminer.iter().any(|r| r.metric == "up" && r.value == 1.0));
    }

    #[tokio::test]
    async fn http_resolve_handles_bad_urls_and_localhost() {
        let http = HttpFetch::new().unwrap();
        assert!(http.resolve("not a url").await.is_empty());
        assert!(
            http.resolve("unknown://host/metrics").await.is_empty(),
            "no port"
        );
        let local = http.resolve("http://localhost:9100/metrics").await;
        assert!(!local.is_empty());
        assert!(
            local
                .iter()
                .all(|a| a.port() == 9100 && a.ip().is_loopback()),
            "{local:?}"
        );
    }

    #[test]
    fn default_targets_cover_the_five_services() {
        let jobs: Vec<String> = default_targets().into_iter().map(|t| t.job).collect();
        assert_eq!(
            jobs,
            [
                "tayga-ingest",
                "tayga-writer",
                "tayga-assembler",
                "tayga-logminer",
                "tayga-notifier"
            ]
        );
    }
}
