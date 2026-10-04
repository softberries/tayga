use flate2::read::GzDecoder;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use tayga_drain::drain::{Cluster, Drain, DrainConfig};

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

fn fixture_lines() -> Vec<(String, String)> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/log_bodies.tsv.gz"
    );
    let file = std::fs::File::open(path).expect("fixture");
    BufReader::new(GzDecoder::new(file))
        .lines()
        .map(|line| {
            let line = line.expect("utf-8 line");
            let (svc, body) = line.split_once('\t').unwrap_or((line.as_str(), ""));
            (svc.to_string(), body.to_string())
        })
        .collect()
}

/// Spec §11: restart equivalence on a real corpus. Mining the first half, persisting the
/// clusters, restoring them into a fresh `Drain` in first-seen order (as the logminer does) and
/// mining the second half must assign every line the same template id as a single pass.
#[test]
fn restore_mid_corpus_matches_a_single_pass() {
    let lines = fixture_lines();
    let half = lines.len() / 2;
    let mine_range = |d: &mut Drain, range: std::ops::Range<usize>| -> Vec<u64> {
        range
            .map(|i| d.add(&lines[i].0, &lines[i].1, i as i64, 9).template_id)
            .collect()
    };

    let mut single = Drain::new(DrainConfig::default());
    let single_ids = mine_range(&mut single, 0..lines.len());
    let single_set: HashSet<u64> = single.take_dirty().iter().map(|c| c.id).collect();

    let mut first = Drain::new(DrainConfig::default());
    let mut split_ids = mine_range(&mut first, 0..half);
    let mut persisted: Vec<Cluster> = first.take_dirty();
    persisted.sort_by_key(|c| (c.first_seen_ns, c.id));
    let mut split_set: HashSet<u64> = persisted.iter().map(|c| c.id).collect();
    let mut restored = Drain::new(DrainConfig::default());
    for c in persisted {
        restored.restore(c);
    }
    split_ids.extend(mine_range(&mut restored, half..lines.len()));
    split_set.extend(restored.take_dirty().iter().map(|c| c.id));

    let differing = single_ids
        .iter()
        .zip(&split_ids)
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differing,
        0,
        "{differing} of {} lines got a different template id after restore",
        lines.len()
    );
    assert_eq!(
        single_set,
        split_set,
        "template ids differ: {} single-pass vs {} restored",
        single_set.len(),
        split_set.len()
    );
}
