//! Span tree over a trace bundle. All walks are iterative: traces can be 10 000 spans deep.

use crate::model::{Endpoint, SpanRec, TraceBundle, endpoint_name};
use std::collections::{HashMap, VecDeque};

pub const FLAG_INCOMPLETE: &str = "incomplete";

pub struct SpanTree<'a> {
    pub bundle: &'a TraceBundle,
    /// Earliest-starting parentless span.
    pub root: usize,
    pub parent: Vec<Option<usize>>,
    /// Sorted by start time.
    pub children: Vec<Vec<usize>>,
    /// Every span exactly once, parents before children.
    pub order: Vec<usize>,
    /// Log indices per span; logs without a known span attach to the root.
    pub span_logs: Vec<Vec<usize>>,
    /// Missing parents, several roots, or a parent cycle that had to be cut.
    pub incomplete: bool,
}

impl<'a> SpanTree<'a> {
    pub fn build(bundle: &'a TraceBundle) -> Option<Self> {
        let spans = &bundle.spans;
        let n = spans.len();
        if n == 0 {
            return None;
        }
        let mut index: HashMap<&str, usize> = HashMap::with_capacity(n);
        for (i, s) in spans.iter().enumerate() {
            if !s.span_id.is_empty() {
                index.entry(s.span_id.as_str()).or_insert(i);
            }
        }
        let mut incomplete = false;
        let mut parent = vec![None; n];
        for (i, s) in spans.iter().enumerate() {
            if s.parent_span_id.is_empty() {
                continue;
            }
            match index.get(s.parent_span_id.as_str()) {
                Some(&p) if p != i => parent[i] = Some(p),
                _ => incomplete = true,
            }
        }
        let mut children = vec![Vec::new(); n];
        for (i, p) in parent.iter().enumerate() {
            if let Some(p) = p {
                children[*p].push(i);
            }
        }
        let by_start =
            |a: &usize, b: &usize| (spans[*a].start_ns, *a).cmp(&(spans[*b].start_ns, *b));
        let mut genuine_roots: Vec<usize> = (0..n).filter(|&i| parent[i].is_none()).collect();
        genuine_roots.sort_by(by_start);
        let mut cut_roots: Vec<usize> = Vec::new();
        let mut visited = vec![false; n];
        let mut order = Vec::with_capacity(n);

        // BFS from all genuine roots first
        for &r in &genuine_roots {
            bfs(r, &children, &mut visited, &mut order);
        }

        // Handle cycles: find and cut cycle nodes from unvisited spans.
        let mut rest: Vec<usize> = (0..n).filter(|&i| !visited[i]).collect();
        rest.sort_by(by_start);
        for i in rest {
            if visited[i] {
                continue;
            }
            // Find the cycle node by following parent links; detect cycle when we revisit in this walk.
            let cycle_node = find_cycle_node(i, &parent);
            if let Some(p) = parent[cycle_node].take() {
                children[p].retain(|&c| c != cycle_node);
            }
            incomplete = true;
            cut_roots.push(cycle_node);
            bfs(cycle_node, &children, &mut visited, &mut order);
        }

        // Determine root: genuine root if any, else earliest cut root.
        if genuine_roots.len() > 1 {
            incomplete = true;
        }
        if !cut_roots.is_empty() {
            incomplete = true;
        }

        for c in &mut children {
            c.sort_by(by_start);
        }

        let root = if !genuine_roots.is_empty() {
            genuine_roots[0]
        } else {
            *cut_roots.iter().min_by(|a, b| by_start(a, b))?
        };

        // Reorder: put root's subtree first so order[0] == root.
        let mut final_order = Vec::with_capacity(n);
        let mut visited2 = vec![false; n];
        bfs(root, &children, &mut visited2, &mut final_order);
        for (i, &was_visited) in visited2.iter().enumerate() {
            if !was_visited {
                final_order.push(i);
            }
        }

        let mut span_logs = vec![Vec::new(); n];
        for (li, log) in bundle.logs.iter().enumerate() {
            let target = index.get(log.span_id.as_str()).copied().unwrap_or(root);
            span_logs[target].push(li);
        }
        Some(Self {
            bundle,
            root,
            parent,
            children,
            order: final_order,
            span_logs,
            incomplete,
        })
    }

    pub fn span(&self, i: usize) -> &'a SpanRec {
        &self.bundle.spans[i]
    }

    /// Top-level ancestor first, `i` last.
    pub fn path_to(&self, i: usize) -> Vec<usize> {
        let mut path = vec![i];
        let mut cur = i;
        while let Some(p) = self.parent[cur] {
            path.push(p);
            cur = p;
        }
        path.reverse();
        path
    }

    pub fn endpoint(&self) -> Endpoint {
        let root = self.span(self.root);
        Endpoint {
            service: root.service.clone(),
            name: endpoint_name(root),
        }
    }
}

/// Find a node on the cycle reachable from start by following parent links.
/// Returns the first node we revisit in the walk (which is on the cycle).
fn find_cycle_node(start: usize, parent: &[Option<usize>]) -> usize {
    let mut seen_in_walk = HashMap::new();
    let mut cur = start;
    let mut step = 0;
    loop {
        if seen_in_walk.contains_key(&cur) {
            // We've revisited cur in this walk: it's on the cycle.
            return cur;
        }
        seen_in_walk.insert(cur, step);
        match parent[cur] {
            Some(p) => {
                cur = p;
                step += 1;
            }
            None => {
                // Reached a parentless node (shouldn't happen for unvisited spans in cycles).
                return start;
            }
        }
    }
}

fn bfs(start: usize, children: &[Vec<usize>], visited: &mut [bool], order: &mut Vec<usize>) {
    let mut queue = VecDeque::from([start]);
    visited[start] = true;
    while let Some(i) = queue.pop_front() {
        order.push(i);
        for &c in &children[i] {
            if !visited[c] {
                visited[c] = true;
                queue.push_back(c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{bundle, log, span};

    /// Verify that order is a permutation of 0..n and every parent appears before its children.
    fn verify_order(tree: &SpanTree, n: usize) {
        // Check it's a permutation
        let mut seen = vec![false; n];
        for &i in &tree.order {
            assert!(i < n, "order contains out-of-range index {}", i);
            assert!(!seen[i], "order contains duplicate {}", i);
            seen[i] = true;
        }
        assert_eq!(
            tree.order.len(),
            n,
            "order has {} elements, expected {}",
            tree.order.len(),
            n
        );

        // Check parents come before children
        let mut position = vec![0; n];
        for (pos, &idx) in tree.order.iter().enumerate() {
            position[idx] = pos;
        }
        for (i, &p) in tree.parent.iter().enumerate() {
            if let Some(parent_idx) = p {
                assert!(
                    position[parent_idx] < position[i],
                    "parent {} at position {} comes after child {} at position {}",
                    parent_idx,
                    position[parent_idx],
                    i,
                    position[i]
                );
            }
        }
    }

    #[test]
    fn builds_parent_child_links_sorted_by_start() {
        let b = bundle(vec![
            span("c", "a", "svc", "c", 30, 40),
            span("a", "", "svc", "a", 0, 100),
            span("b", "a", "svc", "b", 10, 20),
        ]);
        let t = SpanTree::build(&b).unwrap();
        assert_eq!(t.root, 1);
        assert_eq!(t.children[1], vec![2, 0]);
        assert_eq!(t.parent[0], Some(1));
        assert_eq!(t.order[0], 1);
        assert_eq!(t.order.len(), 3);
        assert!(!t.incomplete);
        assert_eq!(t.path_to(0), vec![1, 0]);
        verify_order(&t, 3);
    }

    #[test]
    fn empty_bundle_has_no_tree() {
        assert!(SpanTree::build(&bundle(vec![])).is_none());
    }

    #[test]
    fn missing_parent_flags_incomplete_and_earliest_root_wins() {
        let b = bundle(vec![
            span("x", "gone", "svc", "x", 5, 9),
            span("y", "", "svc", "y", 3, 4),
        ]);
        let t = SpanTree::build(&b).unwrap();
        assert!(t.incomplete);
        assert_eq!(t.root, 1);
        assert_eq!(t.order.len(), 2);
        verify_order(&t, 2);
    }

    #[test]
    fn self_parent_is_root() {
        let b = bundle(vec![span("a", "a", "svc", "a", 0, 1)]);
        let t = SpanTree::build(&b).unwrap();
        assert_eq!(t.root, 0);
        assert!(t.incomplete);
        verify_order(&t, 1);
    }

    #[test]
    fn cycle_is_cut_and_flagged() {
        let b = bundle(vec![
            span("a", "b", "svc", "a", 0, 10),
            span("b", "a", "svc", "b", 1, 5),
        ]);
        let t = SpanTree::build(&b).unwrap();
        assert!(t.incomplete);
        assert_eq!(t.order.len(), 2);
        assert_eq!(t.root, 0);
        assert_eq!(t.parent[0], None);
        assert_eq!(t.path_to(1), vec![0, 1]);
        verify_order(&t, 2);
    }

    #[test]
    fn logs_attach_to_their_span_or_root() {
        let mut b = bundle(vec![
            span("a", "", "svc", "a", 0, 10),
            span("b", "a", "svc", "b", 1, 5),
        ]);
        b.logs = vec![
            log("b", 17, "x", 2),
            log("", 9, "y", 3),
            log("zz", 9, "z", 4),
        ];
        let t = SpanTree::build(&b).unwrap();
        assert_eq!(t.span_logs[1], vec![0]);
        assert_eq!(t.span_logs[0], vec![1, 2]);
        verify_order(&t, 2);
    }

    #[test]
    fn deep_chain_builds() {
        let spans = (0..10_000)
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
        assert_eq!(t.path_to(9_999).len(), 10_000);
        assert!(!t.incomplete);
        verify_order(&t, 10_000);
    }

    #[test]
    fn cycle_beside_real_root_keeps_real_root() {
        // Real root R starts at 10 with a child; separate cycle A↔B starting at 0.
        let b = bundle(vec![
            span("a", "b", "svc", "a", 0, 5),
            span("b", "a", "svc", "b", 1, 4),
            span("r", "", "svc", "r", 10, 20),
            span("rc", "r", "svc", "rc", 11, 19),
        ]);
        let t = SpanTree::build(&b).unwrap();
        assert_eq!(t.root, 2, "root should be R (the genuine root)");
        assert_eq!(t.order[0], 2, "order[0] should be root");
        assert!(t.incomplete, "cycle should mark incomplete");
        verify_order(&t, 4);
    }

    #[test]
    fn cycle_tail_child_keeps_its_parent() {
        // Cycle A↔B (starts 5, 6) plus C with parent A starting at 0 → C's parent stays A.
        let b = bundle(vec![
            span("c", "a", "svc", "c", 0, 2),
            span("a", "b", "svc", "a", 5, 10),
            span("b", "a", "svc", "b", 6, 9),
        ]);
        let t = SpanTree::build(&b).unwrap();
        // C should have A as parent (not cut)
        assert_eq!(t.parent[0], Some(1), "C should have A as parent");
        // Exactly one of A/B is a root (the one whose parent was cut)
        let a_is_root = t.parent[1].is_none();
        let b_is_root = t.parent[2].is_none();
        assert!(a_is_root ^ b_is_root, "exactly one of A/B should be a root");
        assert!(t.incomplete);
        verify_order(&t, 3);
    }

    #[test]
    fn duplicate_span_ids_keep_first() {
        // Two spans with id "a" → no panic, children of "a" attach to the first.
        let b = bundle(vec![
            span("a", "", "svc", "a1", 0, 5),
            span("a", "", "svc", "a2", 1, 4),
            span("c", "a", "svc", "c", 2, 3),
        ]);
        let t = SpanTree::build(&b).unwrap();
        // "c" should be a child of the first "a" (index 0)
        assert_eq!(t.parent[2], Some(0));
        assert!(t.children[0].contains(&2));
        verify_order(&t, 3);
    }
}
