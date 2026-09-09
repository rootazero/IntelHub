-- SP4: geo_events — normalized geographic signal events (Crucix sweep output
-- is the first producer; directive §26 Global Radar + §51 aggregate-not-replicate).
-- Idempotent upsert on (source, external_id); plain lat/lon doubles (no PostGIS
-- at this scale, per spec §7).

CREATE TABLE IF NOT EXISTS geo_events (
    event_id     uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    source       text NOT NULL,               -- e.g. 'crucix:firms', 'crucix:acled'
    external_id  text NOT NULL,               -- producer-side stable id (or content hash)
    kind         text NOT NULL,               -- fire | conflict | flight | radiation | maritime | news | health | economic | other
    title        text NOT NULL,
    lat          double precision,
    lon          double precision,
    severity     text NOT NULL DEFAULT 'info',-- flash | priority | routine | info
    occurred_at  timestamptz NOT NULL DEFAULT now(),
    payload      jsonb NOT NULL DEFAULT '{}'::jsonb,
    ingested_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (source, external_id)
);

CREATE INDEX IF NOT EXISTS geo_events_occurred_idx  ON geo_events (occurred_at DESC);
CREATE INDEX IF NOT EXISTS geo_events_severity_idx  ON geo_events (severity);
CREATE INDEX IF NOT EXISTS geo_events_source_kind   ON geo_events (source, kind);
CREATE INDEX IF NOT EXISTS geo_events_geo_idx       ON geo_events (lat, lon) WHERE lat IS NOT NULL;
