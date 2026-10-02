//! Analysis of traces captured from the OTel demo with failure flags (see Task 13 of plan 2).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use tayga_analysis::baseline::{Baseline, Thresholds, baselines_from_summaries};
use tayga_analysis::model::{Endpoint, SpanKind, TraceBundle, trace_id_of};
use tayga_analysis::story::{Story, StoryKind, analyze};
use tayga_model::envelope::read_framed;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(format!("{name}.pb.gz"))
}

fn bundles(name: &str) -> Vec<TraceBundle> {
    let file =
        std::fs::File::open(fixture(name)).unwrap_or_else(|e| panic!("open fixture {name}: {e}"));
    let envelopes = read_framed(&mut flate2::read::GzDecoder::new(file)).unwrap();
    let mut by_trace: BTreeMap<String, TraceBundle> = BTreeMap::new();
    for env in &envelopes {
        let Some(id) = trace_id_of(env) else { continue };
        let id = id.to_hex();
        by_trace
            .entry(id.clone())
            .or_insert_with(|| TraceBundle::new(id))
            .add_envelope(env);
    }
    by_trace.into_values().collect()
}

fn stories(name: &str, baselines: &HashMap<Endpoint, Baseline>, t: &Thresholds) -> Vec<Story> {
    bundles(name)
        .iter()
        .filter_map(|b| analyze(b, baselines, t, &[])?.story)
        .collect()
}

fn describe(stories: &[Story]) -> String {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for s in stories {
        *counts
            .entry(format!("{} | {}", s.root_cause.span.service, s.summary))
            .or_default() += 1;
    }
    counts
        .iter()
        .map(|(k, v)| format!("{v:>4}  {k}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn payment_failure_blames_payment() {
    let all = stories("payment_failure", &HashMap::new(), &Thresholds::default());
    println!("payment_failure:\n{}", describe(&all));
    let via_payment: Vec<&Story> = all
        .iter()
        .filter(|s| s.path_services.iter().any(|x| x == "payment"))
        .collect();
    assert!(
        via_payment.len() >= 3,
        "too few stories through payment:\n{}",
        describe(&all)
    );
    assert!(
        via_payment
            .iter()
            .all(|s| s.root_cause.span.service == "payment"),
        "a story through payment blames another service:\n{}",
        describe(&all)
    );
}

#[test]
fn payment_unreachable_blames_the_checkout_client_call() {
    let all = stories(
        "payment_unreachable",
        &HashMap::new(),
        &Thresholds::default(),
    );
    println!("payment_unreachable:\n{}", describe(&all));
    let hits = all
        .iter()
        .filter(|s| {
            s.root_cause.span.service == "checkout"
                && s.root_cause.span.kind == SpanKind::Client
                && s.summary
                    .contains("could not reach oteldemo.PaymentService")
        })
        .count();
    assert!(
        hits >= 3,
        "expected checkout → payment connection failures:\n{}",
        describe(&all)
    );
}

#[test]
fn product_catalog_failure_blames_product_catalog() {
    let all = stories(
        "product_catalog_failure",
        &HashMap::new(),
        &Thresholds::default(),
    );
    println!("product_catalog_failure:\n{}", describe(&all));
    let hits = all
        .iter()
        .filter(|s| s.root_cause.span.service == "product-catalog")
        .count();
    assert!(
        hits >= 3,
        "expected product-catalog root causes:\n{}",
        describe(&all)
    );
}

#[test]
fn healthy_traffic_mostly_has_no_error_stories() {
    let traces = bundles("healthy");
    let errors = stories("healthy", &HashMap::new(), &Thresholds::default())
        .into_iter()
        .filter(|s| s.kind == StoryKind::Error)
        .collect::<Vec<_>>();
    println!("healthy ({} traces):\n{}", traces.len(), describe(&errors));
    assert!(
        traces.len() >= 50,
        "healthy fixture too small: {}",
        traces.len()
    );
    assert!(
        (errors.len() as f64) / (traces.len() as f64) < 0.25,
        "{} of {} healthy traces produced error stories:\n{}",
        errors.len(),
        traces.len(),
        describe(&errors)
    );
}

#[test]
fn shipping_slowdown_produces_slow_stories_blaming_shipping() {
    if !fixture("healthy_checkout").exists() || !fixture("shipping_slowdown").exists() {
        eprintln!("healthy_checkout or shipping_slowdown fixture absent; skipping");
        return;
    }
    let t = Thresholds {
        min_baseline_traces: 5,
        ..Thresholds::default()
    };
    let healthy: Vec<_> = bundles("healthy_checkout")
        .iter()
        .filter_map(|b| analyze(b, &HashMap::new(), &t, &[]))
        .map(|a| a.summary)
        .collect();
    let baselines = baselines_from_summaries(&healthy);
    let slow: Vec<Story> = stories("shipping_slowdown", &baselines, &t)
        .into_iter()
        .filter(|s| s.kind == StoryKind::Slow)
        .collect();
    println!("shipping_slowdown slow stories:\n{}", describe(&slow));
    assert!(
        slow.iter().any(|s| s.root_cause.span.service == "shipping"),
        "no slow story blames shipping:\n{}",
        describe(&slow)
    );
}
