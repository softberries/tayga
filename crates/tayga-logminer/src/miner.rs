//! Drain state plus the conversions between Drain/alert types and storage rows. Pure: no I/O.

use serde_json::{Value, json};
use tayga_drain::detect::{Alert, AlertKind};
use tayga_drain::drain::{Assignment, Cluster, Drain, DrainConfig};
use tayga_store::logs::{LogAlertRow, LogHitRow, LogTemplateRow};
use tayga_store::rows::LogRow;

/// `log_alerts.kind` (Enum8): 1 new, 2 spike.
const KIND_NEW: i8 = 1;
const KIND_SPIKE: i8 = 2;
const KIND_SILENCE: i8 = 3;

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

    /// Whether a `new` candidate `template` (a stored template string) of `service` is only a kept
    /// status code split out of a template that existed before the masking epoch: see
    /// [`Drain::would_have_matched_pre_epoch`]. Restored clusters carry their stored
    /// `first_seen`, so this works across restarts.
    pub fn would_have_matched_pre_epoch(
        &self,
        service: &str,
        template: &str,
        epoch_start_ns: i64,
    ) -> bool {
        let tokens: Vec<String> = template.split(' ').map(str::to_string).collect();
        self.drain
            .would_have_matched_pre_epoch(service, &tokens, epoch_start_ns)
    }

    /// Current template text of `template_id`, if the miner knows it.
    pub fn template(&self, template_id: u64) -> Option<String> {
        self.drain.cluster(template_id).map(Cluster::template)
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
            AlertKind::Silence => KIND_SILENCE,
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
        baseline_day: a.baseline_day,
        baseline_week: a.baseline_week,
    }
}

/// Inverse of [`alert_row`], for restoring active spike alerts. `None` for an unknown kind.
pub fn alert_from_row(r: &LogAlertRow) -> Option<Alert> {
    let kind = match r.kind {
        KIND_NEW => AlertKind::New,
        KIND_SPIKE => AlertKind::Spike,
        KIND_SILENCE => AlertKind::Silence,
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
        baseline_day: r.baseline_day,
        baseline_week: r.baseline_week,
        example_trace_ids: r.example_trace_ids.clone(),
    })
}

/// The `tayga.alerts` message: `template_id` as a decimal string (u64 does not fit a JS number).
pub fn alert_json(a: &Alert) -> Value {
    let mut v = json!({
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
    });
    // Seasonal comparators are optional: absent in flat mode and when no past window counted.
    if let Some(d) = a.baseline_day {
        v["baseline_day"] = json!(d);
    }
    if let Some(w) = a.baseline_week {
        v["baseline_week"] = json!(w);
    }
    v
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
            baseline_day: Some(4.0),
            baseline_week: None,
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
    fn kept_status_codes_stay_exact_across_a_restart() {
        let line = |status: u16, path: &str| {
            format!(
                r#"[2026-10-05T10:00:00.000Z] "GET {path} HTTP/1.1" {status} - 0 9 1 - "-" "py""#
            )
        };
        let mut m = Miner::new(DrainConfig::default());
        let (_, ok) = m.mine(&log(1, &line(200, "/a"), "t1"));
        let (_, err) = m.mine(&log(2, &line(503, "/a"), "t2"));
        m.mine(&log(3, &line(503, "/b"), "t3"));
        assert_ne!(ok.template_id, err.template_id);
        let rows = m.dirty_templates(5);
        assert_eq!(rows.len(), 2);
        let err_row = rows
            .iter()
            .find(|r| r.template_id == err.template_id)
            .unwrap();
        assert!(err_row.template.contains(" 503 "), "{}", err_row.template);

        // Rebuilt from the stored strings, the 503 template is still exact-match only.
        let mut r = Miner::new(DrainConfig::default());
        r.restore(rows);
        let (_, a) = r.mine(&log(4, &line(503, "/c"), "t4"));
        assert_eq!((a.created, a.template_id), (false, err.template_id));
        let (_, a) = r.mine(&log(5, &line(200, "/c"), "t5"));
        assert_eq!((a.created, a.template_id), (false, ok.template_id));
        let (_, a) = r.mine(&log(6, &line(500, "/c"), "t6"));
        assert!(
            a.created,
            "a 500 is absorbed by neither the 200 nor the 503 template"
        );
        let after = r.dirty_templates(7);
        let restored_err = after
            .iter()
            .find(|t| t.template_id == err.template_id)
            .unwrap();
        assert!(
            restored_err.template.contains(" 503 "),
            "{}",
            restored_err.template
        );
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

    fn access(status: u16, flags: &str) -> String {
        format!(
            r#"[2026-10-05T18:49:39.000Z] "GET /api/cart HTTP/1.1" {status} {flags} upstream_reset 0 95 2 - "-" "py""#
        )
    }

    #[test]
    fn pre_epoch_match_survives_a_restore_and_spares_non_http_templates() {
        const EPOCH: i64 = 10_000;
        // Before the epoch: one `<*>`-status template (masking v1 shape), stored and restored.
        let old_row = LogTemplateRow {
            template_id: 1,
            service: "payment".into(),
            template: r#"<*> "GET <*> <*> <*> <*> upstream_reset <*> <*> <*> - "-" "py""#.into(),
            first_seen: EPOCH - 5,
            last_seen: EPOCH - 1,
            count: 13,
            max_severity: 9,
            sample: String::new(),
            version: 1,
        };
        let mut m = Miner::new(DrainConfig::default());
        m.restore(vec![old_row]);
        let mut l = log(1, &access(503, "UC"), "t1");
        l.ts = EPOCH + 100;
        let (_, a) = m.mine(&l);
        assert!(a.created);
        let rows = m.dirty_templates(1);
        let new = rows
            .iter()
            .find(|r| r.template_id == a.template_id)
            .unwrap();
        assert!(new.template.contains(" 503 "), "{}", new.template);
        assert!(m.would_have_matched_pre_epoch("payment", &new.template, EPOCH));

        // A new non-HTTP template is judged as before, even with an old template of its shape.
        let mut l = log(2, "Payment failed for order 1234", "t2");
        l.ts = EPOCH + 200;
        let (_, b) = m.mine(&l);
        let rows = m.dirty_templates(2);
        let t = &rows
            .iter()
            .find(|r| r.template_id == b.template_id)
            .unwrap()
            .template;
        assert!(!m.would_have_matched_pre_epoch("payment", t, EPOCH));
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
        assert_eq!(v["baseline_day"], 4.0);
        assert!(v.get("baseline_week").is_none(), "absent, not null");
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
        n.kind = AlertKind::Silence;
        let row = alert_row(&n, 1);
        assert_eq!(row.kind, KIND_SILENCE);
        assert_eq!(alert_from_row(&row).unwrap().kind, AlertKind::Silence);
        assert_eq!(alert_json(&n)["kind"], "silence");
        let mut bad = row;
        bad.kind = 0;
        assert_eq!(alert_from_row(&bad), None);
    }
}
