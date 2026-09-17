-- 0021_cctv_cameras.sql — GEV P3 (T8): CCTV static catalog base load.
-- Seeded from the vendor static directories console/gev-engine/config/
-- cctv_sources.<city>.json (contracts.md §3 CameraSource field set).
-- pose/geometry columns (heading/fov/pitch/range/mount/ground) are the
-- camera-intrinsic seed values the client projection pipeline consumes;
-- heading_confidence/pose_source pass through verbatim from the catalog
-- ('curated' → CAL badge on the client). health_* are reserved for the
-- T9 periodic upstream refresh; active defaults true so a catalog row
-- can be soft-disabled without losing its curated pose.
CREATE TABLE cctv_cameras (
  id                   TEXT PRIMARY KEY,
  city                 TEXT NOT NULL,
  city_id              TEXT,
  name                 TEXT NOT NULL,
  lat                  DOUBLE PRECISION NOT NULL,
  lon                  DOUBLE PRECISION NOT NULL,
  heading_deg          REAL,
  fov_deg              REAL,
  pitch_deg            REAL,
  range_m              REAL,
  mount_height_m       REAL,
  ground_elevation_m   REAL,
  feed_type            TEXT NOT NULL,
  frame_url            TEXT,           -- static-image feed (feed_type=image)
  media_url            TEXT,           -- video/stream feed (mp4/hls/webm/mjpeg)
  provider             TEXT NOT NULL,
  source_kind          TEXT,
  heading_confidence   TEXT,
  pose_source          TEXT,
  license_note         TEXT,
  credit               TEXT,
  code                 TEXT,
  active               BOOLEAN NOT NULL DEFAULT true,
  health_status        TEXT,
  health_checked_at    TIMESTAMPTZ,
  fetched_at           TIMESTAMPTZ NOT NULL DEFAULT now()  -- round marker; stale sweep = fetched_at < round_ts
);
CREATE INDEX cctv_cameras_lat_idx ON cctv_cameras(lat);
CREATE INDEX cctv_cameras_lon_idx ON cctv_cameras(lon);
CREATE INDEX cctv_cameras_city_idx ON cctv_cameras(city);
