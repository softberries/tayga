//! Query and path parameter parsing shared by JSON and HTML routes.

const MAX_SINCE_SECS: u32 = 7 * 24 * 3600;

/// How far back a window may start, in seconds before now: the longest TTL among the tables the
/// windowed queries read (`error_stories`, `service_edges`, `log_alerts` and `metric_samples`
/// keep 7 days; see `crates/tayga-store/migrations`). Shorter-lived tables (`spans`, `logs` and
/// `log_template_hits` 3 days, `trace_summaries` 2 days) simply return nothing that old.
pub const RETENTION_SECS: i64 = 7 * 86_400;
/// How far `until` may lie ahead of the API's clock (browser clock skew).
pub const MAX_UNTIL_AHEAD_SECS: i64 = 60;
/// A live window reads rows this far past its end, so rows stamped by a producer whose clock
/// runs slightly ahead are not hidden.
pub const LIVE_SLACK_SECS: i64 = 60;

/// A query window `[until - since, until)` in unix seconds, where `until` defaults to now
/// (rounded up to the next whole second; then the window is `live`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub start: i64,
    pub end: i64,
    /// No `until` was given: the window ends now.
    pub live: bool,
}

impl Window {
    /// The window's length in seconds (`since`).
    pub fn secs(&self) -> u32 {
        u32::try_from(self.end - self.start).unwrap_or(MAX_SINCE_SECS)
    }

    /// Bucket width for the window's series (see `bucket_secs`). Buckets lie on the epoch grid
    /// (multiples of the width), so a live window that moves keeps its bucket edges.
    pub fn step(&self) -> u32 {
        bucket_secs(self.secs())
    }

    /// The exclusive upper bound rows are read up to: `end`, plus `LIVE_SLACK_SECS` when live.
    pub fn upper(&self) -> i64 {
        if self.live {
            self.end + LIVE_SLACK_SECS
        } else {
            self.end
        }
    }
}

/// The current time in unix milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

const UNTIL_FORMAT: &str = "expected RFC 3339 (2026-10-04T12:00:00Z) or unix seconds";

fn digits(s: &str) -> Option<i64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)` as unix seconds; a fraction is dropped.
fn parse_rfc3339(raw: &str) -> Option<i64> {
    let b = raw.as_bytes();
    if b.len() < 20 || !raw.is_ascii() {
        return None;
    }
    let num = |r: std::ops::Range<usize>| digits(&raw[r]);
    let sep = |i: usize, c: &[u8]| c.contains(&b[i]);
    if !(sep(4, b"-") && sep(7, b"-") && sep(10, b"Tt") && sep(13, b":") && sep(16, b":")) {
        return None;
    }
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let mut rest = &raw[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let n = frac.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        rest = &frac[n..];
    }
    let offset = match rest.as_bytes() {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let (oh, om) = (digits(&rest[1..3])?, digits(&rest[4..6])?);
            if oh > 23 || om > 59 {
                return None;
            }
            let o = oh * 3600 + om * 60;
            if *sign == b'+' { o } else { -o }
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s - offset)
}

/// `until`: RFC 3339 (`2026-10-04T12:00:00Z`, any offset) or unix seconds, as unix seconds.
pub fn parse_until(raw: &str) -> Result<i64, String> {
    let raw = raw.trim();
    digits(raw)
        .or_else(|| parse_rfc3339(raw))
        .ok_or_else(|| format!("invalid until {raw:?}: {UNTIL_FORMAT}"))
}

/// The window `[until - since, until]` from the query values. `since` is `<n>[smhd]` (1s to
/// 7d, `default_since` when absent or empty); `until` defaults to now and must lie at most
/// `MAX_UNTIL_AHEAD_SECS` ahead, and the window must start within `RETENTION_SECS`.
pub fn window(
    since: Option<&str>,
    until: Option<&str>,
    default_since: &str,
    now_ms: i64,
) -> Result<Window, String> {
    let secs = i64::from(parse_since(since_or(since, default_since))?);
    let now = now_ms.div_euclid(1000);
    let live = non_empty(until).is_none();
    let end = match non_empty(until) {
        None => (now_ms + 999).div_euclid(1000),
        Some(raw) => {
            let until = parse_until(raw)?;
            if until > now + MAX_UNTIL_AHEAD_SECS {
                return Err(format!(
                    "until must not be more than {MAX_UNTIL_AHEAD_SECS} s in the future"
                ));
            }
            until
        }
    };
    let start = end - secs;
    if start < now - RETENTION_SECS {
        return Err(format!(
            "the window must start within the last {} days (data retention): until minus since is older",
            RETENTION_SECS / 86_400
        ));
    }
    Ok(Window { start, end, live })
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupFilter {
    pub window: Window,
    pub kind: Option<String>,
    pub service: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertFilter {
    pub window: Window,
    /// `new` or `spike`.
    pub kind: Option<String>,
    pub service: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateFilter {
    pub window: Window,
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
    window: Window,
    kind: Option<&str>,
    service: Option<&str>,
) -> Result<GroupFilter, String> {
    let kind = kind.filter(|k| !k.is_empty()).map(str::to_string);
    if let Some(k) = &kind
        && k != "error"
        && k != "slow"
    {
        return Err(format!("invalid kind {k:?}: expected error or slow"));
    }
    Ok(GroupFilter {
        window,
        kind,
        service: service.filter(|s| !s.is_empty()).map(str::to_string),
    })
}

/// Alert filter (the routes default the window to 24h).
pub fn alert_filter(
    window: Window,
    kind: Option<&str>,
    service: Option<&str>,
) -> Result<AlertFilter, String> {
    let kind = kind.filter(|k| !k.is_empty()).map(str::to_string);
    if let Some(k) = &kind
        && k != "new"
        && k != "spike"
    {
        return Err(format!("invalid kind {k:?}: expected new or spike"));
    }
    Ok(AlertFilter {
        window,
        kind,
        service: service.filter(|s| !s.is_empty()).map(str::to_string),
    })
}

/// Template filter (the routes default the window to 1h).
pub fn template_filter(
    window: Window,
    service: Option<&str>,
    q: Option<&str>,
) -> Result<TemplateFilter, String> {
    let q = q.map(str::trim).filter(|q| !q.is_empty());
    if let Some(q) = q
        && q.chars().count() > MAX_Q_CHARS
    {
        return Err(format!("q must be at most {MAX_Q_CHARS} characters"));
    }
    Ok(TemplateFilter {
        window,
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
    pub window: Window,
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

/// Trace search filter (the routes default the window to 1h); durations are whole milliseconds.
pub fn trace_filter(window: Window, p: &TraceParams) -> Result<TraceFilter, String> {
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
        window,
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
    pub window: Window,
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

/// Pipeline series query (the routes default the window to 1h). `kind` is `rate`, `gauge`,
/// `q50` or `q99`. For a quantile, `metric` names the histogram with or without its `_bucket`
/// suffix.
pub fn series_query(
    window: Window,
    metric: Option<&str>,
    job: Option<&str>,
    kind: Option<&str>,
    labels: Option<&str>,
) -> Result<SeriesQuery, String> {
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
        window,
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

    /// 2026-10-04T12:00:00Z, the clock of the window tests.
    const NOW_S: i64 = 1_791_115_200;
    const NOW_MS: i64 = NOW_S * 1000;

    fn w(secs: i64) -> Window {
        Window {
            start: NOW_S - secs,
            end: NOW_S,
            live: false,
        }
    }

    #[test]
    fn until_parses_both_formats() {
        assert_eq!(parse_until("2026-10-04T12:00:00Z"), Ok(NOW_S));
        assert_eq!(parse_until(" 2026-10-04t12:00:00z "), Ok(NOW_S));
        assert_eq!(parse_until("2026-10-04T12:00:00.999Z"), Ok(NOW_S));
        assert_eq!(parse_until("2026-10-04T14:00:00+02:00"), Ok(NOW_S));
        assert_eq!(parse_until("2026-10-04T10:30:00-01:30"), Ok(NOW_S));
        assert_eq!(parse_until("1791115200"), Ok(NOW_S));
        assert_eq!(parse_until("1970-01-01T00:00:00Z"), Ok(0));
        assert_eq!(parse_until("2024-02-29T00:00:00Z"), Ok(1_709_164_800));
        for bad in [
            "",
            "-5",
            "1.5",
            "2026-10-04",
            "2026-10-04T12:00:00",
            "2026-10-04 12:00:00Z",
            "2026-13-04T12:00:00Z",
            "2026-02-29T12:00:00Z",
            "2026-10-04T24:00:00Z",
            "2026-10-04T12:00:00.Z",
            "2026-10-04T12:00:00+2:00",
            "2026-10-04T12:00:00 02:00",
            "２026-10-04T12:00:00Z",
            "99999999999999999999",
        ] {
            assert!(parse_until(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn window_defaults_to_now_and_validates_until() {
        // Live: ends at now, rounded up to the next whole second.
        let live = Window {
            live: true,
            ..w(3600)
        };
        assert_eq!(window(None, None, "1h", NOW_MS), Ok(live));
        assert_eq!((live.upper(), w(3600).upper()), (NOW_S + 60, NOW_S));
        assert_eq!(
            window(Some(""), Some(" "), "24h", NOW_MS + 1),
            Ok(Window {
                start: NOW_S + 1 - 86_400,
                end: NOW_S + 1,
                live: true
            })
        );
        let past = window(Some("2h"), Some("2026-10-03T12:00:00Z"), "1h", NOW_MS).unwrap();
        assert_eq!(
            past,
            Window {
                start: NOW_S - 86_400 - 7200,
                end: NOW_S - 86_400,
                live: false
            }
        );
        assert_eq!((past.secs(), past.step()), (7200, 60));
        // At most 60 s ahead.
        assert!(window(None, Some(&(NOW_S + 60).to_string()), "1h", NOW_MS).is_ok());
        let ahead = window(None, Some(&(NOW_S + 61).to_string()), "1h", NOW_MS);
        assert!(ahead.unwrap_err().contains("future"));
        // The start stays within retention (7 days).
        let edge = (NOW_S - 6 * 86_400).to_string();
        assert!(window(Some("1d"), Some(&edge), "1h", NOW_MS).is_ok());
        let old = window(
            Some("1d"),
            Some(&(NOW_S - 6 * 86_400 - 1).to_string()),
            "1h",
            NOW_MS,
        );
        assert!(old.unwrap_err().contains("retention"));
        assert!(window(Some("7d"), None, "1h", NOW_MS).is_ok());
        assert!(window(Some("15m"), Some("nope"), "1h", NOW_MS).is_err());
        assert!(window(Some("8d"), None, "1h", NOW_MS).is_err());
        assert!(window(Some("0s"), Some(&NOW_S.to_string()), "1h", NOW_MS).is_err());
    }

    #[test]
    fn filter_defaults_and_validation() {
        assert_eq!(
            group_filter(w(3600), Some(""), None).unwrap(),
            GroupFilter {
                window: w(3600),
                kind: None,
                service: None
            }
        );
        assert!(group_filter(w(3600), Some("bogus"), None).is_err());
        assert_eq!(since_or(Some(" 7d "), "1h"), "7d");
        assert_eq!(since_or(None, "24h"), "24h");
        assert_eq!(since_or(Some("  "), "24h"), "24h");
        assert_eq!(
            group_filter(w(300), Some("slow"), Some("payment"))
                .unwrap()
                .service
                .as_deref(),
            Some("payment")
        );
    }

    #[test]
    fn alert_and_template_filters() {
        let a = alert_filter(w(86_400), Some(""), None).unwrap();
        assert_eq!((a.window, a.kind), (w(86_400), None));
        assert!(alert_filter(w(60), Some("error"), None).is_err());
        assert_eq!(
            alert_filter(w(300), Some("spike"), Some("payment"))
                .unwrap()
                .kind
                .as_deref(),
            Some("spike")
        );
        let t = template_filter(w(3600), None, Some("  Found  ")).unwrap();
        assert_eq!((t.window, t.q.as_deref()), (w(3600), Some("Found")));
        assert_eq!(template_filter(w(60), None, Some("   ")).unwrap().q, None);
        assert!(template_filter(w(60), None, Some(&"é".repeat(200))).is_ok());
        assert!(template_filter(w(60), None, Some(&"é".repeat(201))).is_err());
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
        let f = trace_filter(w(3600), &TraceParams::default()).unwrap();
        assert_eq!(
            f,
            TraceFilter {
                window: w(3600),
                service: None,
                touched: false,
                endpoint: None,
                min_ns: 0,
                max_ns: u64::MAX,
                errors_only: false,
                limit: DEFAULT_TRACE_LIMIT,
            }
        );
        let f = trace_filter(
            w(900),
            &TraceParams {
                service: Some(" payment "),
                touched: Some("1"),
                endpoint: Some(""),
                min_ms: Some("5"),
                max_ms: Some("100"),
                errors: Some("true"),
                limit: Some("500"),
            },
        )
        .unwrap();
        assert_eq!(
            (f.window, f.service.as_deref(), f.touched, f.endpoint),
            (w(900), Some("payment"), true, None)
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
        ] {
            assert!(trace_filter(w(60), &bad).is_err(), "{bad:?}");
        }
        let long = "a".repeat(201);
        assert!(
            trace_filter(
                w(60),
                &TraceParams {
                    service: Some(&long),
                    ..Default::default()
                }
            )
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
        let q = series_query(w(3600), Some("x_total"), Some(""), Some("rate"), None).unwrap();
        assert_eq!(
            q,
            SeriesQuery {
                window: w(3600),
                metric: "x_total".into(),
                job: None,
                kind: SeriesKind::Rate,
                labels: vec![],
            }
        );
        let q = series_query(
            w(604_800),
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
            series_query(w(60), Some("h_bucket"), None, Some("q50"), None)
                .unwrap()
                .metric,
            "h_bucket"
        );
        assert!(series_query(w(60), None, None, Some("rate"), None).is_err());
        assert!(series_query(w(60), Some("x"), None, None, None).is_err());
        assert!(series_query(w(60), Some("x"), None, Some("q95"), None).is_err());
        assert!(series_query(w(60), Some("X"), None, Some("gauge"), None).is_err());
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
