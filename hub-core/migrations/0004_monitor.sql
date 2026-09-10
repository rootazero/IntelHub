-- SP6: native monitor replaces the Crucix container.
-- 1) Rename historical geo_events source prefixes in place so the Radar's
--    history stays continuous (Crucix wrote 'crucix:<section>' rows).
-- 2) Seed the static maritime chokepoint reference layer (the only salvageable
--    part of Crucix's stub AIS module — coordinates verbatim).

UPDATE geo_events SET source = 'monitor:firms'     WHERE source = 'crucix:thermal';
UPDATE geo_events SET source = 'monitor:acled'     WHERE source = 'crucix:acled';
UPDATE geo_events SET source = 'monitor:gdelt'     WHERE source = 'crucix:gdelt';
UPDATE geo_events SET source = 'monitor:noaa'      WHERE source = 'crucix:noaa';
UPDATE geo_events SET source = 'monitor:rss'       WHERE source IN ('crucix:news', 'crucix:newsfeed');
UPDATE geo_events SET source = 'monitor:radiation' WHERE source IN ('crucix:nuke', 'crucix:epa');
UPDATE geo_events SET source = 'monitor:opensky'   WHERE source IN ('crucix:air', 'crucix:airmeta');
UPDATE geo_events SET source = 'monitor:legacy'    WHERE source LIKE 'crucix:%';

-- Static chokepoint reference layer (kind=maritime; excluded from retention
-- purge by source name in the monitor retention loop).
INSERT INTO geo_events (source, external_id, kind, title, lat, lon, severity, payload) VALUES
 ('monitor:chokepoint', 'straitOfHormuz',    'maritime', 'Strait of Hormuz',      26.5,  56.5, 'info', '{"note":"20% of world oil"}'),
 ('monitor:chokepoint', 'suezCanal',         'maritime', 'Suez Canal',            30.5,  32.3, 'info', '{"note":"12% of world trade"}'),
 ('monitor:chokepoint', 'straitOfGibraltar', 'maritime', 'Strait of Gibraltar',   36.0,  -5.7, 'info', '{"note":"Gateway to Mediterranean"}'),
 ('monitor:chokepoint', 'straitOfMalacca',   'maritime', 'Strait of Malacca',      2.5, 101.5, 'info', '{"note":"25% of world trade"}'),
 ('monitor:chokepoint', 'babElMandeb',       'maritime', 'Bab el-Mandeb',         12.6,  43.3, 'info', '{"note":"Red Sea gateway"}'),
 ('monitor:chokepoint', 'taiwanStrait',      'maritime', 'Taiwan Strait',         24.0, 119.0, 'info', '{"note":"88% of largest container ships"}'),
 ('monitor:chokepoint', 'bosporusStrait',    'maritime', 'Bosphorus',             41.1,  29.1, 'info', '{"note":"Black Sea access"}'),
 ('monitor:chokepoint', 'panamaCanal',       'maritime', 'Panama Canal',           9.1, -79.7, 'info', '{"note":"5% of world trade"}'),
 ('monitor:chokepoint', 'capeOfGoodHope',    'maritime', 'Cape of Good Hope',    -34.4,  18.5, 'info', '{"note":"Suez alternative"}')
ON CONFLICT (source, external_id) DO NOTHING;
