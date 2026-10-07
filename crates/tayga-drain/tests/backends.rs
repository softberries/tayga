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

#[cfg(feature = "gpu")]
mod gpu {
    use super::*;
    use std::sync::OnceLock;
    use tayga_drain::gpu::{GPU_MIN_BATCH, GpuFingerprinter};

    /// One device for every test; `None` without a usable adapter.
    fn gpu() -> Option<&'static GpuFingerprinter> {
        static GPU: OnceLock<Option<GpuFingerprinter>> = OnceLock::new();
        let g = GPU.get_or_init(GpuFingerprinter::new).as_ref();
        if g.is_none() {
            eprintln!("no GPU adapter: skipped");
        }
        g
    }

    #[test]
    fn gpu_equals_scalar_on_the_corpus() {
        let Some(g) = gpu() else { return };
        let lines = corpus::load();
        assert_same(g, &batch_of(lines.iter().map(|l| l.body.as_str())));
    }

    /// Either side of the CPU threshold, and GPU batches that end inside, at and just past a
    /// 64-invocation workgroup.
    #[test]
    fn gpu_equals_scalar_on_edge_cases_either_side_of_the_threshold() {
        let Some(g) = gpu() else { return };
        let edge = [
            "",
            " ",
            "a\0b",
            "\0",
            "x \0",
            "\0 tail 42",
            "caf\u{e9} au lait",
            "\u{1F600} emoji 42",
            "GET /a 200 12ms",
            "tab\tand\nnewline  42",
            r#""GET /api/cart HTTP/1.1" 503 UF"#,
            "deadbeefcafe a<*>b",
        ];
        for n in [
            0,
            1,
            63,
            64,
            65,
            GPU_MIN_BATCH - 1,
            GPU_MIN_BATCH,
            GPU_MIN_BATCH + 1,
            GPU_MIN_BATCH + 63,
            GPU_MIN_BATCH + 64,
            GPU_MIN_BATCH + 65,
            5_000,
        ] {
            let bodies: Vec<&str> = edge.iter().copied().cycle().take(n).collect();
            let batch = batch_of(bodies.iter().copied());
            assert_same(g, &batch);
            let mut out = Vec::new();
            g.fingerprint(&batch, true, &mut out);
            assert_eq!(out.len(), n);
        }
    }

    #[test]
    fn gpu_replaces_the_output_instead_of_appending() {
        let Some(g) = gpu() else { return };
        let batch = batch_of(["a b c"; GPU_MIN_BATCH + 10]);
        let mut out = vec![None; 5];
        g.fingerprint(&batch, true, &mut out);
        assert_eq!(out.len(), GPU_MIN_BATCH + 10);
        let small = batch_of(["a b c"; 3]);
        g.fingerprint(&small, true, &mut out);
        assert_eq!(out.len(), 3);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]
        #[test]
        fn gpu_equals_scalar_on_random_batches(bodies in prop::collection::vec(strategies::body(), 0..3_000)) {
            if let Some(g) = gpu() {
                assert_same(g, &batch_of(bodies.iter().map(String::as_str)));
            }
        }
    }
}
