//! Rayon over the bodies of one batch (sub-project 4 spec §3.2).

use crate::fingerprint::{BatchFingerprinter, BodyBatch, Fingerprint, fingerprint_body};
use rayon::prelude::*;

/// Below this many bodies a batch runs on the calling thread: rayon was slower than one thread
/// at 64 bodies and faster at 512 in the spike (spec §2.5).
pub const PAR_MIN_BATCH: usize = 512;
/// Fewest bodies per rayon task.
const PAR_MIN_LEN: usize = 64;

pub struct ParallelFingerprinter;

impl BatchFingerprinter for ParallelFingerprinter {
    fn name(&self) -> &'static str {
        "parallel"
    }

    fn fingerprint(
        &self,
        batch: &BodyBatch,
        keep_http_status: bool,
        out: &mut Vec<Option<Fingerprint>>,
    ) {
        out.clear();
        if batch.len() < PAR_MIN_BATCH {
            out.extend((0..batch.len()).map(|i| fingerprint_body(batch.body(i), keep_http_status)));
            return;
        }
        (0..batch.len())
            .into_par_iter()
            .with_min_len(PAR_MIN_LEN)
            .map(|i| fingerprint_body(batch.body(i), keep_http_status))
            .collect_into_vec(out);
    }
}
