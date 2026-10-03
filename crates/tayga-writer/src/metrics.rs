use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::registry::Registry;
use tayga_common::metrics::KindLabel;

/// `kind` label: `spans` | `logs`.
#[derive(Clone, Default)]
pub struct WriterMetrics {
    pub rows_inserted: Family<KindLabel, Counter>,
    pub batches_committed: Counter,
    pub insert_failures: Counter,
    pub commit_failures: Counter,
    pub undecodable_records: Counter,
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
        m
    }
}
