-- 2026-09-14: Canonical finding ↔ entity association table.
--
-- Pre-phase-3 the v2 compiler's `mark_finding_about_entity` op
-- (graph_v2/compiler.rs) wrote the link only to graph_change_log
-- (audit-only) and let the mirror worker translate it into a Neo4j
-- :ABOUT edge. No PG row, so the 83 pre-existing findings had no
-- PG-canonical way to express their entity references.
--
-- This table makes the link first-class:
--   * mark_finding_about_entity now INSERTs here (canonical) AND keeps
--     the change_log emit (audit / mirror).
--   * backfill-finding-entities.py mines past change_log rows that had
--     entity_id + relation:"about" and replays them here.
--   * Backfill script ALSO derives historical finding→entity links from
--     observations↔finding_evidence (with a 50% coverage threshold and
--     confidence=0.6) to recover the 0/83 gap.
--
-- The mirror worker's `link_finding_about_entity` op (phase 2) is the
-- Neo4j side; this table is its PG side. mirror worker fires when it
-- sees the matching change_log row.

CREATE TABLE IF NOT EXISTS finding_entities (
    finding_id    uuid        NOT NULL REFERENCES findings(finding_id) ON DELETE CASCADE,
    entity_id     uuid        NOT NULL REFERENCES entities(entity_id) ON DELETE CASCADE,
    role          text        NOT NULL DEFAULT 'about',
    confidence    real        NOT NULL DEFAULT 1.0
                              CHECK (confidence >= 0.0 AND confidence <= 1.0),
    created_by    text        NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (finding_id, entity_id, role)
);

CREATE INDEX IF NOT EXISTS finding_entities_entity_idx
    ON finding_entities (entity_id);

CREATE INDEX IF NOT EXISTS finding_entities_finding_idx
    ON finding_entities (finding_id);

-- Reconciler bookkeeping — a small log table so the reconcile worker
-- can record when it last cleaned up Neo4j orphans. Lets accept-sp10
-- verify the worker is alive without grepping logs.
CREATE TABLE IF NOT EXISTS mirror_reconcile_runs (
    run_id         bigserial   PRIMARY KEY,
    started_at     timestamptz NOT NULL DEFAULT now(),
    finished_at    timestamptz,
    orphans_removed integer    NOT NULL DEFAULT 0,
    details        jsonb       NOT NULL DEFAULT '{}'::jsonb
);
