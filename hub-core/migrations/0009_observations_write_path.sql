-- 0009_observations_write_path.sql — fix the previously orphaned
-- observations table so list_evidence_for_entity (graph_queries.rs)
-- can actually populate it.
--
-- Pre-migration diagnosis (Task 4 review, Critical #1):
--   The brief/spec §6 mandate that list_evidence_for_entity writes
--   observations rows as a side-effect (idempotent via
--   ON CONFLICT DO NOTHING). The current INSERT fails silently on
--   three counts:
--     1. observations.observation_id has no DEFAULT (NOT NULL violation)
--     2. observations.created_by has no DEFAULT (NOT NULL violation)
--     3. ON CONFLICT DO NOTHING references no unique constraint
--        (PK is observation_id, which the INSERT doesn't supply), so
--        the conflict resolution never fires — every call re-tries
--        to insert the same logical row.
--   Additionally the caller uses `let _ = ... .await;`, swallowing
--   the error entirely. Net effect: side-effect never fires.
--
-- Post-migration:
--   - observation_id auto-fills via gen_random_uuid() (PG 13+ core,
--     no pgcrypto extension needed — IntelHub runs on postgres:17.x).
--   - created_by auto-fills via DEFAULT 'list_evidence_for_entity',
--     stamping the writer clearly in audit trail.
--   - The UNIQUE (entity_id, document_id, observed_at) constraint
--     gives ON CONFLICT DO NOTHING a real conflict to dedupe on,
--     preserving the idempotent contract.
--
-- Idempotency: ALTER ... SET DEFAULT is a no-op if the default already
-- matches; ADD CONSTRAINT will fail if re-applied, so guard with
-- IF NOT EXISTS (PG 9.6+ on constraints via DO block).

-- 1. observation_id: add DEFAULT so INSERTs don't have to supply it.
ALTER TABLE observations
  ALTER COLUMN observation_id SET DEFAULT gen_random_uuid();

-- 2. created_by: add DEFAULT so the caller doesn't have to provide it.
ALTER TABLE observations
  ALTER COLUMN created_by SET DEFAULT 'list_evidence_for_entity';

-- 3. Add the dedupe constraint. Guard against re-application.
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
     WHERE conname = 'observations_entity_doc_observed_uniq'
  ) THEN
    ALTER TABLE observations
      ADD CONSTRAINT observations_entity_doc_observed_uniq
      UNIQUE (entity_id, document_id, observed_at);
  END IF;
END $$;
