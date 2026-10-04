//! Run with `make e2e` (stack up, flags at defaults). Each test flips one flag and restores it.
//! A scenario passes only when its group has new stories after the flip (see `wait_for_group`).

use serde_json::Value;
use tayga_e2e::*;

fn s(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

macro_rules! scenario {
    ($name:ident, $flag:literal, $variant:literal, $q:literal, $min:literal, $label:literal, $pred:expr) => {
        #[tokio::test]
        #[ignore = "end-to-end: requires `make up`"]
        async fn $name() -> anyhow::Result<()> {
            let api = Api::new(API);
            let flipped = now_ns();
            let _flag = FlagGuard::set($flag, $variant)?;
            let (_g, waited) =
                wait_for_group(&api, $q, flipped, $min, SCENARIO_TIMEOUT, $pred).await?;
            report($label, waited);
            Ok(())
        }
    };
}

#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn payment_failure_blames_payment() -> anyhow::Result<()> {
    let api = Api::new(API);
    let q = "kind=error&service=payment";
    let flipped = now_ns();
    let _flag = FlagGuard::set("paymentFailure", "100%")?;
    let (g, waited) = wait_for_group(&api, q, flipped, 3, SCENARIO_TIMEOUT, |g| {
        s(g, "rc_span_name").to_lowercase().contains("charge")
    })
    .await?;
    report("paymentFailure", waited);
    let story = api.story(&s(&g, "sample_story_id")).await?;
    let path: Vec<String> = story["path_services"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert!(
        path.contains(&"checkout".to_string()),
        "path {path:?} should go through checkout"
    );
    Ok(())
}

/// Spec §14: the root cause is the checkout client span calling payment.
#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn payment_unreachable_blames_checkout_client() -> anyhow::Result<()> {
    let api = Api::new(API);
    let flipped = now_ns();
    let _flag = FlagGuard::set("paymentUnreachable", "on")?;
    let (g, waited) = wait_for_group(
        &api,
        "kind=error&service=checkout",
        flipped,
        3,
        SCENARIO_TIMEOUT,
        |g| s(g, "summary").contains("could not reach oteldemo.PaymentService"),
    )
    .await?;
    report("paymentUnreachable", waited);
    let story = api.story(&s(&g, "sample_story_id")).await?;
    assert_eq!(
        story["root_cause"]["span_kind"], "client",
        "root cause: {}",
        story["root_cause"]
    );
    Ok(())
}

scenario!(
    product_catalog_failure_blames_product_catalog,
    "productCatalogFailure",
    "on",
    "kind=error&service=product-catalog",
    3,
    "productCatalogFailure",
    |g| s(g, "summary").contains("Product Catalog Fail Feature Flag Enabled")
);

scenario!(
    ad_failure_blames_ad,
    "adFailure",
    "on",
    "kind=error&service=ad",
    3,
    "adFailure",
    |g| s(g, "summary").contains("GetAds failed")
);

/// The flag only delays international orders, which are rare in the load generator's traffic,
/// so this waits up to `SHIPPING_TIMEOUT` (600 s) rather than `SCENARIO_TIMEOUT`. The sample
/// story must carry the injected 5 s delay, which rules out a spontaneous shipping slow story.
#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn shipping_slowdown_produces_slow_story_blaming_shipping() -> anyhow::Result<()> {
    let api = Api::new(API);
    let flipped = now_ns();
    let _flag = FlagGuard::set("intlShippingSlowdown", "5sec")?;
    let (g, waited) = wait_for_group(
        &api,
        "kind=slow&service=shipping",
        flipped,
        1,
        SHIPPING_TIMEOUT,
        |g| s(g, "rc_service") == "shipping",
    )
    .await?;
    report("intlShippingSlowdown", waited);
    let story = api.story(&s(&g, "sample_story_id")).await?;
    let duration_ns = story["duration_ns"].as_u64().unwrap_or(0);
    assert!(
        duration_ns >= 4_500_000_000,
        "sample story lasted {duration_ns} ns; the flag adds 5 s"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn service_map_has_frontend_to_checkout() -> anyhow::Result<()> {
    let edges = Api::new(API).service_map("1h").await?;
    assert!(
        edges
            .iter()
            .any(|e| s(e, "parent") == "frontend" && s(e, "child") == "checkout"),
        "edges: {:?}",
        edges
            .iter()
            .map(|e| format!("{}→{}", s(e, "parent"), s(e, "child")))
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn raw_span_counts_match_jaeger() -> anyhow::Result<()> {
    let (checked, mismatches) = tayga_devtools::verify::verify_raw(
        "http://localhost:18123",
        "http://localhost:8080/jaeger/ui",
        20,
    )
    .await?;
    assert!(checked > 0, "no traces sampled");
    assert!(mismatches.is_empty(), "mismatches: {mismatches:?}");
    Ok(())
}

/// A 100% payment failure makes the "Payment request failed" template spike. The template is
/// older than the 65-minute spike-age rule, so it is reported as a spike, not as new.
/// Detection runs every 60 s, so the wait is long.
#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn log_spike_on_payment_failure() -> anyhow::Result<()> {
    let api = Api::new(API);
    let query = "kind=spike&service=payment&since=1h";
    // An alert still open from an earlier run would only be updated by the logminer, never
    // started anew, so the scenario could not tell whether the flag had any effect.
    let open = api
        .log_alerts(query)
        .await?
        .into_iter()
        .any(|a| s(&a, "template").contains("Payment request failed") && a["active"] == true);
    anyhow::ensure!(
        !open,
        "a payment spike alert is still active from an earlier run; wait ~10 minutes for it to lapse and re-run"
    );
    let flipped = now_ns();
    let _flag = FlagGuard::set("paymentFailure", "100%")?;
    let (alert, waited) = wait_for_alert(&api, query, flipped, LOG_SPIKE_TIMEOUT, |a| {
        s(a, "template").contains("Payment request failed")
            && a["started_at_ns"].as_i64().is_some_and(|n| n > flipped)
    })
    .await?;
    println!("[e2e] log_spike: alert after {:.0}s", waited.as_secs_f64());
    let traces = alert["example_traces"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        traces.iter().any(|t| !t["story_id"].is_null()),
        "no example trace links to a story: {traces:?}"
    );
    Ok(())
}

/// Emits a log with a per-run random first word, so Drain creates a fresh template every run
/// (`<word> <*> marker`; the `e2e` token contains a digit and is masked). The checkout service
/// is older than 15 min, so the template is reported as a new-template alert. Each run leaves
/// one probe template in checkout's template set (spec section 12.7).
#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn new_template_from_probe() -> anyhow::Result<()> {
    let api = Api::new(API);
    let word = tayga_devtools::emit::random_word(12);
    let trace = tayga_devtools::emit::random_trace_id();
    let trace_hex: String = trace.iter().map(|b| format!("{b:02x}")).collect();
    let flipped = now_ns();
    tayga_devtools::emit::emit_log(
        "http://localhost:14318",
        "checkout",
        &format!("{word} tayga-e2e-probe marker"),
        &trace,
        9,
    )
    .await?;
    let (alert, waited) = wait_for_alert(
        &api,
        "kind=new&service=checkout&since=1h",
        flipped,
        NEW_TEMPLATE_TIMEOUT,
        |a| s(a, "template").starts_with(&word),
    )
    .await?;
    println!(
        "[e2e] new_template: alert after {:.0}s",
        waited.as_secs_f64()
    );
    let traces = alert["example_traces"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        traces.iter().any(|t| s(t, "trace_id") == trace_hex),
        "example_traces {traces:?} should contain {trace_hex}"
    );
    Ok(())
}
