//! Trace assembly (sub-project 4 spec §5): every envelope of `fixtures/healthy.pb.gz` into the
//! window, every trace closed, then analysed.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::collections::HashMap;
use std::hint::black_box;
use std::time::{Duration, Instant};
use tayga_analysis::baseline::Thresholds;
use tayga_analysis::model::trace_id_of;
use tayga_assembler::pipeline::process;
use tayga_assembler::window::{WindowConfig, Windows};
use tayga_model::envelope::read_framed;

fn cfg() -> WindowConfig {
    WindowConfig {
        gap: Duration::from_secs(10),
        max_age: Duration::from_secs(60),
        max_spans: 10_000,
        max_bytes: 1 << 30,
        recent_per_partition: 100_000,
    }
}

fn close(c: &mut Criterion) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/healthy.pb.gz");
    let file = std::fs::File::open(path).expect("fixture");
    let envelopes = read_framed(&mut flate2::read::GzDecoder::new(file)).expect("envelopes");
    let ids: Vec<Option<String>> = envelopes
        .iter()
        .map(|e| trace_id_of(e).map(|t| t.to_hex()))
        .collect();
    let sizes: Vec<usize> = envelopes.iter().map(|e| e.encode().len()).collect();
    let mut g = c.benchmark_group("assemble");
    g.throughput(Throughput::Elements(envelopes.len() as u64));
    g.sample_size(20);
    g.bench_function("ingest_close_process", |b| {
        b.iter(|| {
            let now = Instant::now();
            let mut w = Windows::new(cfg());
            for (i, env) in envelopes.iter().enumerate() {
                black_box(w.ingest(0, i as i64, ids[i].as_deref(), env, sizes[i], now));
            }
            let closed = w.close_due(now + Duration::from_secs(3_600));
            black_box(process(&closed, &HashMap::new(), &Thresholds::default()))
        })
    });
    g.finish();
}

criterion_group!(benches, close);
criterion_main!(benches);
