CREATE TABLE IF NOT EXISTS notifier_deliveries (
    alert_id String,
    target String,
    status Enum8('pending' = 1, 'delivered' = 2, 'failed' = 3),
    attempts UInt32,
    last_error String,
    updated DateTime64(9)
) ENGINE = ReplacingMergeTree(updated)
ORDER BY (alert_id, target)
TTL toDateTime(updated) + INTERVAL 30 DAY;
