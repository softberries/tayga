//! Run with `make e2e` (stack up, flags at defaults). Each test flips one flag and restores it.
//! A scenario passes only when its group has new stories after the flip (see `wait_for_group`).

use serde_json::Value;
use std::time::{Duration, Instant};
use tayga_e2e::*;

fn s(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

macro_rules! scenario {
    ($name:ident, $flag:literal, $variant:literal, $q:literal, $min:literal, $timeout:expr, $label:literal, $pred:expr) => {
        #[tokio::test]
        #[ignore = "end-to-end: requires `make up`"]
        async fn $name() -> anyhow::Result<()> {
            let api = Api::new(API);
            let flipped = now_ns();
            let _flag = FlagGuard::set($flag, $variant)?;
            let (_g, waited) = wait_for_group(&api, $q, flipped, $min, $timeout, $pred).await?;
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
    // The stories split across one group per checkout endpoint, so the count is summed.
    let (g, waited) = wait_for_group_sum(
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
    SCENARIO_TIMEOUT,
    "productCatalogFailure",
    |g| s(g, "summary").contains("Product Catalog Fail Feature Flag Enabled")
);

/// The stories split across one group per calling endpoint (`frontend-web GET /api/data`,
/// `load-generator user_get_ads`, ...), so the count is summed over the GetAds groups.
#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn ad_failure_blames_ad() -> anyhow::Result<()> {
    let api = Api::new(API);
    let flipped = now_ns();
    let _flag = FlagGuard::set("adFailure", "on")?;
    let (_g, waited) = wait_for_group_sum(
        &api,
        "kind=error&service=ad",
        flipped,
        3,
        AD_FAILURE_TIMEOUT,
        |g| s(g, "summary").contains("GetAds failed"),
    )
    .await?;
    report("adFailure", waited);
    Ok(())
}

/// Places its own international orders (one every `ORDER_EVERY`) instead of waiting for the
/// load generator's rare ones; the baseline pre-check still fails fast when no checkout endpoint
/// can flag a 5 s trace. The group's sample story is its latest, which may be a spontaneous
/// sub-second one, so the wait runs until an example story after the flip carries the injected
/// 5 s delay.
#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn shipping_slowdown_produces_slow_story_blaming_shipping() -> anyhow::Result<()> {
    ensure_checkout_baseline_detects("http://localhost:18123", 5.0).await?;
    let api = Api::new(API);
    let flipped = now_ns();
    let _flag = FlagGuard::set("intlShippingSlowdown", "5sec")?;
    // Its own international orders; the 5 s delay needs a client timeout above it.
    let orders = tokio::spawn(async {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("client builds");
        loop {
            match place_intl_order(&http, FRONTEND).await {
                Ok(trace) => println!("[e2e] intlShippingSlowdown: order placed, trace {trace}"),
                Err(e) => eprintln!("[e2e] order failed (continuing): {e:#}"),
            }
            tokio::time::sleep(ORDER_EVERY).await;
        }
    });
    let found = wait_for_slow_story(
        &api,
        "kind=slow&service=shipping",
        flipped,
        4_500_000_000,
        SHIPPING_TIMEOUT,
        |g| s(g, "rc_service") == "shipping",
    )
    .await;
    orders.abort();
    let (g, story_id, waited) = found?;
    report("intlShippingSlowdown", waited);
    println!(
        "[e2e] intlShippingSlowdown: delayed story {story_id} in group {}",
        s(&g, "fingerprint")
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

/// Emits a probe log under the dedicated `tayga-e2e-probe` service, never a demo service:
/// `{word} probe … probe marker` with a random 12-letter `word` and 2 to 56 `probe`s (spec
/// §12.7). Drain routes on the token count, then on `word`, so each run adds one child to one of
/// ~55 length nodes; that is ~4,000 runs (simulated: first node full at 4,000–4,650) within the 30-day template TTL before a node fills and
/// probes start merging. The new-template rule needs the service to have had a template for
/// 15 min, so every run also emits the constant seed `tayga e2e probe seed`, and the first run
/// waits up to 16 min for that seed to age.
#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn new_template_from_probe() -> anyhow::Result<()> {
    use tayga_devtools::emit::{
        PROBE_SEED, PROBE_SERVICE, emit_log, probe_body, random_probe_repeats, random_trace_id,
        random_word,
    };
    const INGEST: &str = "http://localhost:14318";
    let api = Api::new(API);
    emit_log(INGEST, PROBE_SERVICE, PROBE_SEED, &random_trace_id(), 9).await?;
    let warmed = wait_for_service_warmup(&api, PROBE_SERVICE, PROBE_WARMUP_TIMEOUT).await?;
    println!(
        "[e2e] new_template: probe service warm after {:.0}s",
        warmed.as_secs_f64()
    );

    let word = random_word(12);
    let body = probe_body(&word, random_probe_repeats());
    let trace = random_trace_id();
    let trace_hex: String = trace.iter().map(|b| format!("{b:02x}")).collect();
    let flipped = now_ns();
    emit_log(INGEST, PROBE_SERVICE, &body, &trace, 9).await?;
    let (alert, waited) = wait_for_alert(
        &api,
        &format!("kind=new&service={PROBE_SERVICE}&since=1h"),
        flipped,
        NEW_TEMPLATE_TIMEOUT,
        |a| s(a, "template") == body,
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

/// Spec §5 (plan 7b): two fresh probe templates A and B; silence on A with `SILENCE_MINUTES`;
/// B keeps the probe service logging every `SILENCE_KEEPALIVE` while A stays quiet, so A goes
/// silent in log time. Each run adds two probe templates (see `new_template_from_probe` for the
/// Drain capacity this uses). With `TAYGA_E2E_NOTIFIER=1` (`make e2e-notifier`, which points
/// the notifier at a mock on the host) it also checks that the alert is delivered exactly once,
/// and still once after `docker restart tayga-notifier`. Silence on A is switched off at the end.
#[tokio::test]
#[ignore = "end-to-end: requires `make up`"]
async fn silence_alert_and_delivery() -> anyhow::Result<()> {
    use std::net::SocketAddr;
    use tayga_devtools::emit::{
        PROBE_REPEATS, PROBE_SERVICE, emit_log, probe_body, random_probe_repeats, random_trace_id,
        random_word,
    };
    const INGEST: &str = "http://localhost:14318";
    let api = Api::new(API);
    let notifier_check = std::env::var(NOTIFIER_CHECK_ENV).is_ok_and(|v| v == "1");
    // Started first, so the notifier never meets a closed port for the alert under test.
    let mock = if notifier_check {
        Some(MockWebhook::start(SocketAddr::from(([0, 0, 0, 0], NOTIFIER_MOCK_PORT)), &[]).await?)
    } else {
        None
    };

    let (word_a, word_b) = (random_word(12), random_word(12));
    let repeats_a = random_probe_repeats();
    // A different token count for B keeps the two in different Drain length nodes.
    let repeats_b = if repeats_a == *PROBE_REPEATS.end() {
        repeats_a - 1
    } else {
        repeats_a + 1
    };
    let (body_a, body_b) = (
        probe_body(&word_a, repeats_a),
        probe_body(&word_b, repeats_b),
    );
    let started = Instant::now();
    emit_log(INGEST, PROBE_SERVICE, &body_a, &random_trace_id(), 9).await?;
    emit_log(INGEST, PROBE_SERVICE, &body_b, &random_trace_id(), 9).await?;
    let mut last_b = Instant::now();
    let id_a = wait_for_template(&api, PROBE_SERVICE, &word_a, &body_a, TEMPLATE_TIMEOUT).await?;
    wait_for_template(&api, PROBE_SERVICE, &word_b, &body_b, TEMPLATE_TIMEOUT).await?;
    println!(
        "[e2e] silence: templates A={id_a} and B after {:.0}s",
        started.elapsed().as_secs_f64()
    );

    let enabled_at = now_ns();
    api.put_silence(&id_a, true, SILENCE_MINUTES).await?;
    let outcome = async {
        let since_enable = Instant::now();
        let query = format!("kind=silence&service={PROBE_SERVICE}&since=1h");
        let alert = loop {
            if last_b.elapsed() >= SILENCE_KEEPALIVE {
                emit_log(INGEST, PROBE_SERVICE, &body_b, &random_trace_id(), 9).await?;
                last_b = Instant::now();
            }
            match api.log_alerts(&query).await {
                Ok(alerts) => {
                    if let Some(a) = alerts.into_iter().find(|a| {
                        s(a, "template_id") == id_a
                            && a["last_at_ns"].as_i64().is_some_and(|n| n > enabled_at)
                    }) {
                        break a;
                    }
                }
                Err(e) => eprintln!("[e2e] poll error (continuing): {e}"),
            }
            anyhow::ensure!(
                since_enable.elapsed() < SILENCE_TIMEOUT,
                "no silence alert for template {id_a} within {SILENCE_TIMEOUT:?}"
            );
            tokio::time::sleep(POLL_EVERY).await;
        };
        let quiet_s = (s_i64(&alert, "last_at_ns") - s_i64(&alert, "started_at_ns")) / 1_000_000_000;
        println!(
            "[e2e] silence: alert {} after {:.0}s from enabling (started_at to last_at {quiet_s}s, {:.0}s from the first emit)",
            s(&alert, "alert_id"),
            since_enable.elapsed().as_secs_f64(),
            started.elapsed().as_secs_f64()
        );
        anyhow::ensure!(
            quiet_s >= i64::from(SILENCE_MINUTES) * 60,
            "a silence alert's started_at is the last hit, so last_at - started_at >= minutes: {alert}"
        );
        if let Some(mock) = &mock {
            check_single_delivery(mock, &s(&alert, "alert_id"), &id_a).await?;
        }
        anyhow::Ok(())
    }
    .await;
    // Switch silence off, also after a failure, so A's alert lapses instead of being refreshed.
    let reset = api.put_silence(&id_a, false, SILENCE_MINUTES).await;
    outcome?;
    reset?;
    Ok(())
}

fn s_i64(v: &Value, key: &str) -> i64 {
    v[key].as_i64().unwrap_or_default()
}

/// The mock gets exactly one delivery for `alert_id`, and still one after a notifier restart
/// during which the logminer keeps re-publishing the alert.
async fn check_single_delivery(
    mock: &MockWebhook,
    alert_id: &str,
    template_id: &str,
) -> anyhow::Result<()> {
    let waiting = Instant::now();
    let first = loop {
        if let Some(r) = deliveries_of(&mock.received(), alert_id).first() {
            break (*r).clone();
        }
        anyhow::ensure!(
            waiting.elapsed() < DELIVERY_TIMEOUT,
            "no delivery of {alert_id} within {DELIVERY_TIMEOUT:?}; the mock got {} requests",
            mock.received().len()
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    println!(
        "[e2e] notifier: delivered after {:.0}s: {}",
        waiting.elapsed().as_secs_f64(),
        first.body
    );
    anyhow::ensure!(
        first.content_type.as_deref() == Some("application/json"),
        "content type {:?}",
        first.content_type
    );
    anyhow::ensure!(
        first.body["kind"] == "silence"
            && first.body["template_id"] == template_id
            && s(&first.body, "summary").contains("has been silent for")
            && first.body["last_at"].is_string(),
        "unexpected payload {}",
        first.body
    );

    let restart = std::process::Command::new("docker")
        .args(["restart", "tayga-notifier"])
        .output()?;
    anyhow::ensure!(
        restart.status.success(),
        "docker restart tayga-notifier: {}",
        String::from_utf8_lossy(&restart.stderr)
    );
    println!("[e2e] notifier: restarted; watching {RESEND_WATCH:?} for a resend");
    tokio::time::sleep(RESEND_WATCH).await;
    let got = mock.received();
    let n = deliveries_of(&got, alert_id).len();
    println!(
        "[e2e] notifier: {n} delivery of {alert_id} ({} requests in all)",
        got.len()
    );
    anyhow::ensure!(n == 1, "{alert_id} was delivered {n} times");
    Ok(())
}
