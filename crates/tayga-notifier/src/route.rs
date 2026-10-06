//! What the loop does with one decoded `tayga.alerts` record, and when its offset may be
//! committed. Pure, so `main.rs` only wires Kafka to it.

use crate::config::NotifierSettings;
use crate::deliver::Resolution;
use crate::payload::AlertMsg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// No target is configured: commit.
    NoTargets,
    /// The kind is not in `kinds`: commit.
    ExcludedKind,
    /// `last_at` is older than `max_age_secs` (a retained backlog): commit, counted as `stale`.
    Stale,
    /// Deliver to every target; commit when [`may_commit`] says so.
    Deliver,
}

pub fn route(cfg: &NotifierSettings, alert: &AlertMsg, now_ns: i64) -> Route {
    let max_age_ns = i64::try_from(cfg.max_age_secs)
        .unwrap_or(i64::MAX)
        .saturating_mul(1_000_000_000);
    if cfg.targets.is_empty() {
        Route::NoTargets
    } else if !cfg.delivers(alert.kind) {
        Route::ExcludedKind
    } else if now_ns.saturating_sub(alert.last_at_ns) > max_age_ns {
        Route::Stale
    } else {
        Route::Deliver
    }
}

/// A delivered record may be committed once no target was interrupted: every target delivered,
/// gave up, or had been resolved earlier. An interrupted one is re-read after restart and
/// deduplicated.
pub fn may_commit(results: &[Resolution]) -> bool {
    results.iter().all(|r| *r != Resolution::Interrupted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Target, TargetKind, WebhookUrl};
    use crate::payload::AlertKind;

    const NOW: i64 = 1_700_000_000_000_000_000;
    const SEC: i64 = 1_000_000_000;

    fn cfg(kinds: &[AlertKind], with_target: bool) -> NotifierSettings {
        NotifierSettings {
            kinds: kinds.to_vec(),
            targets: if with_target {
                vec![Target {
                    name: "hook".into(),
                    kind: TargetKind::Webhook,
                    url: WebhookUrl::new("http://h/hook"),
                }]
            } else {
                vec![]
            },
            ..NotifierSettings::default()
        }
    }

    fn alert(kind: AlertKind, last_at_ns: i64) -> AlertMsg {
        AlertMsg {
            alert_id: "a".into(),
            kind,
            template_id: "1".into(),
            service: "s".into(),
            template: "t".into(),
            started_at_ns: last_at_ns - 60 * SEC,
            last_at_ns,
            window_count: 0,
            peak_count: 0,
            baseline_per_window: 0.0,
            example_trace_ids: vec![],
        }
    }

    #[test]
    fn routes_each_case() {
        let all = &AlertKind::ALL;
        let fresh = alert(AlertKind::Spike, NOW - 10 * SEC);
        assert_eq!(route(&cfg(all, false), &fresh, NOW), Route::NoTargets);
        assert_eq!(
            route(&cfg(&[AlertKind::Silence], true), &fresh, NOW),
            Route::ExcludedKind
        );
        assert_eq!(route(&cfg(all, true), &fresh, NOW), Route::Deliver);
        // max_age_secs defaults to 3600: exactly that old is still delivered.
        let edge = alert(AlertKind::New, NOW - 3_600 * SEC);
        assert_eq!(route(&cfg(all, true), &edge, NOW), Route::Deliver);
        let old = alert(AlertKind::New, NOW - 3_601 * SEC);
        assert_eq!(route(&cfg(all, true), &old, NOW), Route::Stale);
        assert_eq!(
            route(&cfg(all, false), &old, NOW),
            Route::NoTargets,
            "no targets wins: nothing is counted"
        );
        let future = alert(AlertKind::Spike, NOW + 60 * SEC);
        assert_eq!(route(&cfg(all, true), &future, NOW), Route::Deliver);
    }

    #[test]
    fn commits_unless_a_target_was_interrupted() {
        use Resolution::*;
        assert!(may_commit(&[Delivered, Failed, AlreadyResolved]));
        assert!(may_commit(&[]));
        assert!(!may_commit(&[Delivered, Interrupted]));
    }
}
