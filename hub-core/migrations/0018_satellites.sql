-- 0018_satellites.sql — Globe P1: CelesTrak TLE catalog.
-- Per-category full replace by celestrak.rs (decayed sats vanish naturally).
CREATE TABLE satellites (
  norad_id   INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  category   TEXT NOT NULL,
  tle_line1  TEXT NOT NULL,
  tle_line2  TEXT NOT NULL,
  epoch      TIMESTAMPTZ NOT NULL,
  fetched_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX satellites_category_idx ON satellites(category);
