use tayga_api::params::GroupFilter;
use tayga_api::repo::{ChRepo, Repo};
use tayga_store::ClickHouseSettings;

fn repo() -> ChRepo {
    let url =
        std::env::var("TAYGA_IT_CLICKHOUSE").unwrap_or_else(|_| "http://localhost:18123".into());
    ChRepo::new(&ClickHouseSettings {
        url,
        database: "tayga".into(),
    })
}

#[tokio::test]
#[ignore = "requires the live stack's ClickHouse with assembler data"]
async fn reads_live_groups_story_trace_and_map() {
    let r = repo();
    let groups = r
        .story_groups(&GroupFilter {
            since_secs: 7 * 86_400,
            kind: None,
            service: None,
        })
        .await
        .unwrap();
    assert!(!groups.is_empty(), "no story groups in the last 7 days");
    let g = &groups[0];
    assert!(g.group.fingerprint.parse::<u64>().is_ok());
    assert!(g.group.stories >= 1 && !g.per_minute.is_empty());
    let detail = r
        .story_group(&g.group.fingerprint, 7 * 86_400)
        .await
        .unwrap()
        .expect("group exists");
    assert!(!detail.examples.is_empty());
    let story = r
        .story(&g.group.sample_story_id)
        .await
        .unwrap()
        .expect("story exists");
    assert_eq!(story.fingerprint, g.group.fingerprint);
    let trace = r.trace(&story.trace_id).await.unwrap();
    assert!(!trace.spans.is_empty());
    let ids: std::collections::HashSet<_> =
        trace.spans.iter().map(|s| s.span_id.as_str()).collect();
    assert_eq!(ids.len(), trace.spans.len(), "spans deduplicated");
    assert!(r.story(&"0".repeat(32)).await.unwrap().is_none());
    assert!(!r.service_map(3600).await.unwrap().is_empty());
}
