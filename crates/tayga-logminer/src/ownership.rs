//! Services this replica owns: those it mined a log of within the ownership window. Ownership
//! follows the Kafka assignment (records are keyed by service), so it is learned from traffic
//! rather than from the partition list. Alert detection is scoped to this set.

use std::collections::HashMap;

const MIN_NS: i64 = 60_000_000_000;

#[derive(Debug)]
pub struct Ownership {
    window_ns: i64,
    last_seen_ns: HashMap<String, i64>,
}

impl Ownership {
    pub fn new(window_min: u32) -> Self {
        Self {
            window_ns: i64::from(window_min) * MIN_NS,
            last_seen_ns: HashMap::new(),
        }
    }

    /// Records that a log of `service` was mined at wall time `now_ns`.
    pub fn touch(&mut self, service: &str, now_ns: i64) {
        match self.last_seen_ns.get_mut(service) {
            Some(t) => *t = (*t).max(now_ns),
            None => {
                self.last_seen_ns.insert(service.to_string(), now_ns);
            }
        }
    }

    /// Forgets every service, on a new Kafka assignment: ownership is then relearned from the
    /// records of the partitions assigned now.
    pub fn clear(&mut self) {
        self.last_seen_ns.clear();
    }

    /// Services mined within the window ending at `now_ns`, sorted. Expired ones are dropped.
    pub fn owned(&mut self, now_ns: i64) -> Vec<String> {
        let window = self.window_ns;
        self.last_seen_ns
            .retain(|_, t| now_ns.saturating_sub(*t) <= window);
        let mut v: Vec<String> = self.last_seen_ns.keys().cloned().collect();
        v.sort_unstable();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_empty() {
        assert!(Ownership::new(60).owned(0).is_empty());
    }

    #[test]
    fn owns_services_seen_within_the_window_sorted() {
        let mut o = Ownership::new(60);
        o.touch("b", 10 * MIN_NS);
        o.touch("a", 20 * MIN_NS);
        assert_eq!(o.owned(30 * MIN_NS), ["a", "b"]);
    }

    #[test]
    fn expires_after_the_window_and_touch_renews() {
        let mut o = Ownership::new(60);
        o.touch("a", 0);
        o.touch("b", 0);
        assert_eq!(o.owned(60 * MIN_NS), ["a", "b"], "the bound is inclusive");
        o.touch("b", 50 * MIN_NS);
        assert_eq!(o.owned(61 * MIN_NS), ["b"]);
        assert!(o.owned(111 * MIN_NS).is_empty());
    }

    #[test]
    fn clear_forgets_every_service_and_touch_relearns() {
        let mut o = Ownership::new(60);
        o.touch("a", 10 * MIN_NS);
        o.touch("b", 10 * MIN_NS);
        o.clear();
        assert!(o.owned(11 * MIN_NS).is_empty());
        o.touch("b", 12 * MIN_NS);
        assert_eq!(o.owned(13 * MIN_NS), ["b"]);
    }

    #[test]
    fn an_older_touch_does_not_shorten_ownership() {
        let mut o = Ownership::new(10);
        o.touch("a", 100 * MIN_NS);
        o.touch("a", 5 * MIN_NS);
        assert_eq!(o.owned(105 * MIN_NS), ["a"]);
    }
}
