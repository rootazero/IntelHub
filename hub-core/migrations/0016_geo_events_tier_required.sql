-- 0016_geo_events_tier_required.sql
-- Adds geo_events.tier_required; backfills existing rows from source name.
-- DOWN: ALTER TABLE geo_events DROP COLUMN tier_required;

ALTER TABLE geo_events ADD COLUMN tier_required TEXT NOT NULL DEFAULT 'free';
CREATE INDEX idx_geo_events_tier_required ON geo_events(tier_required);

-- Backfill: every source matching monitor:<x> gets the tier from §4.1.1 of the spec.
-- 'opensky' is the only admin tier (NC clause); all others default to free.
-- PR2 will overwrite this column at emit-time; the WHERE clause ensures
-- idempotent re-runs.
UPDATE geo_events
SET tier_required = CASE
  WHEN source LIKE 'monitor:opensky%' THEN 'admin'
  ELSE 'free'
END
WHERE tier_required = 'free';
