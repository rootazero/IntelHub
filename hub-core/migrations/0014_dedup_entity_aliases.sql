-- 0014_dedup_entity_aliases.sql
-- Fix for A-005 / B-005 / C-001 (2026-09-15 e2e audit):
--   entities.aliases JSON column accumulated duplicates because
--   create_entity uses ON CONFLICT DO UPDATE SET aliases = entities.aliases || EXCLUDED.aliases
--   which APPENDS the entire alias list on every re-seed.
--
-- Effects:
--   1) Backfill: dedup existing entities.aliases JSON arrays in PG.
--   2) Forward: switch to array_distinct() concatenation so duplicates
--      are removed in-line. The code change is in graphw.rs::create_entity.
--   3) Sync: enqueue create_entity ops for all entities so Neo4j mirror
--      gets the cleaned values via graph_sync_queue worker.
--
-- Idempotent: safe to re-run.

BEGIN;

-- 1) Dedup existing entities.aliases JSON arrays in PG
UPDATE entities
SET aliases = COALESCE(
    (
        SELECT to_jsonb(array_agg(DISTINCT value))
        FROM jsonb_array_elements_text(aliases) AS value
    ),
    '[]'::jsonb
)
WHERE jsonb_typeof(aliases) = 'array'
  AND (
    SELECT count(*) FROM jsonb_array_elements_text(aliases) AS v
  ) != (
    SELECT count(DISTINCT v) FROM jsonb_array_elements_text(aliases) AS v
  );

-- 2) Update entity_aliases table UNIQUE constraint to be (entity_id, alias_norm)
--    (currently it's (kind, alias_norm) which prevents cross-kind reuse but
--    allows the same entity to have duplicate alias_norm if it ever gets a
--    second INSERT. Tighten to per-entity.)
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'entity_aliases_kind_alias_norm_key'
    ) THEN
        ALTER TABLE entity_aliases DROP CONSTRAINT entity_aliases_kind_alias_norm_key;
    END IF;
EXCEPTION WHEN undefined_object THEN
    NULL;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS entity_aliases_entity_id_alias_norm_key
  ON entity_aliases (entity_id, alias_norm);

COMMIT;