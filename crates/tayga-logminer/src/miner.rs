//! Drain state plus the conversions between Drain/alert types and storage rows. Pure: no I/O.

use serde_json::{Value, json};
use tayga_drain::detect::{Alert, AlertKind};
use tayga_drain::drain::{Assignment, Cluster, Drain, DrainConfig};
use tayga_store::logs::{LogAlertRow, LogHitRow, LogTemplateRow};
use tayga_store::rows::LogRow;

/// `log_alerts.kind` (Enum8): 1 new, 2 spike.
const KIND_NEW: i8 = 1;
const KIND_SPIKE: i8 = 2;

pub struct Miner {
    drain: Drain,
}

impl Miner {
    pub fn new(cfg: DrainConfig) -> Self {
        Self {
            drain: Drain::new(cfg),
        }
    }

    /// Restores persisted templates in ascending `first_seen` (then id) order, so leaf and
    /// tie order do not depend on the order ClickHouse returned them in.
    pub fn restore(&mut self, mut rows: Vec<LogTemplateRow>) {
        rows.sort_by_key(|r| (r.first_seen, r.template_id));
        for row in &rows {
            self.drain.restore(cluster_from_row(row));
        }
    }

    pub fn mine(&mut self, log: &LogRow) -> (LogHitRow, Assignment) {
        let a = self
            .drain
            .add(&log.service_name, &log.body, log.ts, log.severity_number);
        let hit = LogHitRow {
            log_id: log.log_id,
            template_id: a.template_id,
            service: log.service_name.clone(),
            ts: log.ts,
            severity_number: log.severity_number,
            trace_id: log.trace_id.clone(),
            span_id: log.span_id.clone(),
        };
        (hit, a)
    }

    /// Templates created or changed since the last call, versioned with `now_ns`.
    pub fn dirty_templates(&mut self, now_ns: i64) -> Vec<LogTemplateRow> {
        let version = u64::try_from(now_ns).unwrap_or(0);
        self.drain
            .take_dirty()
            .iter()
            .map(|c| row_from_cluster(c, version))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.drain.len()
    }

    pub fn is_empty(&self) -> bool {
        self.drain.is_empty()
    }
}

pub fn cluster_from_row(r: &LogTemplateRow) -> Cluster {
    Cluster {
        id: r.template_id,
        service: r.service.clone(),
        tokens: r.template.split(' ').map(str::to_string).collect(),
        count: r.count,
        first_seen_ns: r.first_seen,
        last_seen_ns: r.last_seen,
        max_severity: r.max_severity,
        sample: r.sample.clone(),
    }
}

pub fn row_from_cluster(c: &Cluster, version: u64) -> LogTemplateRow {
    LogTemplateRow {
        template_id: c.id,
        service: c.service.clone(),
        template: c.template(),
        first_seen: c.first_seen_ns,
        last_seen: c.last_seen_ns,
        count: c.count,
        max_severity: c.max_severity,
        sample: c.sample.clone(),
        version,
    }
}

pub fn alert_row(a: &Alert, version: u64) -> LogAlertRow {
    LogAlertRow {
        alert_id: a.alert_id.clone(),
        kind: match a.kind {
            AlertKind::New => KIND_NEW,
            AlertKind::Spike => KIND_SPIKE,
        },
        template_id: a.template_id,
        service: a.service.clone(),
        template: a.template.clone(),
        started_at: a.started_at_ns,
        last_at: a.last_at_ns,
        window_count: a.window_count,
        peak_count: a.peak_count,
        baseline_per_window: a.baseline_per_window,
        example_trace_ids: a.example_trace_ids.clone(),
        version,
    }
}

/// Inverse of [`alert_row`], for restoring active spike alerts. `None` for an unknown kind.
pub fn alert_from_row(r: &LogAlertRow) -> Option<Alert> {
    let kind = match r.kind {
        KIND_NEW => AlertKind::New,
        KIND_SPIKE => AlertKind::Spike,
        _ => return None,
    };
    Some(Alert {
        alert_id: r.alert_id.clone(),
        kind,
        template_id: r.template_id,
        service: r.service.clone(),
        template: r.template.clone(),
        started_at_ns: r.started_at,
        last_at_ns: r.last_at,
        window_count: r.window_count,
        peak_count: r.peak_count,
        baseline_per_window: r.baseline_per_window,
        example_trace_ids: r.example_trace_ids.clone(),
    })
}

/// The `tayga.alerts` message: `template_id` as a decimal string (u64 does not fit a JS number).
pub fn alert_json(a: &Alert) -> Value {
    json!({
        "alert_id": a.alert_id,
        "kind": a.kind.as_str(),
        "template_id": a.template_id.to_string(),
        "service": a.service,
        "template": a.template,
        "started_at_ns": a.started_at_ns,
        "last_at_ns": a.last_at_ns,
        "window_count": a.window_count,
        "peak_count": a.peak_count,
        "baseline_per_window": a.baseline_per_window,
        "example_trace_ids": a.example_trace_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(log_id: u64, body: &str, trace_id: &str) -> LogRow {
        LogRow {
            log_id,
            ts: 1_000 + log_id as i64,
            observed_ts: 0,
            trace_id: trace_id.into(),
            span_id: "s1".into(),
            severity_number: 17,
            severity_text: "ERROR".into(),
            service_name: "payment".into(),
            body: body.into(),
            resource_attrs: vec![],
            log_attrs: vec![],
        }
    }

    fn alert() -> Alert {
        Alert {
            alert_id: "00000000000000ab".into(),
            kind: AlertKind::Spike,
            template_id: u64::MAX - 1,
            service: "payment".into(),
            template: "Payment failed <*>".into(),
            started_at_ns: 10,
            last_at_ns: 20,
            window_count: 30,
            peak_count: 40,
            baseline_per_window: 1.5,
            example_trace_ids: vec!["t1".into()],
        }
    }

    #[test]
    fn same_shape_logs_share_one_template_and_yield_two_hits() {
        let mut m = Miner::new(DrainConfig::default());
        let (h1, a1) = m.mine(&log(1, "Payment failed for order 1234", "t1"));
        let (h2, a2) = m.mine(&log(2, "Payment failed for order 9876", "t2"));
        assert!(a1.created);
        assert!(!a2.created);
        assert_eq!(a1.template_id, a2.template_id);
        assert_eq!(m.len(), 1);
        assert_eq!((h1.log_id, h2.log_id), (1, 2));
        assert_eq!(h1.template_id, a1.template_id);
        assert_eq!(h2.template_id, a1.template_id);
        assert_eq!(h2.service, "payment");
        assert_eq!(h2.ts, 1_002);
        assert_eq!(h2.severity_number, 17);
        assert_eq!((h2.trace_id.as_str(), h2.span_id.as_str()), ("t2", "s1"));
    }

    #[test]
    fn restore_round_trip_through_rows_keeps_the_id() {
        let mut m = Miner::new(DrainConfig::default());
        let (_, a) = m.mine(&log(1, "Payment failed for order 1234", "t1"));
        m.mine(&log(2, "Payment failed for order 9876", "t2"));
        let rows = m.dirty_templates(5);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, 5);
        assert_eq!(cluster_from_row(&rows[0]).template(), rows[0].template);

        let mut restored = Miner::new(DrainConfig::default());
        restored.restore(rows.clone());
        assert_eq!(restored.len(), 1);
        assert!(
            restored.dirty_templates(6).is_empty(),
            "restore is not dirty"
        );
        let (hit, b) = restored.mine(&log(3, "Payment failed for order 5555", "t3"));
        assert!(!b.created);
        assert_eq!(b.template_id, a.template_id);
        assert_eq!(hit.template_id, a.template_id);
        let after = restored.dirty_templates(7);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].count, rows[0].count + 1);
    }

    #[test]
    fn restore_order_does_not_depend_on_input_order() {
        let mut m = Miner::new(DrainConfig::default());
        m.mine(&log(1, "Payment failed for order 1234", "t1"));
        m.mine(&log(2, "Connection refused by upstream", "t2"));
        let rows = m.dirty_templates(1);
        let mut reversed = rows.clone();
        reversed.reverse();
        let (mut a, mut b) = (
            Miner::new(DrainConfig::default()),
            Miner::new(DrainConfig::default()),
        );
        a.restore(rows);
        b.restore(reversed);
        let probe = log(3, "Payment failed for order 42", "t3");
        assert_eq!(a.mine(&probe).1.template_id, b.mine(&probe).1.template_id);
    }

    #[test]
    fn dirty_templates_empties_after_a_call() {
        let mut m = Miner::new(DrainConfig::default());
        m.mine(&log(1, "Payment failed for order 1234", "t1"));
        assert_eq!(m.dirty_templates(1).len(), 1);
        assert!(m.dirty_templates(2).is_empty());
    }

    #[test]
    fn alert_json_has_a_string_template_id() {
        let v = alert_json(&alert());
        assert_eq!(v["template_id"], Value::String((u64::MAX - 1).to_string()));
        assert_eq!(v["kind"], "spike");
        assert_eq!(v["started_at_ns"], 10);
        assert_eq!(v["last_at_ns"], 20);
        assert_eq!(v["example_trace_ids"], json!(["t1"]));
    }

    #[test]
    fn alert_row_round_trips_and_encodes_kind() {
        let a = alert();
        let row = alert_row(&a, 99);
        assert_eq!((row.kind, row.version), (KIND_SPIKE, 99));
        assert_eq!(alert_from_row(&row), Some(a.clone()));
        let mut n = a;
        n.kind = AlertKind::New;
        assert_eq!(alert_row(&n, 1).kind, KIND_NEW);
        let mut bad = row;
        bad.kind = 0;
        assert_eq!(alert_from_row(&bad), None);
    }
}
