CREATE TABLE IF NOT EXISTS trace_summaries
(
    trace_id String,
    ts DateTime64(9),
    endpoint_service LowCardinality(String),
    endpoint_name LowCardinality(String),
    duration_ns UInt64,
    is_error UInt8,
    op_durations Map(String, UInt64)
)
ENGINE = ReplacingMergeTree
PARTITION BY toDate(ts)
ORDER BY (endpoint_service, endpoint_name, trace_id)
TTL toDateTime(ts) + INTERVAL 2 DAY
SETTINGS ttl_only_drop_parts = 1;

CREATE TABLE IF NOT EXISTS service_edges
(
    minute DateTime,
    parent_service LowCardinality(String),
    child_service LowCardinality(String),
    calls UInt64,
    errors UInt64,
    duration_ns_sum UInt64
)
ENGINE = SummingMergeTree
PARTITION BY toDate(minute)
ORDER BY (minute, parent_service, child_service)
TTL minute + INTERVAL 7 DAY
SETTINGS ttl_only_drop_parts = 1;

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
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ReplacingMergeTree
PARTITION BY toDate(ts)
ORDER BY (fingerprint, ts, story_id)
TTL toDateTime(ts) + INTERVAL 7 DAY
SETTINGS ttl_only_drop_parts = 1
