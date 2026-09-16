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
pub mod ripe_as_overview;
pub mod ipapi_co;
pub mod ip_api_com;
pub mod ripe_prefix_overview;
