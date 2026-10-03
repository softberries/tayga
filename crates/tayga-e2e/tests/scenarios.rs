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
