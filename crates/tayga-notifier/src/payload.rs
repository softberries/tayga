//! The alert as the logminer publishes it on `tayga.alerts` (`tayga_logminer::miner::alert_json`),
//! and the two outbound formats built from it: a generic JSON webhook and a Slack `blocks` message.
//! Both are pure functions of the alert and the app's public URL.

use serde::Deserialize;
use serde_json::{Value, json};

const MIN_NS: i64 = 60_000_000_000;
/// Trace links per message, in both formats; `example_trace_ids` itself is passed on whole.
pub const MAX_TRACE_LINKS: usize = 3;
/// Slack caps a header's plain text at 150 characters and a section's text at 3000.
const SLACK_HEADER_MAX: usize = 150;
const SLACK_TEMPLATE_MAX: usize = 2_800;
/// The top-level `text` fallback (Slack truncates beyond 40 000).
const SLACK_TEXT_MAX: usize = 3_000;

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AlertKind {
    New,
    Spike,
    Silence,
}

impl AlertKind {
    pub const ALL: [AlertKind; 3] = [AlertKind::New, AlertKind::Spike, AlertKind::Silence];

    pub fn as_str(self) -> &'static str {
        match self {
            AlertKind::New => "new",
            AlertKind::Spike => "spike",
            AlertKind::Silence => "silence",
        }
    }
}

/// One record of `tayga.alerts`. A silence alert starts at the template's last hit, has
/// `window_count` 0 and no example traces; `last_at_ns − started_at_ns` is how long it has been
/// quiet. The seasonal `baseline_day` / `baseline_week` keys are optional and not delivered.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct AlertMsg {
    pub alert_id: String,
    pub kind: AlertKind,
    /// A u64 in decimal, as a string.
    pub template_id: String,
    pub service: String,
    pub template: String,
    pub started_at_ns: i64,
    pub last_at_ns: i64,
    pub window_count: u64,
    pub peak_count: u64,
    pub baseline_per_window: f64,
    pub example_trace_ids: Vec<String>,
}

impl AlertMsg {
    /// Whole minutes a silent template has been quiet.
    pub fn silent_min(&self) -> i64 {
        self.last_at_ns.saturating_sub(self.started_at_ns).max(0) / MIN_NS
    }
}

/// UTC, millisecond precision: `2023-11-14T22:13:20.123Z`.
pub fn rfc3339(ns: i64) -> String {
    let secs = ns.div_euclid(1_000_000_000);
    let millis = ns.rem_euclid(1_000_000_000) / 1_000_000;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        sod / 3_600,
        sod % 3_600 / 60,
        sod % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// One line for notifications and Slack's `text` fallback.
pub fn summary(alert: &AlertMsg) -> String {
    let (t, s) = (&alert.template, &alert.service);
    match alert.kind {
        AlertKind::New => format!("New log template in {s}: {t}"),
        AlertKind::Spike => format!(
            "{t} spiked to {} per window in {s} (baseline {:.1})",
            alert.window_count, alert.baseline_per_window
        ),
        AlertKind::Silence => format!("{t} has been silent for {} min in {s}", alert.silent_min()),
    }
}

fn template_link(alert: &AlertMsg, base: &str) -> String {
    format!("{base}/logs/templates/{}", alert.template_id)
}

fn trace_links(alert: &AlertMsg, base: &str) -> Vec<String> {
    alert
        .example_trace_ids
        .iter()
        .take(MAX_TRACE_LINKS)
        .map(|id| format!("{base}/traces/{id}"))
        .collect()
}

/// The generic webhook body (spec 7b §4).
pub fn webhook_payload(alert: &AlertMsg, public_url: &str) -> Value {
    let base = public_url.trim_end_matches('/');
    json!({
        "alert_id": alert.alert_id,
        "kind": alert.kind.as_str(),
        "service": alert.service,
        "template_id": alert.template_id,
        "template": alert.template,
        "started_at": rfc3339(alert.started_at_ns),
        "last_at": rfc3339(alert.last_at_ns),
        "count": alert.window_count,
        "baseline": alert.baseline_per_window,
        // Unescaped: a webhook consumer formats it for its own medium.
        "summary": summary(alert),
        "example_trace_ids": alert.example_trace_ids,
        "links": {
            "template": template_link(alert, base),
            "traces": trace_links(alert, base),
        },
    })
}

/// Slack mrkdwn control characters escaped, at most `max` characters (the last one `…` when
/// cut). An entity is never cut.
fn escape(text: &str, max: usize) -> String {
    let entity = |c: char| match c {
        '&' => Some("&amp;"),
        '<' => Some("&lt;"),
        '>' => Some("&gt;"),
        _ => None,
    };
    let len = |c: char| entity(c).map_or(1, str::len);
    let cut = text.chars().map(len).sum::<usize>() > max;
    let budget = if cut { max - 1 } else { max };
    let (mut out, mut used) = (String::new(), 0);
    for c in text.chars() {
        if used + len(c) > budget {
            break;
        }
        match entity(c) {
            Some(e) => out.push_str(e),
            None => out.push(c),
        }
        used += len(c);
    }
    if cut {
        out.push('…');
    }
    out
}

/// At most `max` characters, the last one `…` when cut.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn mrkdwn(text: String) -> Value {
    json!({ "type": "mrkdwn", "text": text })
}

fn button(action_id: &str, label: &str, url: String) -> Value {
    json!({
        "type": "button",
        "action_id": action_id,
        "text": { "type": "plain_text", "text": label },
        "url": url,
    })
}

/// A Slack incoming-webhook message: kind and service in the header, the template in a code
/// block, count vs baseline, and buttons to the template and up to [`MAX_TRACE_LINKS`] traces.
pub fn slack_payload(alert: &AlertMsg, public_url: &str) -> Value {
    let base = public_url.trim_end_matches('/');
    let title = match alert.kind {
        AlertKind::New => "New log template",
        AlertKind::Spike => "Log spike",
        AlertKind::Silence => "Log silence",
    };
    let header = truncate(&format!("{title} in {}", alert.service), SLACK_HEADER_MAX);
    // No backtick at all, so the template can neither close the code block nor merge with its
    // fences (a template starting or ending with one).
    let template = escape(&alert.template.replace('`', "\u{2CB}"), SLACK_TEMPLATE_MAX);
    let started = rfc3339(alert.started_at_ns);
    let baseline = format!("*Baseline*\n{:.1} per window", alert.baseline_per_window);
    let fields = match alert.kind {
        AlertKind::New => vec![mrkdwn(format!("*First seen*\n{started}"))],
        AlertKind::Spike => vec![
            mrkdwn(format!("*Count*\n{} per window", alert.window_count)),
            mrkdwn(baseline),
            mrkdwn(format!("*Started*\n{started}")),
        ],
        AlertKind::Silence => vec![
            mrkdwn(format!("*Silent for*\n{} min", alert.silent_min())),
            mrkdwn(format!("*Last hit*\n{started}")),
            mrkdwn(baseline),
        ],
    };
    let mut buttons = vec![button(
        "template",
        "Open template",
        template_link(alert, base),
    )];
    for (i, url) in trace_links(alert, base).into_iter().enumerate() {
        let n = i + 1;
        buttons.push(button(&format!("trace-{n}"), &format!("Trace {n}"), url));
    }
    json!({
        "text": escape(&summary(alert), SLACK_TEXT_MAX),
        "blocks": [
            { "type": "header", "text": { "type": "plain_text", "text": header } },
            { "type": "section", "text": mrkdwn(format!("```{template}```")) },
            { "type": "section", "fields": fields },
            { "type": "actions", "elements": buttons },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000_123_000_000; // 2023-11-14T22:13:20.123Z

    fn spike() -> AlertMsg {
        AlertMsg {
            alert_id: "a1b2c3d4e5f60718".into(),
            kind: AlertKind::Spike,
            template_id: "1234567890123".into(),
            service: "checkout".into(),
            template: "payment <*> declined for order <*>".into(),
            started_at_ns: T0,
            last_at_ns: T0 + 5 * MIN_NS,
            window_count: 42,
            peak_count: 42,
            baseline_per_window: 3.25,
            example_trace_ids: ["t1", "t2", "t3", "t4"].map(String::from).to_vec(),
        }
    }

    fn silence() -> AlertMsg {
        AlertMsg {
            alert_id: "ffee000011112222".into(),
            kind: AlertKind::Silence,
            started_at_ns: T0,
            last_at_ns: T0 + 12 * MIN_NS + 59_000_000_000,
            window_count: 0,
            peak_count: 0,
            example_trace_ids: vec![],
            ..spike()
        }
    }

    #[test]
    fn rfc3339_is_utc_with_millis() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(T0), "2023-11-14T22:13:20.123Z");
        assert_eq!(
            rfc3339(1_709_208_000 * 1_000_000_000),
            "2024-02-29T12:00:00.000Z"
        );
        assert_eq!(
            rfc3339(951_868_800 * 1_000_000_000),
            "2000-03-01T00:00:00.000Z"
        );
        assert_eq!(rfc3339(-1_000_000), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn decodes_the_logminer_record_and_ignores_seasonal_keys() {
        let raw = r#"{"alert_id":"x","kind":"silence","template_id":"18446744073709551614",
            "service":"s","template":"t","started_at_ns":1,"last_at_ns":2,"window_count":0,
            "peak_count":0,"baseline_per_window":0.5,"example_trace_ids":[],"baseline_day":4.0}"#;
        let a: AlertMsg = serde_json::from_str(raw).unwrap();
        assert_eq!(a.kind, AlertKind::Silence);
        assert_eq!(a.template_id, "18446744073709551614");
        assert!(serde_json::from_str::<AlertMsg>(&raw.replace("silence", "other")).is_err());
    }

    #[test]
    fn silence_summary_counts_whole_quiet_minutes() {
        assert_eq!(silence().silent_min(), 12);
        assert_eq!(
            summary(&silence()),
            "payment <*> declined for order <*> has been silent for 12 min in checkout"
        );
        assert_eq!(
            summary(&spike()),
            "payment <*> declined for order <*> spiked to 42 per window in checkout (baseline 3.2)"
        );
        let new = AlertMsg {
            kind: AlertKind::New,
            ..spike()
        };
        assert_eq!(
            summary(&new),
            "New log template in checkout: payment <*> declined for order <*>"
        );
    }

    #[test]
    fn webhook_payload_snapshot() {
        let v = webhook_payload(&spike(), "http://localhost:8090/");
        assert_eq!(
            v,
            json!({
                "alert_id": "a1b2c3d4e5f60718",
                "kind": "spike",
                "service": "checkout",
                "template_id": "1234567890123",
                "template": "payment <*> declined for order <*>",
                "started_at": "2023-11-14T22:13:20.123Z",
                "last_at": "2023-11-14T22:18:20.123Z",
                "count": 42,
                "baseline": 3.25,
                "summary": "payment <*> declined for order <*> spiked to 42 per window in checkout (baseline 3.2)",
                "example_trace_ids": ["t1", "t2", "t3", "t4"],
                "links": {
                    "template": "http://localhost:8090/logs/templates/1234567890123",
                    "traces": [
                        "http://localhost:8090/traces/t1",
                        "http://localhost:8090/traces/t2",
                        "http://localhost:8090/traces/t3"
                    ]
                }
            })
        );
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys[0], "alert_id", "field order follows the spec");
        let s = webhook_payload(&silence(), "https://tayga.example");
        assert_eq!(
            (s["kind"].as_str(), s["count"].as_u64()),
            (Some("silence"), Some(0))
        );
        assert_eq!(s["links"]["traces"], json!([]));
        assert_eq!(s["last_at"], "2023-11-14T22:26:19.123Z");
        assert_eq!(
            s["summary"],
            "payment <*> declined for order <*> has been silent for 12 min in checkout"
        );
    }

    #[test]
    fn slack_payload_snapshot() {
        let v = slack_payload(&spike(), "http://localhost:8090");
        assert_eq!(
            v,
            json!({
                "text": "payment &lt;*&gt; declined for order &lt;*&gt; spiked to 42 per window in checkout (baseline 3.2)",
                "blocks": [
                    { "type": "header", "text": { "type": "plain_text", "text": "Log spike in checkout" } },
                    { "type": "section", "text": { "type": "mrkdwn", "text": "```payment &lt;*&gt; declined for order &lt;*&gt;```" } },
                    { "type": "section", "fields": [
                        { "type": "mrkdwn", "text": "*Count*\n42 per window" },
                        { "type": "mrkdwn", "text": "*Baseline*\n3.2 per window" },
                        { "type": "mrkdwn", "text": "*Started*\n2023-11-14T22:13:20.123Z" }
                    ] },
                    { "type": "actions", "elements": [
                        { "type": "button", "action_id": "template", "text": { "type": "plain_text", "text": "Open template" }, "url": "http://localhost:8090/logs/templates/1234567890123" },
                        { "type": "button", "action_id": "trace-1", "text": { "type": "plain_text", "text": "Trace 1" }, "url": "http://localhost:8090/traces/t1" },
                        { "type": "button", "action_id": "trace-2", "text": { "type": "plain_text", "text": "Trace 2" }, "url": "http://localhost:8090/traces/t2" },
                        { "type": "button", "action_id": "trace-3", "text": { "type": "plain_text", "text": "Trace 3" }, "url": "http://localhost:8090/traces/t3" }
                    ] }
                ]
            })
        );
    }

    #[test]
    fn slack_silence_and_new_messages() {
        let v = slack_payload(&silence(), "http://localhost:8090");
        assert_eq!(
            v["text"],
            "payment &lt;*&gt; declined for order &lt;*&gt; has been silent for 12 min in checkout"
        );
        assert_eq!(v["blocks"][0]["text"]["text"], "Log silence in checkout");
        assert_eq!(v["blocks"][2]["fields"][0]["text"], "*Silent for*\n12 min");
        assert_eq!(
            v["blocks"][2]["fields"][1]["text"],
            "*Last hit*\n2023-11-14T22:13:20.123Z"
        );
        assert_eq!(
            v["blocks"][2]["fields"][2]["text"],
            "*Baseline*\n3.2 per window"
        );
        let buttons = v["blocks"][3]["elements"].as_array().unwrap();
        assert_eq!(buttons.len(), 1, "no traces for a silence: {buttons:?}");

        let new = AlertMsg {
            kind: AlertKind::New,
            example_trace_ids: vec!["t9".into()],
            ..spike()
        };
        let v = slack_payload(&new, "http://localhost:8090");
        assert_eq!(
            v["blocks"][0]["text"]["text"],
            "New log template in checkout"
        );
        assert_eq!(
            v["blocks"][2]["fields"],
            json!([{ "type": "mrkdwn", "text": "*First seen*\n2023-11-14T22:13:20.123Z" }])
        );
        assert_eq!(
            v["blocks"][3]["elements"][1]["url"],
            "http://localhost:8090/traces/t9"
        );
    }

    #[test]
    fn slack_text_is_escaped_and_bounded() {
        let long = AlertMsg {
            service: "s".repeat(300),
            template: format!("a ``` b & {}", "x".repeat(5_000)),
            ..spike()
        };
        let v = slack_payload(&long, "http://h");
        let header = v["blocks"][0]["text"]["text"].as_str().unwrap();
        assert!(
            header.chars().count() <= SLACK_HEADER_MAX,
            "{}",
            header.len()
        );
        assert!(header.ends_with('…'));
        let code = v["blocks"][1]["text"]["text"].as_str().unwrap();
        assert!(code.chars().count() <= 3_000);
        assert!(code.starts_with("```a ˋˋˋ b &amp; x"), "{}", &code[..30]);
        assert!(code.ends_with("…```"));
        assert_eq!(
            code.matches("```").count(),
            2,
            "the template cannot close the code block"
        );
        let amps = AlertMsg {
            template: "&".repeat(5_000),
            ..spike()
        };
        let code = slack_payload(&amps, "http://h")["blocks"][1]["text"]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(code.chars().count() <= 3_000, "{}", code.len());
        assert!(code.ends_with("&amp;…```"), "an entity is never cut");
    }

    #[test]
    fn slack_code_block_has_no_backtick_but_its_fences() {
        for template in ["`edge`", "``x", "y``", "```", "a`b"] {
            let a = AlertMsg {
                template: template.into(),
                ..spike()
            };
            let v = slack_payload(&a, "http://h");
            let code = v["blocks"][1]["text"]["text"].as_str().unwrap();
            let inner = code
                .strip_prefix("```")
                .and_then(|c| c.strip_suffix("```"))
                .unwrap();
            assert!(!inner.contains('`'), "{template:?} -> {code:?}");
            assert_eq!(inner, template.replace('`', "ˋ"));
        }
        let v = slack_payload(
            &AlertMsg {
                template: "`edge`".into(),
                ..spike()
            },
            "http://h",
        );
        assert_eq!(v["blocks"][1]["text"]["text"], "```ˋedgeˋ```");
    }
}
