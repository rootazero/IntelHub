//! Phase-A collectors (spec §3). One file per source; each is self-contained
//! and unit-testable (parsers are pure functions).

pub mod acled;
pub mod bls;
pub mod cisakev;
pub mod comtrade;
pub mod eia;
pub mod epa;
pub mod finintel;
pub mod firms;
pub mod fred;
pub mod gdelt;
pub mod gscpi;
pub mod kiwisdr;
pub mod markets;
pub mod noaa;
pub mod ofac;
pub mod opensky;
pub mod radiation;
pub mod reliefweb;
pub mod rss;
pub(crate) mod textclass;
pub mod treasury;
pub mod usaspending;
pub mod usgs;
pub mod who;
