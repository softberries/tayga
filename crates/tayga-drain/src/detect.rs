//! Alert rules (spec §6). Counts come from storage; these functions only decide.

use crate::drain::OVERFLOW;
use std::collections::HashMap;
use tayga_analysis::fingerprint::fingerprint;

const MIN_NS: i64 = 60_000_000_000;

/// How a spike is judged (spec 7a §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BaselineMode {
    /// The flat rule only: the template's own preceding hour.
    #[default]
    Flat,
    /// The flat rule, and the window must also beat the same window 1 day and 7 days earlier.
    Seasonal,
}

impl std::str::FromStr for BaselineMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "flat" => Ok(Self::Flat),
            "seasonal" => Ok(Self::Seasonal),
            other => Err(format!(
                "unknown baseline mode {other:?}, expected \"flat\" or \"seasonal\""
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetectConfig {
    pub spike_window_min: u32,
    pub baseline_window_min: u32,
    pub spike_factor: f64,
    pub spike_min_count: u64,
    pub new_template_recent_min: u32,
    pub new_template_warmup_min: u32,
    pub alert_active_min: u32,
    pub baseline_mode: BaselineMode,
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
            baseline_mode: BaselineMode::Flat,
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
    Silence,
}

impl AlertKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AlertKind::New => "new",
            AlertKind::Spike => "spike",
            AlertKind::Silence => "silence",
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
    /// Seasonal mode: hits of the same window 1 day / 7 days earlier; `None` when that window
    /// had no coverage or the mode is flat.
    pub baseline_day: Option<f64>,
    pub baseline_week: Option<f64>,
    pub example_trace_ids: Vec<String>,
}

fn id_hex(parts: &[&str]) -> String {
    format!("{:016x}", fingerprint(parts))
}

/// Minutes of the template's own baseline period that had any log at all (any template).
/// Per template: see [`template_coverage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    pub covered_min: u32,
}

/// Why a template that may have spiked was not judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpikeSkip {
    /// Under half of the minutes the template could have been seen in had any logs at all
    /// (pipeline gap).
    Coverage,
}

/// Coverage of one template: the covered minute buckets (unix minutes, any template, within the
/// baseline window) at or after `max(first_seen minute, baseline window start)`, so minutes
/// before the template existed do not count as its coverage. Buckets are whole clock minutes
/// while the window edges are `now`-relative instants, so the count can be off by one at the
/// edges; it is capped at `baseline_window_min`.
pub fn template_coverage(
    cfg: &DetectConfig,
    covered_buckets: &[i64],
    first_seen_ns: i64,
    now_ns: i64,
) -> Coverage {
    let window_start_ns =
        now_ns - i64::from(cfg.baseline_window_min + cfg.spike_window_min) * MIN_NS;
    let from = first_seen_ns
        .div_euclid(MIN_NS)
        .max(window_start_ns.div_euclid(MIN_NS));
    let n = covered_buckets.iter().filter(|&&m| m >= from).count();
    Coverage {
        covered_min: u32::try_from(n)
            .unwrap_or(u32::MAX)
            .min(cfg.baseline_window_min),
    }
}

/// `Ok(Some(baseline per spike window))` when the template spikes now, `Ok(None)` when it does
/// not, `Err` when the baseline is too thin to judge (spec 7a §2.1, §2.2).
///
/// The baseline counts only minutes with data and only minutes the template existed before the
/// spike window, so a gap or a young template cannot shrink the divisor into a false spike.
/// `cov` is the template's own coverage; the 50% gate compares it with the minutes the template
/// could have been seen in, `min(existed, baseline_window_min)`.
pub fn spike_baseline(
    cfg: &DetectConfig,
    w: &TemplateWindow,
    cov: Coverage,
    now_ns: i64,
) -> Result<Option<f64>, SpikeSkip> {
    if w.template == OVERFLOW {
        return Ok(None);
    }
    let spike_min = i64::from(cfg.spike_window_min.max(1));
    let age_min = now_ns.saturating_sub(w.first_seen_ns) / MIN_NS;
    if age_min < i64::from(cfg.new_template_recent_min) {
        return Ok(None);
    }
    let existed_min = age_min - spike_min;
    if existed_min < spike_min {
        return Ok(None);
    }
    let possible_min = existed_min.min(i64::from(cfg.baseline_window_min));
    if i64::from(cov.covered_min) * 2 < possible_min {
        return Err(SpikeSkip::Coverage);
    }
    let effective_min = possible_min.min(i64::from(cov.covered_min));
    // A full baseline keeps the whole-window divisor; a shortened one (young template, gap) is
    // scaled by the minutes actually covered, so 3 minutes are not read as a full window.
    let per_window = if effective_min >= i64::from(cfg.baseline_window_min) {
        w.baseline_total as f64 / (effective_min / spike_min).max(1) as f64
    } else {
        w.baseline_total as f64 * spike_min as f64 / effective_min as f64
    };
    Ok((w.current >= cfg.spike_min_count
        && w.current as f64 >= cfg.spike_factor * per_window.max(1.0))
    .then_some(per_window))
}

/// Seasonal spike rule (spec 7a §2.3): the flat rule fires and `current` also beats
/// `spike_factor * max(c, 1)` for every comparator that counts. `day` / `week` are the hit
/// counts of the window shifted by 1 / 7 days, `None` when that past window had no global
/// coverage. With no comparator this is the flat rule.
pub fn seasonal_decision(
    cfg: &DetectConfig,
    current: u64,
    flat_per_window: f64,
    day: Option<u64>,
    week: Option<u64>,
) -> bool {
    let beats = |c: u64| current as f64 >= cfg.spike_factor * (c.max(1) as f64);
    current >= cfg.spike_min_count
        && current as f64 >= cfg.spike_factor * flat_per_window.max(1.0)
        && day.is_none_or(beats)
        && week.is_none_or(beats)
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
/// had templates `new_template_warmup_min` minutes before it appeared. After a masking change
/// (`epoch_start_ns > 0`) it must also have appeared `new_template_warmup_min` after the epoch
/// start, so templates re-created by the new masking do not alert.
pub fn is_new(cfg: &DetectConfig, c: &NewCandidate, since_ns: i64, epoch_start_ns: i64) -> bool {
    let warmup_ns = i64::from(cfg.new_template_warmup_min) * MIN_NS;
    c.template != OVERFLOW
        && c.first_seen_ns > since_ns
        && (epoch_start_ns <= 0 || c.first_seen_ns >= epoch_start_ns.saturating_add(warmup_ns))
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
        baseline_day: None,
        baseline_week: None,
        example_trace_ids: examples,
    }
}

/// Detection inputs of one template for silence alerts (log time, ns). `t_last_ns` is the
/// template's newest hit, `s_last_ns` the newest hit over its service's templates; both are
/// `None` when no hit is left within the hits TTL.
#[derive(Debug, Clone, PartialEq)]
pub struct SilenceInput {
    pub template_id: u64,
    pub service: String,
    pub first_seen_ns: i64,
    pub t_last_ns: Option<i64>,
    pub s_last_ns: Option<i64>,
}

/// Silence test in log time (spec 7b §2.2): the template is silent when the newest hit of its
/// service is at least `minutes` past the template's own newest hit, or past `first_seen` when it
/// has none inside the hits TTL. A service without any hit (`s_last_ns` is `None`, e.g. a
/// pipeline outage) is never judged, so a missing feed cannot look like silence.
/// `clock_ns` is the detection's partition data clock, which holds back for a lagging partition:
/// the service's newest hit counts only up to it (`min(s_last, clock)`), so lines still sitting
/// in the lag cannot make a template look silent.
pub fn is_silent(
    minutes: u32,
    first_seen_ns: i64,
    t_last_ns: Option<i64>,
    s_last_ns: Option<i64>,
    clock_ns: i64,
) -> bool {
    let Some(s_last) = s_last_ns.map(|s| s.min(clock_ns)) else {
        return false;
    };
    let since = t_last_ns.unwrap_or(first_seen_ns);
    s_last.saturating_sub(since) >= i64::from(minutes) * MIN_NS
}

/// The alert of a silent template. The id hashes the template and the `t_last` (or `first_seen`)
/// at which the silence began, so it is the same on every pass of one silence period, and across
/// a logminer restart, and changes once the template gets a hit and goes silent again.
/// `started_at` is that `t_last` plus `minutes`; `last_at` is `now_ns`. `baseline_per_window`
/// is informational.
pub fn silence_alert(
    input: &SilenceInput,
    template: &str,
    minutes: u32,
    baseline_per_window: f64,
    now_ns: i64,
) -> Alert {
    let since = input.t_last_ns.unwrap_or(input.first_seen_ns);
    Alert {
        alert_id: id_hex(&[
            "silence",
            &input.template_id.to_string(),
            &since.to_string(),
        ]),
        kind: AlertKind::Silence,
        template_id: input.template_id,
        service: input.service.clone(),
        template: template.to_string(),
        started_at_ns: since.saturating_add(i64::from(minutes) * MIN_NS),
        last_at_ns: now_ns,
        window_count: 0,
        peak_count: 0,
        baseline_per_window,
        baseline_day: None,
        baseline_week: None,
        example_trace_ids: Vec::new(),
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
        seasonal: (Option<f64>, Option<f64>),
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
            (a.baseline_day, a.baseline_week) = seasonal;
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
            baseline_day: seasonal.0,
            baseline_week: seasonal.1,
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

    fn cov(covered_min: u32) -> Coverage {
        Coverage { covered_min }
    }

    #[test]
    fn spike_needs_min_count_factor_and_age() {
        let cfg = DetectConfig::default();
        let spike = |w: &TemplateWindow| spike_baseline(&cfg, w, cov(60), NOW);
        assert_eq!(spike(&window(10, 0, 120)), Ok(Some(0.0))); // 10 >= 5*max(0,1)
        assert_eq!(spike(&window(9, 0, 120)), Ok(None)); // below min count
        assert_eq!(spike(&window(50, 120, 120)), Ok(Some(10.0))); // 50 >= 5*10
        assert_eq!(spike(&window(49, 120, 120)), Ok(None));
        let mut o = window(100, 0, 120);
        o.template = OVERFLOW.into();
        assert_eq!(spike(&o), Ok(None));
    }

    #[test]
    fn thin_coverage_is_skipped() {
        let cfg = DetectConfig::default();
        let w = window(100, 0, 120);
        assert_eq!(
            spike_baseline(&cfg, &w, cov(0), NOW),
            Err(SpikeSkip::Coverage)
        );
        assert_eq!(
            spike_baseline(&cfg, &w, cov(29), NOW),
            Err(SpikeSkip::Coverage)
        );
        assert_eq!(spike_baseline(&cfg, &w, cov(30), NOW), Ok(Some(0.0)));
        let mut o = window(100, 0, 120);
        o.template = OVERFLOW.into();
        assert_eq!(
            spike_baseline(&cfg, &o, cov(0), NOW),
            Ok(None),
            "overflow is not a skip"
        );
    }

    #[test]
    fn coverage_shrinks_the_baseline_windows() {
        let cfg = DetectConfig::default();
        let w = window(60, 120, 120);
        // 60 covered minutes: 12 windows, 10 per window; 60 >= 5*10.
        assert_eq!(spike_baseline(&cfg, &w, cov(60), NOW), Ok(Some(10.0)));
        // 30 covered minutes: 6 windows, 20 per window; 60 < 5*20.
        assert_eq!(spike_baseline(&cfg, &w, cov(30), NOW), Ok(None));
        assert_eq!(
            spike_baseline(&cfg, &window(100, 120, 120), cov(30), NOW),
            Ok(Some(20.0))
        );
    }

    #[test]
    fn young_templates_can_spike_after_ten_minutes() {
        let cfg = DetectConfig::default();
        assert_eq!(
            spike_baseline(&cfg, &window(100, 0, 9), cov(60), NOW),
            Ok(None)
        );
        // 10 minutes old: existed 5 = one spike window.
        assert_eq!(
            spike_baseline(&cfg, &window(100, 0, 10), cov(60), NOW),
            Ok(Some(0.0))
        );
        // 11 minutes old with a burst: existed 6 min, 4 hits -> 4 * 5 / 6 per window.
        assert_eq!(
            spike_baseline(&cfg, &window(100, 4, 11), cov(60), NOW),
            Ok(Some(4.0 * 5.0 / 6.0))
        );
        assert_eq!(
            spike_baseline(&cfg, &window(16, 4, 11), cov(60), NOW),
            Ok(None),
            "16 < 5 * 3.33"
        );
        // 64 minutes old: effective 59 of 60 minutes, scaled proportionally.
        assert_eq!(
            spike_baseline(&cfg, &window(100, 110, 64), cov(60), NOW),
            Ok(Some(110.0 * 5.0 / 59.0))
        );
        // Coverage below the existed minutes wins: effective 30 minutes -> 120 * 5 / 30.
        assert_eq!(
            spike_baseline(&cfg, &window(100, 120, 64), cov(30), NOW),
            Ok(Some(20.0))
        );
    }

    #[test]
    fn a_young_template_spanning_an_outage_is_not_judged() {
        let cfg = DetectConfig::default();
        let now_min = NOW / MIN_NS;
        // Logs flowed until 30 min ago, then the pipeline was down through the spike window's
        // start; the template (30 min old) saw only its first minute covered.
        let buckets: Vec<i64> = ((now_min - 65)..=(now_min - 30)).collect();
        let w = window(40, 20, 30);
        let c = template_coverage(&cfg, &buckets, w.first_seen_ns, NOW);
        assert_eq!(c.covered_min, 1);
        assert_eq!(spike_baseline(&cfg, &w, c, NOW), Err(SpikeSkip::Coverage));
    }

    #[test]
    fn a_young_template_with_full_lifetime_coverage_can_spike() {
        let cfg = DetectConfig::default();
        let now_min = NOW / MIN_NS;
        let buckets: Vec<i64> = ((now_min - 65)..=(now_min - 5)).collect();
        let w = window(40, 20, 30); // existed 25 min -> 5 windows -> 4 per window
        let c = template_coverage(&cfg, &buckets, w.first_seen_ns, NOW);
        assert!(c.covered_min >= 25);
        assert_eq!(spike_baseline(&cfg, &w, c, NOW), Ok(Some(4.0)));
    }

    #[test]
    fn coverage_before_the_template_existed_does_not_count() {
        let cfg = DetectConfig::default();
        let now_min = NOW / MIN_NS;
        let buckets: Vec<i64> = ((now_min - 65)..=(now_min - 5)).collect();
        let old = template_coverage(&cfg, &buckets, NOW - 120 * MIN_NS, NOW);
        assert_eq!(old.covered_min, 60, "capped at the baseline window");
        let young = template_coverage(&cfg, &buckets, NOW - 20 * MIN_NS, NOW);
        assert!((16..=17).contains(&young.covered_min), "{young:?}");
    }

    #[test]
    fn a_short_baseline_is_scaled_by_its_minutes() {
        let cfg = DetectConfig {
            baseline_window_min: 6,
            ..DetectConfig::default()
        };
        // 3 of 6 minutes covered: 12 hits in 3 minutes = 20 per 5-minute window, not 12.
        let w = window(100, 12, 120);
        assert_eq!(spike_baseline(&cfg, &w, cov(3), NOW), Ok(Some(20.0)));
        assert_eq!(
            spike_baseline(&cfg, &window(99, 12, 120), cov(3), NOW),
            Ok(None),
            "99 < 5 * 20"
        );
        // Full coverage of the same config: the whole-window divisor (6 / 5 = 1 window).
        assert_eq!(spike_baseline(&cfg, &w, cov(6), NOW), Ok(Some(12.0)));
        // A non-divisible default-window config keeps the old integer maths at full coverage.
        let odd = DetectConfig {
            baseline_window_min: 62,
            ..DetectConfig::default()
        };
        assert_eq!(
            spike_baseline(&odd, &window(100, 120, 120), cov(62), NOW),
            Ok(Some(10.0)),
            "62 / 5 = 12 windows"
        );
    }

    #[test]
    fn seasonal_without_comparators_is_flat() {
        let cfg = DetectConfig::default();
        assert!(seasonal_decision(&cfg, 50, 10.0, None, None));
        assert!(!seasonal_decision(&cfg, 49, 10.0, None, None), "flat rule");
        assert!(!seasonal_decision(&cfg, 9, 0.0, None, None), "min count");
    }

    #[test]
    fn seasonal_day_comparator_alone() {
        let cfg = DetectConfig::default();
        assert!(seasonal_decision(&cfg, 50, 0.0, Some(10), None)); // 50 >= 5*10
        assert!(!seasonal_decision(&cfg, 49, 0.0, Some(10), None));
        assert!(seasonal_decision(&cfg, 10, 0.0, Some(0), None), "max(c, 1)");
        assert!(!seasonal_decision(&cfg, 10, 0.0, Some(3), None));
    }

    #[test]
    fn seasonal_day_and_week_must_both_be_beaten() {
        let cfg = DetectConfig::default();
        assert!(seasonal_decision(&cfg, 50, 0.0, Some(10), Some(10)));
        assert!(!seasonal_decision(&cfg, 50, 0.0, Some(10), Some(11)));
        assert!(
            seasonal_decision(&cfg, 50, 0.0, None, Some(10)),
            "week only"
        );
    }

    #[test]
    fn a_high_comparator_suppresses_a_flat_spike() {
        let cfg = DetectConfig::default();
        // Flat says spike (baseline 2/window), but the same window yesterday was as busy.
        assert!(seasonal_decision(&cfg, 60, 2.0, None, None));
        assert!(
            !seasonal_decision(&cfg, 60, 2.0, Some(40), None),
            "daily peak"
        );
        assert!(
            !seasonal_decision(&cfg, 60, 2.0, Some(1), Some(40)),
            "weekly peak"
        );
        // The flat rule still gates: a quiet comparator does not rescue a flat non-spike.
        assert!(!seasonal_decision(&cfg, 60, 20.0, Some(0), Some(0)));
    }

    #[test]
    fn baseline_mode_parses() {
        assert_eq!("flat".parse(), Ok(BaselineMode::Flat));
        assert_eq!("seasonal".parse(), Ok(BaselineMode::Seasonal));
        assert!("Seasonal".parse::<BaselineMode>().is_err());
        assert_eq!(BaselineMode::default(), BaselineMode::Flat);
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
            !is_new(&cfg, &candidate(since, oldest), since, 0),
            "bound is exclusive"
        );
        assert!(is_new(&cfg, &candidate(since + 1, oldest), since, 0));
        let mut o = candidate(since + 1, oldest);
        o.template = OVERFLOW.into();
        assert!(!is_new(&cfg, &o, since, 0));
    }

    #[test]
    fn epoch_warmup_suppresses_new_templates() {
        let cfg = DetectConfig::default();
        let epoch = NOW;
        let since = epoch - 60 * MIN_NS;
        let oldest = since - 60 * MIN_NS;
        let at = |min: i64| candidate(epoch + min * MIN_NS, oldest);
        assert!(!is_new(&cfg, &at(5), since, epoch));
        assert!(is_new(&cfg, &at(16), since, epoch));
        assert!(is_new(&cfg, &at(15), since, epoch), "bound is inclusive");
        assert!(is_new(&cfg, &at(5), since, 0), "epoch 0 is no gate");
    }

    #[test]
    fn new_template_warmup_is_measured_from_first_seen() {
        let cfg = DetectConfig::default();
        let first = NOW - 30 * MIN_NS; // long before the wall clock: only data time matters
        let since = first - MIN_NS;
        assert!(is_new(
            &cfg,
            &candidate(first, first - 15 * MIN_NS),
            since,
            0
        ));
        assert!(
            !is_new(&cfg, &candidate(first, first - 15 * MIN_NS + 1), since, 0),
            "service one nanosecond short of the warmup"
        );
        assert!(
            !is_new(&cfg, &candidate(first, first), since, 0),
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
        let (a1, created) = t.observe(
            &cfg,
            &window(20, 0, 120),
            0.0,
            (None, None),
            vec!["t1".into()],
            NOW,
        );
        assert!(created);
        let (a2, created) = t.observe(
            &cfg,
            &window(30, 0, 120),
            0.0,
            (Some(2.0), None),
            vec![],
            NOW + 5 * MIN_NS,
        );
        assert!(!created);
        assert_eq!(a2.alert_id, a1.alert_id);
        assert_eq!((a2.peak_count, a2.window_count), (30, 30));
        assert_eq!((a2.baseline_day, a2.baseline_week), (Some(2.0), None));
        assert_eq!(
            a2.example_trace_ids,
            vec!["t1".to_string()],
            "empty examples keep the old ones"
        );
        assert_eq!(t.expire(&cfg, NOW + 15 * MIN_NS), 1);
        assert_eq!(t.expire(&cfg, NOW + 16 * MIN_NS), 0);
        let (a3, created) = t.observe(
            &cfg,
            &window(20, 0, 120),
            0.0,
            (None, None),
            vec![],
            NOW + 30 * MIN_NS,
        );
        assert!(created);
        assert_ne!(a3.alert_id, a1.alert_id);
    }

    fn silence_input(t_last: Option<i64>, s_last: Option<i64>) -> SilenceInput {
        SilenceInput {
            template_id: 7,
            service: "svc".into(),
            first_seen_ns: 5 * MIN_NS,
            t_last_ns: t_last,
            s_last_ns: s_last,
        }
    }

    #[test]
    fn silent_at_exactly_the_minutes_and_not_one_ns_before() {
        let t = 100 * MIN_NS;
        assert!(is_silent(10, 0, Some(t), Some(t + 10 * MIN_NS), i64::MAX));
        assert!(!is_silent(
            10,
            0,
            Some(t),
            Some(t + 10 * MIN_NS - 1),
            i64::MAX
        ));
    }

    #[test]
    fn a_clock_below_the_service_hit_suppresses_silence() {
        let t = 100 * MIN_NS;
        let s = Some(t + 20 * MIN_NS);
        assert!(is_silent(10, 0, Some(t), s, t + 20 * MIN_NS));
        // The clock holds at 5 min past t_last: the lagging lines may still hold a hit.
        assert!(!is_silent(10, 0, Some(t), s, t + 5 * MIN_NS));
        assert!(!is_silent(10, 0, Some(t), s, t + 10 * MIN_NS - 1));
        assert!(is_silent(10, 0, Some(t), s, t + 10 * MIN_NS));
    }

    #[test]
    fn a_clock_at_or_above_the_service_hit_changes_nothing() {
        let t = 100 * MIN_NS;
        let s = Some(t + 12 * MIN_NS);
        for clock in [t + 12 * MIN_NS, t + 13 * MIN_NS, i64::MAX] {
            assert!(is_silent(10, 0, Some(t), s, clock));
        }
        assert!(!is_silent(20, 0, Some(t), s, i64::MAX));
    }

    #[test]
    fn no_service_hit_is_never_silent() {
        assert!(!is_silent(1, 0, Some(0), None, i64::MAX));
        assert!(!is_silent(1, 0, None, None, i64::MAX));
    }

    #[test]
    fn a_template_without_hits_falls_back_to_first_seen() {
        let fs = 5 * MIN_NS;
        assert!(is_silent(10, fs, None, Some(fs + 10 * MIN_NS), i64::MAX));
        assert!(!is_silent(10, fs, None, Some(fs + 9 * MIN_NS), i64::MAX));
    }

    #[test]
    fn silence_alert_id_is_stable_per_period() {
        let t = 100 * MIN_NS;
        let a = silence_alert(
            &silence_input(Some(t), Some(t + 20 * MIN_NS)),
            "x",
            10,
            0.0,
            1,
        );
        let b = silence_alert(
            &silence_input(Some(t), Some(t + 30 * MIN_NS)),
            "x",
            10,
            0.0,
            2,
        );
        assert_eq!(a.alert_id, b.alert_id);
        assert_eq!(a.kind, AlertKind::Silence);
        assert_eq!(a.started_at_ns, t + 10 * MIN_NS);
        assert_eq!((a.last_at_ns, b.last_at_ns), (1, 2));
        assert_eq!(a.window_count, 0);
        let t2 = t + 40 * MIN_NS; // a new hit, then silent again: a new period
        let c = silence_alert(
            &silence_input(Some(t2), Some(t2 + 20 * MIN_NS)),
            "x",
            10,
            0.0,
            3,
        );
        assert_ne!(a.alert_id, c.alert_id);
        let d = silence_alert(&silence_input(None, Some(t)), "x", 10, 0.0, 3);
        assert_eq!(d.started_at_ns, 15 * MIN_NS);
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
