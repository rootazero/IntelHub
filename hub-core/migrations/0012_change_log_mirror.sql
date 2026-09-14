-- 2026-09-14: graph_change_log replay-worker progress marker.
-- The mirror worker (graphw::run_change_log_mirror) reads rows where
-- mirrored_at IS NULL, translates to v1 op shapes, enqueues into
-- graph_sync_queue, then stamps mirrored_at = now(). Idempotent on
-- re-run because the downstream v1 ops themselves are MERGE-based.
ALTER TABLE graph_change_log
    ADD COLUMN IF NOT EXISTS mirrored_at timestamptz NULL;
CREATE INDEX IF NOT EXISTS graph_change_log_unmirrored_idx
    ON graph_change_log (change_id)
    WHERE mirrored_at IS NULL;
