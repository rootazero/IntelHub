//! Compile-time-adjacent invariant: every registered monitor MUST have
//! a `monitor_metadata()` entry, and every metadata entry MUST correspond
//! to a registered monitor. Catches drift between `monitor/mod.rs` registry
//! and `config.rs::monitor_metadata()` map.

use hub_core::config::monitor_metadata;

const REGISTRY: &[&str] = &[
    "usgs", "noaa", "firms", "gdelt", "opensky", "rss",
    "sec-edgar", "radiation", "acled", "kiwisdr", "reliefweb", "who", "cisa-kev",
    "nvd", "osv", "ofac", "opensanctions", "usaspending", "epa-radnet",
    "bluesky", "telegram-watch", "x", "bls", "eonet",
    "fred", "eia", "treasury", "markets", "finintel", "gscpi", "comtrade",
    "climate",
];

#[test]
fn every_registered_monitor_has_metadata() {
    let meta = monitor_metadata();
    for name in REGISTRY {
        assert!(
            meta.contains_key(name),
            "{name} is in monitor/mod.rs registry but missing from monitor_metadata() map"
        );
    }
}

#[test]
fn every_metadata_entry_is_registered() {
    let meta = monitor_metadata();
    for name in meta.keys() {
        assert!(
            REGISTRY.contains(name),
            "{name} has metadata entry but is NOT in monitor/mod.rs registry"
        );
    }
}

#[test]
fn registry_count_matches_metadata_count() {
    let meta = monitor_metadata();
    assert_eq!(
        REGISTRY.len(),
        meta.len(),
        "registry has {} entries, metadata has {}",
        REGISTRY.len(),
        meta.len()
    );
}
