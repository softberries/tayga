//! Query and path parameter parsing shared by JSON and HTML routes.

const MAX_SINCE_SECS: u32 = 7 * 24 * 3600;

#[derive(Debug, Clone, PartialEq)]
pub struct GroupFilter {
    pub since_secs: u32,
    pub kind: Option<String>,
    pub service: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertFilter {
    pub since_secs: u32,
    /// `new` or `spike`.
    pub kind: Option<String>,
    pub service: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateFilter {
    pub since_secs: u32,
    pub service: Option<String>,
    /// Case-insensitive substring of the template, trimmed, at most `MAX_Q_CHARS`.
    pub q: Option<String>,
}

pub const MAX_Q_CHARS: usize = 200;

/// Sparkline points per window, at most (one more when the window straddles bucket edges).
pub const SPARK_BUCKETS: u32 = 120;

/// Sparkline bucket width for a window: `since / 120` rounded up to whole minutes, at least 60 s.
/// Keeps every group to about 120 points whatever the window (7d -> 5040 s buckets).
pub fn bucket_secs(since_secs: u32) -> u32 {
    since_secs.div_ceil(SPARK_BUCKETS).div_ceil(60).max(1) * 60
}

/// The trimmed `since` value, or `default` when it is absent, empty or whitespace (a cleared
/// form field submits `since=`).
pub fn since_or<'a>(raw: Option<&'a str>, default: &'a str) -> &'a str {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
}

/// `<n>[smhd]`, between 1 second and 7 days.
pub fn parse_since(raw: &str) -> Result<u32, String> {
    let raw = raw.trim();
    let last_char_pos = raw
        .char_indices()
        .next_back()
        .map(|(i, _)| i)
        .ok_or_else(|| format!("invalid since {raw:?}: expected e.g. 15m, 1h, 7d"))?;
    let (digits, unit) = raw.split_at(last_char_pos);
    let n: u64 = digits
        .parse()
        .map_err(|_| format!("invalid since {raw:?}: expected e.g. 15m, 1h, 7d"))?;
    let secs = match unit {
        "s" => n,
        "m" => n.saturating_mul(60),
        "h" => n.saturating_mul(3600),
        "d" => n.saturating_mul(86_400),
        _ => return Err(format!("invalid since {raw:?}: unit must be s, m, h or d")),
    };
    if secs == 0 || secs > u64::from(MAX_SINCE_SECS) {
        return Err(format!("since must be between 1s and 7d (got {raw})"));
    }
    Ok(secs as u32)
}

/// Fingerprints are u64 rendered as decimal strings.
pub fn parse_fingerprint(raw: &str) -> Result<String, String> {
    raw.parse::<u64>()
        .map(|v| v.to_string())
        .map_err(|_| format!("invalid fingerprint {raw:?}"))
}

/// Story/trace ids: 32 lowercase hex characters.
pub fn parse_hex_id(raw: &str) -> Result<String, String> {
    if raw.len() == 32 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(raw.to_ascii_lowercase())
    } else {
        Err(format!("invalid id {raw:?}: expected 32 hex characters"))
    }
}

/// Builds a filter from optional query values; empty strings mean "no filter".
pub fn group_filter(
    since: Option<&str>,
    kind: Option<&str>,
    service: Option<&str>,
) -> Result<GroupFilter, String> {
    let since_secs = parse_since(since_or(since, "1h"))?;
    let kind = kind.filter(|k| !k.is_empty()).map(str::to_string);
    if let Some(k) = &kind
        && k != "error"
        && k != "slow"
    {
        return Err(format!("invalid kind {k:?}: expected error or slow"));
    }
    Ok(GroupFilter {
        since_secs,
        kind,
        service: service.filter(|s| !s.is_empty()).map(str::to_string),
    })
}

/// Alert filter; the default window is 24h.
pub fn alert_filter(
    since: Option<&str>,
    kind: Option<&str>,
    service: Option<&str>,
) -> Result<AlertFilter, String> {
    let since_secs = parse_since(since_or(since, "24h"))?;
    let kind = kind.filter(|k| !k.is_empty()).map(str::to_string);
    if let Some(k) = &kind
        && k != "new"
        && k != "spike"
    {
        return Err(format!("invalid kind {k:?}: expected new or spike"));
    }
    Ok(AlertFilter {
        since_secs,
        kind,
        service: service.filter(|s| !s.is_empty()).map(str::to_string),
    })
}

/// Template filter; the default window is 1h.
pub fn template_filter(
    since: Option<&str>,
    service: Option<&str>,
    q: Option<&str>,
) -> Result<TemplateFilter, String> {
    let since_secs = parse_since(since_or(since, "1h"))?;
    let q = q.map(str::trim).filter(|q| !q.is_empty());
    if let Some(q) = q
        && q.chars().count() > MAX_Q_CHARS
    {
        return Err(format!("q must be at most {MAX_Q_CHARS} characters"));
    }
    Ok(TemplateFilter {
        since_secs,
        service: service.filter(|s| !s.is_empty()).map(str::to_string),
        q: q.map(str::to_string),
    })
}

/// Most rows a trace search returns.
pub const MAX_TRACE_LIMIT: u32 = 500;
/// Rows a trace search returns when `limit` is not given.
pub const DEFAULT_TRACE_LIMIT: u32 = 100;

/// Service-map health (spec §8). A node is `error` when at least this share of its server and
/// consumer spans in the window failed.
pub const HEALTH_ERROR_RATIO: f64 = 0.05;
/// A node is `slow` when its p99 in the window exceeds this multiple of its baseline p99.
pub const HEALTH_SLOW_FACTOR: f64 = 2.0;
/// The baseline p99 is taken over this many seconds (24h), in the same query as the window.
pub const HEALTH_BASELINE_SECS: u32 = 24 * 3600;

/// Node health from its window error ratio and p99 against the baseline p99 (`ok`, `slow` or
/// `error`; `error` wins). A zero baseline never reads as slow.
pub fn health(error_ratio: f64, p99_ns: f64, baseline_p99_ns: f64) -> &'static str {
    if error_ratio >= HEALTH_ERROR_RATIO {
        "error"
    } else if baseline_p99_ns > 0.0 && p99_ns > HEALTH_SLOW_FACTOR * baseline_p99_ns {
        "slow"
    } else {
        "ok"
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraceFilter {
    pub since_secs: u32,
    pub service: Option<String>,
    /// When true, `service` matches any trace with a span of that service rather than the
    /// trace's endpoint service.
    pub touched: bool,
    pub endpoint: Option<String>,
    pub min_ns: u64,
    pub max_ns: u64,
    pub errors_only: bool,
    pub limit: u32,
}

/// Query values of the trace search, as received.
#[derive(Debug, Default, Clone)]
pub struct TraceParams<'a> {
    pub since: Option<&'a str>,
    pub service: Option<&'a str>,
    pub touched: Option<&'a str>,
    pub endpoint: Option<&'a str>,
    pub min_ms: Option<&'a str>,
    pub max_ms: Option<&'a str>,
    pub errors: Option<&'a str>,
    pub limit: Option<&'a str>,
}

fn non_empty(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|v| !v.is_empty())
}

/// `1`/`true` or `0`/`false`; absent or empty is false.
pub fn parse_flag(name: &str, raw: Option<&str>) -> Result<bool, String> {
    match non_empty(raw) {
        None | Some("0") | Some("false") => Ok(false),
        Some("1") | Some("true") => Ok(true),
        Some(v) => Err(format!("invalid {name} {v:?}: expected 0 or 1")),
    }
}

fn parse_ms(name: &str, raw: Option<&str>) -> Result<Option<u64>, String> {
    non_empty(raw)
        .map(|v| {
            v.parse::<u64>()
                .map_err(|_| format!("invalid {name} {v:?}: expected whole milliseconds"))
        })
        .transpose()
}

fn bounded(name: &str, v: Option<&str>) -> Result<Option<String>, String> {
    match non_empty(v) {
        Some(v) if v.chars().count() > MAX_Q_CHARS => {
            Err(format!("{name} must be at most {MAX_Q_CHARS} characters"))
        }
        v => Ok(v.map(str::to_string)),
    }
}

/// Trace search filter; the default window is 1h, durations are whole milliseconds.
pub fn trace_filter(p: &TraceParams) -> Result<TraceFilter, String> {
    let since_secs = parse_since(since_or(p.since, "1h"))?;
    let min_ns = parse_ms("min_ms", p.min_ms)?.map_or(0, |ms| ms.saturating_mul(1_000_000));
    let max_ns = parse_ms("max_ms", p.max_ms)?.map_or(u64::MAX, |ms| ms.saturating_mul(1_000_000));
    if min_ns > max_ns {
        return Err("min_ms must not exceed max_ms".to_string());
    }
    let limit = match non_empty(p.limit) {
        None => DEFAULT_TRACE_LIMIT,
        Some(v) => v
            .parse::<u32>()
            .ok()
            .filter(|l| (1..=MAX_TRACE_LIMIT).contains(l))
            .ok_or_else(|| format!("invalid limit {v:?}: expected 1 to {MAX_TRACE_LIMIT}"))?,
    };
    Ok(TraceFilter {
        since_secs,
        service: bounded("service", p.service)?,
        touched: parse_flag("touched", p.touched)?,
        endpoint: bounded("endpoint", p.endpoint)?,
        min_ns,
        max_ns,
        errors_only: parse_flag("errors", p.errors)?,
        limit,
    })
}

/// A ⌘K query: trimmed, 1 to `MAX_Q_CHARS` characters.
pub fn parse_q(raw: Option<&str>) -> Result<String, String> {
    match non_empty(raw) {
        None => Err("q is required".to_string()),
        Some(q) if q.chars().count() > MAX_Q_CHARS => {
            Err(format!("q must be at most {MAX_Q_CHARS} characters"))
        }
        Some(q) => Ok(q.to_string()),
    }
}

/// A service name from the path: 1 to `MAX_Q_CHARS` characters.
pub fn parse_service(raw: &str) -> Result<String, String> {
    let n = raw.chars().count();
    if n == 0 || n > MAX_Q_CHARS {
        Err(format!(
            "invalid service: expected 1 to {MAX_Q_CHARS} characters"
        ))
    } else {
        Ok(raw.to_string())
    }
}

/// What a pipeline series computes from the stored samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesKind {
    /// Per-second counter rate.
    Rate,
    /// Last value.
    Gauge,
    /// Histogram median from the `_bucket` series.
    Q50,
    /// Histogram p99 from the `_bucket` series.
    Q99,
}

impl SeriesKind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "rate" => Some(Self::Rate),
            "gauge" => Some(Self::Gauge),
            "q50" => Some(Self::Q50),
            "q99" => Some(Self::Q99),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rate => "rate",
            Self::Gauge => "gauge",
            Self::Q50 => "q50",
            Self::Q99 => "q99",
        }
    }

    /// The quantile for the histogram kinds.
    pub fn quantile(self) -> Option<f64> {
        match self {
            Self::Q50 => Some(0.5),
            Self::Q99 => Some(0.99),
            Self::Rate | Self::Gauge => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeriesQuery {
    pub since_secs: u32,
    /// The stored metric name; for a quantile, the `_bucket` series.
    pub metric: String,
    pub job: Option<String>,
    pub kind: SeriesKind,
    /// Required label values, sorted by key.
    pub labels: Vec<(String, String)>,
}

/// Most label pairs one series query may filter on.
pub const MAX_LABELS: usize = 8;
const MAX_METRIC_CHARS: usize = 200;

/// `^[a-z_:][a-z0-9_:]*$`, at most 200 characters.
pub fn parse_metric(raw: &str) -> Result<String, String> {
    let mut bytes = raw.bytes();
    let ok = raw.len() <= MAX_METRIC_CHARS
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b == b'_' || b == b':')
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b':');
    if ok {
        Ok(raw.to_string())
    } else {
        Err(format!(
            "invalid metric {raw:?}: expected [a-z_:][a-z0-9_:]*"
        ))
    }
}

fn is_label_name(k: &str) -> bool {
    let mut b = k.bytes();
    k.len() <= MAX_METRIC_CHARS
        && b.next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// `k=v` pairs separated by commas, e.g. `kind=spans,stage=decode`. Values cannot hold commas.
pub fn parse_labels(raw: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let Some(raw) = non_empty(raw) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for pair in raw.split(',') {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| format!("invalid label {pair:?}: expected key=value"))?;
        let k = k.trim();
        if !is_label_name(k) {
            return Err(format!("invalid label name {k:?}"));
        }
        if v.chars().count() > MAX_Q_CHARS {
            return Err(format!(
                "label values must be at most {MAX_Q_CHARS} characters"
            ));
        }
        out.push((k.to_string(), v.to_string()));
    }
    if out.len() > MAX_LABELS {
        return Err(format!("at most {MAX_LABELS} labels"));
    }
    out.sort();
    if out.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err("each label may appear once".to_string());
    }
    Ok(out)
}

/// Pipeline series query; the default window is 1h. `kind` is `rate`, `gauge`, `q50` or `q99`.
/// For a quantile, `metric` names the histogram with or without its `_bucket` suffix.
pub fn series_query(
    since: Option<&str>,
    metric: Option<&str>,
    job: Option<&str>,
    kind: Option<&str>,
    labels: Option<&str>,
) -> Result<SeriesQuery, String> {
    let since_secs = parse_since(since_or(since, "1h"))?;
    let metric = parse_metric(non_empty(metric).ok_or("metric is required")?)?;
    let raw_kind = non_empty(kind).unwrap_or_default();
    let kind = SeriesKind::parse(raw_kind)
        .ok_or_else(|| format!("invalid kind {raw_kind:?}: expected rate, gauge, q50 or q99"))?;
    let metric = if kind.quantile().is_some() && !metric.ends_with("_bucket") {
        format!("{metric}_bucket")
    } else {
        metric
    };
    Ok(SeriesQuery {
        since_secs,
        metric,
        job: bounded("job", job)?,
        kind,
        labels: parse_labels(labels)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_units() {
        assert_eq!(parse_since("30s"), Ok(30));
        assert_eq!(parse_since("15m"), Ok(900));
        assert_eq!(parse_since("1h"), Ok(3600));
        assert_eq!(parse_since("7d"), Ok(604_800));
    }

    #[test]
    fn since_rejects_out_of_range() {
        assert!(parse_since("0m").is_err());
        assert!(parse_since("8d").is_err());
        assert!(parse_since("99999999999999d").is_err());
        assert!(parse_since("1w").is_err());
        assert!(parse_since("").is_err());
        assert!(parse_since("h").is_err());
        // Regression: Unicode inputs must not panic
        assert!(parse_since("5é").is_err());
        assert!(parse_since("é").is_err());
        assert!(parse_since("５m").is_err()); // full-width digit
        assert!(parse_since("1h\u{0301}").is_err()); // combining accent
    }

    #[test]
    fn bucket_width_scales_with_window() {
        assert_eq!(bucket_secs(1), 60);
        assert_eq!(bucket_secs(3600), 60);
        assert_eq!(bucket_secs(86_400), 720);
        assert_eq!(bucket_secs(604_800), 5040);
        // Not a whole minute after division: round up.
        assert_eq!(bucket_secs(7201 * 60), 3660);
        assert!(604_800 / bucket_secs(604_800) <= SPARK_BUCKETS);
    }

    #[test]
    fn fingerprint_and_ids() {
        assert_eq!(
            parse_fingerprint("17393964261140422938"),
            Ok("17393964261140422938".into())
        );
        assert!(parse_fingerprint("abc").is_err());
        assert!(parse_fingerprint("-1").is_err());
        assert!(parse_hex_id(&"A".repeat(32)).is_ok());
        assert!(parse_hex_id(&"z".repeat(32)).is_err());
        assert!(parse_hex_id(&"a".repeat(10_000)).is_err());
    }

    #[test]
    fn filter_defaults_and_validation() {
        assert_eq!(
            group_filter(None, Some(""), None).unwrap(),
            GroupFilter {
                since_secs: 3600,
                kind: None,
                service: None
            }
        );
        assert!(group_filter(Some("1h"), Some("bogus"), None).is_err());
        assert_eq!(group_filter(Some(""), None, None).unwrap().since_secs, 3600);
        assert_eq!(
            group_filter(Some("  "), None, None).unwrap().since_secs,
            3600
        );
        assert_eq!(since_or(Some(" 7d "), "1h"), "7d");
        assert_eq!(since_or(None, "24h"), "24h");
        assert_eq!(
            group_filter(Some("5m"), Some("slow"), Some("payment"))
                .unwrap()
                .service
                .as_deref(),
            Some("payment")
        );
    }

    #[test]
    fn alert_and_template_filters() {
        let a = alert_filter(None, Some(""), None).unwrap();
        assert_eq!((a.since_secs, a.kind), (86_400, None));
        assert!(alert_filter(None, Some("error"), None).is_err());
        assert_eq!(
            alert_filter(Some("5m"), Some("spike"), Some("payment"))
                .unwrap()
                .kind
                .as_deref(),
            Some("spike")
        );
        let t = template_filter(None, None, Some("  Found  ")).unwrap();
        assert_eq!((t.since_secs, t.q.as_deref()), (3600, Some("Found")));
        assert_eq!(template_filter(None, None, Some("   ")).unwrap().q, None);
        assert!(template_filter(None, None, Some(&"é".repeat(200))).is_ok());
        assert!(template_filter(None, None, Some(&"é".repeat(201))).is_err());
    }

    #[test]
    fn health_thresholds() {
        assert_eq!(health(0.05, 1.0, 1.0), "error");
        assert_eq!(health(0.049, 1.0, 1.0), "ok");
        assert_eq!(health(0.0, 2.1, 1.0), "slow");
        assert_eq!(health(0.0, 2.0, 1.0), "ok");
        assert_eq!(health(0.0, 5.0, 0.0), "ok");
        assert_eq!(health(0.5, 5.0, 1.0), "error", "error wins over slow");
    }

    #[test]
    fn trace_filter_defaults_and_validation() {
        let f = trace_filter(&TraceParams::default()).unwrap();
        assert_eq!(
            f,
            TraceFilter {
                since_secs: 3600,
                service: None,
                touched: false,
                endpoint: None,
                min_ns: 0,
                max_ns: u64::MAX,
                errors_only: false,
                limit: DEFAULT_TRACE_LIMIT,
            }
        );
        let f = trace_filter(&TraceParams {
            since: Some("15m"),
            service: Some(" payment "),
            touched: Some("1"),
            endpoint: Some(""),
            min_ms: Some("5"),
            max_ms: Some("100"),
            errors: Some("true"),
            limit: Some("500"),
        })
        .unwrap();
        assert_eq!(
            (f.since_secs, f.service.as_deref(), f.touched, f.endpoint),
            (900, Some("payment"), true, None)
        );
        assert_eq!(
            (f.min_ns, f.max_ns, f.errors_only, f.limit),
            (5_000_000, 100_000_000, true, 500)
        );
        for bad in [
            TraceParams {
                limit: Some("501"),
                ..Default::default()
            },
            TraceParams {
                limit: Some("0"),
                ..Default::default()
            },
            TraceParams {
                min_ms: Some("-1"),
                ..Default::default()
            },
            TraceParams {
                min_ms: Some("10"),
                max_ms: Some("5"),
                ..Default::default()
            },
            TraceParams {
                errors: Some("yes"),
                ..Default::default()
            },
            TraceParams {
                since: Some("8d"),
                ..Default::default()
            },
        ] {
            assert!(trace_filter(&bad).is_err(), "{bad:?}");
        }
        let long = "a".repeat(201);
        assert!(
            trace_filter(&TraceParams {
                service: Some(&long),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn q_service_and_metric_validation() {
        assert_eq!(parse_q(Some("  pay ")), Ok("pay".into()));
        assert!(parse_q(Some("  ")).is_err());
        assert!(parse_q(None).is_err());
        assert!(parse_q(Some(&"é".repeat(200))).is_ok());
        assert!(parse_q(Some(&"é".repeat(201))).is_err());
        assert!(parse_service("").is_err());
        assert!(parse_service(&"a".repeat(201)).is_err());
        assert!(parse_metric("tayga_writer_rows_total").is_ok());
        assert!(parse_metric(":a:b_1").is_ok());
        for bad in ["", "1abc", "Tayga", "a-b", "a b", "a'", "é"] {
            assert!(parse_metric(bad).is_err(), "{bad}");
        }
        assert!(parse_metric(&"a".repeat(201)).is_err());
    }

    #[test]
    fn series_query_and_labels() {
        let q = series_query(None, Some("x_total"), Some(""), Some("rate"), None).unwrap();
        assert_eq!(
            q,
            SeriesQuery {
                since_secs: 3600,
                metric: "x_total".into(),
                job: None,
                kind: SeriesKind::Rate,
                labels: vec![],
            }
        );
        let q = series_query(
            Some("7d"),
            Some("h_seconds"),
            Some("tayga-writer"),
            Some("q99"),
            Some("b=2,a=1"),
        )
        .unwrap();
        assert_eq!(q.metric, "h_seconds_bucket");
        assert_eq!(q.kind, SeriesKind::Q99);
        assert_eq!((q.kind.as_str(), q.kind.quantile()), ("q99", Some(0.99)));
        assert_eq!(q.job.as_deref(), Some("tayga-writer"));
        assert_eq!(
            q.labels,
            vec![("a".into(), "1".into()), ("b".into(), "2".into())]
        );
        assert_eq!(
            series_query(None, Some("h_bucket"), None, Some("q50"), None)
                .unwrap()
                .metric,
            "h_bucket"
        );
        assert!(series_query(None, None, None, Some("rate"), None).is_err());
        assert!(series_query(None, Some("x"), None, None, None).is_err());
        assert!(series_query(None, Some("x"), None, Some("q95"), None).is_err());
        assert!(series_query(None, Some("X"), None, Some("gauge"), None).is_err());
        assert!(parse_labels(Some("novalue")).is_err());
        assert!(parse_labels(Some("1k=v")).is_err());
        let many = (0..9)
            .map(|i| format!("k{i}=v"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_labels(Some(&many)).is_err());
        assert_eq!(
            parse_labels(Some("k=")).unwrap(),
            vec![("k".into(), String::new())]
        );
        assert!(parse_labels(Some("k=a,k=b")).is_err());
    }
}
