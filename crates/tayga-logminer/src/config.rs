//! Settings and `logminer_state` keys shared with `tayga-devtools remine`, so a re-mine uses
//! exactly the Drain configuration the logminer runs with.

use serde::Deserialize;
use tayga_drain::drain::DrainConfig;

pub const KEY_WATERMARK: &str = "new_template_watermark_ns";
/// Liveness stamp for `tayga-devtools remine`: written after every detection pass.
pub const KEY_HEARTBEAT: &str = "logminer_heartbeat_ns";
pub const KEY_MASKING_VERSION: &str = "masking_version";
pub const KEY_EPOCH_START: &str = "masking_epoch_start_ns";

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
