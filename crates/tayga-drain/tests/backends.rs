//! Every backend returns what `ScalarFingerprinter` returns (sub-project 4 spec §4, item 2).

mod corpus;
mod strategies;

use proptest::prelude::*;
use tayga_drain::fingerprint::{BatchFingerprinter, BodyBatch, ScalarFingerprinter};
use tayga_drain::parallel::{PAR_MIN_BATCH, ParallelFingerprinter};

fn batch_of<'a>(bodies: impl IntoIterator<Item = &'a str>) -> BodyBatch {
    let mut batch = BodyBatch::new();
    for b in bodies {
        assert!(batch.push(b));
    }
    batch
}

fn assert_same(f: &dyn BatchFingerprinter, batch: &BodyBatch) {
    for keep in [true, false] {
        let (mut want, mut got) = (Vec::new(), Vec::new());
        ScalarFingerprinter.fingerprint(batch, keep, &mut want);
        f.fingerprint(batch, keep, &mut got);
        assert_eq!(want.len(), batch.len());
        assert_eq!(got, want, "{} keep_http_status={keep}", f.name());
    }
}

#[test]
fn parallel_equals_scalar_on_the_corpus() {
    let lines = corpus::load();
    assert_same(
        &ParallelFingerprinter,
        &batch_of(lines.iter().map(|l| l.body.as_str())),
    );
}

#[test]
fn parallel_equals_scalar_on_edge_cases_either_side_of_the_threshold() {
    let edge = [
        "",
        " ",
        "a\0b",
        "\0",
        "caf\u{e9} au lait",
        "\u{1F600} emoji 42",
        "GET /a 200 12ms",
        "tab\tand\nnewline  42",
    ];
    for n in [
        0,
        1,
        PAR_MIN_BATCH - 1,
        PAR_MIN_BATCH,
        PAR_MIN_BATCH + 1,
        2_000,
    ] {
        let bodies: Vec<&str> = edge.iter().copied().cycle().take(n).collect();
        let batch = batch_of(bodies.iter().copied());
        assert_same(&ParallelFingerprinter, &batch);
        let mut out = Vec::new();
        ParallelFingerprinter.fingerprint(&batch, true, &mut out);
        assert_eq!(out.len(), n);
    }
}

#[test]
fn parallel_replaces_the_output_instead_of_appending() {
    let batch = batch_of(["a b c"; 700]);
    let mut out = vec![None; 5];
    ParallelFingerprinter.fingerprint(&batch, true, &mut out);
    assert_eq!(out.len(), 700);
    let small = batch_of(["a b c"; 3]);
    ParallelFingerprinter.fingerprint(&small, true, &mut out);
    assert_eq!(out.len(), 3);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]
    #[test]
    fn parallel_equals_scalar_on_random_batches(bodies in prop::collection::vec(strategies::body(), 0..1_500)) {
        assert_same(&ParallelFingerprinter, &batch_of(bodies.iter().map(String::as_str)));
    }
}
