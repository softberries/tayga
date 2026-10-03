use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use tayga_common::metrics::KindLabel;

/// `kind` label: `spans` | `logs`.
#[derive(Clone)]
pub struct WriterMetrics {
    pub rows_inserted: Family<KindLabel, Counter>,
    pub batches_committed: Counter,
    pub insert_failures: Counter,
    pub commit_failures: Counter,
    pub undecodable_records: Counter,
    /// Seconds per successful batch insert (spans + logs), excluding failed attempts.
    pub batch_seconds: Histogram,
}

impl Default for WriterMetrics {
    fn default() -> Self {
        Self {
            rows_inserted: Family::default(),
            batches_committed: Counter::default(),
            insert_failures: Counter::default(),
            commit_failures: Counter::default(),
            undecodable_records: Counter::default(),
            // 5 ms .. ~10 s.
            batch_seconds: Histogram::new(exponential_buckets(0.005, 2.0, 12)),
        }
    }
}

impl WriterMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let m = Self::default();
        registry.register(
            "tayga_writer_rows_inserted",
            "Rows inserted into ClickHouse",
            m.rows_inserted.clone(),
        );
        registry.register(
            "tayga_writer_batches_committed",
            "Batches inserted and committed",
            m.batches_committed.clone(),
        );
        registry.register(
            "tayga_writer_insert_failures",
            "Failed ClickHouse insert attempts",
            m.insert_failures.clone(),
        );
        registry.register(
            "tayga_writer_commit_failures",
            "Failed Kafka offset commits",
            m.commit_failures.clone(),
        );
        registry.register(
            "tayga_writer_undecodable_records",
            "Records skipped because they could not be decoded",
            m.undecodable_records.clone(),
        );
        registry.register(
            "tayga_writer_batch_seconds",
            "Duration of a successful ClickHouse batch insert",
            m.batch_seconds.clone(),
        );
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_seconds_is_exported_as_a_histogram() {
        let mut registry = Registry::default();
        let m = WriterMetrics::register(&mut registry);
        m.batch_seconds.observe(0.02);
        let mut out = String::new();
        prometheus_client::encoding::text::encode(&mut out, &registry).unwrap();
        assert!(out.contains("# TYPE tayga_writer_batch_seconds histogram"));
        assert!(out.contains("tayga_writer_batch_seconds_bucket{le=\"0.04\"} 1"));
        assert!(out.contains("tayga_writer_batch_seconds_count 1"));
    }
}
