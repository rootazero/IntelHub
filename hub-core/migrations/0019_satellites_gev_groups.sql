-- 0019_satellites_gev_groups.sql — GEV P2 (T5): celestrak GROUPS realigned to
-- the GEV six core groups (stations, visual, gps-ops, glo-ops, galileo, geo).
-- The retired categories are no longer refreshed by celestrak.rs, so their rows
-- would sit stale forever (globe/GEV layers would propagate dead TLEs) — delete
-- them. Per-category full-replace means live groups repopulate on the next tick.
DELETE FROM satellites WHERE category IN ('weather', 'gnss', 'military');
