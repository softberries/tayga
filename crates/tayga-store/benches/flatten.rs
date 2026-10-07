//! Envelope to ClickHouse rows (sub-project 4 spec §5): the writer's and the logminer's row
//! building, over every envelope of `fixtures/healthy.pb.gz`.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use tayga_model::envelope::read_framed;
use tayga_store::flatten::rows_from_envelope;

fn flatten(c: &mut Criterion) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/healthy.pb.gz");
    let file = std::fs::File::open(path).expect("fixture");
    let envelopes = read_framed(&mut flate2::read::GzDecoder::new(file)).expect("envelopes");
    let mut g = c.benchmark_group("flatten");
    g.throughput(Throughput::Elements(envelopes.len() as u64));
    g.bench_function("rows_from_envelope", |b| {
        b.iter(|| {
            for env in &envelopes {
                black_box(rows_from_envelope(env));
            }
        })
    });
    g.finish();
}

criterion_group!(benches, flatten);
criterion_main!(benches);
