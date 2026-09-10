//! Phase-A collectors (spec §3). One file per source; each is self-contained
//! and unit-testable (parsers are pure functions).

pub mod acled;
pub mod firms;
pub mod gdelt;
pub mod kiwisdr;
pub mod noaa;
pub mod opensky;
pub mod radiation;
pub mod rss;
pub mod usgs;
