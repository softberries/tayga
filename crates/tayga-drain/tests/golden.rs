use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use tayga_drain::drain::{Drain, DrainConfig};

fn mine() -> Drain {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/log_bodies.tsv.gz"
    );
    let file = std::fs::File::open(path).expect("fixture");
    let mut d = Drain::new(DrainConfig::default());
    for (i, line) in BufReader::new(GzDecoder::new(file)).lines().enumerate() {
        let line = line.expect("utf-8 line");
        let (svc, body) = line.split_once('\t').unwrap_or((line.as_str(), ""));
        d.add(svc, body, i as i64, 9);
    }
    d
}

#[test]
fn demo_logs_collapse_into_few_templates() {
    let mut d = mine();
    let clusters = d.take_dirty();
    let mut per_service: HashMap<&str, usize> = HashMap::new();
    for c in &clusters {
        *per_service.entry(c.service.as_str()).or_default() += 1;
    }
    println!("templates: {} {:?}", clusters.len(), per_service);
    assert!(clusters.len() <= 120, "{} templates", clusters.len());
    assert!(per_service.get("frontend-proxy").copied().unwrap_or(0) <= 10);
    let templates: Vec<String> = clusters.iter().map(|c| c.template()).collect();
    for want in [
        "Product Found",
        "Found <*> products from database",
        "conversion successful",
    ] {
        assert!(
            templates.iter().any(|t| t == want),
            "missing template {want:?}"
        );
    }
}

#[test]
fn mining_is_deterministic() {
    let mut a = mine();
    let mut b = mine();
    let mut ia: Vec<u64> = a.take_dirty().iter().map(|c| c.id).collect();
    let mut ib: Vec<u64> = b.take_dirty().iter().map(|c| c.id).collect();
    ia.sort_unstable();
    ib.sort_unstable();
    assert_eq!(ia, ib);
}
