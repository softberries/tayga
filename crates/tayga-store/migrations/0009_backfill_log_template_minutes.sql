-- One-off backfill of log_template_minutes (0007) from the hits that predate its materialized view,
-- so the 1-day seasonal comparator works right after the upgrade (log_template_hits keeps 3 days).
-- Guard: only minutes up to and including the earliest minute the view has written. The view
-- started mid-minute, so that boundary minute is re-read in full. This overlap is harmless.
-- hits is a uniqExact state of log_id, and every reader merges with uniqExactMerge and GROUP BY,
-- so log_ids counted twice for one (template_id, minute) still count once, before and after
-- background merges. For the same reason a re-run (a crash before the version row is written)
-- only re-reads the then-earliest minute. An empty table (fresh install, or no hits since the view
-- was created) backfills everything, which is then nothing or all of the hits.
INSERT INTO log_template_minutes
SELECT template_id, toStartOfMinute(ts) AS minute, uniqExactState(log_id) AS hits
FROM log_template_hits
WHERE toStartOfMinute(ts) <= (SELECT min(minute) FROM log_template_minutes)
   OR (SELECT count() FROM log_template_minutes) = 0
GROUP BY template_id, minute
