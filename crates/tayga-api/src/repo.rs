//! Read side over the tables written by the writer and the assembler.

use crate::model::*;
use crate::params::GroupFilter;
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

/// Attaches per-minute counts to their groups, preserving group order.
pub fn merge_minutes(groups: Vec<StoryGroupRow>, minutes: Vec<GroupMinuteRow>) -> Vec<GroupView> {
    let mut by_fp: HashMap<String, Vec<(u32, u64)>> = HashMap::new();
    for m in minutes {
        by_fp
            .entry(m.fingerprint)
            .or_default()
            .push((m.minute, m.stories));
    }
    groups
        .into_iter()
        .map(|group| {
            let mut per_minute = by_fp.remove(&group.fingerprint).unwrap_or_default();
            per_minute.sort_unstable();
            GroupView { group, per_minute }
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
        let minutes: Vec<GroupMinuteRow> = bind(self.client.query(&format!(
            "SELECT toString(fingerprint) AS fingerprint, toUInt32(toStartOfMinute(ts)) AS minute, count() AS stories \
             FROM ({FILTERED}) GROUP BY fingerprint, minute ORDER BY minute"
        )))
        .fetch_all()
        .await?;
        Ok(merge_minutes(groups, minutes))
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
    fn merge_minutes_attaches_sorted_points_and_keeps_order() {
        let minutes = vec![
            GroupMinuteRow {
                fingerprint: "2".into(),
                minute: 120,
                stories: 3,
            },
            GroupMinuteRow {
                fingerprint: "1".into(),
                minute: 60,
                stories: 1,
            },
            GroupMinuteRow {
                fingerprint: "2".into(),
                minute: 60,
                stories: 2,
            },
        ];
        let out = merge_minutes(vec![group("2"), group("1"), group("3")], minutes);
        assert_eq!(
            out.iter()
                .map(|g| g.group.fingerprint.as_str())
                .collect::<Vec<_>>(),
            ["2", "1", "3"]
        );
        assert_eq!(out[0].per_minute, vec![(60, 2), (120, 3)]);
        assert!(out[2].per_minute.is_empty());
    }
}
