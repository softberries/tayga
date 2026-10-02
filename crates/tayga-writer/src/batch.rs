//! Rows pending insertion plus the Kafka offsets they cover.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tayga_store::rows::{LogRow, SpanRow};

#[derive(Default)]
pub struct Batch {
    pub spans: Vec<SpanRow>,
    pub logs: Vec<LogRow>,
    last_offsets: BTreeMap<i32, i64>,
    started: Option<Instant>,
}

impl Batch {
    pub fn add(&mut self, partition: i32, offset: i64, spans: Vec<SpanRow>, logs: Vec<LogRow>, now: Instant) {
        self.spans.extend(spans);
        self.logs.extend(logs);
        let last = self.last_offsets.entry(partition).or_insert(offset);
        *last = (*last).max(offset);
        self.started.get_or_insert(now);
    }

    pub fn rows(&self) -> usize {
        self.spans.len() + self.logs.len()
    }

    pub fn should_flush(&self, now: Instant, max_rows: usize, max_age: Duration) -> bool {
        match self.started {
            None => false,
            Some(start) => self.rows() >= max_rows || now.duration_since(start) >= max_age,
        }
    }

    /// Kafka commits the offset of the next message to read, hence `+ 1`.
    pub fn commit_offsets(&self) -> Vec<(i32, i64)> {
        self.last_offsets.iter().map(|(p, o)| (*p, o + 1)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> LogRow {
        LogRow {
            log_id: 1,
            ts: 0,
            observed_ts: 0,
            trace_id: String::new(),
            span_id: String::new(),
            severity_number: 9,
            severity_text: String::new(),
            service_name: "s".into(),
            body: String::new(),
            resource_attrs: vec![],
            log_attrs: vec![],
        }
    }

    #[test]
    fn empty_batch_never_flushes() {
        let b = Batch::default();
        assert!(!b.should_flush(Instant::now() + Duration::from_secs(60), 1, Duration::ZERO));
    }

    #[test]
    fn flushes_on_size() {
        let now = Instant::now();
        let mut b = Batch::default();
        b.add(0, 10, vec![], vec![log(), log()], now);
        assert!(!b.should_flush(now, 3, Duration::from_secs(1)));
        b.add(0, 11, vec![], vec![log()], now);
        assert!(b.should_flush(now, 3, Duration::from_secs(1)));
        assert_eq!(b.rows(), 3);
    }

    #[test]
    fn flushes_on_age_of_first_message() {
        let t0 = Instant::now();
        let mut b = Batch::default();
        b.add(0, 1, vec![], vec![log()], t0);
        b.add(0, 2, vec![], vec![log()], t0 + Duration::from_millis(900));
        assert!(!b.should_flush(t0 + Duration::from_millis(999), 100, Duration::from_secs(1)));
        assert!(b.should_flush(t0 + Duration::from_secs(1), 100, Duration::from_secs(1)));
    }

    #[test]
    fn batch_tracks_offsets_for_messages_without_rows() {
        let now = Instant::now();
        let mut b = Batch::default();
        b.add(3, 41, vec![], vec![], now); // poison/empty message
        b.add(1, 7, vec![], vec![log()], now);
        b.add(3, 40, vec![], vec![], now); // out of order within partition: keep max
        assert_eq!(b.commit_offsets(), vec![(1, 8), (3, 42)]);
        assert!(b.should_flush(now + Duration::from_secs(1), 100, Duration::from_secs(1)));
    }
}
