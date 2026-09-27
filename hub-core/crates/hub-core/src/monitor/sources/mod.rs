//! Phase-A collectors (spec §3). One file per source; each is self-contained
//! and unit-testable (parsers are pure functions).

// Public-API integration roadmap Phase 1.1 (2026-09-27): Open-Meteo global
// weather forecast (keyless). Spec: docs/superpowers/roadmaps/
// 2026-09-27-public-api-integration-roadmap.md §2.1.
pub mod open_meteo;
// Public-API integration roadmap Phase 1.2 (2026-09-27): USGS Water Services
// real-time stream-gauge instantaneous values (keyless, US public domain).
// Spec: docs/superpowers/roadmaps/2026-09-27-public-api-integration-roadmap.md
// §2.1.
pub mod usgs_water;
// Public-API integration roadmap Phase 1.3 (2026-09-27): OpenAQ global air
// quality (requires free API key signup at explore.openaq.org/register).
// Spec: docs/superpowers/roadmaps/2026-09-27-public-api-integration-roadmap.md
// §2.1.
pub mod open_aq;
// Public-API integration roadmap Phase 1.4 (2026-09-27): HDX Humanitarian
// Data Exchange (https://data.humdata.org/ — UN OCHA, keyless CKAN API).
// Spec: docs/superpowers/roadmaps/2026-09-27-public-api-integration-roadmap.md
// §2.1.
pub mod hdx_humanitarian;
// Public-API integration roadmap Phase 1.5 (2026-09-27): Nager.Date public
// holidays (https://date.nager.at/ — keyless, 204 country coverage). Spec:
// docs/superpowers/roadmaps/2026-09-27-public-api-integration-roadmap.md
// §2.1.
pub mod nager_date;
// Public-API integration roadmap Phase 1.6 (2026-09-27): Sunrise-Sunset
// API (https://sunrise-sunset.org/api — keyless). Daylight envelope
// (sunrise/sunset/civil+nautical+astronomical twilight) for 30 OSINT
// cities; pure context signal (info severity) for downstream event
// correlation (e.g. "this drone strike happened during astronomical
// twilight"). Spec: docs/superpowers/roadmaps/2026-09-27-public-api-
// integration-roadmap.md §2.1.
pub mod sunrise_sunset;
// Public-API integration roadmap Phase 1.7 (2026-09-27): INPE Queimadas
// (Brazilian wildfire satellite detection — keyless, INPE public-domain
// satellite data). Top-50 fires/day by FRP from the daily CSV. Spec:
// docs/superpowers/roadmaps/2026-09-27-public-api-integration-roadmap.md
// §2.1.
pub mod queimadas_inpe;
// Public-API integration roadmap Phase 1.8 (2026-09-27): Helium News
// MCP (https://heliumtrades.com/mcp-page/ — keyless, no signup, 50 free
// queries/window). Bias-balanced news synthesis from 5000+ sources per
// query. Single 'geopolitics' query, top-10 results, severity by rank
// (1=priority, 2-5=routine, 6+=info). Spec: docs/superpowers/roadmaps/
// 2026-09-27-public-api-integration-roadmap.md §2.1.
pub mod helium_news;
// Public-API integration roadmap Phase 2.1 (2026-09-27): GreyNoise
// Community API (https://docs.greynoise.io — keyless, free
// unauthenticated, 50 lookups/week for free-tier with API key).
// Scanner/IoT/botnet classification for a curated 25-IP watchlist.
// Severity by classification: malicious=priority, benign=routine,
// unknown=info, ordinary IP=skip. Spec: docs/superpowers/roadmaps/
// 2026-09-27-public-api-integration-roadmap.md §3.
pub mod greynoise;
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
// GEV P13 T4 (2026-09-21): 3rd live-aircraft source — expanded-radius
// adsb.lol point sweep around six US hubs → Redis
// `hub:globe:aircraft:adsbx` (TTL 300s), snapshot-only (no geo Signals).
// Fills the US coverage gap the hotspot-only rotation leaves. See the module
// docstring for the rate-limit pacing (shared Ctx::limiter, 25s hub gap).
pub mod adsbexchange;
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
