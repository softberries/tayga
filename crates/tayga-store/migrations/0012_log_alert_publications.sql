CREATE TABLE IF NOT EXISTS log_alert_publications (
    alert_id String,
    published_at DateTime64(9)
) ENGINE = ReplacingMergeTree(published_at)
ORDER BY alert_id
TTL toDateTime(published_at) + INTERVAL 7 DAY;

INSERT INTO log_alert_publications (alert_id, published_at)
SELECT alert_id, now64(9) FROM log_alerts FINAL
