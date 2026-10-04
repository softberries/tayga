//! A small parser for the OpenMetrics / Prometheus text exposition that `prometheus-client`
//! renders (spec §7). Comment lines (`# HELP`, `# TYPE`, `# EOF`) are skipped; each sample
//! line becomes one `Sample` with its name exactly as written (`_total`, `_bucket`, ...).

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub value: f64,
}

/// The samples of one exposition plus the number of lines that could not be parsed.
#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub samples: Vec<Sample>,
    pub malformed: usize,
}

/// Parses an exposition; malformed lines are skipped.
pub fn parse(text: &str) -> Vec<Sample> {
    parse_counted(text).samples
}

/// Parses an exposition, counting the malformed lines it skips. Never panics.
pub fn parse_counted(text: &str) -> Parsed {
    let mut out = Parsed::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_line(line) {
            Some(s) => out.samples.push(s),
            None => out.malformed += 1,
        }
    }
    out
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == ':'
}

fn parse_line(line: &str) -> Option<Sample> {
    let name_end = line.find(|c: char| !is_name_char(c)).unwrap_or(line.len());
    let name = &line[..name_end];
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let mut rest = &line[name_end..];
    let mut labels = Vec::new();
    if let Some(after) = rest.strip_prefix('{') {
        rest = parse_labels(after, &mut labels)?;
    }
    // An exemplar (` # {...} v [ts]`) follows the sample; it is not part of it.
    let rest = rest.split(" # ").next().unwrap_or_default();
    let mut tokens = rest.split_whitespace();
    let value = parse_value(tokens.next()?)?;
    if let Some(ts) = tokens.next() {
        ts.parse::<f64>().ok()?;
    }
    if tokens.next().is_some() {
        return None;
    }
    Some(Sample {
        name: name.to_string(),
        labels,
        value,
    })
}

/// Parses `k="v",...}` (the text after `{`) into `labels`; returns the text after `}`.
fn parse_labels<'a>(mut s: &'a str, labels: &mut Vec<(String, String)>) -> Option<&'a str> {
    loop {
        s = s.trim_start();
        if let Some(after) = s.strip_prefix('}') {
            return Some(after);
        }
        let key_end = s.find(|c: char| !is_name_char(c))?;
        let key = &s[..key_end];
        if key.is_empty() {
            return None;
        }
        s = s[key_end..].trim_start().strip_prefix('=')?;
        s = s.trim_start().strip_prefix('"')?;
        let mut value = String::new();
        let mut chars = s.char_indices();
        let end = loop {
            let (i, c) = chars.next()?;
            match c {
                '"' => break i,
                '\\' => match chars.next()?.1 {
                    'n' => value.push('\n'),
                    other => value.push(other),
                },
                c => value.push(c),
            }
        };
        labels.push((key.to_string(), value));
        s = s[end + 1..].trim_start();
        if let Some(after) = s.strip_prefix(',') {
            s = after;
        } else if !s.starts_with('}') {
            return None;
        }
    }
}

fn parse_value(token: &str) -> Option<f64> {
    match token {
        "NaN" => Some(f64::NAN),
        "+Inf" | "Inf" => Some(f64::INFINITY),
        "-Inf" => Some(f64::NEG_INFINITY),
        t if t.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.') => {
            t.parse().ok().filter(|v: &f64| v.is_finite())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus_client::encoding::EncodeLabelSet;
    use prometheus_client::metrics::counter::Counter;
    use prometheus_client::metrics::family::Family;
    use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
    use prometheus_client::registry::Registry;

    const WRITER: &str = include_str!("../fixtures/metrics/writer.txt");
    const ASSEMBLER: &str = include_str!("../fixtures/metrics/assembler.txt");
    const LOGMINER: &str = include_str!("../fixtures/metrics/logminer.txt");
    const INGEST: &str = include_str!("../fixtures/metrics/ingest.txt");
    const API: &str = include_str!("../fixtures/metrics/api.txt");
    const EDGE: &str = include_str!("../fixtures/metrics/edge_cases.txt");

    fn labels(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn find<'a>(s: &'a [Sample], name: &str, l: &[(&str, &str)]) -> &'a Sample {
        s.iter()
            .find(|x| x.name == name && x.labels == labels(l))
            .unwrap_or_else(|| panic!("{name} {l:?} not in {s:?}"))
    }

    #[test]
    fn real_fixtures_parse_without_malformed_lines() {
        for (text, n) in [
            (WRITER, 21),
            (ASSEMBLER, 10),
            (LOGMINER, 23),
            (INGEST, 6),
            (API, 1),
        ] {
            let p = parse_counted(text);
            assert_eq!(p.malformed, 0, "{text}");
            assert_eq!(p.samples.len(), n, "{text}");
        }
    }

    #[test]
    fn histogram_keeps_bucket_names_and_inf_bound() {
        let s = parse(WRITER);
        let inf = find(&s, "tayga_writer_batch_seconds_bucket", &[("le", "+Inf")]);
        assert_eq!(inf.value, 3621.0);
        let b = find(&s, "tayga_writer_batch_seconds_bucket", &[("le", "0.005")]);
        assert_eq!(b.value, 62.0);
        assert_eq!(
            find(&s, "tayga_writer_batch_seconds_sum", &[]).value,
            35.428838471999899
        );
        assert_eq!(
            find(&s, "tayga_writer_batch_seconds_count", &[]).value,
            3621.0
        );
    }

    #[test]
    fn counters_and_gauges_from_each_binary() {
        let s = parse(WRITER);
        assert_eq!(
            find(&s, "tayga_writer_rows_inserted_total", &[("kind", "spans")]).value,
            492277.0
        );
        let s = parse(ASSEMBLER);
        assert_eq!(find(&s, "tayga_assembler_open_traces", &[]).value, 275.0);
        let s = parse(LOGMINER);
        assert_eq!(
            find(&s, "tayga_logminer_data_lag_seconds", &[]).value,
            0.394905334
        );
        let s = parse(INGEST);
        assert_eq!(
            find(
                &s,
                "tayga_ingest_records_published_total",
                &[("kind", "logs")]
            )
            .value,
            118985.0
        );
        let s = parse(API);
        assert_eq!(find(&s, "tayga_api_repo_errors_total", &[]).value, 0.0);
    }

    #[test]
    fn edge_cases_parse_and_malformed_lines_are_counted() {
        let p = parse_counted(EDGE);
        let s = &p.samples;
        assert_eq!(
            find(
                s,
                "demo_requests_total",
                &[("method", "GET"), ("path", "/a \"quoted\" \\ path")]
            )
            .value,
            3.0
        );
        assert_eq!(
            find(
                s,
                "demo_requests_total",
                &[("method", "POST"), ("path", "/b")]
            )
            .value,
            4.0,
            "a trailing timestamp is accepted and ignored"
        );
        assert!(find(s, "demo_temperature", &[]).value.is_nan());
        assert_eq!(find(s, "demo_upper", &[]).value, f64::INFINITY);
        assert_eq!(find(s, "demo_lower", &[]).value, f64::NEG_INFINITY);
        assert_eq!(
            find(s, "demo_spaced", &[("a", "x"), ("b", "y")]).value,
            1500.0
        );
        assert_eq!(
            find(s, "demo_exemplar_bucket", &[("le", "0.1")]).value,
            2.0,
            "the exemplar is dropped"
        );
        assert_eq!(s.len(), 7);
        assert_eq!(p.malformed, 6);
    }

    #[test]
    fn garbage_never_panics() {
        for text in [
            "",
            "{",
            "a{",
            "a{b",
            "a{b=",
            "a{b=\"",
            "a{b=\"\\",
            "a{b=\"c\"",
            "a{b=\"c\"}",
            "a 1 x",
            "é 1",
            "a{é=\"x\"} 1",
            "a{b=\"é\\é\"} 1",
            "a{,} 1",
        ] {
            let p = parse_counted(text);
            assert!(p.samples.len() + p.malformed <= 1, "{text}");
        }
        assert_eq!(parse("a{b=\"é\\é\"} 1")[0].labels, labels(&[("b", "éé")]));
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    struct TwoLabels {
        method: String,
        path: String,
    }

    /// A live render of a two-label Family and a histogram roundtrips.
    #[test]
    fn roundtrips_prometheus_client_output() {
        let mut registry = Registry::default();
        let fam: Family<TwoLabels, Counter> = Family::default();
        registry.register("demo_requests", "Requests", fam.clone());
        let hist = Histogram::new(exponential_buckets(0.005, 2.0, 3));
        registry.register("demo_seconds", "Durations", hist.clone());
        fam.get_or_create(&TwoLabels {
            method: "GET".into(),
            path: "/a".into(),
        })
        .inc_by(5);
        hist.observe(0.007);
        let text = tayga_common::metrics::render(&registry);
        assert!(text.ends_with("# EOF\n"), "{text}");

        let p = parse_counted(&text);
        assert_eq!(p.malformed, 0, "{text}");
        assert_eq!(
            find(
                &p.samples,
                "demo_requests_total",
                &[("method", "GET"), ("path", "/a")]
            )
            .value,
            5.0
        );
        assert_eq!(
            find(&p.samples, "demo_seconds_bucket", &[("le", "+Inf")]).value,
            1.0
        );
        assert_eq!(
            find(&p.samples, "demo_seconds_bucket", &[("le", "0.005")]).value,
            0.0
        );
    }

    /// prometheus-client 0.25 writes label values verbatim (no escaping), so a value holding a
    /// quote yields a line that is not valid exposition. It must be counted as malformed, not
    /// misread. Tayga's own label values (kind, job) never contain quotes.
    #[test]
    fn unescaped_quote_from_prometheus_client_is_malformed() {
        let mut registry = Registry::default();
        let fam: Family<TwoLabels, Counter> = Family::default();
        registry.register("demo_requests", "Requests", fam.clone());
        fam.get_or_create(&TwoLabels {
            method: "GET".into(),
            path: "/a \"q\"".into(),
        })
        .inc();
        let text = tayga_common::metrics::render(&registry);
        assert!(
            text.contains("path=\"/a \"q\"\""),
            "encoder escaping changed: {text}"
        );
        let p = parse_counted(&text);
        assert_eq!((p.samples.len(), p.malformed), (0, 1), "{text}");
    }
}
