CREATE TABLE IF NOT EXISTS metric_samples (
    ts DateTime64(3),
    job LowCardinality(String),
    metric LowCardinality(String),
    labels Map(LowCardinality(String), String),
    value Float64
) ENGINE = MergeTree
PARTITION BY toDate(ts)
ORDER BY (job, metric, ts)
TTL toDateTime(ts) + INTERVAL 7 DAY;
