CREATE TABLE IF NOT EXISTS log_template_silence (
    template_id UInt64,
    enabled UInt8,
    minutes UInt32,
    updated DateTime64(9)
) ENGINE = ReplacingMergeTree(updated)
ORDER BY template_id;

ALTER TABLE log_alerts MODIFY COLUMN kind Enum8('new' = 1, 'spike' = 2, 'silence' = 3);
