ALTER TABLE log_alerts ADD COLUMN IF NOT EXISTS baseline_day Nullable(Float64), ADD COLUMN IF NOT EXISTS baseline_week Nullable(Float64);
