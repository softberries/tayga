//! Settings and `logminer_state` keys shared with `tayga-devtools remine`, so a re-mine uses
//! exactly the Drain configuration the logminer runs with.

use serde::Deserialize;
use std::collections::BTreeMap;
use tayga_drain::drain::DrainConfig;

/// The legacy global new-template watermark, and the prefix of the per-partition keys
/// ([`watermark_key`]). The legacy value seeds a partition that has no key of its own.
pub const KEY_WATERMARK: &str = "new_template_watermark_ns";
/// Liveness stamps for `tayga-devtools remine`, written after every detection pass: the prefix
/// of the per-replica keys ([`heartbeat_key`]), and the legacy single-replica key.
pub const KEY_HEARTBEAT: &str = "logminer_heartbeat_ns";
pub const KEY_MASKING_VERSION: &str = "masking_version";
pub const KEY_EPOCH_START: &str = "masking_epoch_start_ns";

/// New-template watermark key of one `tayga.logs` partition: `new_template_watermark_ns:p<N>`.
pub fn watermark_key(partition: i32) -> String {
    format!("{KEY_WATERMARK}:p{partition}")
}

/// Heartbeat key of one logminer replica: `logminer_heartbeat_ns:<replica_id>`.
pub fn heartbeat_key(replica_id: &str) -> String {
    format!("{KEY_HEARTBEAT}:{replica_id}")
}

/// Stored watermarks, as read by key prefix [`KEY_WATERMARK`].
#[derive(Debug, Default, PartialEq)]
pub struct Watermarks {
    /// The legacy global key.
    pub legacy: Option<i64>,
    /// Per-partition keys, by partition.
    pub partitions: BTreeMap<i32, i64>,
}

impl Watermarks {
    /// Sorts `(key, value)` rows into the legacy and per-partition values; other keys are
    /// ignored.
    pub fn from_rows(rows: &[(String, i64)]) -> Self {
        let mut out = Self::default();
        for (key, value) in rows {
            if key == KEY_WATERMARK {
                out.legacy = Some(*value);
            } else if let Some(p) = key
                .strip_prefix(KEY_WATERMARK)
                .and_then(|rest| rest.strip_prefix(":p"))
                .and_then(|n| n.parse::<i32>().ok())
            {
                out.partitions.insert(p, *value);
            }
        }
        out
    }

    /// The watermark of a replica assigned `assigned` (spec §2.2): the minimum over the
    /// partitions, each its own stored value, else the legacy value, else `initial` (nothing
    /// stored yet). Never ahead of `now_ns`. With no partition assigned, the legacy value or
    /// `initial`: the replica owns nothing, so the value is not used until the next assignment.
    pub fn for_partitions(&self, assigned: &[i32], initial: i64, now_ns: i64) -> i64 {
        let seed = self.legacy.unwrap_or(initial);
        assigned
            .iter()
            .map(|p| self.partitions.get(p).copied().unwrap_or(seed))
            .min()
            .unwrap_or(seed)
            .min(now_ns)
    }
}

/// The Drain keys of `[logminer]`. Missing keys take the [`DrainConfig`] defaults; other keys of
/// the section are ignored here.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct DrainSettings {
    pub sim_threshold: f64,
    pub max_clusters_per_service: usize,
    pub keep_http_status: bool,
}

impl Default for DrainSettings {
    fn default() -> Self {
        let drain = DrainConfig::default();
        Self {
            sim_threshold: drain.sim_threshold,
            max_clusters_per_service: drain.max_clusters_per_service,
            keep_http_status: drain.keep_http_status,
        }
    }
}

impl DrainSettings {
    /// Checks shared by the logminer and `remine`.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (0.0..=1.0).contains(&self.sim_threshold),
            "logminer.sim_threshold must be within 0..=1"
        );
        anyhow::ensure!(
            self.max_clusters_per_service > 0,
            "logminer.max_clusters_per_service must be positive"
        );
        Ok(())
    }

    pub fn drain(&self) -> DrainConfig {
        DrainConfig {
            sim_threshold: self.sim_threshold,
            max_clusters_per_service: self.max_clusters_per_service,
            keep_http_status: self.keep_http_status,
            ..DrainConfig::default()
        }
    }

    /// Reads `[logminer]` the way the logminer does (`TAYGA_CONFIG` file, then `TAYGA__` env).
    pub fn load() -> anyhow::Result<Self> {
        #[derive(Deserialize, Default)]
        struct Root {
            #[serde(default)]
            logminer: DrainSettings,
        }
        let root: Root = tayga_common::load_settings()?;
        root.logminer.validate()?;
        Ok(root.logminer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_keys_are_suffixed_by_partition_and_replica() {
        assert_eq!(watermark_key(7), "new_template_watermark_ns:p7");
        assert_eq!(heartbeat_key("host-1"), "logminer_heartbeat_ns:host-1");
    }

    fn rows(v: &[(&str, i64)]) -> Vec<(String, i64)> {
        v.iter().map(|&(k, n)| (k.to_string(), n)).collect()
    }

    #[test]
    fn watermark_rows_split_into_legacy_and_partitions() {
        let w = Watermarks::from_rows(&rows(&[
            ("new_template_watermark_ns", 5),
            ("new_template_watermark_ns:p0", 10),
            ("new_template_watermark_ns:p11", 30),
            ("new_template_watermark_ns:px", 99),
            ("new_template_watermark_ns_other", 99),
        ]));
        assert_eq!(w.legacy, Some(5));
        assert_eq!(w.partitions, BTreeMap::from([(0, 10), (11, 30)]));
    }

    #[test]
    fn the_watermark_is_the_minimum_over_the_assigned_partitions() {
        let w = Watermarks::from_rows(&rows(&[
            ("new_template_watermark_ns:p0", 40),
            ("new_template_watermark_ns:p1", 20),
            ("new_template_watermark_ns:p2", 30),
        ]));
        assert_eq!(w.for_partitions(&[0, 2], 1, 1_000), 30);
        // Scale 2 -> 1: the remaining replica takes every partition and their minimum.
        assert_eq!(w.for_partitions(&[0, 1, 2], 1, 1_000), 20);
        assert_eq!(w.for_partitions(&[0, 1, 2], 1, 15), 15);
        assert_eq!(
            w.for_partitions(&[0], 1, 35),
            35,
            "never ahead of the wall clock"
        );
    }

    #[test]
    fn the_legacy_key_seeds_partitions_without_a_value() {
        let w = Watermarks::from_rows(&rows(&[
            ("new_template_watermark_ns", 15),
            ("new_template_watermark_ns:p0", 40),
        ]));
        assert_eq!(w.for_partitions(&[0, 5], 1, 1_000), 15);
        assert_eq!(w.for_partitions(&[0], 1, 1_000), 40);
        assert_eq!(w.for_partitions(&[], 1, 1_000), 15);
        // Nothing stored at all: the fresh-install initial watermark.
        let none = Watermarks::default();
        assert_eq!(none.for_partitions(&[3, 4], 7, 1_000), 7);
        // Partition keys without a legacy key: a partition with none takes `initial`.
        let partial = Watermarks::from_rows(&rows(&[("new_template_watermark_ns:p0", 40)]));
        assert_eq!(partial.for_partitions(&[0, 1], 7, 1_000), 7);
    }

    #[test]
    fn defaults_match_the_drain_defaults() {
        assert_eq!(DrainSettings::default().drain(), DrainConfig::default());
    }

    #[test]
    fn invalid_drain_settings_are_rejected() {
        let bad = |json: &str| {
            serde_json::from_str::<DrainSettings>(json)
                .unwrap()
                .validate()
                .is_err()
        };
        assert!(bad(r#"{"sim_threshold":1.5}"#));
        assert!(bad(r#"{"sim_threshold":-0.1}"#));
        assert!(bad(r#"{"max_clusters_per_service":0}"#));
        assert!(!bad(r#"{"sim_threshold":1.0}"#));
        assert!(DrainSettings::default().validate().is_ok());
    }

    #[test]
    fn other_logminer_keys_are_ignored() {
        let s: DrainSettings =
            serde_json::from_str(r#"{"sim_threshold":0.7,"max_batch":10,"flush_ms":5}"#).unwrap();
        assert_eq!(s.drain().sim_threshold, 0.7);
        assert_eq!(s.max_clusters_per_service, 5_000);
    }
}
