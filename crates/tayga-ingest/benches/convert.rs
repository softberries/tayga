//! OTLP traces to Kafka records (sub-project 4 spec §5): every trace request of
//! `fixtures/healthy.pb.gz` merged into one export request, as a collector batch.

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use tayga_ingest::records::trace_records;
use tayga_model::envelope::{Payload, read_framed};
use tayga_model::otlp::collector::trace::v1::ExportTraceServiceRequest;

/// `kafka.max_record_bytes` default.
const MAX_RECORD_BYTES: usize = 900_000;

fn request() -> ExportTraceServiceRequest {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/healthy.pb.gz");
    let file = std::fs::File::open(path).expect("fixture");
    let envelopes = read_framed(&mut flate2::read::GzDecoder::new(file)).expect("envelopes");
    let mut req = ExportTraceServiceRequest::default();
    for env in envelopes {
        if let Some(Payload::Traces(t)) = env.payload {
            req.resource_spans.extend(t.resource_spans);
        }
    }
    req
}

fn convert(c: &mut Criterion) {
    let req = request();
    let spans: usize = req
        .resource_spans
        .iter()
        .flat_map(|r| &r.scope_spans)
        .map(|s| s.spans.len())
        .sum();
    let mut g = c.benchmark_group("convert");
    g.throughput(Throughput::Elements(spans as u64));
    g.bench_function("trace_records", |b| {
        b.iter_batched(
            || req.clone(),
            |r| black_box(trace_records(r, 1, MAX_RECORD_BYTES)),
            BatchSize::LargeInput,
        )
    });
    g.finish();
}

criterion_group!(benches, convert);
criterion_main!(benches);
