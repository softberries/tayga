CREATE TABLE IF NOT EXISTS spans
(
    trace_id String,
    span_id String,
    parent_span_id String,
    service_name LowCardinality(String),
    span_name LowCardinality(String),
    kind Enum8('unspecified' = 0, 'internal' = 1, 'server' = 2, 'client' = 3, 'producer' = 4, 'consumer' = 5),
    start_ts DateTime64(9),
    duration_ns UInt64,
    status_code Enum8('unset' = 0, 'ok' = 1, 'error' = 2),
    status_message String,
    resource_attrs Map(LowCardinality(String), String),
    span_attrs Map(LowCardinality(String), String),
    events Nested(ts DateTime64(9), name LowCardinality(String), attrs Map(LowCardinality(String), String)),
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ReplacingMergeTree
PARTITION BY toDate(start_ts)
ORDER BY (service_name, toStartOfMinute(start_ts), trace_id, span_id)
TTL toDateTime(start_ts) + INTERVAL 3 DAY
SETTINGS ttl_only_drop_parts = 1;

CREATE TABLE IF NOT EXISTS logs
(
    log_id UInt64,
    ts DateTime64(9),
    observed_ts DateTime64(9),
    trace_id String,
    span_id String,
    severity_number UInt8,
    severity_text LowCardinality(String),
    service_name LowCardinality(String),
    body String,
    resource_attrs Map(LowCardinality(String), String),
    log_attrs Map(LowCardinality(String), String),
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.01) GRANULARITY 1
)
ENGINE = ReplacingMergeTree
PARTITION BY toDate(ts)
ORDER BY (service_name, toStartOfMinute(ts), trace_id, log_id)
TTL toDateTime(ts) + INTERVAL 3 DAY
SETTINGS ttl_only_drop_parts = 1
