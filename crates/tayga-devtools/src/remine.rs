//! `tayga-devtools remine`: rebuilds the log templates from the stored logs of the last 3 days
//! with the logminer's current Drain configuration (spec 7b §3). The mining is
//! `tayga_logminer::miner::Miner`; this module only pages the logs and writes the result.

use anyhow::Context;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::time::Instant;
use tayga_drain::drain::DrainConfig;
use tayga_drain::preprocess::masking_version;
use tayga_logminer::config::{
    KEY_EPOCH_START, KEY_HEARTBEAT, KEY_MASKING_VERSION, KEY_WATERMARK, Watermarks, watermark_key,
};
use tayga_logminer::miner::Miner;
use tayga_store::logs::LogMineRow;
use tayga_store::rows::LogRow;
use tayga_store::store::Store;

/// Logs per page, also the hits written per insert.
pub const BATCH: u32 = 10_000;
const NS_PER_SEC: i64 = 1_000_000_000;
/// Logs older than this are past the hits TTL and are not re-mined.
const WINDOW_NS: i64 = 3 * 86_400 * NS_PER_SEC;
/// A logminer heartbeat younger than this means the logminer is alive.
const HEARTBEAT_MAX_AGE_NS: i64 = 3 * 60 * NS_PER_SEC;

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub dry_run: bool,
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemineSummary {
    pub logs_read: u64,
    pub templates_before: usize,
    pub templates_after: usize,
    pub added: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub hits: u64,
    /// Enabled silence settings whose template id does not exist after the re-mine.
    pub orphaned_silence: Vec<u64>,
    pub secs: f64,
    /// Templates `(before, after)` per service.
    pub by_service: BTreeMap<String, (usize, usize)>,
}

impl RemineSummary {
    pub fn render(&self, dry_run: bool) -> String {
        let mut out = String::new();
        let mode = if dry_run {
            "dry run, nothing written"
        } else {
            "written"
        };
        let _ = writeln!(out, "remine summary ({mode})");
        let _ = writeln!(out, "logs read:        {}", self.logs_read);
        let _ = writeln!(
            out,
            "templates:        {} before, {} after",
            self.templates_before, self.templates_after
        );
        let _ = writeln!(
            out,
            "by id:            {} added, {} removed, {} unchanged",
            self.added, self.removed, self.unchanged
        );
        let _ = writeln!(
            out,
            "hits {}:   {}",
            if dry_run { "mined" } else { "written" },
            self.hits
        );
        for (service, (before, after)) in &self.by_service {
            let _ = writeln!(out, "  {service}: {before} -> {after}");
        }
        if self.orphaned_silence.is_empty() {
            let _ = writeln!(out, "orphaned silence settings: none");
        } else {
            let ids: Vec<String> = self.orphaned_silence.iter().map(u64::to_string).collect();
            let _ = writeln!(
                out,
                "orphaned silence settings: {} ({})",
                ids.len(),
                ids.join(", ")
            );
        }
        let _ = writeln!(out, "duration:         {:.1}s", self.secs);
        if !dry_run {
            let _ = writeln!(out, "Start the logminer again now.");
        }
        out
    }
}

/// The newest of the heartbeat values read by prefix [`KEY_HEARTBEAT`]: every replica's key and
/// the legacy single-replica key. `None` when there is none.
pub fn newest_heartbeat(rows: &[(String, i64)]) -> Option<i64> {
    rows.iter().map(|&(_, v)| v).max()
}

/// Refuses while a logminer replica is alive: its last heartbeat is under 3 minutes old. A
/// heartbeat in the future counts as fresh.
pub fn heartbeat_ok(heartbeat_ns: Option<i64>, now_ns: i64, force: bool) -> Result<(), String> {
    let Some(hb) = heartbeat_ns else {
        return Ok(());
    };
    let age = now_ns.saturating_sub(hb);
    if age >= HEARTBEAT_MAX_AGE_NS || force {
        return Ok(());
    }
    Err(format!(
        "the logminer looks alive (heartbeat {}s ago, limit {}s). Stop it first with \
         `docker compose ... stop tayga-logminer`, or pass --force",
        (age / NS_PER_SEC).max(0),
        HEARTBEAT_MAX_AGE_NS / NS_PER_SEC
    ))
}

/// `(added, removed, unchanged)` template ids between the stored and the re-mined set.
pub fn diff_ids(before: &BTreeSet<u64>, after: &BTreeSet<u64>) -> (usize, usize, usize) {
    let unchanged = before.intersection(after).count();
    (after.len() - unchanged, before.len() - unchanged, unchanged)
}

/// Enabled silence settings (ids, ascending) that name no template of `after`.
pub fn orphaned(enabled: &[u64], after: &BTreeSet<u64>) -> Vec<u64> {
    let mut ids: Vec<u64> = enabled
        .iter()
        .copied()
        .filter(|id| !after.contains(id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn log_row(r: LogMineRow) -> LogRow {
    LogRow {
        log_id: r.log_id,
        ts: r.ts,
        observed_ts: 0,
        trace_id: r.trace_id,
        span_id: r.span_id,
        severity_number: r.severity_number,
        severity_text: String::new(),
        service_name: r.service_name,
        body: r.body,
        resource_attrs: Vec::new(),
        log_attrs: Vec::new(),
    }
}

/// Mines the stored logs of the last 3 days into fresh templates. With `opts.dry_run` nothing is
/// written and the logminer heartbeat is not checked (read-only, safe on a live stack).
/// Otherwise it refuses while the logminer is alive (unless `opts.force`), the three template
/// tables are truncated first, and afterwards the watermark (the legacy key and every stored
/// per-partition key), the masking epoch start (`now_ns`) and the masking version are stored.
/// The logminer is alive when any replica's heartbeat is fresh.
pub async fn remine(
    store: &Store,
    drain: &DrainConfig,
    opts: Options,
    now_ns: i64,
) -> anyhow::Result<RemineSummary> {
    remine_paged(store, drain, opts, now_ns, BATCH).await
}

/// [`remine`] reading `batch` logs per page; tests use a small one to span several pages.
///
/// Templates written after page `n` (from 0) carry version `now_ns + n`. A template that changes
/// on several pages is upserted once per page into `log_templates`
/// (`ReplacingMergeTree(version)`), so the versions must strictly increase for the row of the
/// last page, with its final `count` and `last_seen`, to win without an equal-version tie.
pub async fn remine_paged(
    store: &Store,
    drain: &DrainConfig,
    opts: Options,
    now_ns: i64,
    batch: u32,
) -> anyhow::Result<RemineSummary> {
    anyhow::ensure!(batch > 0, "remine: the page size must be positive");
    let started = Instant::now();
    let heartbeat = if opts.dry_run {
        None
    } else {
        let rows = store
            .state_get_prefix(KEY_HEARTBEAT)
            .await
            .context("read logminer heartbeats")?;
        newest_heartbeat(&rows)
    };
    if !opts.dry_run {
        heartbeat_ok(heartbeat, now_ns, opts.force).map_err(anyhow::Error::msg)?;
    }

    let stored = store
        .load_templates()
        .await
        .context("load stored templates")?;
    let before: BTreeSet<u64> = stored.iter().map(|t| t.template_id).collect();
    let mut by_service: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for t in &stored {
        by_service.entry(t.service.clone()).or_default().0 += 1;
    }
    let enabled: Vec<u64> = store
        .silence_enabled()
        .await
        .context("load silence settings")?
        .into_iter()
        .map(|(id, _)| id)
        .collect();

    if !opts.dry_run {
        store
            .truncate_templates()
            .await
            .context("truncate template tables")?;
    }

    let mut miner = Miner::new(drain.clone());
    // Template id -> service, over every template the miner created.
    let mut mined: BTreeMap<u64, String> = BTreeMap::new();
    let (mut logs_read, mut hits) = (0u64, 0u64);
    let mut max_ts: Option<i64> = None;
    let (mut after_ts, mut after_id) = (now_ns.saturating_sub(WINDOW_NS) - 1, u64::MAX);
    let mut version = now_ns;
    loop {
        let page = store
            .logs_batch(after_ts, after_id, batch)
            .await
            .context("read logs")?;
        let Some(last) = page.last() else { break };
        (after_ts, after_id) = (last.ts, last.log_id);
        let full = page.len() == batch as usize;
        logs_read += page.len() as u64;
        let mut page_hits = Vec::with_capacity(page.len());
        for row in page {
            let (hit, _) = miner.mine(&log_row(row));
            max_ts = max_ts.max(Some(hit.ts));
            page_hits.push(hit);
        }
        hits += page_hits.len() as u64;
        let templates = miner.dirty_templates(version);
        version = version.saturating_add(1);
        for t in &templates {
            mined.insert(t.template_id, t.service.clone());
        }
        if !opts.dry_run {
            store
                .insert_log_hits(&page_hits)
                .await
                .context("insert log hits")?;
            store
                .upsert_templates(&templates)
                .await
                .context("insert templates")?;
        }
        if !full {
            break;
        }
    }

    let after: BTreeSet<u64> = mined.keys().copied().collect();
    for service in mined.values() {
        by_service.entry(service.clone()).or_default().1 += 1;
    }
    let (added, removed, unchanged) = diff_ids(&before, &after);

    if !opts.dry_run {
        if let Some(ts) = max_ts {
            // The per-partition keys take precedence over the legacy one, so each stored one is
            // moved to the re-mined watermark too.
            let stored = store
                .state_get_prefix(KEY_WATERMARK)
                .await
                .context("read watermarks")?;
            let mut entries = vec![(KEY_WATERMARK.to_string(), ts)];
            entries.extend(
                Watermarks::from_rows(&stored)
                    .partitions
                    .keys()
                    .map(|&p| (watermark_key(p), ts)),
            );
            store
                .state_put_many(&entries)
                .await
                .context("store watermark")?;
        }
        // The epoch start goes first, as in the logminer: a crash between the writes is
        // re-detected as a masking change.
        store
            .state_put(KEY_EPOCH_START, now_ns)
            .await
            .context("store masking epoch")?;
        store
            .state_put(
                KEY_MASKING_VERSION,
                i64::from(masking_version(drain.keep_http_status)),
            )
            .await
            .context("store masking version")?;
    }

    Ok(RemineSummary {
        logs_read,
        templates_before: before.len(),
        templates_after: after.len(),
        added,
        removed,
        unchanged,
        hits,
        orphaned_silence: orphaned(&enabled, &after),
        secs: started.elapsed().as_secs_f64(),
        by_service,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000 * NS_PER_SEC;

    #[test]
    fn no_heartbeat_is_ok() {
        assert!(heartbeat_ok(None, NOW, false).is_ok());
    }

    #[test]
    fn an_old_heartbeat_is_ok() {
        assert!(heartbeat_ok(Some(NOW - 3 * 60 * NS_PER_SEC), NOW, false).is_ok());
        assert!(heartbeat_ok(Some(NOW - 3_600 * NS_PER_SEC), NOW, false).is_ok());
    }

    #[test]
    fn a_fresh_heartbeat_refuses_and_names_the_fix() {
        let e = heartbeat_ok(Some(NOW - 30 * NS_PER_SEC), NOW, false).unwrap_err();
        assert!(e.contains("stop tayga-logminer"), "{e}");
        assert!(e.contains("--force"), "{e}");
        let just_under = NOW - (3 * 60 * NS_PER_SEC - 1);
        assert!(heartbeat_ok(Some(just_under), NOW, false).is_err());
        assert!(heartbeat_ok(Some(NOW + 5 * NS_PER_SEC), NOW, false).is_err());
    }

    #[test]
    fn the_freshest_of_all_replica_heartbeats_decides() {
        let rows = |v: &[(&str, i64)]| -> Vec<(String, i64)> {
            v.iter().map(|&(k, n)| (k.to_string(), n)).collect()
        };
        assert_eq!(newest_heartbeat(&[]), None);
        // The legacy key is stale, one replica is stale, another is fresh: refuse.
        let fresh = rows(&[
            ("logminer_heartbeat_ns", NOW - 3_600 * NS_PER_SEC),
            ("logminer_heartbeat_ns:a", NOW - 600 * NS_PER_SEC),
            ("logminer_heartbeat_ns:b", NOW - 10 * NS_PER_SEC),
        ]);
        assert_eq!(newest_heartbeat(&fresh), Some(NOW - 10 * NS_PER_SEC));
        assert!(heartbeat_ok(newest_heartbeat(&fresh), NOW, false).is_err());
        // Only the legacy key is fresh (a logminer from before replicas): refuse too.
        let legacy = rows(&[
            ("logminer_heartbeat_ns", NOW - 10 * NS_PER_SEC),
            ("logminer_heartbeat_ns:a", NOW - 600 * NS_PER_SEC),
        ]);
        assert!(heartbeat_ok(newest_heartbeat(&legacy), NOW, false).is_err());
        let stale = rows(&[
            ("logminer_heartbeat_ns:a", NOW - 600 * NS_PER_SEC),
            ("logminer_heartbeat_ns:b", NOW - 300 * NS_PER_SEC),
        ]);
        assert!(heartbeat_ok(newest_heartbeat(&stale), NOW, false).is_ok());
    }

    #[test]
    fn force_overrides_a_fresh_heartbeat() {
        assert!(heartbeat_ok(Some(NOW - NS_PER_SEC), NOW, true).is_ok());
    }

    fn set(v: &[u64]) -> BTreeSet<u64> {
        v.iter().copied().collect()
    }

    #[test]
    fn diff_counts_added_removed_and_unchanged_ids() {
        assert_eq!(diff_ids(&set(&[1, 2, 3]), &set(&[2, 3, 4, 5])), (2, 1, 2));
        assert_eq!(diff_ids(&set(&[]), &set(&[7])), (1, 0, 0));
        assert_eq!(diff_ids(&set(&[7]), &set(&[])), (0, 1, 0));
        assert_eq!(diff_ids(&set(&[1, 2]), &set(&[1, 2])), (0, 0, 2));
    }

    #[test]
    fn orphaned_silence_ids_are_those_missing_after_the_remine() {
        assert_eq!(orphaned(&[9, 1, 2, 9], &set(&[2, 3])), vec![1, 9]);
        assert!(orphaned(&[], &set(&[1])).is_empty());
    }

    #[test]
    fn render_marks_dry_runs_and_lists_orphans() {
        let s = RemineSummary {
            logs_read: 5,
            templates_before: 2,
            templates_after: 3,
            added: 2,
            removed: 1,
            unchanged: 1,
            hits: 5,
            orphaned_silence: vec![4, 8],
            secs: 0.25,
            by_service: BTreeMap::from([("api".to_string(), (2, 3))]),
        };
        let dry = s.render(true);
        assert!(dry.contains("dry run, nothing written"));
        assert!(dry.contains("api: 2 -> 3"));
        assert!(dry.contains("2 (4, 8)"));
        assert!(!dry.contains("Start the logminer"));
        assert!(s.render(false).contains("Start the logminer again"));
    }
}
