CREATE TABLE IF NOT EXISTS log_template_minutes (
    template_id UInt64,
    minute DateTime,
    hits AggregateFunction(uniqExact, UInt64)
) ENGINE = AggregatingMergeTree
PARTITION BY toDate(minute)
ORDER BY (template_id, minute)
TTL minute + INTERVAL 8 DAY;

CREATE MATERIALIZED VIEW IF NOT EXISTS log_template_minutes_mv TO log_template_minutes AS
SELECT template_id, toStartOfMinute(ts) AS minute, uniqExactState(log_id) AS hits
FROM log_template_hits GROUP BY template_id, minute;
