//! Consumer-group lag read straight from the broker: watermarks per partition plus each group's
//! committed offsets. Nothing here subscribes to a group or commits.

use anyhow::Context;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{BaseConsumer, Consumer};
use rdkafka::{Offset, TopicPartitionList};
use serde::Serialize;
use std::time::Duration;

pub const GROUPS: [&str; 3] = ["tayga-writer", "tayga-assembler", "tayga-logminer"];

const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lag {
    pub group: String,
    pub committed: i64,
    pub end: i64,
    pub lag: i64,
}

/// Sums `(committed, high)` pairs into `(committed, end, lag)`. A negative committed offset
/// means the group never committed on that partition, so the whole partition counts as lag and
/// contributes nothing to `committed`. Lag per partition never goes below 0.
pub fn sum(partitions: &[(i64, i64)]) -> (i64, i64, i64) {
    partitions
        .iter()
        .fold((0, 0, 0), |(c, e, l), &(committed, high)| {
            if committed < 0 {
                (c, e + high, l + high.max(0))
            } else {
                (c + committed, e + high, l + (high - committed).max(0))
            }
        })
}

fn consumer(brokers: &str, group: &str) -> anyhow::Result<BaseConsumer> {
    Ok(ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("group.id", group)
        .set("enable.auto.commit", "false")
        .create()?)
}

fn fetch_blocking(brokers: &str, topic: &str, groups: &[String]) -> anyhow::Result<Vec<Lag>> {
    let probe = consumer(brokers, "tayga-api-lag")?;
    let meta = probe
        .fetch_metadata(Some(topic), CALL_TIMEOUT)
        .context("fetch topic metadata")?;
    let topic_meta = meta
        .topics()
        .iter()
        .find(|t| t.name() == topic)
        .with_context(|| format!("topic {topic} not found"))?;
    let ids: Vec<i32> = topic_meta.partitions().iter().map(|p| p.id()).collect();
    anyhow::ensure!(!ids.is_empty(), "topic {topic} has no partitions");
    let highs = ids
        .iter()
        .map(|&p| {
            probe
                .fetch_watermarks(topic, p, CALL_TIMEOUT)
                .map(|(_, high)| high)
                .with_context(|| format!("watermarks for {topic}[{p}]"))
        })
        .collect::<anyhow::Result<Vec<i64>>>()?;
    let mut tpl = TopicPartitionList::new();
    for &p in &ids {
        tpl.add_partition(topic, p);
    }
    groups
        .iter()
        .map(|group| {
            let c = consumer(brokers, group)?;
            let committed = c
                .committed_offsets(tpl.clone(), CALL_TIMEOUT)
                .with_context(|| format!("committed offsets for {group}"))?;
            let pairs: Vec<(i64, i64)> = ids
                .iter()
                .zip(&highs)
                .map(|(&p, &high)| {
                    let off = committed
                        .find_partition(topic, p)
                        .map(|e| e.offset())
                        .unwrap_or(Offset::Invalid);
                    (
                        match off {
                            Offset::Offset(o) => o,
                            _ => -1,
                        },
                        high,
                    )
                })
                .collect();
            let (committed, end, lag) = sum(&pairs);
            Ok(Lag {
                group: group.clone(),
                committed,
                end,
                lag,
            })
        })
        .collect()
}

/// Lag per group on `topic`, bounded to 5 s overall.
pub async fn fetch(brokers: &str, topic: &str, groups: &[&str]) -> anyhow::Result<Vec<Lag>> {
    let (brokers, topic) = (brokers.to_owned(), topic.to_owned());
    let groups: Vec<String> = groups.iter().map(|g| (*g).to_owned()).collect();
    let task = tokio::task::spawn_blocking(move || fetch_blocking(&brokers, &topic, &groups));
    tokio::time::timeout(TOTAL_TIMEOUT, task)
        .await
        .context("kafka lag fetch timed out")??
}

#[cfg(test)]
mod tests {
    use super::sum;

    #[test]
    fn sums_committed_end_and_lag() {
        assert_eq!(sum(&[(5, 10), (7, 7)]), (12, 17, 5));
    }

    #[test]
    fn negative_committed_counts_whole_partition_as_lag() {
        assert_eq!(sum(&[(-1, 10), (3, 4)]), (3, 14, 11));
    }

    #[test]
    fn lag_never_negative() {
        assert_eq!(sum(&[(12, 10)]), (12, 10, 0));
        assert_eq!(sum(&[(-1, 0)]), (0, 0, 0));
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(sum(&[]), (0, 0, 0));
    }
}
