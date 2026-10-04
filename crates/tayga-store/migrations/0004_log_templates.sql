CREATE TABLE IF NOT EXISTS log_templates (
    template_id UInt64,
    service LowCardinality(String),
    template String,
    first_seen DateTime64(9),
    last_seen DateTime64(9),
    count UInt64,
    max_severity UInt8,
    sample String,
    version UInt64
) ENGINE = ReplacingMergeTree(version)
ORDER BY template_id
TTL toDateTime(last_seen) + INTERVAL 30 DAY;

CREATE TABLE IF NOT EXISTS log_template_hits (
    log_id UInt64,
    template_id UInt64,
    service LowCardinality(String),
    ts DateTime64(9),
    severity_number UInt8,
    trace_id String,
    span_id String
) ENGINE = ReplacingMergeTree
PARTITION BY toDate(ts)
ORDER BY (template_id, ts, log_id)
TTL toDateTime(ts) + INTERVAL 3 DAY;

CREATE TABLE IF NOT EXISTS log_alerts (
    alert_id String,
    kind Enum8('new' = 1, 'spike' = 2),
    template_id UInt64,
    service LowCardinality(String),
    template String,
    started_at DateTime64(9),
    last_at DateTime64(9),
    window_count UInt64,
    peak_count UInt64,
    baseline_per_window Float64,
    example_trace_ids Array(String),
    version UInt64
) ENGINE = ReplacingMergeTree(version)
ORDER BY alert_id
TTL toDateTime(last_at) + INTERVAL 7 DAY;
