//! Series math over stored metric samples (spec §7): counter rates, gauges and
//! `histogram_quantile`-equivalent quantiles. Timestamps in the output are bucket starts in
//! Unix milliseconds.

use std::collections::BTreeMap;
use tayga_store::metrics_store::MetricPointRow;

/// A series' identity: its scrape job and its sorted label set.
type SeriesKey = (String, Vec<(String, String)>);

/// The bucket grid of a series: buckets are `step_secs` wide and start at `origin_ms` (the
/// query window's start), as the stored samples were bucketed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub step_secs: u32,
    pub origin_ms: i64,
}

impl Grid {
    fn step_ms(self) -> i64 {
        i64::from(self.step_secs.max(1)) * 1000
    }

    fn bucket_of(self, ts_ms: i64) -> i64 {
        ts_ms - (ts_ms - self.origin_ms).rem_euclid(self.step_ms())
    }
}

/// Per series, the last finite value in each step bucket.
fn last_per_bucket(
    points: &[MetricPointRow],
    grid: Grid,
) -> BTreeMap<SeriesKey, BTreeMap<i64, f64>> {
    let mut sorted: Vec<&MetricPointRow> = points.iter().filter(|p| p.value.is_finite()).collect();
    sorted.sort_by_key(|p| p.ts_ms);
    let mut out: BTreeMap<SeriesKey, BTreeMap<i64, f64>> = BTreeMap::new();
    for p in sorted {
        let mut labels = p.labels.clone();
        labels.sort();
        out.entry((p.job.clone(), labels))
            .or_default()
            .insert(grid.bucket_of(p.ts_ms), p.value);
    }
    out
}

/// Counter increases between consecutive buckets of one series, as
/// `(bucket, increase, elapsed_ms)`. A drop is a reset: the new value is the increase.
fn increases(buckets: &BTreeMap<i64, f64>) -> impl Iterator<Item = (i64, f64, i64)> + '_ {
    buckets
        .iter()
        .zip(buckets.iter().skip(1))
        .map(|((t0, v0), (t1, v1))| {
            let inc = if v1 >= v0 { v1 - v0 } else { *v1 };
            (*t1, inc, t1 - t0)
        })
}

/// Per-second counter rate per step, summed over series. Each series contributes its increase
/// since its previous sampled bucket divided by the time between them, which is the step
/// when there is no gap.
pub fn rate(points: &[MetricPointRow], grid: Grid) -> Vec<(i64, f64)> {
    let mut sum: BTreeMap<i64, f64> = BTreeMap::new();
    for buckets in last_per_bucket(points, grid).values() {
        for (t, inc, elapsed_ms) in increases(buckets) {
            *sum.entry(t).or_default() += inc * 1000.0 / elapsed_ms as f64;
        }
    }
    sum.into_iter().collect()
}

/// Last value per step, summed over series.
pub fn gauge(points: &[MetricPointRow], grid: Grid) -> Vec<(i64, f64)> {
    let mut sum: BTreeMap<i64, f64> = BTreeMap::new();
    for buckets in last_per_bucket(points, grid).values() {
        for (t, v) in buckets {
            *sum.entry(*t).or_default() += v;
        }
    }
    sum.into_iter().collect()
}

fn parse_le(labels: &[(String, String)]) -> Option<f64> {
    let le = &labels.iter().find(|(k, _)| k == "le")?.1;
    match le.as_str() {
        "+Inf" | "Inf" => Some(f64::INFINITY),
        v => v.parse().ok().filter(|b: &f64| !b.is_nan()),
    }
}

/// The q-quantile per step from `_bucket` counter samples, as Prometheus
/// `histogram_quantile(q, sum by (le) (increase(..._bucket[step])))` computes it. `None` when
/// a step has no observations or the buckets are unusable.
pub fn quantile(bucket_points: &[MetricPointRow], q: f64, grid: Grid) -> Vec<(i64, Option<f64>)> {
    // step -> le -> summed increase.
    let mut steps: BTreeMap<i64, Vec<(f64, f64)>> = BTreeMap::new();
    for ((_, labels), buckets) in last_per_bucket(bucket_points, grid) {
        let Some(le) = parse_le(&labels) else {
            continue;
        };
        for (t, inc, _) in increases(&buckets) {
            steps.entry(t).or_default().push((le, inc));
        }
    }
    steps
        .into_iter()
        .map(|(t, mut b)| {
            b.sort_by(|x, y| x.0.total_cmp(&y.0));
            b.dedup_by(|later, kept| {
                let same = later.0 == kept.0;
                if same {
                    kept.1 += later.1;
                }
                same
            });
            (t, bucket_quantile(q, &b))
        })
        .collect()
}

/// Prometheus `bucketQuantile` over cumulative `(upper_bound, count)` buckets sorted by bound
/// with distinct bounds.
fn bucket_quantile(q: f64, buckets: &[(f64, f64)]) -> Option<f64> {
    if !(0.0..=1.0).contains(&q) || buckets.len() < 2 {
        return None;
    }
    if buckets.last()?.0 != f64::INFINITY {
        return None;
    }
    // Counts of a cumulative histogram never decrease; scrape skew can break that.
    let mut counts: Vec<f64> = Vec::with_capacity(buckets.len());
    for &(_, c) in buckets {
        let prev = counts.last().copied().unwrap_or(0.0);
        counts.push(c.max(prev));
    }
    let observations = *counts.last()?;
    if observations <= 0.0 {
        return None;
    }
    let mut rank = q * observations;
    let last = buckets.len() - 1;
    let b = counts[..last]
        .iter()
        .position(|&c| c >= rank)
        .unwrap_or(last);
    if b == last {
        return Some(buckets[last - 1].0);
    }
    let end = buckets[b].0;
    if b == 0 && end <= 0.0 {
        return Some(end);
    }
    let (start, mut count) = (if b == 0 { 0.0 } else { buckets[b - 1].0 }, counts[b]);
    if b > 0 {
        count -= counts[b - 1];
        rank -= counts[b - 1];
    }
    let v = start + (end - start) * (rank / count);
    v.is_finite().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid starting at the epoch.
    fn epoch(step_secs: u32) -> Grid {
        Grid {
            step_secs,
            origin_ms: 0,
        }
    }

    #[test]
    fn buckets_start_at_the_grid_origin() {
        let g = Grid {
            step_secs: 10,
            origin_ms: 3_000,
        };
        assert_eq!(
            [3_000, 12_999, 13_000, 2_999].map(|t| g.bucket_of(t)),
            [3_000, 3_000, 13_000, -7_000]
        );
        let pts: Vec<MetricPointRow> = [(4_000, 1.0), (12_000, 2.0), (14_000, 6.0)]
            .iter()
            .map(|(ts_ms, value)| MetricPointRow {
                ts_ms: *ts_ms,
                job: "j".into(),
                labels: vec![],
                value: *value,
            })
            .collect();
        assert_eq!(gauge(&pts, g), vec![(3_000, 2.0), (13_000, 6.0)]);
        assert_eq!(rate(&pts, g), vec![(13_000, 0.4)]);
    }

    fn p(ts_s: i64, labels: &[(&str, &str)], value: f64) -> MetricPointRow {
        MetricPointRow {
            ts_ms: ts_s * 1000,
            job: "tayga-writer".into(),
            labels: labels
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            value,
        }
    }

    #[test]
    fn rate_handles_resets() {
        // Step 10 s; one sample per step. 100 -> 130 is +30; 130 -> 20 is a reset (the new
        // value, 20, is the increase); 20 -> 50 is +30.
        let pts = vec![
            p(0, &[], 100.0),
            p(10, &[], 130.0),
            p(20, &[], 20.0),
            p(30, &[], 50.0),
        ];
        assert_eq!(
            rate(&pts, epoch(10)),
            vec![(10_000, 3.0), (20_000, 2.0), (30_000, 3.0)]
        );
    }

    #[test]
    fn rate_uses_last_value_per_bucket_and_input_order_does_not_matter() {
        // Two samples in bucket 0 (the later, 110, counts) and two in bucket 10 (150 counts).
        let pts = vec![
            p(17, &[], 150.0),
            p(5, &[], 110.0),
            p(12, &[], 120.0),
            p(1, &[], 100.0),
        ];
        assert_eq!(rate(&pts, epoch(10)), vec![(10_000, 4.0)]);
    }

    #[test]
    fn rate_sums_label_sets() {
        let pts = vec![
            p(0, &[("kind", "spans")], 0.0),
            p(0, &[("kind", "logs")], 1000.0),
            p(15, &[("kind", "spans")], 150.0),
            p(15, &[("kind", "logs")], 1300.0),
            p(30, &[("kind", "spans")], 300.0),
            // logs has no sample in the third step: only spans contribute there.
        ];
        assert_eq!(rate(&pts, epoch(15)), vec![(15_000, 30.0), (30_000, 10.0)]);
    }

    #[test]
    fn rate_keeps_jobs_apart() {
        let mut a = p(0, &[], 10.0);
        a.job = "a".into();
        let mut b = p(0, &[], 1000.0);
        b.job = "b".into();
        let mut a2 = p(10, &[], 20.0);
        a2.job = "a".into();
        let mut b2 = p(10, &[], 1010.0);
        b2.job = "b".into();
        // Merged into one series, 1000 -> 20 would read as a reset.
        assert_eq!(rate(&[a, b, a2, b2], epoch(10)), vec![(10_000, 2.0)]);
    }

    #[test]
    fn rate_over_a_gap_spreads_the_increase_over_the_elapsed_time() {
        let pts = vec![p(0, &[], 0.0), p(30, &[], 60.0)];
        assert_eq!(rate(&pts, epoch(10)), vec![(30_000, 2.0)]);
    }

    #[test]
    fn gauge_takes_last_value() {
        let pts = vec![
            p(0, &[("g", "a")], 5.0),
            p(9, &[("g", "a")], 7.0),
            p(3, &[("g", "b")], 1.0),
            p(12, &[("g", "a")], 4.0),
            p(14, &[("g", "a")], f64::NAN),
        ];
        // Bucket 0: a's last is 7, b's is 1 -> 8. Bucket 10: a's last finite value is 4.
        assert_eq!(gauge(&pts, epoch(10)), vec![(0, 8.0), (10_000, 4.0)]);
    }

    fn hist(ts_s: i64, counts: &[(&str, f64)]) -> Vec<MetricPointRow> {
        counts
            .iter()
            .map(|(le, c)| p(ts_s, &[("le", le)], *c))
            .collect()
    }

    const EXAMPLE: [(&str, f64); 4] =
        [("0.05", 10.0), ("0.1", 15.0), ("0.2", 20.0), ("+Inf", 20.0)];

    /// Cumulative bucket counts in one step (deltas from a zero baseline):
    /// le 0.05: 10, le 0.1: 15, le 0.2: 20, le +Inf: 20. Observations = 20 (the +Inf count).
    ///
    /// Prometheus `histogram_quantile` (promql/quantile.go `bucketQuantile`):
    ///   rank = q * observations; b = the first bucket whose cumulative count >= rank;
    ///   result = lower(b) + (upper(b) - lower(b)) * (rank - count(b-1)) / (count(b) - count(b-1)),
    ///   with lower(first bucket) = 0.
    ///
    /// q = 0.5:   rank = 10. The first bucket with count >= 10 is le 0.05 (count 10), so
    ///            0 + (0.05 - 0) * (10 - 0) / (10 - 0) = 0.05.
    /// q = 0.625: rank = 12.5. le 0.05 has 10 < 12.5; le 0.1 has 15 >= 12.5, so
    ///            0.05 + (0.1 - 0.05) * (12.5 - 10) / (15 - 10) = 0.05 + 0.05 * 0.5 = 0.075.
    /// q = 0.9:   rank = 18. le 0.2 (count 20): 0.1 + 0.1 * (18 - 15) / (20 - 15) = 0.16.
    /// q = 1.0:   rank = 20; le 0.2 has 20 >= 20, so 0.1 + 0.1 * 5 / 5 = 0.2.
    ///
    /// The brief expected 0.075 at q = 0.5; by the algorithm above that value belongs to
    /// q = 0.625, and q = 0.5 lands exactly on the 0.05 bound.
    #[test]
    fn quantile_matches_prometheus_example() {
        let mut pts = hist(0, &EXAMPLE.map(|(le, _)| (le, 0.0)));
        pts.extend(hist(10, &EXAMPLE));
        let at = |q: f64| {
            let s = quantile(&pts, q, epoch(10));
            assert_eq!(s.len(), 1, "{s:?}");
            assert_eq!(s[0].0, 10_000);
            s[0].1.expect("observations present")
        };
        assert!((at(0.5) - 0.05).abs() < 1e-12, "{}", at(0.5));
        assert!((at(0.625) - 0.075).abs() < 1e-12, "{}", at(0.625));
        assert!((at(0.9) - 0.16).abs() < 1e-12, "{}", at(0.9));
        assert!((at(1.0) - 0.2).abs() < 1e-12, "{}", at(1.0));
    }

    #[test]
    fn quantile_in_inf_bucket_returns_highest_finite_bound() {
        let mut pts = hist(0, &[("0.1", 0.0), ("0.2", 0.0), ("+Inf", 0.0)]);
        pts.extend(hist(10, &[("0.1", 1.0), ("0.2", 2.0), ("+Inf", 10.0)]));
        assert_eq!(quantile(&pts, 0.99, epoch(10)), vec![(10_000, Some(0.2))]);
    }

    #[test]
    fn quantile_of_empty_buckets_is_none() {
        // Counters unchanged between the two steps: no observations in step 10.
        let mut pts = hist(0, &EXAMPLE);
        pts.extend(hist(10, &EXAMPLE));
        assert_eq!(quantile(&pts, 0.5, epoch(10)), vec![(10_000, None)]);
        assert!(quantile(&[], 0.5, epoch(10)).is_empty());
    }

    #[test]
    fn quantile_merges_label_sets_except_le_and_handles_resets() {
        // Two label sets (kind a, kind b), each with buckets 1, 2, +Inf.
        let b = |ts, kind, c: [f64; 3]| {
            [("1", c[0]), ("2", c[1]), ("+Inf", c[2])]
                .iter()
                .map(|(le, v)| p(ts, &[("kind", kind), ("le", le)], *v))
                .collect::<Vec<_>>()
        };
        let mut pts = b(0, "a", [100.0, 100.0, 100.0]);
        pts.extend(b(0, "b", [0.0, 0.0, 0.0]));
        // kind a resets: its deltas are the new values (2, 4, 4).
        pts.extend(b(10, "a", [2.0, 4.0, 4.0]));
        // kind b: deltas (2, 4, 4).
        pts.extend(b(10, "b", [2.0, 4.0, 4.0]));
        // Merged: le 1: 4, le 2: 8, +Inf: 8. q 0.75 -> rank 6 -> bucket le 2:
        // 1 + (2 - 1) * (6 - 4) / (8 - 4) = 1.5.
        assert_eq!(quantile(&pts, 0.75, epoch(10)), vec![(10_000, Some(1.5))]);
    }
}
