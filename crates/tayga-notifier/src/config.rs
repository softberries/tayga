//! `[notifier]` settings (spec 7b §4). A Slack incoming-webhook URL is a bearer credential, so
//! [`WebhookUrl`] never prints its value: `Debug` and `Display` show [`REDACTED`], and a target
//! shows as its `name`. Targets are a list of tables, so they come from the file config only.

use crate::payload::AlertKind;
use serde::Deserialize;
use std::collections::HashSet;
use std::fmt;
use std::net::SocketAddr;

pub const REDACTED: &str = "<redacted>";
const NAME_MAX: usize = 64;
/// Upper bound of `timeout_secs`: an attempt in flight at shutdown must finish, and its state be
/// written, inside the compose `stop_grace_period` (see `deliver::RECORD_GRACE`).
pub const TIMEOUT_MAX_SECS: u64 = 15;

/// A webhook URL. Read it with [`WebhookUrl::expose`] only to send a request.
#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct WebhookUrl(String);

impl WebhookUrl {
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for WebhookUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl fmt::Display for WebhookUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    Webhook,
    Slack,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Target {
    /// Logged, used as the `target` metric label and stored in `notifier_deliveries`.
    pub name: String,
    pub kind: TargetKind,
    pub url: WebhookUrl,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// Missing keys take the values of [`NotifierSettings::default`].
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct NotifierSettings {
    /// Base of the links back into the app.
    pub public_url: String,
    /// Alert kinds to deliver; others are skipped (and their offsets committed).
    pub kinds: Vec<AlertKind>,
    /// Attempts per alert and target, the first included.
    pub max_attempts: u32,
    /// Per-request timeout, `1..=TIMEOUT_MAX_SECS`.
    pub timeout_secs: u64,
    /// Alerts whose `last_at` is older than this are skipped (and committed): a first start with
    /// targets must not deliver the whole retained backlog of `tayga.alerts`.
    pub max_age_secs: u64,
    pub targets: Vec<Target>,
    pub alerts_topic: String,
    pub metrics_addr: SocketAddr,
}

impl Default for NotifierSettings {
    fn default() -> Self {
        Self {
            public_url: "http://localhost:8090".to_string(),
            kinds: AlertKind::ALL.to_vec(),
            max_attempts: 8,
            timeout_secs: 10,
            max_age_secs: 3_600,
            targets: Vec::new(),
            alerts_topic: "tayga.alerts".to_string(),
            metrics_addr: SocketAddr::from(([0, 0, 0, 0], 9100)),
        }
    }
}

impl NotifierSettings {
    /// Error messages name a target by its `name`, never by its URL.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_attempts > 0,
            "notifier.max_attempts must be positive"
        );
        anyhow::ensure!(
            (1..=TIMEOUT_MAX_SECS).contains(&self.timeout_secs),
            "notifier.timeout_secs must be within 1..={TIMEOUT_MAX_SECS}"
        );
        anyhow::ensure!(
            self.max_age_secs > 0,
            "notifier.max_age_secs must be positive"
        );
        anyhow::ensure!(
            is_http_url(&self.public_url),
            "notifier.public_url must be an http(s) URL"
        );
        let mut names = HashSet::new();
        for t in &self.targets {
            anyhow::ensure!(
                !t.name.is_empty()
                    && t.name.len() <= NAME_MAX
                    && t.name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
                "notifier.targets: a name must be 1-{NAME_MAX} of [A-Za-z0-9._-] (got {:?})",
                t.name
            );
            anyhow::ensure!(
                names.insert(t.name.as_str()),
                "notifier.targets: duplicate name {:?}",
                t.name
            );
            anyhow::ensure!(
                is_http_url(t.url.expose()),
                "notifier.targets[{}].url must be an http(s) URL with a host",
                t.name
            );
        }
        Ok(())
    }

    pub fn delivers(&self, kind: AlertKind) -> bool {
        self.kinds.contains(&kind)
    }
}

fn is_http_url(raw: &str) -> bool {
    reqwest::Url::parse(raw)
        .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "https://hooks.slack.com/services/T000/B000/XXXXSECRETXXXX";

    fn target(name: &str, url: &str) -> Target {
        Target {
            name: name.into(),
            kind: TargetKind::Slack,
            url: WebhookUrl::new(url),
        }
    }

    #[test]
    fn defaults_match_the_spec() {
        let s: NotifierSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.public_url, "http://localhost:8090");
        assert_eq!(s.kinds, AlertKind::ALL);
        assert_eq!((s.max_attempts, s.timeout_secs), (8, 10));
        assert_eq!(s.max_age_secs, 3_600);
        assert!(s.targets.is_empty());
        assert_eq!(s.alerts_topic, "tayga.alerts");
        assert_eq!(s.metrics_addr.port(), 9100);
        s.validate().unwrap();
    }

    #[test]
    fn the_deploy_file_has_no_targets() {
        #[derive(Deserialize)]
        struct Root {
            notifier: NotifierSettings,
        }
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deploy/tayga-notifier.toml"
        );
        let root: Root = config::Config::builder()
            .add_source(config::File::new(path, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let s = root.notifier;
        s.validate().unwrap();
        assert!(s.targets.is_empty());
        assert_eq!(s.kinds, AlertKind::ALL);
        assert_eq!((s.max_attempts, s.timeout_secs), (8, 10));
        assert_eq!(s.max_age_secs, 3_600);
    }

    #[test]
    fn parses_targets_and_kinds() {
        let s: NotifierSettings = serde_json::from_value(serde_json::json!({
            "kinds": ["silence"],
            "targets": [
                { "name": "ops-slack", "kind": "slack", "url": SECRET },
                { "name": "hook", "kind": "webhook", "url": "http://mock:9000/hook" }
            ]
        }))
        .unwrap();
        s.validate().unwrap();
        assert!(s.delivers(AlertKind::Silence) && !s.delivers(AlertKind::Spike));
        assert_eq!(s.targets[0].kind, TargetKind::Slack);
        assert_eq!(s.targets[1].url.expose(), "http://mock:9000/hook");
        assert!(serde_json::from_str::<NotifierSettings>(r#"{"kinds":["nope"]}"#).is_err());
    }

    #[test]
    fn debug_and_display_never_show_the_url() {
        let t = target("ops-slack", SECRET);
        assert_eq!(t.to_string(), "ops-slack");
        assert_eq!(t.url.to_string(), REDACTED);
        let s = NotifierSettings {
            targets: vec![t.clone()],
            ..NotifierSettings::default()
        };
        for text in [format!("{t:?}"), format!("{t:#?}"), format!("{s:?}")] {
            assert!(!text.contains("hooks.slack.com"), "{text}");
            assert!(!text.contains("SECRET"), "{text}");
            assert!(text.contains(REDACTED), "{text}");
        }
    }

    #[test]
    fn invalid_settings_are_rejected_without_the_url() {
        let bad = |s: NotifierSettings| s.validate().unwrap_err().to_string();
        let with = |targets: Vec<Target>| NotifierSettings {
            targets,
            ..NotifierSettings::default()
        };
        let secret_ftp = "ftp://hooks.example/SECRET";
        let e = bad(with(vec![target("ops", secret_ftp)]));
        assert!(
            e.contains("targets[ops].url") && !e.contains("SECRET"),
            "{e}"
        );
        assert!(bad(with(vec![target("a", SECRET), target("a", SECRET)])).contains("duplicate"));
        assert!(bad(with(vec![target("", SECRET)])).contains("name"));
        assert!(bad(with(vec![target("has space", SECRET)])).contains("name"));
        assert!(bad(with(vec![target("ops", "not a url")])).contains("url"));
        assert!(
            bad(NotifierSettings {
                max_attempts: 0,
                ..NotifierSettings::default()
            })
            .contains("max_attempts")
        );
        assert!(
            bad(NotifierSettings {
                timeout_secs: 0,
                ..NotifierSettings::default()
            })
            .contains("timeout_secs")
        );
        for (timeout_secs, ok) in [(15, true), (16, false)] {
            let s = NotifierSettings {
                timeout_secs,
                ..NotifierSettings::default()
            };
            assert_eq!(s.validate().is_ok(), ok, "{timeout_secs}");
        }
        assert!(
            bad(NotifierSettings {
                max_age_secs: 0,
                ..NotifierSettings::default()
            })
            .contains("max_age_secs")
        );
        assert!(
            bad(NotifierSettings {
                public_url: "localhost".into(),
                ..NotifierSettings::default()
            })
            .contains("public_url")
        );
    }
}
