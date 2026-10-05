CREATE TABLE IF NOT EXISTS logminer_state (
    key String,
    value Int64,
    updated DateTime64(9)
) ENGINE = ReplacingMergeTree(updated)
ORDER BY key;
