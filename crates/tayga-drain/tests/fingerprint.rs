//! `fingerprint_body` equals `reference_fingerprint` (sub-project 4 spec §4, items 1).

mod corpus;
mod strategies;

use proptest::prelude::*;
use tayga_drain::fingerprint::{fingerprint_body, reference_fingerprint};

#[test]
fn every_corpus_body_matches_the_reference() {
    let mut none = 0;
    for line in corpus::load() {
        for keep in [true, false] {
            let fast = fingerprint_body(line.body.as_bytes(), keep);
            if line.body.is_ascii() && !line.body.contains('\0') {
                assert_eq!(
                    fast,
                    Some(reference_fingerprint(&line.body, keep)),
                    "{:?}",
                    line.body
                );
            } else {
                assert_eq!(fast, None, "{:?}", line.body);
                none += 1;
            }
        }
    }
    println!("non-ASCII bodies (counted twice): {none}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2_000))]
    #[test]
    fn random_bodies_match_the_reference(body in strategies::body(), keep in any::<bool>()) {
        let expected = (body.is_ascii() && !body.contains('\0')).then(|| reference_fingerprint(&body, keep));
        prop_assert_eq!(fingerprint_body(body.as_bytes(), keep), expected);
    }
}
