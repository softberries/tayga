//! Chain of spans that determined end-to-end latency (spec §9.3). Iterative: deep traces
//! must not overflow the stack.

use crate::tree::SpanTree;
use std::cmp::Reverse;
use std::collections::HashMap;

/// A child may end this much after its parent and still count as part of the parent's work:
/// spans from different hosts carry independently read clocks (observed: a server span ending
/// 57 us after its client span in the demo), and excluding such a child would credit its whole
/// duration to the parent. Children ending later than this are treated as asynchronous.
const CLOCK_SKEW_TOLERANCE_NS: u64 = 5_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub span: usize,
    pub start_ns: u64,
    pub end_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CriticalPath {
    /// Chronological; together they cover the root span exactly.
    pub segments: Vec<Segment>,
    /// Self time on the path per span, largest first.
    pub self_time: Vec<(usize, u64)>,
}

pub fn critical_path(tree: &SpanTree) -> CriticalPath {
    struct Frame {
        span: usize,
        lo: u64,
        hi: u64,
        cursor: u64,
        kids: Vec<usize>,
        next: usize,
    }

    // Window [lo, hi] of a span on the path; its children are walked latest-ending first.
    let frame = |span: usize, lo: u64, hi: u64| -> Frame {
        let mut kids: Vec<usize> = tree.children[span]
            .iter()
            .copied()
            .filter(|&c| {
                let s = tree.span(c);
                s.end_ns <= hi.saturating_add(CLOCK_SKEW_TOLERANCE_NS)
                    && s.end_ns > lo
                    && s.start_ns < hi
            })
            .collect();
        kids.sort_by_key(|&c| Reverse((tree.span(c).end_ns.min(hi), c)));
        Frame {
            span,
            lo,
            hi,
            cursor: hi,
            kids,
            next: 0,
        }
    };
    let root = tree.span(tree.root);
    let mut stack = vec![frame(tree.root, root.start_ns, root.end_ns)];
    let mut reversed = Vec::new();
    while let Some(top) = stack.last_mut() {
        if top.next < top.kids.len() {
            let c = top.kids[top.next];
            top.next += 1;
            let child = tree.span(c);
            let child_hi = child.end_ns.min(top.hi); // clamp skew overshoot
            if child_hi > top.cursor {
                continue; // overlaps a later child already on the path
            }
            if top.cursor > child_hi {
                reversed.push(Segment {
                    span: top.span,
                    start_ns: child_hi,
                    end_ns: top.cursor,
                });
            }
            let child_lo = child.start_ns.max(top.lo);
            top.cursor = child_lo;
            stack.push(frame(c, child_lo, child_hi));
        } else {
            if top.cursor > top.lo {
                reversed.push(Segment {
                    span: top.span,
                    start_ns: top.lo,
                    end_ns: top.cursor,
                });
            }
            stack.pop();
        }
    }
    reversed.reverse();
    let mut totals: HashMap<usize, u64> = HashMap::new();
    for s in &reversed {
        *totals.entry(s.span).or_default() += s.end_ns - s.start_ns;
    }
    let mut self_time: Vec<(usize, u64)> = totals.into_iter().collect();
    self_time.sort_by_key(|&(span, t)| (Reverse(t), span));
    CriticalPath {
        segments: reversed,
        self_time,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{bundle, span};
    use proptest::prelude::*;

    fn seg(span: usize, start_ns: u64, end_ns: u64) -> Segment {
        Segment {
            span,
            start_ns,
            end_ns,
        }
    }

    #[test]
    fn nested_and_sequential_children() {
        // root [0,100]; A [10,40]; B [30,90] with C [50,80] inside B.
        let b = bundle(vec![
            span("r", "", "fe", "root", 0, 100),
            span("a", "r", "x", "A", 10, 40),
            span("b", "r", "y", "B", 30, 90),
            span("c", "b", "z", "C", 50, 80),
        ]);
        let t = SpanTree::build(&b).unwrap();
        let cp = critical_path(&t);
        assert_eq!(
            cp.segments,
            vec![
                seg(0, 0, 30),
                seg(2, 30, 50),
                seg(3, 50, 80),
                seg(2, 80, 90),
                seg(0, 90, 100)
            ]
        );
        assert_eq!(cp.self_time, vec![(0, 40), (2, 30), (3, 30)]);
    }

    #[test]
    fn child_ending_after_parent_is_excluded() {
        let b = bundle(vec![
            span("r", "", "fe", "root", 0, 100_000_000),
            span("a", "r", "x", "async", 50_000_000, 150_000_000),
        ]);
        let t = SpanTree::build(&b).unwrap();
        assert_eq!(critical_path(&t).segments, vec![seg(0, 0, 100_000_000)]);
    }

    #[test]
    fn child_ending_slightly_after_parent_is_clamped_not_excluded() {
        // Cross-host clock skew: server span ends 57 us after its client span.
        let b = bundle(vec![
            span("r", "", "fe", "root", 0, 5_000_000_000),
            span("c", "r", "checkout", "POST", 1_000, 5_000_000_000),
            span("s", "c", "shipping", "ship", 1_200, 5_000_057_000),
        ]);
        let t = SpanTree::build(&b).unwrap();
        let cp = critical_path(&t);
        assert_eq!(cp.self_time[0].0, 2, "shipping span owns the path");
        let total: u64 = cp.segments.iter().map(|s| s.end_ns - s.start_ns).sum();
        assert_eq!(total, 5_000_000_000);
    }

    #[test]
    fn child_starting_before_parent_is_clipped() {
        let b = bundle(vec![
            span("r", "", "fe", "root", 10, 100),
            span("a", "r", "x", "skewed", 0, 60),
        ]);
        let t = SpanTree::build(&b).unwrap();
        assert_eq!(
            critical_path(&t).segments,
            vec![seg(1, 10, 60), seg(0, 60, 100)]
        );
    }

    #[test]
    fn single_zero_length_span_has_no_segments() {
        let b = bundle(vec![span("r", "", "fe", "root", 5, 5)]);
        let t = SpanTree::build(&b).unwrap();
        assert_eq!(critical_path(&t), CriticalPath::default());
    }

    #[test]
    fn deep_chain_does_not_overflow() {
        let spans = (0..10_000u64)
            .map(|i| {
                let parent = if i == 0 {
                    String::new()
                } else {
                    format!("s{}", i - 1)
                };
                span(&format!("s{i}"), &parent, "svc", "op", i, 20_000 - i)
            })
            .collect();
        let b = bundle(spans);
        let t = SpanTree::build(&b).unwrap();
        let cp = critical_path(&t);
        let total: u64 = cp.segments.iter().map(|s| s.end_ns - s.start_ns).sum();
        assert_eq!(total, 20_000);
    }

    proptest! {
        #[test]
        fn segments_tile_the_root_window(raw in prop::collection::vec((any::<u16>(), 0u64..1_000, 0u64..1_000), 1..60)) {
            let spans = raw.iter().enumerate().map(|(i, &(p, start, len))| {
                let parent = if i == 0 { String::new() } else { format!("s{}", p as usize % i) };
                span(&format!("s{i}"), &parent, "svc", "op", start, start + len)
            }).collect();
            let b = bundle(spans);
            let t = SpanTree::build(&b).unwrap();
            let root = t.span(t.root);
            let cp = critical_path(&t);
            let total: u64 = cp.segments.iter().map(|s| s.end_ns - s.start_ns).sum();
            prop_assert_eq!(total, root.duration_ns());
            for w in cp.segments.windows(2) {
                prop_assert!(w[0].end_ns <= w[1].start_ns);
            }
            for s in &cp.segments {
                prop_assert!(s.start_ns >= root.start_ns && s.end_ns <= root.end_ns && s.start_ns < s.end_ns);
            }
        }
    }
}
