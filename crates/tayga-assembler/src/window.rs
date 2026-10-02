//! Per-partition session windows keyed by trace id (spec §8). Pure: time is passed in.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use tayga_analysis::model::TraceBundle;
use tayga_model::envelope::{Envelope, Payload};

pub const FLAG_TRUNCATED: &str = "truncated";

#[derive(Debug, Clone)]
pub struct WindowConfig {
    /// Close after this long without new data (processing time).
    pub gap: Duration,
    /// Close after this long open, even if still receiving data.
    pub max_age: Duration,
    /// Close as truncated above this many spans.
    pub max_spans: usize,
    /// Close oldest traces as truncated while buffered record bytes exceed this.
    pub max_bytes: usize,
    /// Recently closed trace ids remembered per partition for late-data detection.
    pub recent_per_partition: usize,
}

#[derive(Debug)]
pub struct ClosedTrace {
    pub partition: i32,
    pub bundle: TraceBundle,
    pub flags: Vec<String>,
}

struct Buffer {
    bundle: TraceBundle,
    first_offset: i64,
    opened: Instant,
    last_seen: Instant,
    bytes: usize,
}

#[derive(Default)]
struct Recent {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl Recent {
    fn insert(&mut self, id: String, cap: usize) {
        if self.set.insert(id.clone()) {
            self.order.push_back(id);
            while self.order.len() > cap {
                if let Some(old) = self.order.pop_front() {
                    self.set.remove(&old);
                }
            }
        }
    }
}

#[derive(Default)]
struct PartitionState {
    open: HashMap<String, Buffer>,
    recent: Recent,
    /// Offset after the last record seen.
    next_offset: Option<i64>,
}

pub struct Windows {
    cfg: WindowConfig,
    partitions: HashMap<i32, PartitionState>,
    total_bytes: usize,
    late_items: u64,
}

/// Spans plus log records in an envelope.
pub fn envelope_items(env: &Envelope) -> usize {
    match &env.payload {
        Some(Payload::Traces(t)) => t
            .resource_spans
            .iter()
            .flat_map(|r| &r.scope_spans)
            .map(|s| s.spans.len())
            .sum(),
        Some(Payload::Logs(l)) => l
            .resource_logs
            .iter()
            .flat_map(|r| &r.scope_logs)
            .map(|s| s.log_records.len())
            .sum(),
        None => 0,
    }
}

impl Windows {
    pub fn new(cfg: WindowConfig) -> Self {
        Self {
            cfg,
            partitions: HashMap::new(),
            total_bytes: 0,
            late_items: 0,
        }
    }

    /// Records one Kafka record. `trace_id` is `None` for service-keyed records, which only
    /// advance the partition's offset. Returns traces closed for size reasons.
    pub fn ingest(
        &mut self,
        partition: i32,
        offset: i64,
        trace_id: Option<&str>,
        env: &Envelope,
        bytes: usize,
        now: Instant,
    ) -> Vec<ClosedTrace> {
        let state = self.partitions.entry(partition).or_default();
        state.next_offset = Some(state.next_offset.map_or(offset + 1, |n| n.max(offset + 1)));
        let Some(trace_id) = trace_id else {
            return Vec::new();
        };
        if state.recent.set.contains(trace_id) {
            self.late_items += envelope_items(env) as u64;
            return Vec::new();
        }
        let buffer = state
            .open
            .entry(trace_id.to_string())
            .or_insert_with(|| Buffer {
                bundle: TraceBundle::new(trace_id),
                first_offset: offset,
                opened: now,
                last_seen: now,
                bytes: 0,
            });
        buffer.bundle.add_envelope(env);
        buffer.last_seen = now;
        buffer.bytes += bytes;
        let over_spans = buffer.bundle.spans.len() > self.cfg.max_spans;
        self.total_bytes += bytes;
        let mut closed = Vec::new();
        if over_spans {
            closed.extend(self.close(partition, trace_id, Some(FLAG_TRUNCATED)));
        }
        while self.total_bytes > self.cfg.max_bytes {
            let Some((p, id)) = self.oldest_open() else {
                break;
            };
            closed.extend(self.close(p, &id, Some(FLAG_TRUNCATED)));
        }
        closed
    }

    /// Traces inactive for `gap` or open for `max_age`, oldest first.
    pub fn close_due(&mut self, now: Instant) -> Vec<ClosedTrace> {
        let mut due: Vec<(Instant, i32, String)> = Vec::new();
        for (p, state) in &self.partitions {
            for (id, b) in &state.open {
                if now.duration_since(b.last_seen) >= self.cfg.gap
                    || now.duration_since(b.opened) >= self.cfg.max_age
                {
                    due.push((b.opened, *p, id.clone()));
                }
            }
        }
        due.sort();
        due.into_iter()
            .filter_map(|(_, p, id)| self.close(p, &id, None))
            .collect()
    }

    /// Next offset to commit per partition: never past the first record of an open trace.
    pub fn commit_offsets(&self) -> Vec<(i32, i64)> {
        let mut out: Vec<(i32, i64)> = self
            .partitions
            .iter()
            .filter_map(|(p, state)| {
                let next = state.next_offset?;
                let held = state.open.values().map(|b| b.first_offset).min();
                Some((*p, held.map_or(next, |h| h.min(next))))
            })
            .collect();
        out.sort();
        out
    }

    /// Drops all state of partitions taken away by a rebalance; the new owner re-reads them.
    pub fn revoke(&mut self, partitions: &[i32]) {
        for p in partitions {
            if let Some(state) = self.partitions.remove(p) {
                let bytes: usize = state.open.values().map(|b| b.bytes).sum();
                self.total_bytes = self.total_bytes.saturating_sub(bytes);
            }
        }
    }

    pub fn open_traces(&self) -> usize {
        self.partitions.values().map(|p| p.open.len()).sum()
    }

    pub fn buffered_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn late_items(&self) -> u64 {
        self.late_items
    }

    fn oldest_open(&self) -> Option<(i32, String)> {
        self.partitions
            .iter()
            .flat_map(|(p, state)| state.open.iter().map(move |(id, b)| (b.opened, *p, id)))
            .min_by_key(|&(opened, p, id)| (opened, p, id))
            .map(|(_, p, id)| (p, id.clone()))
    }

    fn close(&mut self, partition: i32, trace_id: &str, flag: Option<&str>) -> Option<ClosedTrace> {
        let cap = self.cfg.recent_per_partition;
        let state = self.partitions.get_mut(&partition)?;
        let buffer = state.open.remove(trace_id)?;
        state.recent.insert(trace_id.to_string(), cap);
        self.total_bytes = self.total_bytes.saturating_sub(buffer.bytes);
        Some(ClosedTrace {
            partition,
            bundle: buffer.bundle,
            flags: flag.map(|f| vec![f.to_string()]).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tayga_model::ids::TraceId;
    use tayga_model::otlp::collector::trace::v1::ExportTraceServiceRequest;
    use tayga_model::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};

    fn cfg() -> WindowConfig {
        WindowConfig {
            gap: Duration::from_secs(10),
            max_age: Duration::from_secs(60),
            max_spans: 100,
            max_bytes: 1_000_000,
            recent_per_partition: 10,
        }
    }

    fn env(trace: u8, span_ids: &[u8]) -> Envelope {
        let spans = span_ids
            .iter()
            .map(|&s| Span {
                trace_id: vec![trace; 16],
                span_id: vec![s; 8],
                name: format!("op{s}"),
                start_time_unix_nano: 1,
                end_time_unix_nano: 2,
                ..Default::default()
            })
            .collect();
        Envelope::traces(
            ExportTraceServiceRequest {
                resource_spans: vec![ResourceSpans {
                    scope_spans: vec![ScopeSpans {
                        spans,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
            },
            0,
        )
    }

    fn tid(t: u8) -> String {
        TraceId([t; 16]).to_hex()
    }

    #[test]
    fn closes_after_gap_not_before() {
        let t0 = Instant::now();
        let mut w = Windows::new(cfg());
        assert!(
            w.ingest(0, 5, Some(&tid(1)), &env(1, &[1]), 10, t0)
                .is_empty()
        );
        assert!(w.close_due(t0 + Duration::from_secs(9)).is_empty());
        let closed = w.close_due(t0 + Duration::from_secs(10));
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].bundle.spans.len(), 1);
        assert!(closed[0].flags.is_empty());
        assert_eq!(w.open_traces(), 0);
        assert_eq!(w.buffered_bytes(), 0);
    }

    #[test]
    fn records_of_one_trace_merge_and_dedup() {
        let t0 = Instant::now();
        let mut w = Windows::new(cfg());
        w.ingest(0, 1, Some(&tid(1)), &env(1, &[1, 2]), 10, t0);
        w.ingest(
            0,
            2,
            Some(&tid(1)),
            &env(1, &[2, 3]),
            10,
            t0 + Duration::from_secs(5),
        );
        assert!(
            w.close_due(t0 + Duration::from_secs(14)).is_empty(),
            "activity resets the gap"
        );
        let closed = w.close_due(t0 + Duration::from_secs(15));
        assert_eq!(closed[0].bundle.spans.len(), 3);
    }

    #[test]
    fn commit_holds_back_open_traces() {
        let t0 = Instant::now();
        let mut w = Windows::new(cfg());
        w.ingest(0, 5, Some(&tid(1)), &env(1, &[1]), 10, t0);
        w.ingest(
            0,
            6,
            Some(&tid(2)),
            &env(2, &[1]),
            10,
            t0 + Duration::from_secs(5),
        );
        w.ingest(
            0,
            7,
            None,
            &Envelope::default(),
            3,
            t0 + Duration::from_secs(5),
        );
        assert_eq!(w.commit_offsets(), vec![(0, 5)]);
        w.close_due(t0 + Duration::from_secs(10));
        assert_eq!(w.commit_offsets(), vec![(0, 6)]);
        w.close_due(t0 + Duration::from_secs(15));
        assert_eq!(w.commit_offsets(), vec![(0, 8)]);
    }

    #[test]
    fn service_keyed_records_only_advance_offsets() {
        let mut w = Windows::new(cfg());
        assert!(
            w.ingest(3, 41, None, &env(9, &[1]), 10, Instant::now())
                .is_empty()
        );
        assert_eq!(w.open_traces(), 0);
        assert_eq!(w.commit_offsets(), vec![(3, 42)]);
    }

    #[test]
    fn late_items_are_counted_after_close() {
        let t0 = Instant::now();
        let mut w = Windows::new(cfg());
        w.ingest(0, 1, Some(&tid(1)), &env(1, &[1]), 10, t0);
        w.close_due(t0 + Duration::from_secs(10));
        assert!(
            w.ingest(
                0,
                2,
                Some(&tid(1)),
                &env(1, &[2, 3]),
                10,
                t0 + Duration::from_secs(11)
            )
            .is_empty()
        );
        assert_eq!(w.late_items(), 2);
        assert_eq!(w.open_traces(), 0);
    }

    #[test]
    fn max_spans_truncates() {
        let mut w = Windows::new(WindowConfig {
            max_spans: 2,
            ..cfg()
        });
        let closed = w.ingest(0, 1, Some(&tid(1)), &env(1, &[1, 2, 3]), 10, Instant::now());
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].flags, vec![FLAG_TRUNCATED.to_string()]);
    }

    #[test]
    fn byte_cap_closes_oldest() {
        let t0 = Instant::now();
        let mut w = Windows::new(WindowConfig {
            max_bytes: 100,
            ..cfg()
        });
        assert!(
            w.ingest(0, 1, Some(&tid(1)), &env(1, &[1]), 60, t0)
                .is_empty()
        );
        let closed = w.ingest(
            1,
            1,
            Some(&tid(2)),
            &env(2, &[1]),
            60,
            t0 + Duration::from_secs(1),
        );
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].bundle.trace_id, tid(1));
        assert_eq!(closed[0].partition, 0);
        assert_eq!(w.buffered_bytes(), 60);
    }

    #[test]
    fn max_age_closes_a_busy_trace() {
        let t0 = Instant::now();
        let mut w = Windows::new(cfg());
        for i in 0..=12u64 {
            let now = t0 + Duration::from_secs(i * 5);
            w.ingest(0, i as i64, Some(&tid(1)), &env(1, &[i as u8]), 10, now);
            let closed = w.close_due(now);
            if i < 12 {
                assert!(closed.is_empty(), "closed early at {i}");
            } else {
                assert_eq!(closed.len(), 1);
            }
        }
    }

    #[test]
    fn revoke_drops_partition_state() {
        let mut w = Windows::new(cfg());
        w.ingest(0, 1, Some(&tid(1)), &env(1, &[1]), 10, Instant::now());
        w.ingest(1, 1, Some(&tid(2)), &env(2, &[1]), 10, Instant::now());
        w.revoke(&[0]);
        assert_eq!(w.open_traces(), 1);
        assert_eq!(w.buffered_bytes(), 10);
        assert_eq!(w.commit_offsets(), vec![(1, 1)]);
    }

    #[test]
    fn envelope_items_counts_spans_and_logs() {
        assert_eq!(envelope_items(&env(1, &[1, 2, 3])), 3);
        assert_eq!(envelope_items(&Envelope::default()), 0);
    }
}
