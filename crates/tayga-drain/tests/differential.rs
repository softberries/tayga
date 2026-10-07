//! The fingerprint cache never changes what Drain assigns (sub-project 4 spec §4).

mod corpus;

use proptest::prelude::*;
use tayga_drain::drain::{Cluster, Drain, DrainConfig, Lookup};
use tayga_drain::fingerprint::{Fingerprint, fingerprint_body, reference_fingerprint};

/// One line: service, body, timestamp, severity.
type Line = (String, String, i64, u8);

/// Feeds `lines` to a Drain through `add` and to another through `add_fingerprinted` with
/// `fp` of each body; every assignment and every final cluster must be equal. Returns how many
/// lines were cache hits.
fn compare(cfg: &DrainConfig, lines: &[Line], fp: impl Fn(&str) -> Option<Fingerprint>) -> usize {
    let mut plain = Drain::new(cfg.clone());
    let mut cached = Drain::new(cfg.clone());
    let mut hits = 0;
    for (i, (service, body, ts, sev)) in lines.iter().enumerate() {
        let a = plain.add(service, body, *ts, *sev);
        let b = cached.add_fingerprinted(service, body, fp(body), *ts, *sev);
        assert_eq!(
            (a.template_id, a.created, a.overflow),
            (b.template_id, b.created, b.overflow),
            "line {i} {service} {body:?} ({:?})",
            b.lookup
        );
        hits += usize::from(b.lookup == Lookup::Hit);
    }
    assert_eq!(sorted(plain.take_dirty()), sorted(cached.take_dirty()));
    hits
}

/// Both Drains mine `warm` (one through the cache), then restore the templates a third Drain
/// mined from `other`, then mine `last`: a restore after lines are cached must not change any
/// assignment.
fn compare_warm_restore(cfg: &DrainConfig, warm: &[Line], other: &[Line], last: &[Line]) {
    let keep = cfg.keep_http_status;
    let mut seed = Drain::new(cfg.clone());
    for (s, b, ts, sev) in other {
        seed.add(s, b, *ts, *sev);
    }
    let mut stored = seed.take_dirty();
    stored.sort_by_key(|c| (c.first_seen_ns, c.id));
    let (mut plain, mut cached) = (Drain::new(cfg.clone()), Drain::new(cfg.clone()));
    let step = |lines: &[Line], plain: &mut Drain, cached: &mut Drain| {
        for (i, (s, b, ts, sev)) in lines.iter().enumerate() {
            let a = plain.add(s, b, *ts, *sev);
            let fp = fingerprint_body(b.as_bytes(), keep);
            let c = cached.add_fingerprinted(s, b, fp, *ts, *sev);
            assert_eq!(
                (a.template_id, a.created, a.overflow),
                (c.template_id, c.created, c.overflow),
                "line {i} {s} {b:?} ({:?})",
                c.lookup
            );
        }
    };
    step(warm, &mut plain, &mut cached);
    for c in &stored {
        plain.restore(c.clone());
        cached.restore(c.clone());
    }
    step(last, &mut plain, &mut cached);
    assert_eq!(sorted(plain.take_dirty()), sorted(cached.take_dirty()));
}

fn sorted(mut v: Vec<Cluster>) -> Vec<Cluster> {
    v.sort_by_key(|c| c.id);
    v
}

fn corpus_lines() -> Vec<Line> {
    corpus::load()
        .into_iter()
        .map(|l| (l.service, l.body, l.ts_ns, l.sev))
        .collect()
}

fn configs() -> Vec<DrainConfig> {
    let d = DrainConfig::default();
    vec![
        d.clone(),
        DrainConfig {
            max_clusters_per_service: 20,
            ..d.clone()
        },
        DrainConfig {
            max_children: 2,
            ..d.clone()
        },
        DrainConfig {
            sim_threshold: 0.75,
            ..d.clone()
        },
        DrainConfig {
            keep_http_status: false,
            ..d
        },
    ]
}

#[test]
fn the_cache_assigns_exactly_like_drain_on_the_corpus() {
    let lines = corpus_lines();
    for cfg in configs() {
        let keep = cfg.keep_http_status;
        let hits = compare(&cfg, &lines, |b| fingerprint_body(b.as_bytes(), keep));
        assert!(
            hits * 100 >= lines.len() * 95,
            "{cfg:?}: {hits} hits of {}",
            lines.len()
        );
    }
}

#[test]
fn a_restored_drain_with_the_cache_assigns_like_one_without() {
    let lines = corpus_lines();
    let (first, second) = lines.split_at(lines.len() / 2);
    let mut seed = Drain::new(DrainConfig::default());
    for (s, b, ts, sev) in first {
        seed.add(s, b, *ts, *sev);
    }
    let mut stored = seed.take_dirty();
    stored.sort_by_key(|c| (c.first_seen_ns, c.id));
    let restored = || {
        let mut d = Drain::new(DrainConfig::default());
        for c in &stored {
            d.restore(c.clone());
        }
        d
    };
    let (mut plain, mut cached) = (restored(), restored());
    for (s, b, ts, sev) in second {
        let a = plain.add(s, b, *ts, *sev);
        let c = cached.add_fingerprinted(s, b, fingerprint_body(b.as_bytes(), true), *ts, *sev);
        assert_eq!(
            (a.template_id, a.created, a.overflow),
            (c.template_id, c.created, c.overflow)
        );
    }
    assert_eq!(sorted(plain.take_dirty()), sorted(cached.take_dirty()));
}

/// Every body gets the same key; only `check` tells sequences apart. Assignments must not
/// change, and the collisions must be seen.
#[test]
fn colliding_keys_are_caught_by_the_check() {
    let lines: Vec<Line> = corpus_lines().into_iter().take(5_000).collect();
    let mut cached = Drain::new(DrainConfig::default());
    let mut collisions = 0;
    for (s, b, ts, sev) in &lines {
        let fp = Fingerprint {
            key: 1,
            check: reference_fingerprint(b, true).check,
        };
        collisions += usize::from(
            cached.add_fingerprinted(s, b, Some(fp), *ts, *sev).lookup == Lookup::Collision,
        );
    }
    assert!(collisions > 0);
    compare(&DrainConfig::default(), &lines, |b| {
        Some(Fingerprint {
            key: 1,
            check: reference_fingerprint(b, true).check,
        })
    });
}

/// Bodies from a few words, so templates generalise, collide on length and overflow often.
fn small_body() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just("a"),
            Just("b"),
            Just("c"),
            Just("x1"),
            Just("y2"),
            Just("HTTP/1.1"),
            Just("503"),
            Just("200"),
            Just("deadbeefcafe"),
            Just("\u{e9}t\u{e9}")
        ],
        1..6,
    )
    .prop_map(|v| v.join(" "))
}

proptest! {
    #[test]
    fn the_cache_assigns_exactly_like_drain_on_random_logs(
        raw in prop::collection::vec((0..2u8, small_body(), 0..1_000i64, 0..24u8), 1..300),
        max_children in 1usize..4,
        cap in 1usize..12,
        keep in any::<bool>(),
        sim in prop_oneof![Just(0.0), Just(0.5), Just(1.0), 0.0..=1.0],
    ) {
        let cfg = DrainConfig {
            sim_threshold: sim,
            max_children,
            max_clusters_per_service: cap,
            keep_http_status: keep,
            ..DrainConfig::default()
        };
        let lines: Vec<Line> = raw
            .into_iter()
            .map(|(s, b, ts, sev)| (format!("svc{s}"), b, ts, sev))
            .collect();
        compare(&cfg, &lines, |b| fingerprint_body(b.as_bytes(), keep));
    }

    /// Cache, restore, then compare (the restore clears the service's cache).
    #[test]
    fn a_restore_after_cached_lines_assigns_like_one_without(
        warm in prop::collection::vec((0..2u8, small_body(), 0..1_000i64, 0..24u8), 1..150),
        other in prop::collection::vec((0..2u8, small_body(), 0..1_000i64, 0..24u8), 1..150),
        last in prop::collection::vec((0..2u8, small_body(), 0..1_000i64, 0..24u8), 1..150),
        max_children in 1usize..4,
        cap in 1usize..12,
        keep in any::<bool>(),
        sim in prop_oneof![Just(0.0), Just(0.5), Just(1.0), 0.0..=1.0],
    ) {
        let cfg = DrainConfig {
            sim_threshold: sim,
            max_children,
            max_clusters_per_service: cap,
            keep_http_status: keep,
            ..DrainConfig::default()
        };
        let lines = |raw: Vec<(u8, String, i64, u8)>| -> Vec<Line> {
            raw.into_iter()
                .map(|(s, b, ts, sev)| (format!("svc{s}"), b, ts, sev))
                .collect()
        };
        compare_warm_restore(&cfg, &lines(warm), &lines(other), &lines(last));
    }
}
