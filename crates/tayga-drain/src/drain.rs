//! Drain (He et al., 2017) with one tree per service. Similarity counts template wildcards as
//! matches (spec §4.2; measured 67 templates on a 20k-log demo sample vs 5,942 without).

use crate::preprocess::{WILDCARD, tokens};
use std::collections::{HashMap, HashSet};
use tayga_analysis::fingerprint::fingerprint;

pub const OVERFLOW: &str = "<overflow>";
const SAMPLE_MAX_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct DrainConfig {
    pub depth: usize,
    pub sim_threshold: f64,
    pub max_children: usize,
    pub max_clusters_per_service: usize,
}

impl Default for DrainConfig {
    fn default() -> Self {
        Self {
            depth: 4,
            sim_threshold: 0.5,
            max_children: 100,
            max_clusters_per_service: 5_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    pub id: u64,
    pub service: String,
    pub tokens: Vec<String>,
    pub count: u64,
    pub first_seen_ns: i64,
    pub last_seen_ns: i64,
    pub max_severity: u8,
    pub sample: String,
}

impl Cluster {
    pub fn template(&self) -> String {
        self.tokens.join(" ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assignment {
    pub template_id: u64,
    pub created: bool,
    pub overflow: bool,
}

/// Stable id of a cluster: hash of its service and its *initial* template.
pub fn template_id(service: &str, template: &str) -> u64 {
    fingerprint(&[service, template])
}

#[derive(Default)]
struct Node {
    children: HashMap<String, Node>,
    clusters: Vec<usize>,
}

#[derive(Default)]
struct ServiceTree {
    by_len: HashMap<usize, Node>,
    clusters: usize,
    overflow: Option<usize>,
}

pub struct Drain {
    cfg: DrainConfig,
    trees: HashMap<String, ServiceTree>,
    clusters: Vec<Cluster>,
    by_id: HashMap<u64, usize>,
    dirty: HashSet<u64>,
}

fn similarity(template: &[String], tokens: &[String]) -> f64 {
    let same = template
        .iter()
        .zip(tokens)
        .filter(|(t, m)| t == m || t.as_str() == WILDCARD)
        .count();
    same as f64 / tokens.len() as f64
}

fn truncate_utf8(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

impl Drain {
    pub fn new(cfg: DrainConfig) -> Self {
        Self {
            cfg,
            trees: HashMap::new(),
            clusters: Vec::new(),
            by_id: HashMap::new(),
            dirty: HashSet::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.clusters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clusters.is_empty()
    }

    pub fn cluster(&self, id: u64) -> Option<&Cluster> {
        self.by_id.get(&id).map(|&i| &self.clusters[i])
    }

    /// Clusters created or changed since the last call.
    pub fn take_dirty(&mut self) -> Vec<Cluster> {
        let ids: Vec<u64> = self.dirty.drain().collect();
        ids.into_iter()
            .filter_map(|id| self.cluster(id).cloned())
            .collect()
    }

    /// Leaf for `tokens` in `service`'s tree, creating nodes on the way.
    fn leaf<'a>(tree: &'a mut ServiceTree, cfg: &DrainConfig, tokens: &[String]) -> &'a mut Node {
        let mut node = tree.by_len.entry(tokens.len()).or_default();
        for tok in tokens.iter().take(cfg.depth.saturating_sub(2)) {
            let key = if tok == WILDCARD
                || (!node.children.contains_key(tok) && node.children.len() >= cfg.max_children)
            {
                WILDCARD.to_string()
            } else {
                tok.clone()
            };
            node = node.children.entry(key).or_default();
        }
        node
    }

    fn push_cluster(&mut self, mut c: Cluster) -> usize {
        // Ids derive from the initial template; on the (unlikely) collision, salt until free.
        let base = c.id;
        let mut salt = 0u32;
        while self.by_id.contains_key(&c.id) {
            salt += 1;
            c.id = fingerprint(&[&c.service, &base.to_string(), &salt.to_string()]);
        }
        let idx = self.clusters.len();
        self.by_id.insert(c.id, idx);
        self.clusters.push(c);
        idx
    }

    pub fn add(&mut self, service: &str, body: &str, ts_ns: i64, severity: u8) -> Assignment {
        let toks = tokens(body);
        let cfg = self.cfg.clone();
        let tree = self.trees.entry(service.to_string()).or_default();
        let leaf = Self::leaf(tree, &cfg, &toks);
        let mut best: Option<(usize, f64)> = None;
        for &i in &leaf.clusters {
            let s = similarity(&self.clusters[i].tokens, &toks);
            if best.is_none_or(|(_, b)| s > b) {
                best = Some((i, s));
            }
        }
        let (idx, created, overflow) = match best {
            Some((i, s)) if s >= cfg.sim_threshold => {
                let c = &mut self.clusters[i];
                for (t, m) in c.tokens.iter_mut().zip(&toks) {
                    if t != m {
                        *t = WILDCARD.to_string();
                    }
                }
                (i, false, false)
            }
            _ if tree.clusters >= cfg.max_clusters_per_service => match tree.overflow {
                Some(i) => (i, false, true),
                None => {
                    let c = Cluster {
                        id: template_id(service, OVERFLOW),
                        service: service.to_string(),
                        tokens: vec![OVERFLOW.to_string()],
                        count: 0,
                        first_seen_ns: ts_ns,
                        last_seen_ns: ts_ns,
                        max_severity: 0,
                        sample: truncate_utf8(body, SAMPLE_MAX_BYTES),
                    };
                    let i = self.push_cluster(c);
                    self.trees.get_mut(service).expect("tree exists").overflow = Some(i);
                    (i, true, true)
                }
            },
            _ => {
                let c = Cluster {
                    id: template_id(service, &toks.join(" ")),
                    service: service.to_string(),
                    tokens: toks.clone(),
                    count: 0,
                    first_seen_ns: ts_ns,
                    last_seen_ns: ts_ns,
                    max_severity: 0,
                    sample: truncate_utf8(body, SAMPLE_MAX_BYTES),
                };
                let i = self.push_cluster(c);
                let tree = self.trees.get_mut(service).expect("tree exists");
                tree.clusters += 1;
                Self::leaf(tree, &cfg, &toks).clusters.push(i);
                (i, true, false)
            }
        };
        let c = &mut self.clusters[idx];
        c.count += 1;
        c.first_seen_ns = c.first_seen_ns.min(ts_ns);
        c.last_seen_ns = c.last_seen_ns.max(ts_ns);
        c.max_severity = c.max_severity.max(severity);
        self.dirty.insert(c.id);
        Assignment {
            template_id: c.id,
            created,
            overflow,
        }
    }

    /// Re-inserts a persisted cluster (id and template unchanged). Not marked dirty.
    pub fn restore(&mut self, c: Cluster) {
        if self.by_id.contains_key(&c.id) {
            return;
        }
        let cfg = self.cfg.clone();
        let service = c.service.clone();
        let is_overflow = c.tokens.len() == 1 && c.tokens[0] == OVERFLOW;
        let toks = c.tokens.clone();
        let idx = self.clusters.len();
        self.by_id.insert(c.id, idx);
        self.clusters.push(c);
        let tree = self.trees.entry(service).or_default();
        if is_overflow {
            tree.overflow = Some(idx);
        } else {
            tree.clusters += 1;
            Self::leaf(tree, &cfg, &toks).clusters.push(idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain() -> Drain {
        Drain::new(DrainConfig::default())
    }

    #[test]
    fn same_shape_merges_and_generalizes() {
        let mut d = drain();
        let a = d.add("cart", "GetCart called with user alice", 1, 9);
        let b = d.add("cart", "GetCart called with user bob", 2, 13);
        assert!(a.created && !b.created);
        assert_eq!(a.template_id, b.template_id);
        let c = d.cluster(a.template_id).unwrap();
        assert_eq!(c.template(), "GetCart called with user <*>");
        assert_eq!(
            (c.count, c.first_seen_ns, c.last_seen_ns, c.max_severity),
            (2, 1, 2, 13)
        );
        assert_eq!(c.sample, "GetCart called with user alice");
    }

    #[test]
    fn different_lengths_services_and_prefixes_stay_apart() {
        let mut d = drain();
        let a = d.add("cart", "item added", 0, 9);
        let b = d.add("cart", "item added twice", 0, 9);
        let c = d.add("email", "item added", 0, 9);
        let e = d.add("cart", "order placed", 0, 9);
        let ids: HashSet<u64> = [a, b, c, e].iter().map(|x| x.template_id).collect();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn low_similarity_creates_a_new_cluster() {
        let mut d = drain();
        let a = d.add("svc", "alpha beta gamma delta", 0, 9);
        let b = d.add("svc", "alpha beta one two", 0, 9); // 2/4 = 0.5 → merges
        let c = d.add("svc", "alpha beta three four", 0, 9); // vs "alpha beta <*> <*>" = 1.0
        assert_eq!(a.template_id, b.template_id);
        assert_eq!(b.template_id, c.template_id);
        let mut d = Drain::new(DrainConfig {
            sim_threshold: 0.75,
            ..DrainConfig::default()
        });
        let a = d.add("svc", "alpha beta gamma delta", 0, 9);
        let b = d.add("svc", "alpha beta one two", 0, 9);
        assert_ne!(a.template_id, b.template_id);
    }

    #[test]
    fn id_is_stable_under_generalization() {
        let mut d = drain();
        let a = d.add("svc", "user logged in as alice", 0, 9);
        for name in ["bob", "carol", "dave"] {
            assert_eq!(
                d.add("svc", &format!("user logged in as {name}"), 0, 9)
                    .template_id,
                a.template_id
            );
        }
        assert_eq!(a.template_id, template_id("svc", "user logged in as alice"));
    }

    #[test]
    fn max_children_routes_to_wildcard_branch() {
        let mut d = Drain::new(DrainConfig {
            max_children: 2,
            ..DrainConfig::default()
        });
        d.add("svc", "a x y", 0, 9);
        d.add("svc", "b x y", 0, 9);
        // third distinct first token goes under <*> and merges with the next such message
        let c = d.add("svc", "c x y", 0, 9);
        let e = d.add("svc", "e x y", 0, 9);
        assert_eq!(c.template_id, e.template_id);
        assert_eq!(d.cluster(c.template_id).unwrap().template(), "<*> x y");
    }

    #[test]
    fn masked_routing_token_takes_the_wildcard_branch() {
        let mut d = drain();
        let a = d.add("svc", "123 apples", 0, 9);
        let b = d.add("svc", "456 apples", 0, 9);
        assert!(a.created && !b.created);
        assert_eq!(a.template_id, b.template_id);
        assert_eq!(d.cluster(a.template_id).unwrap().template(), "<*> apples");
        let c = d.add("svc", "pear apples", 0, 9);
        assert!(c.created, "a literal routing token gets its own branch");

        // The `<*>` branch is taken even when the node is full, and a full node's overflow
        // traffic lands on the same branch.
        let mut d = Drain::new(DrainConfig {
            max_children: 1,
            ..DrainConfig::default()
        });
        d.add("svc", "pear apples", 0, 9);
        let w = d.add("svc", "123 apples", 0, 9);
        let full = d.add("svc", "plum apples", 0, 9);
        assert!(w.created && !full.created);
        assert_eq!(full.template_id, w.template_id);
    }

    #[test]
    fn cluster_cap_routes_to_overflow() {
        let mut d = Drain::new(DrainConfig {
            max_clusters_per_service: 2,
            ..DrainConfig::default()
        });
        d.add("svc", "one", 0, 9);
        d.add("svc", "two words", 0, 9);
        let o1 = d.add("svc", "three little words", 0, 9);
        let o2 = d.add("svc", "four little words here", 0, 9);
        assert!(o1.overflow && o1.created && o2.overflow && !o2.created);
        assert_eq!(o1.template_id, o2.template_id);
        assert_eq!(d.cluster(o1.template_id).unwrap().template(), OVERFLOW);
        assert!(
            !d.add("other", "three little words", 0, 9).overflow,
            "cap is per service"
        );
    }

    #[test]
    fn restore_round_trip_keeps_ids_and_matching() {
        let mut d = drain();
        let a = d.add("svc", "user logged in as alice", 5, 9);
        d.add("svc", "user logged in as bob", 6, 9);
        let saved: Vec<Cluster> = d.take_dirty();
        assert_eq!(saved.len(), 1);
        assert!(d.take_dirty().is_empty());
        let mut r = drain();
        for c in saved {
            r.restore(c);
        }
        let again = r.add("svc", "user logged in as zed", 7, 9);
        assert!(!again.created);
        assert_eq!(again.template_id, a.template_id);
        assert_eq!(r.cluster(a.template_id).unwrap().count, 3);
    }

    #[test]
    fn sample_is_truncated_on_a_char_boundary() {
        let mut d = drain();
        let body = format!("{} end", "ż".repeat(400));
        let a = d.add("svc", &body, 0, 9);
        let s = &d.cluster(a.template_id).unwrap().sample;
        assert!(s.len() <= SAMPLE_MAX_BYTES && body.starts_with(s.as_str()));
    }
}
