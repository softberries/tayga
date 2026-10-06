//! Reads real consumer-group lag from Redpanda. Read-only: no subscribe, no commit.

use tayga_api::lag::{self, ALERT_GROUPS, GROUPS, LOG_GROUPS};

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
        assert_eq!(l.topic, "tayga.signals");
    }
}

#[tokio::test]
#[ignore = "needs Redpanda with tayga.alerts (make up); set TAYGA_IT_KAFKA to override localhost:19092"]
async fn fetch_all_reads_each_group_on_its_own_topic() {
    let brokers = std::env::var("TAYGA_IT_KAFKA").unwrap_or_else(|_| "localhost:19092".into());
    let lags = lag::fetch_all(&brokers, "tayga.signals", "tayga.logs")
        .await
        .unwrap();
    let groups: Vec<&str> = lags.iter().map(|l| l.group.as_str()).collect();
    let expected: Vec<&str> = GROUPS
        .iter()
        .chain(&LOG_GROUPS)
        .chain(&ALERT_GROUPS)
        .copied()
        .collect();
    assert_eq!(groups, expected);
    let topics: Vec<&str> = lags.iter().map(|l| l.topic.as_str()).collect();
    let mut want = vec!["tayga.signals"; GROUPS.len()];
    want.extend(["tayga.logs"; LOG_GROUPS.len()]);
    want.extend([lag::ALERTS_TOPIC; ALERT_GROUPS.len()]);
    assert_eq!(topics, want);
}
