//! Run with `make e2e` (stack up, flags at defaults). Each test flips one flag and restores it.

use serde_json::Value;
use tayga_e2e::*;

fn s(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn payment_failure_blames_payment() -> anyhow::Result<()> {
    let api = Api::new(API);
    let flipped = now_ns();
    let _flag = FlagGuard::set("paymentFailure", "100%")?;
    let (g, waited) = wait_for_group(
        &api,
        "since=10m&kind=error&service=payment",
        flipped,
        SCENARIO_TIMEOUT,
        |g| s(g, "rc_span_name").to_lowercase().contains("charge"),
    )
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

#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn payment_unreachable_blames_checkout_client() -> anyhow::Result<()> {
    let api = Api::new(API);
    let flipped = now_ns();
    let _flag = FlagGuard::set("paymentUnreachable", "on")?;
    let (_g, waited) = wait_for_group(
        &api,
        "since=10m&kind=error&service=checkout",
        flipped,
        SCENARIO_TIMEOUT,
        |g| s(g, "summary").contains("could not reach oteldemo.PaymentService"),
    )
    .await?;
    report("paymentUnreachable", waited);
    Ok(())
}

#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn product_catalog_failure_blames_product_catalog() -> anyhow::Result<()> {
    let api = Api::new(API);
    let flipped = now_ns();
    let _flag = FlagGuard::set("productCatalogFailure", "on")?;
    let (_g, waited) = wait_for_group(
        &api,
        "since=10m&kind=error&service=product-catalog",
        flipped,
        SCENARIO_TIMEOUT,
        |_| true,
    )
    .await?;
    report("productCatalogFailure", waited);
    Ok(())
}

#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn ad_failure_blames_ad() -> anyhow::Result<()> {
    let api = Api::new(API);
    let flipped = now_ns();
    let _flag = FlagGuard::set("adFailure", "on")?;
    let (_g, waited) = wait_for_group(
        &api,
        "since=10m&kind=error&service=ad",
        flipped,
        SCENARIO_TIMEOUT,
        |_| true,
    )
    .await?;
    report("adFailure", waited);
    Ok(())
}

#[tokio::test]
#[ignore = "end-to-end: requires `make up` and ≥20 min of healthy traffic for checkout baselines"]
async fn shipping_slowdown_produces_slow_story_blaming_shipping() -> anyhow::Result<()> {
    let api = Api::new(API);
    let flipped = now_ns();
    let _flag = FlagGuard::set("intlShippingSlowdown", "5sec")?;
    let (_g, waited) = wait_for_group(
        &api,
        "since=10m&kind=slow&service=shipping",
        flipped,
        SCENARIO_TIMEOUT,
        |_| true,
    )
    .await?;
    report("intlShippingSlowdown", waited);
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
