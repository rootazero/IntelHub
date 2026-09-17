//! Phase-A collectors (spec §3). One file per source; each is self-contained
//! and unit-testable (parsers are pure functions).

pub mod acled;
pub mod ahmia;
pub mod courtlistener;
pub mod bls;
pub mod bluesky;
pub mod cisakev;
pub mod climateseries;
pub mod comtrade;
pub mod leaksify;
pub mod eia;
pub mod eonet;
pub mod epa;
pub mod etherscan;
pub mod defillama;
pub mod otx;
pub mod urlscan;
pub mod gfw;
pub mod finintel;
pub mod firms;
pub mod fred;
pub mod gdelt;
pub mod gscpi;
pub mod kiwisdr;
pub mod markets;
pub mod noaa;
pub mod nvd;
pub mod ofac;
pub mod opensanctions;
pub mod opencorp;
pub mod osv;
pub mod opensky;
pub mod overpass;
pub mod wikidata;
pub mod radiation;
pub mod reliefweb;
pub mod rss;
pub mod sec_edgar;
pub mod telegram;
pub(crate) mod textclass;
pub mod treasury;
pub mod usaspending;
pub mod usgs;
pub mod who;
pub mod x;
// OSINT Framework bridge 4 (2026-09-17): TorExit + IPsum — see
// each module's docstring for anchor + cadence rationale.
pub mod ipsum;
pub mod tor_exit;
// OSINT Framework bridge 5 (2026-09-17): crt.sh (cert transparency)
// + OpenPhish (phishing URL catalog) + Shodan InternetDB (free
// per-IP enrichment). See each module's docstring for watchlist
// + cadence rationale.
pub mod crtsh;
pub mod openphish;
pub mod shodan_internetdb;
pub mod blocklist_de;
pub mod spamhaus_drop;
pub mod nominatim;
pub mod ripestat;
pub mod wayback;
pub mod aws_ip_ranges;
pub mod gcp_ip_ranges;
// Globe P1 (2026-09-17): CelesTrak TLE catalog → `satellites` table.
pub mod celestrak;
// Globe P1 (2026-09-17): adsb.lol live aircraft snapshot → Redis ring +
// notable events → geo_events (spec §2.2: positions never touch PG).
pub mod adsb;
// GEV P3 (2026-09-17): AISStream.io live vessel positions → Redis
// `hub:globe:vessels` + per-MMSI track ring. Env-gated: registered only
// when AISSTREAM_API_KEY/HUB_AISSTREAM_API_KEY is set (opensky pattern);
// without a key the REST layer serves status "missing-key".
pub mod ais;
// GEV P3 (2026-09-17): Overpass military-installations harvest → PG
// `military_installations` (migration 0020). Keyless, 24h cadence, 4
// quadrants with ≥60s politeness; restart-safe (<24h → skip round).
pub mod installations;
// GEV P3 (2026-09-17): CCTV static catalog loader (T8) — vendor
// cctv_sources.*.json → PG cctv_cameras (migration 0021). Keyless,
// idempotent upsert, 24h placeholder cadence; T9 adds the periodic
// upstream refresh on top of this base load.
pub mod cctv;
pub mod ripe_as_overview;
pub mod ipapi_co;
pub mod ip_api_com;
pub mod ripe_prefix_overview;
pub mod misp_dynamic_dns;
pub mod urlhaus;
pub mod firehol_level1;
pub mod misp_rfc5735;
pub mod misp_rfc6761;
pub mod tor_exit_details;
pub mod romainmarcoux_malicious_ip;
pub mod ihr_hegemony;
pub mod misp_second_level_tlds;
