-- 0022_annotations_v1.sql — GEV P8: annotation persistence.
--
-- Hand-drawn / world-anchored marks (pin / line / area) survive across
-- sessions, unlike the vendored engine's in-memory annotation store. The
-- geometry is lon/lat world-anchored ({vertices:[{lon,lat,height?},...]}) so
-- any renderer (world drape, screen SVG, hybrid) can project it.
--
-- Numbering note: the P8 plan wrote this as `core/migrations/0011_annotations_v1.sql`.
-- The real migration directory is `hub-core/migrations/` (sqlx::migrate!
-- "../../migrations" from hub-core/crates/hub-core/src/state.rs) and version
-- 0011 is already taken by 0011_resolution_queue_processing.sql — a duplicate
-- version makes sqlx::migrate! fail to compile. Next free version is 0022.
--
-- Tracking is sqlx's own `_sqlx_migrations` table, written by the migrator at
-- boot (state.rs / admin.rs). Do NOT INSERT INTO schema_migrations here.

CREATE TABLE IF NOT EXISTS annotations_v1 (
    id          uuid PRIMARY KEY,
    agent_id    text,
    shape       text NOT NULL CHECK (shape IN ('pin','line','area')),
    label       text,
    color       text NOT NULL DEFAULT 'primary',
    geometry    jsonb NOT NULL,
    ttl_ms      integer,
    meta        jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz,
    CONSTRAINT ttl_with_expires CHECK (
        (ttl_ms IS NULL AND expires_at IS NULL)
        OR (ttl_ms IS NOT NULL AND expires_at IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS annotations_v1_created_idx
    ON annotations_v1 (created_at DESC);
CREATE INDEX IF NOT EXISTS annotations_v1_expires_idx
    ON annotations_v1 (expires_at)
    WHERE expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS annotations_v1_geom_idx
    ON annotations_v1 USING GIN (geometry jsonb_path_ops);

-- annotation_links: placeholder for future Neo4j/Signal binding; not used in P8
CREATE TABLE IF NOT EXISTS annotation_links (
    annotation_id uuid NOT NULL REFERENCES annotations_v1(id) ON DELETE CASCADE,
    entity_id     uuid,
    rel           text,
    PRIMARY KEY (annotation_id, entity_id, rel)
);
