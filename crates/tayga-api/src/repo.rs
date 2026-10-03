//! Read side over the tables written by the writer and the assembler.

use crate::model::*;
use crate::params::{GroupFilter, bucket_secs};
use std::collections::HashMap;
use std::future::Future;
use tayga_store::ClickHouseSettings;

pub trait Repo: Send + Sync + 'static {
    fn story_groups(
        &self,
        f: &GroupFilter,
    ) -> impl Future<Output = anyhow::Result<Vec<GroupView>>> + Send;
    fn story_group(
        &self,
        fingerprint: &str,
        since_secs: u32,
    ) -> impl Future<Output = anyhow::Result<Option<GroupDetail>>> + Send;
    fn story(
        &self,
        story_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<StoryView>>> + Send;
    fn trace(&self, trace_id: &str) -> impl Future<Output = anyhow::Result<TraceView>> + Send;
    fn service_map(
        &self,
        since_secs: u32,
    ) -> impl Future<Output = anyhow::Result<Vec<EdgeView>>> + Send;
}

/// Attaches bucketed counts to their groups, preserving group order.
pub fn merge_buckets(
    groups: Vec<StoryGroupRow>,
    rows: Vec<GroupBucketRow>,
    bucket_secs: u32,
) -> Vec<GroupView> {
    let mut by_fp: HashMap<String, Vec<(u32, u64)>> = HashMap::new();
    for r in rows {
        by_fp
            .entry(r.fingerprint)
            .or_default()
            .push((r.bucket, r.stories));
    }
    groups
        .into_iter()
        .map(|group| {
            let mut buckets = by_fp.remove(&group.fingerprint).unwrap_or_default();
            buckets.sort_unstable();
            GroupView {
                group,
                bucket_secs,
                buckets,
            }
        })
        .collect()
}

/// Filtered stories as a subquery so outer aliases never shadow filter columns.
const FILTERED: &str = "SELECT * FROM error_stories FINAL WHERE ts > now64(9) - toIntervalSecond(?) \
     AND (? = '' OR toString(kind) = ?) AND (? = '' OR rc_service = ?) AND (? = '' OR toString(fingerprint) = ?)";

const GROUP_COLUMNS: &str = "toString(fingerprint) AS fingerprint, toString(any(kind)) AS kind, \
     argMax(summary, ts) AS summary, any(rc_service) AS rc_service, any(rc_span_name) AS rc_span_name, \
     any(endpoint_service) AS endpoint_service, any(endpoint_name) AS endpoint_name, count() AS stories, \
     toUnixTimestamp64Nano(min(ts)) AS first_seen_ns, toUnixTimestamp64Nano(max(ts)) AS last_seen_ns, \
     argMax(story_id, ts) AS sample_story_id";

pub struct ChRepo {
    client: clickhouse::Client,
}

impl ChRepo {
    pub fn new(s: &ClickHouseSettings) -> Self {
        Self {
            client: clickhouse::Client::default()
                .with_url(&s.url)
                .with_database(&s.database),
        }
    }

    async fn groups(&self, f: &GroupFilter, fingerprint: &str) -> anyhow::Result<Vec<GroupView>> {
        let kind = f.kind.clone().unwrap_or_default();
        let service = f.service.clone().unwrap_or_default();
        let bind = |q: clickhouse::query::Query| {
            q.bind(f.since_secs)
                .bind(kind.as_str())
                .bind(kind.as_str())
                .bind(service.as_str())
                .bind(service.as_str())
                .bind(fingerprint)
                .bind(fingerprint)
        };
        let groups: Vec<StoryGroupRow> = bind(self.client.query(&format!(
            "SELECT {GROUP_COLUMNS} FROM ({FILTERED}) GROUP BY fingerprint ORDER BY stories DESC LIMIT 100"
        )))
        .fetch_all()
        .await?;
        if groups.is_empty() {
            return Ok(Vec::new());
        }
        // Only the groups kept above, bucketed to about 120 points per group.
        let step = bucket_secs(f.since_secs);
        let top: Vec<&str> = groups.iter().map(|g| g.fingerprint.as_str()).collect();
        // Placeholder order: bucket width, the FILTERED binds, then the fingerprint list.
        let query = self
            .client
            .query(&format!(
                "SELECT toString(fingerprint) AS fingerprint, \
                 toUInt32(toStartOfInterval(ts, toIntervalSecond(?))) AS bucket, count() AS stories \
                 FROM ({FILTERED} AND has(?, toString(fingerprint))) GROUP BY fingerprint, bucket ORDER BY bucket"
            ))
            .bind(step);
        let rows: Vec<GroupBucketRow> = bind(query).bind(&top).fetch_all().await?;
        Ok(merge_buckets(groups, rows, step))
    }
}

impl Repo for ChRepo {
    async fn story_groups(&self, f: &GroupFilter) -> anyhow::Result<Vec<GroupView>> {
        self.groups(f, "").await
    }

    async fn story_group(
        &self,
        fingerprint: &str,
        since_secs: u32,
    ) -> anyhow::Result<Option<GroupDetail>> {
        let f = GroupFilter {
            since_secs,
            kind: None,
            service: None,
        };
        let Some(group) = self.groups(&f, fingerprint).await?.into_iter().next() else {
            return Ok(None);
        };
        let examples: Vec<StorySummaryRow> = self
            .client
            .query(
                "SELECT story_id, toUnixTimestamp64Nano(ts) AS ts_ns, trace_id, duration_ns, summary \
                 FROM error_stories FINAL WHERE fingerprint = toUInt64(?) ORDER BY ts DESC LIMIT 20",
            )
            .bind(fingerprint)
            .fetch_all()
            .await?;
        Ok(Some(GroupDetail { group, examples }))
    }

    async fn story(&self, story_id: &str) -> anyhow::Result<Option<StoryView>> {
        let rows: Vec<StoryRecordRow> = self
            .client
            .query(
                "SELECT story_id, toString(fingerprint) AS fingerprint, toString(kind) AS kind, \
                 toUnixTimestamp64Nano(ts) AS ts_ns, trace_id, endpoint_service, endpoint_name, rc_service, \
                 rc_span_name, rc_span_kind, rc_span_id, rc_message, rc_exception_type, summary, duration_ns, \
                 path_services, path_spans, critical_path, baseline_diff, logs, also_failed, span_count, flags \
                 FROM error_stories FINAL WHERE story_id = ? LIMIT 1",
            )
            .bind(story_id)
            .fetch_all()
            .await?;
        Ok(rows.into_iter().next().map(StoryView::from_record))
    }

    async fn trace(&self, trace_id: &str) -> anyhow::Result<TraceView> {
        let spans: Vec<TraceSpanRow> = self
            .client
            .query(
                "SELECT span_id, parent_span_id, service_name, span_name, toString(kind) AS kind, \
                 toUnixTimestamp64Nano(start_ts) AS start_ns, duration_ns, toString(status_code) AS status, status_message \
                 FROM spans WHERE trace_id = ? ORDER BY start_ts LIMIT 1 BY span_id LIMIT 10000",
            )
            .bind(trace_id)
            .fetch_all()
            .await?;
        let logs: Vec<TraceLogRow> = self
            .client
            .query(
                "SELECT toUnixTimestamp64Nano(ts) AS ts_ns, span_id, service_name, severity_number, severity_text, body \
                 FROM logs WHERE trace_id = ? ORDER BY ts LIMIT 1 BY log_id LIMIT 1000",
            )
            .bind(trace_id)
            .fetch_all()
            .await?;
        Ok(TraceView {
            trace_id: trace_id.to_string(),
            spans,
            logs,
        })
    }

    async fn service_map(&self, since_secs: u32) -> anyhow::Result<Vec<EdgeView>> {
        let rows: Vec<EdgeRow> = self
            .client
            .query(
                "SELECT parent_service, child_service, sum(calls) AS calls, sum(errors) AS errors, \
                 sum(duration_ns_sum) AS duration_ns_sum FROM service_edges \
                 WHERE minute > now() - toIntervalSecond(?) GROUP BY parent_service, child_service \
                 ORDER BY calls DESC LIMIT 500",
            )
            .bind(since_secs)
            .fetch_all()
            .await?;
        Ok(rows.into_iter().map(EdgeView::from_row).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(fp: &str) -> StoryGroupRow {
        StoryGroupRow {
            fingerprint: fp.into(),
            kind: "error".into(),
            summary: "s".into(),
            rc_service: "payment".into(),
            rc_span_name: "charge".into(),
            endpoint_service: "e".into(),
            endpoint_name: "n".into(),
            stories: 1,
            first_seen_ns: 0,
            last_seen_ns: 0,
            sample_story_id: "x".into(),
        }
    }

    #[test]
    fn merge_buckets_attaches_sorted_points_and_keeps_order() {
        let rows = vec![
            GroupBucketRow {
                fingerprint: "2".into(),
                bucket: 120,
                stories: 3,
            },
            GroupBucketRow {
                fingerprint: "1".into(),
                bucket: 60,
                stories: 1,
            },
            GroupBucketRow {
                fingerprint: "2".into(),
                bucket: 60,
                stories: 2,
            },
        ];
        let out = merge_buckets(vec![group("2"), group("1"), group("3")], rows, 60);
        assert_eq!(
            out.iter()
                .map(|g| g.group.fingerprint.as_str())
                .collect::<Vec<_>>(),
            ["2", "1", "3"]
        );
        assert_eq!(out[0].buckets, vec![(60, 2), (120, 3)]);
        assert_eq!(out[0].bucket_secs, 60);
        assert!(out[2].buckets.is_empty());
    }
}
