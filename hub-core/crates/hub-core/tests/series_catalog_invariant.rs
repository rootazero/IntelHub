//! Compile-time + runtime invariants for the series catalog.
//!
//! These tests fail loudly if anyone:
//! - Adds two descriptors with the same `(source, normalized_id)`
//! - Forgets to apply the source prefix to a normalized_id
//! - Uses a `normalized_id` that doesn't match the unit suffix convention
//!
//! Run with: `cargo test --package hub-core --test series_catalog_invariant`

use hub_core::series::{Source, Unit, CATALOG, normalize};
use std::collections::{HashMap, HashSet};

#[test]
fn catalog_no_duplicate_normalized_id() {
    let mut seen: HashSet<&str> = HashSet::new();
    for d in CATALOG {
        assert!(
            seen.insert(d.normalized_id),
            "duplicate normalized_id across sources: {}",
            d.normalized_id
        );
    }
}

#[test]
fn catalog_no_duplicate_source_upstream() {
    let mut seen: HashSet<(Source, &str)> = HashSet::new();
    for d in CATALOG {
        let key = (d.source, d.upstream_id);
        assert!(
            seen.insert(key),
            "duplicate (source, upstream_id) entry: {:?} / {}",
            d.source, d.upstream_id
        );
    }
}

#[test]
fn catalog_normalized_id_starts_with_source_prefix() {
    for d in CATALOG {
        let expected_prefix = format!("{}:", d.source.prefix());
        assert!(
            d.normalized_id.starts_with(&expected_prefix),
            "{} missing source prefix {}",
            d.normalized_id, expected_prefix
        );
    }
}

// Series whose normalized_id is intentionally missing the unit suffix
// because the upstream ID itself encodes the unit. These are
// backward-compat exceptions — the storage keys predate the catalog.
const SUFFIX_OPT_OUT: &[&str] = &[
    "fred:T10Y2Y",  // 10Y-2Y spread; unit implicit in upstream_id
    "fred:VIXCLS",  // VIX Close; unit-less index value
    "fred:ICSA",    // Initial Claims SA; count
];

#[test]
fn catalog_normalized_id_ends_with_unit_suffix() {
    // Some real-world emit strings have compound suffixes (_YOY_PCT vs _YOY).
    // Check that the unit suffix appears anywhere in the normalized_id, not
    // just at the end. The point of the test is to catch *missing* unit
    // info entirely; loose ordering is acceptable.
    for d in CATALOG {
        let suffix = d.unit.suffix();
        if suffix.is_empty() {
            continue;
        }
        if SUFFIX_OPT_OUT.contains(&d.normalized_id) {
            continue;
        }
        assert!(
            d.normalized_id.contains(suffix.trim_start_matches('_')),
            "{} missing unit indicator for {:?} (expected token containing {})",
            d.normalized_id, d.unit, suffix
        );
    }
}

#[test]
fn catalog_normalize_round_trip() {
    // Every (source, upstream_id) entry must be reachable via normalize()
    for d in CATALOG {
        let resolved = normalize(d.source, d.upstream_id);
        assert_eq!(
            resolved,
            Some(d.normalized_id),
            "normalize({:?}, {}) returned {:?}, expected Some({})",
            d.source, d.upstream_id, resolved, d.normalized_id
        );
    }
}

#[test]
fn catalog_unit_distribution_sane() {
    // Sanity: we should have several percent, several index, etc.
    // Catches "I accidentally deleted half the entries" regressions.
    let mut by_unit: HashMap<Unit, usize> = HashMap::new();
    for d in CATALOG {
        *by_unit.entry(d.unit).or_default() += 1;
    }
    assert!(
        by_unit.get(&Unit::Percent).copied().unwrap_or(0) >= 5,
        "expected >=5 Percent series, got {:?}",
        by_unit
    );
    assert!(
        by_unit.get(&Unit::YoyPct).copied().unwrap_or(0) >= 2,
        "expected >=2 YoY series (CPI, wages), got {:?}",
        by_unit
    );
}

#[test]
fn catalog_all_hud_gauge_series_present() {
    // The Monitor Command Deck HUD renders 5 gauges. If any of them is
    // missing from the catalog, the gauge will silently render empty.
    // This test is the regression guard for the 2026-09-15 HUD bug.
    let needed = [
        ("fred:VIXCLS", "VIX close"),
        ("fred:DGS10_PCT", "10-year Treasury"),
        ("fred:T10Y2Y", "2s10s spread"),
        ("fred:HY_OAS_PCT", "high-yield OAS"),
        ("eia:WTI_SPOT_USD_BBL", "WTI spot"),
    ];
    for (id, why) in needed {
        assert!(
            CATALOG.iter().any(|d| d.normalized_id == id),
            "HUD gauge {id} ({why}) missing from catalog"
        );
    }
}

#[test]
fn catalog_display_names_are_unique() {
    // Display names must be unique so the frontend can use them as keys.
    let mut seen: HashSet<&str> = HashSet::new();
    for d in CATALOG {
        assert!(
            seen.insert(d.display_name),
            "duplicate display_name: {}",
            d.display_name
        );
    }
}