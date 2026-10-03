//! Server-side rendering helpers. Output contains only numbers and fixed markup, so the
//! templates may emit it with `|safe`.

use crate::model::TraceSpanRow;
use std::collections::{HashMap, HashSet};

/// One point per `step_secs` bucket in `[from, to]` (unix seconds at bucket starts); points are
/// summed into the bucket they fall in, missing buckets = 0. Capped at 10081 points.
pub fn sparkline(
    points: &[(u32, u64)],
    from: u32,
    to: u32,
    step_secs: u32,
    width: u32,
    height: u32,
) -> String {
    let step_secs = step_secs.max(1);
    // Handle reversed or invalid window: treat as single point at zero
    if from > to {
        let y = height.saturating_sub(1);
        return format!(
            "<svg class=\"spark\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><polyline fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" points=\"0.0,{y} {}.0,{y}\"/></svg>",
            width,
        );
    }
    // Allocation guard for degenerate windows/steps: 7 days of minutes + 1.
    const MAX_POINTS: usize = 7 * 24 * 60 + 1;
    let n = (((to - from) / step_secs + 1).max(2) as usize).min(MAX_POINTS);
    let mut series = vec![0u64; n];
    for &(t, count) in points {
        if t >= from && t <= to {
            let idx = ((t - from) / step_secs) as usize;
            if let Some(slot) = series.get_mut(idx) {
                *slot += count;
            }
        }
    }
    let max = series.iter().copied().max().unwrap_or(0).max(1) as f64;
    let step = f64::from(width) / (n - 1) as f64;
    let height_offset = f64::from(height.saturating_sub(2));
    let coords: Vec<String> = series
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = i as f64 * step;
            let y = f64::from(height) - (v as f64 / max) * height_offset - 1.0;
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
    // Use i128 to safely compute window bounds without overflow
    let start_i128 = i128::from(spans.iter().map(|s| s.start_ns).min().unwrap_or(0));
    let end_i128 = spans
        .iter()
        .map(|s| {
            let s_start = i128::from(s.start_ns);
            let duration = s.duration_ns as i128;
            s_start.saturating_add(duration)
        })
        .max()
        .unwrap_or(start_i128);
    let total = (end_i128 - start_i128).max(1) as f64;
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
            let offset = i128::from(s.start_ns).saturating_sub(start_i128) as f64;
            let left = (offset / total * 100.0).clamp(0.0, 100.0);
            let width = (s.duration_ns as f64 / total * 100.0).max(0.3);
            let clamped_width = width.min((100.0 - left).max(0.3));
            WaterfallRow {
                span_id: s.span_id.clone(),
                depth,
                service: s.service_name.clone(),
                name: s.span_name.clone(),
                left: format!("{left:.2}"),
                width: format!("{clamped_width:.2}"),
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
        let svg = sparkline(&[(60, 2), (180, 4)], 60, 180, 60, 100, 20);
        assert!(svg.starts_with("<svg") && svg.contains("polyline"));
        // Count points by parsing coordinates: each point is "x,y" and they're space-separated
        let points_str = svg
            .split("points=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        let point_count = if points_str.is_empty() {
            0
        } else {
            points_str.split(' ').count()
        };
        assert_eq!(point_count, 3, "three minutes → three points: {svg}");
        assert!(sparkline(&[], 0, 0, 60, 100, 20).contains("points=\"0.0,19.0 100.0,19.0\""));
    }

    #[test]
    fn sparkline_caps_huge_windows() {
        // from=0, to=u32::MAX should not allocate unbounded memory
        let svg = sparkline(&[], 0, u32::MAX, 60, 100, 20);
        assert!(svg.starts_with("<svg") && svg.contains("polyline"));
        // Count the number of points in the SVG
        let points_str = svg
            .split("points=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        let point_count = if points_str.is_empty() {
            0
        } else {
            points_str.split(' ').count()
        };
        // Max cap is 7 * 24 * 60 + 1 = 10081 points
        assert!(
            point_count <= 10082,
            "point count {point_count} exceeds cap"
        );
    }

    #[test]
    fn sparkline_steps_by_bucket_width() {
        // 7 days at 5040 s buckets: about 121 points, with per-minute input summed per bucket.
        let to = 1_791_029_520;
        let from = to - 120 * 5040;
        let minutes: Vec<(u32, u64)> = (0..10_080).map(|i| (from + i * 60, 1)).collect();
        let svg = sparkline(&minutes, from, to, 5040, 120, 22);
        let points = svg
            .split("points=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap();
        assert_eq!(points.split(' ').count(), 121);
        assert!(svg.len() < 2_000, "{} bytes", svg.len());
    }

    #[test]
    fn sparkline_tiny_height_does_not_panic() {
        // height 0 and 1 should not cause panics or undefined behavior
        let svg0 = sparkline(&[(0, 1)], 0, 60, 60, 100, 0);
        assert!(svg0.starts_with("<svg"));
        let svg1 = sparkline(&[(0, 1)], 0, 60, 60, 100, 1);
        assert!(svg1.starts_with("<svg"));
    }

    #[test]
    fn sparkline_ignores_points_beyond_cap() {
        // Points far in the future (beyond cap) should not cause out-of-bounds panics
        let svg = sparkline(&[(10_000_000, 1), (60, 5)], 0, u32::MAX, 60, 100, 20);
        assert!(svg.starts_with("<svg") && svg.contains("polyline"));
        // Verify point count is still within cap
        let points_str = svg
            .split("points=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        let point_count = if points_str.is_empty() {
            0
        } else {
            points_str.split(' ').count()
        };
        assert!(
            point_count <= 10082,
            "point count {point_count} exceeds cap"
        );
    }

    #[test]
    fn sparkline_reversed_window_tiny_height() {
        // Reversed window (from > to) with tiny heights should not panic
        let svg0 = sparkline(&[], 10, 5, 60, 100, 0);
        assert!(svg0.starts_with("<svg"));
        let svg1 = sparkline(&[], 10, 5, 60, 100, 1);
        assert!(svg1.starts_with("<svg"));
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

    #[test]
    fn waterfall_extreme_timestamps_do_not_panic() {
        // Extreme timestamps should not cause overflow panics
        let spans = vec![
            span("min", "", i64::MIN, 100),
            span("max", "", i64::MAX - 200, 100),
            span("large_dur", "", 0, u64::MAX),
        ];
        let rows = waterfall(&spans, &HashSet::new(), "");
        assert_eq!(rows.len(), 3);
        // Verify all left/width values parse as valid floats within bounds
        for row in &rows {
            let left: f64 = row.left.parse().expect("left should parse as f64");
            let width: f64 = row.width.parse().expect("width should parse as f64");
            assert!((0.0..=100.0).contains(&left), "left {left} out of bounds");
            assert!(
                (0.0..=100.0).contains(&width),
                "width {width} out of bounds"
            );
        }
    }
}
