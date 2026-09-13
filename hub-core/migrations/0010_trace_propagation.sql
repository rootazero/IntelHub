-- 0010_trace_propagation.sql — D: trace_id propagation for observability.
-- Originally created as 0009_trace_propagation.sql; bumped to 0010 after
-- SP9 took the 0009 slot with observations_write_path.sql. Idempotent
-- (uses IF NOT EXISTS) so the rename is safe.
--
-- Each MCP tool call generates a UUID `trace_id` (caller may supply their
-- own to chain across calls). Propagated to cost_records + embedding_jobs
-- so /api/v1/traces/{trace_id} can walk the full call → embed → write
-- graph for one MCP request or one investigation step.

ALTER TABLE cost_records
  ADD COLUMN IF NOT EXISTS trace_id UUID;

CREATE INDEX IF NOT EXISTS cost_records_trace_idx
  ON cost_records (trace_id, created_at DESC)
  WHERE trace_id IS NOT NULL;

ALTER TABLE embedding_jobs
  ADD COLUMN IF NOT EXISTS trace_id UUID;

CREATE INDEX IF NOT EXISTS embedding_jobs_trace_idx
  ON embedding_jobs (trace_id, created_at DESC)
  WHERE trace_id IS NOT NULL;