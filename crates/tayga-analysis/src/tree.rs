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
        let mut roots: Vec<usize> = (0..n).filter(|&i| parent[i].is_none()).collect();
        roots.sort_by(by_start);
        let mut visited = vec![false; n];
        let mut order = Vec::with_capacity(n);
        for &r in &roots {
            bfs(r, &children, &mut visited, &mut order);
        }
        // Spans unreachable from every root sit on a parent cycle: cut their parent link.
        let mut rest: Vec<usize> = (0..n).filter(|&i| !visited[i]).collect();
        rest.sort_by(by_start);
        for i in rest {
            if visited[i] {
                continue;
            }
            if let Some(p) = parent[i].take() {
                children[p].retain(|&c| c != i);
            }
            incomplete = true;
            roots.push(i);
            bfs(i, &children, &mut visited, &mut order);
        }
        if roots.len() > 1 {
            incomplete = true;
        }
        for c in &mut children {
            c.sort_by(by_start);
        }
        let root = *roots.iter().min_by(|a, b| by_start(a, b))?;
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
            order,
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
    }

    #[test]
    fn self_parent_is_root() {
        let b = bundle(vec![span("a", "a", "svc", "a", 0, 1)]);
        let t = SpanTree::build(&b).unwrap();
        assert_eq!(t.root, 0);
        assert!(t.incomplete);
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
    }
}
