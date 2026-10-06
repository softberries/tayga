//! `AlertMsg` decodes exactly what the logminer publishes on `tayga.alerts`.

use tayga_drain::detect::{Alert, AlertKind as DrainKind, SilenceInput, silence_alert};
use tayga_logminer::miner::alert_json;
use tayga_notifier::payload::{AlertKind, AlertMsg, summary, webhook_payload};

const MIN_NS: i64 = 60_000_000_000;

fn decode(a: &Alert) -> AlertMsg {
    serde_json::from_value(alert_json(a)).expect("logminer alert decodes")
}

#[test]
fn a_seasonal_spike_decodes_with_its_string_template_id() {
    let spike = Alert {
        alert_id: "0123456789abcdef".into(),
        kind: DrainKind::Spike,
        template_id: u64::MAX - 1,
        service: "payment".into(),
        template: "declined <*>".into(),
        started_at_ns: 10,
        last_at_ns: 20,
        window_count: 30,
        peak_count: 31,
        baseline_per_window: 2.5,
        baseline_day: Some(4.0),
        baseline_week: None,
        example_trace_ids: vec!["t1".into()],
    };
    let m = decode(&spike);
    assert_eq!(m.kind, AlertKind::Spike);
    assert_eq!(m.template_id, (u64::MAX - 1).to_string());
    assert_eq!((m.window_count, m.peak_count), (30, 31));
    assert_eq!(m.example_trace_ids, ["t1"]);
    assert_eq!(
        webhook_payload(&m, "http://x")["links"]["template"],
        format!("http://x/logs/templates/{}", u64::MAX - 1)
    );
}

#[test]
fn a_silence_alert_reads_as_quiet_since_its_last_hit() {
    let t_last = 1_700_000_000_000_000_000;
    let input = SilenceInput {
        template_id: 42,
        service: "cart".into(),
        first_seen_ns: t_last - 60 * MIN_NS,
        t_last_ns: Some(t_last),
        s_last_ns: Some(t_last + 15 * MIN_NS),
    };
    let alert = silence_alert(&input, "cart emptied <*>", 1.5, t_last + 17 * MIN_NS);
    let m = decode(&alert);
    assert_eq!(m.kind, AlertKind::Silence);
    assert_eq!((m.started_at_ns, m.window_count), (t_last, 0));
    assert!(m.example_trace_ids.is_empty());
    assert_eq!(
        summary(&m),
        "cart emptied <*> has been silent for 17 min in cart"
    );
}
