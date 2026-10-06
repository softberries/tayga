//! Reads real consumer-group lag from Redpanda. Read-only: no subscribe, no commit.

use tayga_api::lag::{self, ALERT_GROUPS, GROUPS};

#[tokio::test]
#[ignore = "needs Redpanda (make infra-up); set TAYGA_IT_KAFKA to override localhost:19092"]
async fn fetch_returns_lag_for_each_group() {
    let brokers = std::env::var("TAYGA_IT_KAFKA").unwrap_or_else(|_| "localhost:19092".into());
    let lags = lag::fetch(&brokers, "tayga.signals", &GROUPS)
        .await
        .unwrap();
    assert_eq!(lags.len(), GROUPS.len());
    for l in &lags {
        println!("{l:?}");
        assert!(l.lag >= 0, "{l:?}");
        assert!(l.end >= 0, "{l:?}");
    }
}

#[tokio::test]
#[ignore = "needs Redpanda with tayga.alerts (make up); set TAYGA_IT_KAFKA to override localhost:19092"]
async fn fetch_all_adds_the_notifier_on_the_alerts_topic() {
    let brokers = std::env::var("TAYGA_IT_KAFKA").unwrap_or_else(|_| "localhost:19092".into());
    let lags = lag::fetch_all(&brokers, "tayga.signals").await.unwrap();
    let groups: Vec<&str> = lags.iter().map(|l| l.group.as_str()).collect();
    let expected: Vec<&str> = GROUPS.iter().chain(&ALERT_GROUPS).copied().collect();
    assert_eq!(groups, expected);
}
