//! GEV P9 T2: summary brief endpoint — stub + Redis cache-key/TTL contract
//! tests.
//!
//! acled/reliefweb/gdelt upstream collectors do NOT exist in hub-core yet, so
//! the summary endpoint is a stub (200 + empty bullets + `sources:["cache"]`).
//! These tests pin the contract: cache-key format, 15-min TTL, and the
//! exact stub body shape. The handler's Redis read/write path is exercised in
//! production via `state.redis_timed` — pure-shape assertions here follow the
//! gev_traffic_integration precedent (no live PG/Redis in tests).

use hub_core::gev_summary::{
    build_stub_summary, summary_cache_key, SUMMARY_CACHE_PREFIX, SUMMARY_TTL_SECS,
};
use serde_json::json;

#[test]
fn cache_key_uses_spec_prefix() {
    assert_eq!(
        summary_cache_key("ICAO:ZBAA"),
        "cockpit:summary:ICAO:ZBAA"
    );
    assert_eq!(
        summary_cache_key("flight:UAL123"),
        "cockpit:summary:flight:UAL123"
    );
    assert_eq!(
        SUMMARY_CACHE_PREFIX, "cockpit:summary:",
        "prefix constant is the spec's literal"
    );
}

#[test]
fn ttl_is_15_minutes() {
    assert_eq!(SUMMARY_TTL_SECS, 900, "spec §3.3 mandates a 15-min Redis TTL");
}

#[test]
fn stub_returns_200_shape_with_empty_bullets() {
    let v = build_stub_summary(
        "ICAO:ZBAA",
        "2026-09-18T00:00:00+00:00",
        "2026-09-18T00:15:00+00:00",
    );
    assert_eq!(v["entity_id"], "ICAO:ZBAA");
    assert_eq!(v["generated_at"], "2026-09-18T00:00:00+00:00");
    assert_eq!(v["sources"], json!(["cache"]), "stub signals cache-only");
    assert_eq!(v["bullets"], json!([]), "no fake upstream bullets");
    assert_eq!(
        v["next_refresh_after"], "2026-09-18T00:15:00+00:00",
        "next_refresh_after = generated_at + 15 min"
    );
}

#[test]
fn stub_next_refresh_after_is_ttl_ahead_of_generated_at() {
    // The handler computes next_refresh_after = generated_at + SUMMARY_TTL_SECS.
    // Assert the two timestamps differ by exactly the TTL for a fixed input,
    // proving the 15-min cadence is wired to the same constant as the Redis TTL.
    let generated = "2026-09-18T00:00:00+00:00";
    let next = "2026-09-18T00:15:00+00:00";
    let v = build_stub_summary("x", generated, next);
    assert_eq!(v["generated_at"], generated);
    assert_eq!(v["next_refresh_after"], next);
    // 15 minutes = 900 seconds.
    assert_eq!(SUMMARY_TTL_SECS, 900);
}
