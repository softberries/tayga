//! Alert rules (spec §6). Counts come from storage; these functions only decide.

use crate::drain::OVERFLOW;
use std::collections::HashMap;
use tayga_analysis::fingerprint::fingerprint;

const MIN_NS: i64 = 60_000_000_000;

#[derive(Debug, Clone, PartialEq)]
pub struct DetectConfig {
    pub spike_window_min: u32,
    pub baseline_window_min: u32,
    pub spike_factor: f64,
    pub spike_min_count: u64,
    pub new_template_recent_min: u32,
    pub new_template_warmup_min: u32,
    pub alert_active_min: u32,
}

impl Default for DetectConfig {
    fn default() -> Self {
        Self {
            spike_window_min: 5,
            baseline_window_min: 60,
            spike_factor: 5.0,
            spike_min_count: 10,
            new_template_recent_min: 10,
            new_template_warmup_min: 15,
            alert_active_min: 10,
        }
    }
}

/// Counts for one template: `current` in the spike window, `baseline_total` in the
/// `baseline_window_min` minutes before it.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateWindow {
    pub template_id: u64,
    pub service: String,
    pub template: String,
    pub first_seen_ns: i64,
    pub current: u64,
    pub baseline_total: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewCandidate {
    pub template_id: u64,
    pub service: String,
    pub template: String,
    pub first_seen_ns: i64,
    /// Oldest `first_seen` of any template of the same service.
    pub service_oldest_ns: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    New,
    Spike,
}

impl AlertKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AlertKind::New => "new",
            AlertKind::Spike => "spike",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub alert_id: String,
    pub kind: AlertKind,
    pub template_id: u64,
    pub service: String,
    pub template: String,
    pub started_at_ns: i64,
    pub last_at_ns: i64,
    pub window_count: u64,
    pub peak_count: u64,
    pub baseline_per_window: f64,
    pub example_trace_ids: Vec<String>,
}

fn id_hex(parts: &[&str]) -> String {
    format!("{:016x}", fingerprint(parts))
}

/// `Some(baseline per spike window)` when the template spikes now.
pub fn spike_baseline(cfg: &DetectConfig, w: &TemplateWindow, now_ns: i64) -> Option<f64> {
    let min_age = i64::from(cfg.baseline_window_min + cfg.spike_window_min) * MIN_NS;
    if w.template == OVERFLOW || now_ns.saturating_sub(w.first_seen_ns) < min_age {
        return None;
    }
    let windows = f64::from((cfg.baseline_window_min / cfg.spike_window_min.max(1)).max(1));
    let per_window = w.baseline_total as f64 / windows;
    (w.current >= cfg.spike_min_count && w.current as f64 >= cfg.spike_factor * per_window.max(1.0))
        .then_some(per_window)
}

/// Slack below the previous tick's data clock, for logs that arrive slightly out of order.
pub const NEW_TEMPLATE_MARGIN_NS: i64 = MIN_NS;

/// Data clock before the first detection tick: `new_template_recent_min` before the latest hit.
/// With no hits at all (fresh install, or nothing within the hits TTL) the wall clock stands in,
/// so a backlog replayed from scratch does not report its history as new.
pub fn initial_watermark(cfg: &DetectConfig, data_now_ns: i64, wall_now_ns: i64) -> i64 {
    let clock = if data_now_ns > 0 {
        data_now_ns
    } else {
        wall_now_ns
    };
    clock - i64::from(cfg.new_template_recent_min) * MIN_NS
}

/// Lower bound (exclusive) on `first_seen` for new-template candidates this tick.
pub fn new_template_since(watermark_ns: i64) -> i64 {
    watermark_ns - NEW_TEMPLATE_MARGIN_NS
}

/// A template is new when it first appeared after `since_ns` (in log time) and its service already
/// had templates `new_template_warmup_min` minutes before it appeared.
pub fn is_new(cfg: &DetectConfig, c: &NewCandidate, since_ns: i64) -> bool {
    c.template != OVERFLOW
        && c.first_seen_ns > since_ns
        && c.service_oldest_ns
            <= c.first_seen_ns
                .saturating_sub(i64::from(cfg.new_template_warmup_min) * MIN_NS)
}

pub fn new_alert(c: &NewCandidate, examples: Vec<String>, now_ns: i64) -> Alert {
    Alert {
        alert_id: id_hex(&["new", &c.template_id.to_string()]),
        kind: AlertKind::New,
        template_id: c.template_id,
        service: c.service.clone(),
        template: c.template.clone(),
        started_at_ns: c.first_seen_ns,
        last_at_ns: now_ns,
        window_count: 0,
        peak_count: 0,
        baseline_per_window: 0.0,
        example_trace_ids: examples,
    }
}

/// One active spike alert per template; updated while active, rolled over after it lapses.
#[derive(Debug, Default)]
pub struct SpikeTracker {
    active: HashMap<u64, Alert>,
}

impl SpikeTracker {
    pub fn restore(&mut self, alerts: Vec<Alert>) {
        for a in alerts.into_iter().filter(|a| a.kind == AlertKind::Spike) {
            self.active.insert(a.template_id, a);
        }
    }

    pub fn observe(
        &mut self,
        cfg: &DetectConfig,
        w: &TemplateWindow,
        baseline_per_window: f64,
        examples: Vec<String>,
        now_ns: i64,
    ) -> (Alert, bool) {
        let active_ns = i64::from(cfg.alert_active_min) * MIN_NS;
        if let Some(a) = self.active.get_mut(&w.template_id)
            && now_ns - a.last_at_ns <= active_ns
        {
            a.last_at_ns = now_ns;
            a.window_count = w.current;
            a.peak_count = a.peak_count.max(w.current);
            a.baseline_per_window = baseline_per_window;
            if !examples.is_empty() {
                a.example_trace_ids = examples;
            }
            return (a.clone(), false);
        }
        let started_minute = now_ns.div_euclid(MIN_NS) * MIN_NS;
        let a = Alert {
            alert_id: id_hex(&[
                "spike",
                &w.template_id.to_string(),
                &started_minute.to_string(),
            ]),
            kind: AlertKind::Spike,
            template_id: w.template_id,
            service: w.service.clone(),
            template: w.template.clone(),
            started_at_ns: now_ns,
            last_at_ns: now_ns,
            window_count: w.current,
            peak_count: w.current,
            baseline_per_window,
            example_trace_ids: examples,
        };
        self.active.insert(w.template_id, a.clone());
        (a, true)
    }

    /// Drops alerts that lapsed; returns how many remain active.
    pub fn expire(&mut self, cfg: &DetectConfig, now_ns: i64) -> usize {
        let active_ns = i64::from(cfg.alert_active_min) * MIN_NS;
        self.active
            .retain(|_, a| now_ns - a.last_at_ns <= active_ns);
        self.active.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000 * MIN_NS;

    fn window(current: u64, baseline_total: u64, age_min: i64) -> TemplateWindow {
        TemplateWindow {
            template_id: 7,
            service: "payment".into(),
            template: "Payment request failed <*>".into(),
            first_seen_ns: NOW - age_min * MIN_NS,
            current,
            baseline_total,
        }
    }

    #[test]
    fn spike_needs_min_count_factor_and_age() {
        let cfg = DetectConfig::default();
        assert_eq!(spike_baseline(&cfg, &window(10, 0, 120), NOW), Some(0.0)); // 10 ≥ 5×max(0,1)
        assert_eq!(spike_baseline(&cfg, &window(9, 0, 120), NOW), None); // below min count
        assert_eq!(spike_baseline(&cfg, &window(50, 120, 120), NOW), Some(10.0)); // 50 ≥ 5×10
        assert_eq!(spike_baseline(&cfg, &window(49, 120, 120), NOW), None);
        assert_eq!(spike_baseline(&cfg, &window(100, 0, 64), NOW), None); // younger than 65 min
        let mut o = window(100, 0, 120);
        o.template = OVERFLOW.into();
        assert_eq!(spike_baseline(&cfg, &o, NOW), None);
    }

    fn candidate(first_seen_ns: i64, service_oldest_ns: i64) -> NewCandidate {
        NewCandidate {
            template_id: 1,
            service: "checkout".into(),
            template: "x".into(),
            first_seen_ns,
            service_oldest_ns,
        }
    }

    #[test]
    fn new_template_needs_first_seen_after_the_bound() {
        let cfg = DetectConfig::default();
        let since = NOW - 10 * MIN_NS;
        let oldest = since - 60 * MIN_NS;
        assert!(
            !is_new(&cfg, &candidate(since, oldest), since),
            "bound is exclusive"
        );
        assert!(is_new(&cfg, &candidate(since + 1, oldest), since));
        let mut o = candidate(since + 1, oldest);
        o.template = OVERFLOW.into();
        assert!(!is_new(&cfg, &o, since));
    }

    #[test]
    fn new_template_warmup_is_measured_from_first_seen() {
        let cfg = DetectConfig::default();
        let first = NOW - 30 * MIN_NS; // long before the wall clock: only data time matters
        let since = first - MIN_NS;
        assert!(is_new(&cfg, &candidate(first, first - 15 * MIN_NS), since));
        assert!(
            !is_new(&cfg, &candidate(first, first - 15 * MIN_NS + 1), since),
            "service one nanosecond short of the warmup"
        );
        assert!(
            !is_new(&cfg, &candidate(first, first), since),
            "new service"
        );
    }

    #[test]
    fn watermark_starts_from_the_data_clock_or_the_wall_clock() {
        let cfg = DetectConfig::default();
        assert_eq!(
            initial_watermark(&cfg, NOW, NOW + 60 * MIN_NS),
            NOW - 10 * MIN_NS
        );
        assert_eq!(initial_watermark(&cfg, 0, NOW), NOW - 10 * MIN_NS);
        assert_eq!(new_template_since(NOW), NOW - MIN_NS);
    }

    #[test]
    fn new_alert_id_is_one_per_template() {
        let c = candidate(NOW - MIN_NS, NOW - 60 * MIN_NS);
        let a = new_alert(&c, vec!["t1".into()], NOW);
        assert_eq!(
            a.alert_id,
            new_alert(&c, vec![], NOW + MIN_NS).alert_id,
            "one id per template"
        );
        assert_eq!(a.alert_id.len(), 16);
    }

    #[test]
    fn spike_tracker_updates_then_rolls_over() {
        let cfg = DetectConfig::default();
        let mut t = SpikeTracker::default();
        let (a1, created) = t.observe(&cfg, &window(20, 0, 120), 0.0, vec!["t1".into()], NOW);
        assert!(created);
        let (a2, created) = t.observe(&cfg, &window(30, 0, 120), 0.0, vec![], NOW + 5 * MIN_NS);
        assert!(!created);
        assert_eq!(a2.alert_id, a1.alert_id);
        assert_eq!((a2.peak_count, a2.window_count), (30, 30));
        assert_eq!(
            a2.example_trace_ids,
            vec!["t1".to_string()],
            "empty examples keep the old ones"
        );
        assert_eq!(t.expire(&cfg, NOW + 15 * MIN_NS), 1);
        assert_eq!(t.expire(&cfg, NOW + 16 * MIN_NS), 0);
        let (a3, created) = t.observe(&cfg, &window(20, 0, 120), 0.0, vec![], NOW + 30 * MIN_NS);
        assert!(created);
        assert_ne!(a3.alert_id, a1.alert_id);
    }

    #[test]
    fn restore_ignores_new_alerts() {
        let mut t = SpikeTracker::default();
        let mut a = new_alert(
            &NewCandidate {
                template_id: 1,
                service: "s".into(),
                template: "x".into(),
                first_seen_ns: NOW,
                service_oldest_ns: 0,
            },
            vec![],
            NOW,
        );
        t.restore(vec![a.clone()]);
        assert_eq!(t.expire(&DetectConfig::default(), NOW), 0);
        a.kind = AlertKind::Spike;
        t.restore(vec![a]);
        assert_eq!(t.expire(&DetectConfig::default(), NOW), 1);
    }
}
