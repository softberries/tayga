use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;
use tayga_common::metrics::KindLabel;

#[derive(Clone, Default)]
pub struct AssemblerMetrics {
    pub closed_traces: Counter,
    /// `kind`: `error` | `slow`.
    pub stories: Family<KindLabel, Counter>,
    pub analysis_panics: Counter,
    pub serialization_failures: Counter,
    pub write_failures: Counter,
    pub open_traces: Gauge,
    pub buffered_bytes: Gauge,
    /// Exported as `tayga_assembler_late_items_total`.
    pub late_items: Counter,
    pub baseline_endpoints: Gauge,
}

impl AssemblerMetrics {
    pub fn register(registry: &mut Registry) -> Self {
        let m = Self::default();
        registry.register(
            "tayga_assembler_closed_traces",
            "Traces closed and analysed",
            m.closed_traces.clone(),
        );
        registry.register(
            "tayga_assembler_stories",
            "Stories produced",
            m.stories.clone(),
        );
        registry.register(
            "tayga_assembler_analysis_panics",
            "Traces skipped after an analysis panic",
            m.analysis_panics.clone(),
        );
        registry.register(
            "tayga_assembler_serialization_failures",
            "Stories dropped because they could not be serialized",
            m.serialization_failures.clone(),
        );
        registry.register(
            "tayga_assembler_write_failures",
            "Failed ClickHouse/Kafka write attempts",
            m.write_failures.clone(),
        );
        registry.register(
            "tayga_assembler_open_traces",
            "Traces currently buffered",
            m.open_traces.clone(),
        );
        registry.register(
            "tayga_assembler_buffered_bytes",
            "Record bytes currently buffered",
            m.buffered_bytes.clone(),
        );
        registry.register(
            "tayga_assembler_late_items",
            "Spans/logs received for already closed traces",
            m.late_items.clone(),
        );
        registry.register(
            "tayga_assembler_baseline_endpoints",
            "Endpoints with a loaded baseline",
            m.baseline_endpoints.clone(),
        );
        m
    }

    /// Advances `late_items` to the cumulative `total` reported by the window, given the
    /// total seen at the previous call in `last`. A total that went down (a new window) is
    /// taken as the new starting point.
    pub fn record_late_items(&self, last: &mut u64, total: u64) {
        self.late_items.inc_by(total.saturating_sub(*last));
        *last = total;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_items_counter_increments_by_delta() {
        let mut registry = Registry::default();
        let m = AssemblerMetrics::register(&mut registry);
        let mut last = 0;
        m.record_late_items(&mut last, 3);
        m.record_late_items(&mut last, 3);
        m.record_late_items(&mut last, 5);
        assert_eq!(m.late_items.get(), 5);
        m.record_late_items(&mut last, 1);
        assert_eq!(m.late_items.get(), 5, "a reset total does not decrement");
        m.record_late_items(&mut last, 2);
        assert_eq!(m.late_items.get(), 6);
        let mut out = String::new();
        prometheus_client::encoding::text::encode(&mut out, &registry).unwrap();
        assert!(out.contains("# TYPE tayga_assembler_late_items counter"));
        assert!(out.contains("tayga_assembler_late_items_total 6"));
    }
}
