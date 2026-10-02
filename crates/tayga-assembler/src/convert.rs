//! Analysis results → ClickHouse rows.

use tayga_analysis::story::{Story, StoryKind};
use tayga_analysis::summary::TraceSummary;
use tayga_store::rows::{StoryRow, TraceSummaryRow};

fn nanos(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

pub fn summary_row(s: &TraceSummary) -> TraceSummaryRow {
    TraceSummaryRow {
        trace_id: s.trace_id.clone(),
        ts: nanos(s.ts_ns),
        endpoint_service: s.endpoint.service.clone(),
        endpoint_name: s.endpoint.name.clone(),
        duration_ns: s.duration_ns,
        is_error: u8::from(s.is_error),
        op_durations: s.op_durations.clone(),
    }
}

pub fn story_row(s: &Story) -> serde_json::Result<StoryRow> {
    Ok(StoryRow {
        story_id: s.story_id.clone(),
        fingerprint: s.fingerprint,
        kind: match s.kind {
            StoryKind::Error => 1,
            StoryKind::Slow => 2,
        },
        ts: nanos(s.ts_ns),
        trace_id: s.trace_id.clone(),
        endpoint_service: s.endpoint.service.clone(),
        endpoint_name: s.endpoint.name.clone(),
        rc_service: s.root_cause.span.service.clone(),
        rc_span_name: s.root_cause.span.name.clone(),
        rc_span_kind: s.root_cause.span.kind.as_str().to_string(),
        rc_message: s.root_cause.message.clone(),
        rc_exception_type: s.root_cause.exception_type.clone(),
        summary: s.summary.clone(),
        duration_ns: s.duration_ns,
        path_services: s.path_services.clone(),
        path_spans: serde_json::to_string(&s.path_spans)?,
        critical_path: serde_json::to_string(&serde_json::json!({
            "segments": s.critical_path,
            "top": s.top_contributors,
        }))?,
        baseline_diff: match &s.baseline_diff {
            Some(d) => serde_json::to_string(d)?,
            None => String::new(),
        },
        logs: serde_json::to_string(&s.logs)?,
        also_failed: serde_json::to_string(&s.also_failed)?,
        span_count: s.span_count,
        flags: s.flags.clone(),
    })
}
