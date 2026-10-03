//! Server-side rendering helpers. Output contains only numbers and fixed markup, so the
//! templates may emit it with `|safe`.

use crate::model::TraceSpanRow;
use std::collections::{HashMap, HashSet};

/// One point per minute in `[from_minute, to_minute]` (unix seconds at minute starts), missing minutes = 0.
pub fn sparkline(
    points: &[(u32, u64)],
    from_minute: u32,
    to_minute: u32,
    width: u32,
    height: u32,
) -> String {
    let minutes = ((to_minute.saturating_sub(from_minute)) / 60 + 1).max(2) as usize;
    let mut series = vec![0u64; minutes];
    for &(m, count) in points {
        if m >= from_minute && m <= to_minute {
            series[((m - from_minute) / 60) as usize] += count;
        }
    }
    let max = series.iter().copied().max().unwrap_or(0).max(1) as f64;
    let step = f64::from(width) / (minutes - 1) as f64;
    let coords: Vec<String> = series
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = i as f64 * step;
            let y = f64::from(height) - (v as f64 / max) * f64::from(height - 2) - 1.0;
            format!("{x:.1},{y:.1}")
        })
        .collect();
    format!(
        "<svg class=\"spark\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><polyline fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" points=\"{}\"/></svg>",
        coords.join(" ")
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct WaterfallRow {
    pub span_id: String,
    pub depth: usize,
    pub service: String,
    pub name: String,
    /// Percent of the trace window, formatted with two decimals.
    pub left: String,
    pub width: String,
    pub duration_ms: String,
    pub critical: bool,
    pub root_cause: bool,
    pub error: bool,
}

/// Depth-first rows (children by start time); spans unreachable from a root (cycles) are
/// appended at depth 0, so every span appears exactly once.
pub fn waterfall(
    spans: &[TraceSpanRow],
    critical: &HashSet<String>,
    rc_span_id: &str,
) -> Vec<WaterfallRow> {
    if spans.is_empty() {
        return Vec::new();
    }
    let index: HashMap<&str, usize> = spans
        .iter()
        .enumerate()
        .map(|(i, s)| (s.span_id.as_str(), i))
        .collect();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); spans.len()];
    let mut roots = Vec::new();
    for (i, s) in spans.iter().enumerate() {
        match index.get(s.parent_span_id.as_str()) {
            Some(&p) if p != i => children[p].push(i),
            _ => roots.push(i),
        }
    }
    let by_start = |a: &usize, b: &usize| (spans[*a].start_ns, *a).cmp(&(spans[*b].start_ns, *b));
    for c in &mut children {
        c.sort_by(by_start);
    }
    roots.sort_by(by_start);
    let start = spans.iter().map(|s| s.start_ns).min().unwrap_or(0);
    let end = spans
        .iter()
        .map(|s| s.start_ns.saturating_add(s.duration_ns as i64))
        .max()
        .unwrap_or(start);
    let total = (end - start).max(1) as f64;
    let mut visited = vec![false; spans.len()];
    let mut order: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    let walk = |root: usize, visited: &mut Vec<bool>, order: &mut Vec<(usize, usize)>| {
        let mut stack = vec![(root, 0usize)];
        while let Some((i, depth)) = stack.pop() {
            if visited[i] {
                continue;
            }
            visited[i] = true;
            order.push((i, depth));
            for &c in children[i].iter().rev() {
                stack.push((c, depth + 1));
            }
        }
    };
    for &r in &roots {
        walk(r, &mut visited, &mut order);
    }
    for i in 0..spans.len() {
        if !visited[i] {
            walk(i, &mut visited, &mut order);
        }
    }
    order
        .into_iter()
        .map(|(i, depth)| {
            let s = &spans[i];
            let left = (s.start_ns - start) as f64 / total * 100.0;
            let width = (s.duration_ns as f64 / total * 100.0).max(0.3);
            WaterfallRow {
                span_id: s.span_id.clone(),
                depth,
                service: s.service_name.clone(),
                name: s.span_name.clone(),
                left: format!("{left:.2}"),
                width: format!("{:.2}", width.min(100.0 - left).max(0.3)),
                duration_ms: format!("{:.1}", s.duration_ns as f64 / 1e6),
                critical: critical.contains(&s.span_id),
                root_cause: s.span_id == rc_span_id,
                error: s.status == "error",
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(id: &str, parent: &str, start: i64, dur: u64) -> TraceSpanRow {
        TraceSpanRow {
            span_id: id.into(),
            parent_span_id: parent.into(),
            service_name: "svc".into(),
            span_name: format!("op-{id}"),
            kind: "server".into(),
            start_ns: start,
            duration_ns: dur,
            status: if id == "c" {
                "error".into()
            } else {
                "unset".into()
            },
            status_message: String::new(),
        }
    }

    #[test]
    fn sparkline_bins_minutes_and_handles_empty() {
        let svg = sparkline(&[(60, 2), (180, 4)], 60, 180, 100, 20);
        assert!(svg.starts_with("<svg") && svg.contains("polyline"));
        assert_eq!(
            svg.matches(',').count(),
            3,
            "three minutes → three points: {svg}"
        );
        assert!(sparkline(&[], 0, 0, 100, 20).contains("points=\"0.0,19.0 100.0,19.0\""));
    }

    #[test]
    fn waterfall_orders_depth_first_and_marks_spans() {
        let spans = vec![
            span("b", "a", 10, 30),
            span("a", "", 0, 100),
            span("c", "b", 15, 10),
            span("d", "a", 50, 40),
        ];
        let rows = waterfall(
            &spans,
            &HashSet::from(["a".to_string(), "d".to_string()]),
            "c",
        );
        let ids: Vec<(&str, usize)> = rows.iter().map(|r| (r.span_id.as_str(), r.depth)).collect();
        assert_eq!(ids, [("a", 0), ("b", 1), ("c", 2), ("d", 1)]);
        assert!(rows[0].critical && rows[3].critical && !rows[1].critical);
        assert!(rows[2].root_cause && rows[2].error);
        assert_eq!(rows[0].left, "0.00");
        assert_eq!(rows[0].width, "100.00");
    }

    #[test]
    fn waterfall_handles_cycles_and_orphans() {
        let spans = vec![
            span("x", "y", 0, 10),
            span("y", "x", 5, 10),
            span("o", "missing", 1, 1),
            span("s", "s", 2, 1),
        ];
        let rows = waterfall(&spans, &HashSet::new(), "");
        assert_eq!(rows.len(), 4);
        let mut ids: Vec<&str> = rows.iter().map(|r| r.span_id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["o", "s", "x", "y"]);
    }
}
