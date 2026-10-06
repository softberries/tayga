//! The committed corpus is usable: enough lines and services (sub-project 4 spec §5).

mod corpus;

use std::collections::HashSet;

#[test]
fn the_corpus_has_real_demo_logs() {
    let lines = corpus::load();
    assert!(lines.len() >= 1_000, "{} lines", lines.len());
    let services: HashSet<&str> = lines.iter().map(|l| l.service.as_str()).collect();
    assert!(services.len() >= 10, "{services:?}");
}
