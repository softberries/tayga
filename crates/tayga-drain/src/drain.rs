//! Drain (He et al., 2017) with one tree per service. Similarity counts template wildcards as
//! matches (spec §4.2; measured 67 templates on a 20k-log demo sample vs 5,942 without).
//! Kept HTTP status codes ([`is_protected`]) are the exception: they match only themselves, so a
//! `200` line and a `503` line never share a template and a status is never generalised.

use crate::preprocess::{WILDCARD, is_protected, tokens};
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
    pub keep_http_status: bool,
}

impl Default for DrainConfig {
    fn default() -> Self {
        Self {
            depth: 4,
            sim_threshold: 0.5,
            max_children: 100,
            max_clusters_per_service: 5_000,
            keep_http_status: true,
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

/// Share of positions where the template matches the line, its `<*>` matching anything; `None`
/// when a protected token on either side differs from the other side, `<*>` included.
fn similarity(template: &[String], tokens: &[String], keep_http_status: bool) -> Option<f64> {
    let mut same = 0usize;
    for (t, m) in template.iter().zip(tokens) {
        if t == m {
            same += 1;
        } else if is_protected(t, keep_http_status) || is_protected(m, keep_http_status) {
            return None;
        } else if t == WILDCARD {
            same += 1;
        }
    }
    Some(same as f64 / tokens.len() as f64)
}

/// Similarity before status protection: the template's `<*>` matches anything, every other
/// position must be equal.
fn pre_epoch_similarity(template: &[String], tokens: &[String]) -> f64 {
    let same = template
        .iter()
        .zip(tokens)
        .filter(|(t, m)| t == m || *t == WILDCARD)
        .count();
    same as f64 / tokens.len().max(1) as f64
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

    /// Leaf for `tokens` in `service`'s tree, creating nodes on the way. A protected token always
    /// gets its own branch, never the `<*>` overflow branch, so a line and every template it may
    /// match (same literal there) share a leaf. Protected branches are in addition to
    /// `max_children`: a full node can still grow up to 500 of them (one per code 100..=599).
    fn leaf<'a>(tree: &'a mut ServiceTree, cfg: &DrainConfig, tokens: &[String]) -> &'a mut Node {
        let mut node = tree.by_len.entry(tokens.len()).or_default();
        for tok in tokens.iter().take(cfg.depth.saturating_sub(2)) {
            let key = if tok == WILDCARD
                || (!node.children.contains_key(tok)
                    && node.children.len() >= cfg.max_children
                    && !is_protected(tok, cfg.keep_http_status))
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
        let toks = tokens(body, self.cfg.keep_http_status);
        let cfg = self.cfg.clone();
        let tree = self.trees.entry(service.to_string()).or_default();
        let leaf = Self::leaf(tree, &cfg, &toks);
        let mut best: Option<(usize, f64)> = None;
        for &i in &leaf.clusters {
            let Some(s) = similarity(&self.clusters[i].tokens, &toks, cfg.keep_http_status) else {
                continue;
            };
            if best.is_none_or(|(_, b)| s > b) {
                best = Some((i, s));
            }
        }
        let (idx, created, overflow) = match best {
            Some((i, s)) if s >= cfg.sim_threshold => {
                // Protected positions are equal here (`similarity` rejects a mismatch), so only
                // ordinary tokens are generalised.
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

    /// True when `template_tokens` (a template of `service` that contains kept status codes) would
    /// have matched a template that existed before the masking epoch under the pre-epoch rules:
    /// with every protected token read as `<*>`, some template of the same service and length with
    /// `first_seen < epoch_start_ns` shares its routing tokens (the first `depth - 2`, as in the
    /// tree) and has the pre-fix similarity (its `<*>` matching anything, plain equality otherwise)
    /// of at least `sim_threshold`. Such a template is not new behaviour, only a status now split
    /// out of an old `<*>` template (final review I1).
    ///
    /// False with no epoch (`epoch_start_ns <= 0`), without `keep_http_status`, and for a template
    /// with no protected token, so other templates are judged exactly as before. Scans the
    /// service's clusters instead of walking the tree (the max-children `<*>` branch is not
    /// modelled); it runs only for `new` candidates, which are rare.
    pub fn would_have_matched_pre_epoch(
        &self,
        service: &str,
        template_tokens: &[String],
        epoch_start_ns: i64,
    ) -> bool {
        let keep = self.cfg.keep_http_status;
        if epoch_start_ns <= 0 || !template_tokens.iter().any(|t| is_protected(t, keep)) {
            return false;
        }
        let old: Vec<String> = template_tokens
            .iter()
            .map(|t| {
                if is_protected(t, keep) {
                    WILDCARD.to_string()
                } else {
                    t.clone()
                }
            })
            .collect();
        let route = self.cfg.depth.saturating_sub(2).min(old.len());
        self.clusters.iter().any(|c| {
            c.service == service
                && c.first_seen_ns < epoch_start_ns
                && c.tokens.len() == old.len()
                && c.tokens.first().is_none_or(|t| t != OVERFLOW)
                && c.tokens[..route] == old[..route]
                && pre_epoch_similarity(&c.tokens, &old) >= self.cfg.sim_threshold
        })
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

    fn access(ts: &str, path: &str, status: u16, flags: &str) -> String {
        format!(
            r#"[2026-10-05T10:00:{ts}.000Z] "GET {path} HTTP/1.1" {status} {flags} 0 91 2 - "-" "python""#
        )
    }

    fn with_status(keep_http_status: bool) -> Drain {
        Drain::new(DrainConfig {
            keep_http_status,
            ..DrainConfig::default()
        })
    }

    #[test]
    fn status_codes_get_their_own_literal_templates() {
        let mut d = drain();
        let ok = d.add("fp", &access("00", "/api/cart", 200, "-"), 0, 9);
        let err = d.add("fp", &access("01", "/api/cart", 503, "-"), 0, 9);
        assert!(ok.created && err.created);
        assert_ne!(ok.template_id, err.template_id);
        let t_ok = d.cluster(ok.template_id).unwrap().template();
        let t_err = d.cluster(err.template_id).unwrap().template();
        assert!(t_ok.contains(" 200 ") && !t_ok.contains(" 503 "), "{t_ok}");
        assert!(t_err.contains(" 503 "), "{t_err}");

        // Two 503 lines differing elsewhere merge, and the merged template keeps the 503.
        let err2 = d.add("fp", &access("02", "/api/checkout", 503, "UF"), 0, 9);
        assert!(!err2.created);
        assert_eq!(err2.template_id, err.template_id);
        let c = d.cluster(err.template_id).unwrap();
        assert_eq!(c.count, 2);
        assert_eq!(
            c.template(),
            r#"<*> "GET <*> <*> 503 <*> <*> <*> <*> - "-" "python""#
        );
        // ... and the 200 template still neither absorbs a 503 nor loses its code.
        assert_eq!(
            d.add("fp", &access("03", "/x", 200, "UF"), 0, 9)
                .template_id,
            ok.template_id
        );
        assert!(
            d.cluster(ok.template_id)
                .unwrap()
                .template()
                .contains(" 200 ")
        );
    }

    #[test]
    fn restored_wildcard_status_template_does_not_absorb_a_status() {
        // A template from the masking-v1 era: `<*>` where the status code sits.
        let old = r#"<*> "GET <*> <*> <*> <*> <*> <*> <*> - "-" "python""#;
        for threshold in [0.5, 0.0] {
            let mut d = Drain::new(DrainConfig {
                sim_threshold: threshold,
                ..DrainConfig::default()
            });
            let old_id = template_id("fp", old);
            d.restore(Cluster {
                id: old_id,
                service: "fp".into(),
                tokens: old.split(' ').map(str::to_string).collect(),
                count: 1_000,
                first_seen_ns: 0,
                last_seen_ns: 0,
                max_severity: 9,
                sample: String::new(),
            });
            let a = d.add("fp", &access("00", "/api/cart", 503, "UF"), 1, 9);
            assert!(a.created, "threshold {threshold}");
            assert_ne!(a.template_id, old_id);
            assert!(
                d.cluster(a.template_id)
                    .unwrap()
                    .template()
                    .contains(" 503 ")
            );
            let old_c = d.cluster(old_id).unwrap();
            assert_eq!((old_c.count, old_c.template()), (1_000, old.to_string()));
        }
    }

    #[test]
    fn unprotected_numbers_merge_as_before() {
        let mut d = drain();
        // Status-like numbers with no `HTTP/x` before them are masked as usual.
        let a = d.add("svc", "retry 503 times", 0, 9);
        let b = d.add("svc", "retry 404 times", 0, 9);
        assert_eq!(a.template_id, b.template_id);
        assert_eq!(
            d.cluster(a.template_id).unwrap().template(),
            "retry <*> times"
        );
        let a = d.add("svc", r#""GET /x" 200 ok fine"#, 0, 9);
        let b = d.add("svc", r#""GET /x" 503 ok fine"#, 0, 9);
        assert_eq!(a.template_id, b.template_id);
        assert_eq!(
            d.cluster(a.template_id).unwrap().template(),
            r#""GET /x" <*> ok fine"#
        );
        // Access lines with the status absent (truncated) also merge as usual.
        let a = d.add("svc", r#"[t1] "GET /a HTTP/1.1""#, 0, 9);
        let b = d.add("svc", r#"[t2] "GET /b HTTP/1.1""#, 0, 9);
        assert_eq!(a.template_id, b.template_id);
    }

    #[test]
    fn protected_routing_token_gets_its_own_branch_when_the_node_is_full() {
        let mut d = Drain::new(DrainConfig {
            max_children: 1,
            ..DrainConfig::default()
        });
        // Tokens: `<*>` (masked HTTP/1.1), then the status: the status is a routing key.
        let ok = d.add("svc", "HTTP/1.1 200 a", 0, 9);
        let err = d.add("svc", "HTTP/1.1 503 a", 0, 9);
        let nf = d.add("svc", "HTTP/1.1 404 a", 0, 9);
        assert!(ok.created && err.created && nf.created);
        let err2 = d.add("svc", "HTTP/1.1 503 b", 0, 9);
        assert_eq!(err2.template_id, err.template_id);
        assert_eq!(
            d.cluster(err.template_id).unwrap().template(),
            "<*> 503 <*>"
        );
        // A full node still sends an unprotected new token down `<*>`.
        let plain = d.add("svc", "HTTP/1.1 zzz a", 0, 9);
        assert!(plain.created);
        assert_eq!(
            d.add("svc", "HTTP/1.1 yyy a", 0, 9).template_id,
            plain.template_id
        );

        // Restored from template strings, every status is matched exactly again.
        let mut r = Drain::new(DrainConfig {
            max_children: 1,
            ..DrainConfig::default()
        });
        for c in d.take_dirty() {
            r.restore(c);
        }
        for (line, id) in [
            ("HTTP/1.1 200 q", ok.template_id),
            ("HTTP/1.1 503 q", err.template_id),
            ("HTTP/1.1 404 q", nf.template_id),
        ] {
            let a = r.add("svc", line, 0, 9);
            assert_eq!((a.created, a.template_id), (false, id), "{line}");
        }
        assert!(r.add("svc", "HTTP/1.1 500 q", 0, 9).created);
    }

    fn old_cluster(service: &str, template: &str, first_seen_ns: i64) -> Cluster {
        Cluster {
            id: template_id(service, template),
            service: service.into(),
            tokens: template.split(' ').map(str::to_string).collect(),
            count: 100,
            first_seen_ns,
            last_seen_ns: first_seen_ns,
            max_severity: 9,
            sample: String::new(),
        }
    }

    fn toks(t: &str) -> Vec<String> {
        t.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn a_status_split_out_of_a_pre_epoch_wildcard_template_would_have_matched() {
        const EPOCH: i64 = 1_000;
        let old = r#"<*> "GET <*> <*> <*> <*> upstream_reset_before_response_started{connection_termination} <*> <*> <*> - "-" "python""#;
        let mut d = drain();
        d.restore(old_cluster("fp", old, EPOCH - 1));
        let line = r#"[2026-10-05T18:49:39.000Z] "GET /api/cart HTTP/1.1" 503 UC upstream_reset_before_response_started{connection_termination} 0 95 2 - "-" "python""#;
        let a = d.add("fp", line, EPOCH + 10, 9);
        assert!(
            a.created,
            "the 503 is protected, so the old template does not absorb it"
        );
        let new = d.cluster(a.template_id).unwrap().tokens.clone();
        assert!(new.contains(&"503".to_string()), "{new:?}");
        assert!(d.would_have_matched_pre_epoch("fp", &new, EPOCH));
        // The old template must predate the epoch, belong to the service and have the length.
        assert!(!d.would_have_matched_pre_epoch("fp", &new, EPOCH - 1));
        assert!(!d.would_have_matched_pre_epoch("other", &new, EPOCH));
        assert!(!d.would_have_matched_pre_epoch("fp", &new, 0), "no epoch");
    }

    #[test]
    fn a_genuinely_new_shape_with_a_status_would_not_have_matched() {
        const EPOCH: i64 = 1_000;
        let mut d = drain();
        d.restore(old_cluster(
            "fp",
            r#"<*> "GET <*> <*> <*> <*> <*> <*> <*> - "-" "python""#,
            EPOCH - 1,
        ));
        d.restore(old_cluster(
            "fp",
            "connection pool exhausted after <*>",
            EPOCH - 1,
        ));
        // Another length than any pre-epoch template.
        let a = d.add(
            "fp",
            r#"[t] "PUT /admin/reload HTTP/2" 418 teapot brewed coffee instead of tea x y z"#,
            EPOCH + 10,
            9,
        );
        let new = d.cluster(a.template_id).unwrap().tokens.clone();
        assert!(new.contains(&"418".to_string()), "{new:?}");
        assert!(!d.would_have_matched_pre_epoch("fp", &new, EPOCH));
        // The same length, and similar enough by wildcards, but another routing token ("PUT):
        // the old tree would have sent it to another leaf.
        let b = d.add(
            "fp",
            r#"[t] "PUT /admin/reload HTTP/2" 418 teapot brewed coffee instead of tea x"#,
            EPOCH + 20,
            9,
        );
        let new = d.cluster(b.template_id).unwrap().tokens.clone();
        assert_eq!(new.len(), 12);
        assert!(new.contains(&"418".to_string()), "{new:?}");
        assert!(!d.would_have_matched_pre_epoch("fp", &new, EPOCH));
        // Same routing and length, too few equal positions.
        d.restore(old_cluster(
            "fp",
            "<*> \"PUT a b c d e f g h i j",
            EPOCH - 1,
        ));
        assert!(!d.would_have_matched_pre_epoch("fp", &new, EPOCH));
    }

    #[test]
    fn templates_without_a_protected_token_are_never_suppressed() {
        const EPOCH: i64 = 1_000;
        let mut d = drain();
        d.restore(old_cluster(
            "svc",
            "Payment failed for order <*>",
            EPOCH - 1,
        ));
        // Even an exact pre-epoch match does not count: only status splits are suppressed.
        assert!(!d.would_have_matched_pre_epoch(
            "svc",
            &toks("Payment failed for order <*>"),
            EPOCH
        ));
        let mut off = with_status(false);
        off.restore(old_cluster("fp", "<*> <*> x", EPOCH - 1));
        assert!(
            !off.would_have_matched_pre_epoch("fp", &toks("<*> 503 x"), EPOCH),
            "without keep_http_status nothing is protected"
        );
    }

    const CORPUS: &[&str] = &[
        "GetCart called with user alice",
        "GetCart called with user bob",
        "user logged in as alice",
        "Payment failed for order 5555",
        "Payment failed for order 6666",
        "retry 503 times",
        "retry 404 times",
        "item added",
        "order placed",
        r#"[2026-10-05T10:00:00.000Z] "GET /api/cart HTTP/1.1" 200 - 0 91 2 - "-" "python""#,
        r#"[2026-10-05T10:00:01.000Z] "GET /api/cart HTTP/1.1" 503 UF 0 91 2 - "-" "python""#,
    ];

    /// (template id, template after the line) per corpus line.
    fn run(keep_http_status: bool) -> Vec<(u64, String)> {
        let mut d = with_status(keep_http_status);
        CORPUS
            .iter()
            .map(|l| {
                let id = d.add("svc", l, 0, 9).template_id;
                (id, d.cluster(id).unwrap().template())
            })
            .collect()
    }

    /// Recorded with the code before status protection (commit f636ce5).
    const BEFORE: &[(u64, &str)] = &[
        (9461447318419370651, "GetCart called with user alice"),
        (9461447318419370651, "GetCart called with user <*>"),
        (8818505925416069088, "user logged in as alice"),
        (10623876510446226940, "Payment failed for order <*>"),
        (10623876510446226940, "Payment failed for order <*>"),
        (15187620112400810895, "retry <*> times"),
        (15187620112400810895, "retry <*> times"),
        (6909147548793126864, "item added"),
        (181433866763387378, "order placed"),
    ];

    #[test]
    fn keep_http_status_off_behaves_exactly_as_before() {
        let mut expected: Vec<(u64, String)> =
            BEFORE.iter().map(|(i, t)| (*i, t.to_string())).collect();
        let http = r#"<*> "GET /api/cart <*> <*> - <*> <*> <*> - "-" "python""#;
        let merged = r#"<*> "GET /api/cart <*> <*> <*> <*> <*> <*> - "-" "python""#;
        expected.push((5634864414374559589, http.into()));
        expected.push((5634864414374559589, merged.into()));
        assert_eq!(run(false), expected);
    }

    #[test]
    fn non_http_template_ids_are_unchanged() {
        let got = run(true);
        for (i, (id, t)) in BEFORE.iter().enumerate() {
            assert_eq!((got[i].0, got[i].1.as_str()), (*id, *t), "{}", CORPUS[i]);
        }
        // The 200 template keeps its pre-fix id; the 503 line now gets its own template.
        let ok = r#"<*> "GET /api/cart <*> 200 - <*> <*> <*> - "-" "python""#;
        let err = r#"<*> "GET /api/cart <*> 503 UF <*> <*> <*> - "-" "python""#;
        assert_eq!(got[9], (7430681490948431569, ok.to_string()));
        assert_eq!(got[10], (template_id("svc", err), err.to_string()));
    }
}
