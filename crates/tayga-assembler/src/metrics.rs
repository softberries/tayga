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
    pub late_items: Gauge,
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
}
