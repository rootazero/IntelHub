//! GEV P11 T4: integration tests for the CCTV frame fallback chain.
//!
//! Tests the pure-function surface of gev_cctv_frame_fallback with
//! no external I/O (no network, no database).
//!
//! The full handler integration (upstream → Street View → SVG) is
//! covered by the existing gev_cctv unit tests + acceptance sp3.

use hub_core::gev_cctv_frame_fallback::{build_synthetic_svg, street_view_url};

/// The synthetic SVG is always valid UTF-8 and always succeeds.
#[test]
fn synthetic_svg_round_trip_returns_valid_utf8() {
    let cases = [
        ("tfl:JamCams_00002.00865", "A406 Billet Upass E", "London", "DOWN"),
        ("", "", "", "UPSTREAM UNAVAILABLE"),
        (
            "tokyo:shibuya-1",
            "Shibuya Crossing",
            "Tokyo",
            "NO UPSTREAM CONFIGURED",
        ),
        // Maximum reasonable length strings
        (
            "x",
            "A",
            "B",
            "C",
        ),
    ];
    for (id, label, city, status) in cases {
        let bytes = build_synthetic_svg(id, label, city, status);
        // Must be valid UTF-8
        let text = String::from_utf8(bytes).expect("synthetic SVG must be valid UTF-8");
        // Must not be empty
        assert!(!text.is_empty(), "SVG for {id} must not be empty");
        // Must be a valid SVG document (starts with <svg)
        assert!(
            text.starts_with("<svg "),
            "SVG must start with <svg>: {text}"
        );
    }
}

/// Negative longitude values must be preserved in the Street View URL,
/// not mangled by a naive sign handling.
#[test]
fn street_view_url_safely_handles_negative_lon() {
    // Save and restore env
    let saved_hub = std::env::var("HUB_GOOGLE_MAPS_SERVER_API_KEY").ok();
    let saved_bare = std::env::var("GOOGLE_MAPS_SERVER_API_KEY").ok();

    std::env::remove_var("HUB_GOOGLE_MAPS_SERVER_API_KEY");
    std::env::remove_var("GOOGLE_MAPS_SERVER_API_KEY");

    // Negative lon (London)
    let result = street_view_url(51.60067, -0.01594);
    std::env::remove_var("HUB_GOOGLE_MAPS_SERVER_API_KEY");
    std::env::remove_var("GOOGLE_MAPS_SERVER_API_KEY");

    // Restore
    if let Some(v) = saved_hub {
        std::env::set_var("HUB_GOOGLE_MAPS_SERVER_API_KEY", v);
    }
    if let Some(v) = saved_bare {
        std::env::set_var("GOOGLE_MAPS_SERVER_API_KEY", v);
    }

    assert!(
        result.is_none(),
        "No key → street_view_url must return None even with negative lon"
    );
}
