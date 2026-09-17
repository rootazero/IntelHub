-- 0020_military_installations.sql — GEV P3 (T3): Overpass military harvest.
-- Elements are Overpass raw passthrough shape (contracts.md §4): tags /
-- geometry / bounds stored verbatim, normalize happens in the client.
-- mil_class is the hub-side derived filter column
-- (airfield/naval_base/range/barracks/base/military_land/other).
-- Composite PK because OSM ids are only unique per type (node/way/relation
-- namespaces overlap). btree on lat/lon serves the bbox queries the
-- /api/military-installations REST layer (T4) runs.
CREATE TABLE military_installations (
  osm_type   TEXT NOT NULL,              -- node | way | relation
  osm_id     BIGINT NOT NULL,
  name       TEXT,
  mil_class  TEXT NOT NULL,
  lat        DOUBLE PRECISION NOT NULL,
  lon        DOUBLE PRECISION NOT NULL,
  minlat     DOUBLE PRECISION,           -- way/relation bounds
  minlon     DOUBLE PRECISION,
  maxlat     DOUBLE PRECISION,
  maxlon     DOUBLE PRECISION,
  geometry   JSONB,                      -- footprint point list [{lat,lon},...]
  tags       JSONB,
  fetched_at TIMESTAMPTZ NOT NULL DEFAULT now(),  -- round marker; stale sweep = fetched_at < round_ts
  PRIMARY KEY (osm_type, osm_id)
);
CREATE INDEX military_installations_lat_idx ON military_installations(lat);
CREATE INDEX military_installations_lon_idx ON military_installations(lon);
