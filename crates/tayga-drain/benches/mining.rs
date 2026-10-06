//! Drain mining stages, fingerprint backends and the cache (sub-project 4 spec §5).
//! `cargo bench -p tayga-drain --bench mining` (add `--features gpu` for the GPU backend).

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use tayga_drain::drain::{Drain, DrainConfig};
use tayga_drain::fingerprint::{BatchFingerprinter, BodyBatch, ScalarFingerprinter};
use tayga_drain::preprocess::{MAX_TOKENS, tokens};

#[path = "../tests/corpus/mod.rs"]
mod corpus;

/// The `§2.3` breakdown: split, `tokens` (split + mask), and the whole `Drain::add`.
fn stages(c: &mut Criterion) {
    let lines = corpus::load();
    let mut g = c.benchmark_group("stages");
    g.throughput(Throughput::Elements(lines.len() as u64));
    g.sample_size(10);
    g.bench_function("split", |b| {
        b.iter(|| {
            for l in &lines {
                let v: Vec<String> = l
                    .body
                    .split_ascii_whitespace()
                    .take(MAX_TOKENS + 1)
                    .map(str::to_string)
                    .collect();
                black_box(v);
            }
        })
    });
    g.bench_function("tokens", |b| {
        b.iter(|| {
            for l in &lines {
                black_box(tokens(&l.body, true));
            }
        })
    });
    g.bench_function("drain_add", |b| {
        b.iter(|| {
            let mut d = Drain::new(DrainConfig::default());
            for l in &lines {
                black_box(d.add(&l.service, &l.body, l.ts_ns, l.sev));
            }
        })
    });
    g.finish();
}
fn backends() -> Vec<Box<dyn BatchFingerprinter>> {
    vec![Box::new(ScalarFingerprinter)]
}

/// Every backend at the batch sizes of spec §2.5.
fn fingerprint(c: &mut Criterion) {
    let lines = corpus::load();
    let backends = backends();
    let mut g = c.benchmark_group("fingerprint");
    for n in [5usize, 64, 512, 2_048, 5_000, 50_000] {
        let mut batch = BodyBatch::new();
        for l in lines.iter().take(n) {
            batch.push(&l.body);
        }
        g.throughput(Throughput::Elements(batch.len() as u64));
        let mut out = Vec::new();
        for f in &backends {
            g.bench_with_input(BenchmarkId::new(f.name(), n), &batch, |b, batch| {
                b.iter(|| {
                    f.fingerprint(batch, true, &mut out);
                    black_box(&out);
                })
            });
        }
    }
    g.finish();
}

criterion_group!(benches, stages, fingerprint);
criterion_main!(benches);
