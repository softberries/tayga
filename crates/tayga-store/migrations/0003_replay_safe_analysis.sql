-- Re-assembling a trace (restart, rebalance, late data) can pick another root, which changes
-- ts, endpoint and fingerprint. The 0002 sort keys contained those columns, so replays could
-- not be merged. Key on the identity alone and keep the most complete version (most spans).
-- The old tables are kept (renamed) for inspection.
RENAME TABLE trace_summaries TO trace_summaries_v1, error_stories TO error_stories_v1;

CREATE TABLE IF NOT EXISTS trace_summaries
(
    trace_id String,
    ts DateTime64(9),
    endpoint_service LowCardinality(String),
    endpoint_name LowCardinality(String),
    duration_ns UInt64,
    is_error UInt8,
    op_durations Map(String, UInt64),
    span_count UInt32
)
ENGINE = ReplacingMergeTree(span_count)
ORDER BY trace_id
TTL toDateTime(ts) + INTERVAL 2 DAY;

CREATE TABLE IF NOT EXISTS error_stories
(
    story_id String,
    fingerprint UInt64,
    kind Enum8('error' = 1, 'slow' = 2),
    ts DateTime64(9),
    trace_id String,
    endpoint_service LowCardinality(String),
    endpoint_name LowCardinality(String),
    rc_service LowCardinality(String),
    rc_span_name LowCardinality(String),
    rc_span_kind LowCardinality(String),
    rc_message String,
    rc_exception_type String,
    summary String,
    duration_ns UInt64,
    path_services Array(String),
    path_spans String,
    critical_path String,
    baseline_diff String,
    logs String,
    also_failed String,
    span_count UInt32,
    flags Array(LowCardinality(String)),
    rc_span_id String,
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_fingerprint fingerprint TYPE set(1000) GRANULARITY 1,
    INDEX idx_ts ts TYPE minmax GRANULARITY 1
)
ENGINE = ReplacingMergeTree(span_count)
ORDER BY story_id
TTL toDateTime(ts) + INTERVAL 7 DAY
