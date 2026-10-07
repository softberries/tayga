//! The sub-project 4 log corpus: `fixtures/log_corpus.jsonl.gz` (demo logs exported from the
//! stack's ClickHouse), or the JSON-lines file named by `TAYGA_CORPUS` (plain or `.gz`).

use flate2::read::GzDecoder;
use std::io::{BufRead, BufReader, Read};

#[derive(serde::Deserialize)]
pub struct Line {
    pub service: String,
    pub ts_ns: i64,
    pub sev: u8,
    pub body: String,
}

pub fn load() -> Vec<Line> {
    let path = std::env::var("TAYGA_CORPUS").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/log_corpus.jsonl.gz"
        )
        .to_string()
    });
    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    let reader: Box<dyn Read> = if path.ends_with(".gz") {
        Box::new(GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let lines: Vec<Line> = BufReader::new(reader)
        .lines()
        .map(|l| serde_json::from_str(&l.expect("utf-8 line")).expect("corpus JSON line"))
        .collect();
    // Every field is checked here, so each test reads them all (no dead-code warnings).
    assert!(
        lines.windows(2).all(|w| w[0].ts_ns <= w[1].ts_ns),
        "{path}: lines out of time order"
    );
    assert!(
        lines
            .iter()
            .all(|l| !l.service.is_empty() && l.sev <= 24 && l.body.len() < 1 << 24),
        "{path}: a line without a service, with a bad severity or a huge body"
    );
    lines
}
