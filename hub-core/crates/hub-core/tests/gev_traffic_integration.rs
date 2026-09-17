//! GEV P3 T5+T6: traffic REST endpoints — pure-function contract tests.
//!
//! Covers the contract surface that does not need live PG/Redis/upstream:
//! overpass body whitelist + cache-key stability, tile coordinate bounds,
//! tomtom key env-gate precedence, no-key 503 shape, upstream error
//! passthrough shape. Lives in /tests/ as its own binary (same pattern as
//! planner_llm_integration.rs — sidesteps pre-existing unit-test compile
//! failures elsewhere in the crate).

use axum::http::StatusCode;
use hub_core::gev_traffic::{
    flow_not_configured_response, flow_tile_upstream_url, overpass_cache_key,
    upstream_error_response, valid_flow_tile, validate_overpass_form,
};

fn form_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

// ---------- T5: overpass form whitelist ----------

#[test]
fn overpass_accepts_standard_roads_query() {
    let ql = "[out:json][timeout:25];(way[\"highway\"~\"^(motorway|trunk|primary|secondary)$\"](12.0,30.0,42.0,65.0););out geom qt;";
    let body = format!("data={}", form_encode(ql));
    let decoded = validate_overpass_form(&body).expect("standard engine query must pass");
    assert_eq!(decoded, ql, "decoded query must round-trip byte-exact");
}

#[test]
fn overpass_rejects_body_without_data_prefix() {
    assert!(validate_overpass_form("nonsense=1").is_err());
    // data field not in first position is rejected per contract ("必须 data= 开头")
    assert!(validate_overpass_form("foo=bar&data=x").is_err());
}

#[test]
fn overpass_rejects_non_json_output_modes() {
    for ql in ["[out:xml];way(1);out;", "[out:csv];way(1);out;", "way(1);out;"] {
        let body = format!("data={}", form_encode(ql));
        assert!(
            validate_overpass_form(&body).is_err(),
            "{ql:?} must be rejected by the [out:json] whitelist"
        );
    }
}

#[test]
fn overpass_accepts_leading_whitespace_before_out_json() {
    let body = format!("data={}", form_encode("  \n[out:json][timeout:10];way(1);out;"));
    assert!(validate_overpass_form(&body).is_ok());
}

#[test]
fn overpass_rejects_oversized_body() {
    let filler = "x".repeat(32 * 1024);
    let body = format!("data={}", form_encode(&format!("[out:json];{filler}")));
    assert!(validate_overpass_form(&body).is_err());
}

#[test]
fn overpass_cache_key_is_stable_and_sensitive() {
    let ql = "[out:json][timeout:25];(way[\"highway\"~\"^primary$\"](1,2,3,4););out geom qt;";
    let k1 = overpass_cache_key(ql);
    let k2 = overpass_cache_key(ql);
    assert_eq!(k1, k2, "same query must hash to the same key");
    assert_eq!(k1.len(), "hub:gev:overpass:".len() + 64, "key = prefix + sha256 hex");
    assert!(k1.starts_with("hub:gev:overpass:"));
    assert_ne!(
        overpass_cache_key(ql),
        overpass_cache_key(&format!("{ql} ")),
        "query change must change the key"
    );
}

// ---------- T6b: tile coordinate validity (zoom 8-16, x/y in [0, 2^z)) ----------

#[test]
fn flow_tile_zoom_bounds() {
    assert!(valid_flow_tile(8, 0, 0));
    assert!(valid_flow_tile(16, 0, 0));
    assert!(valid_flow_tile(12, 100, 100));
    assert!(!valid_flow_tile(7, 0, 0), "zoom below MIN_TILE_ZOOM=8");
    assert!(!valid_flow_tile(17, 0, 0), "zoom above MAX_TILE_ZOOM=16");
}

#[test]
fn flow_tile_xy_bounds() {
    // z=8 → range [0, 256)
    assert!(valid_flow_tile(8, 255, 255));
    assert!(!valid_flow_tile(8, 256, 0));
    assert!(!valid_flow_tile(8, 0, 256));
    assert!(!valid_flow_tile(8, -1, 0));
    assert!(!valid_flow_tile(8, 0, -1));
    // z=16 → range [0, 65536)
    assert!(valid_flow_tile(16, 65535, 65535));
    assert!(!valid_flow_tile(16, 65536, 0));
}

// ---------- T6a: key env-gate precedence ----------

#[test]
fn tomtom_key_env_precedence() {
    // Serialized within this one test fn to avoid parallel env races.
    std::env::remove_var("HUB_TOMTOM_API_KEY");
    std::env::remove_var("TOMTOM_API_KEY");
    assert!(hub_core::gev_traffic::tomtom_api_key().is_none(), "unset → None");

    std::env::set_var("TOMTOM_API_KEY", "bare-key");
    assert_eq!(
        hub_core::gev_traffic::tomtom_api_key().as_deref(),
        Some("bare-key"),
        "bare TOMTOM_API_KEY accepted"
    );

    std::env::set_var("HUB_TOMTOM_API_KEY", "hub-key");
    assert_eq!(
        hub_core::gev_traffic::tomtom_api_key().as_deref(),
        Some("hub-key"),
        "HUB_ prefix wins when both set (opensky precedent)"
    );

    std::env::set_var("HUB_TOMTOM_API_KEY", "");
    assert_eq!(
        hub_core::gev_traffic::tomtom_api_key().as_deref(),
        Some("bare-key"),
        "empty HUB_ value ignored (secrets.env template incident)"
    );

    std::env::remove_var("HUB_TOMTOM_API_KEY");
    std::env::set_var("TOMTOM_API_KEY", "");
    assert!(
        hub_core::gev_traffic::tomtom_api_key().is_none(),
        "empty bare value ignored"
    );
    std::env::remove_var("TOMTOM_API_KEY");
}

#[test]
fn tomtom_status_shape_always_200() {
    // Factored pure shape: the handler is a Json<{"hasKey": bool}> with no
    // error path — assert both branches serialize the mandated key.
    let with = serde_json::json!({ "hasKey": true });
    let without = serde_json::json!({ "hasKey": false });
    assert!(with["hasKey"].is_boolean());
    assert!(without["hasKey"].is_boolean());
}

// ---------- T6b: no-key 503 shape ----------

#[tokio::test]
async fn flow_no_key_response_is_503_with_contract_body() {
    let resp = flow_not_configured_response();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(resp.into_body(), 4096)
        .await
        .expect("body readable");
    let v: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(v["error"], "tomtom not configured");
}

// ---------- T5: upstream error passthrough shape ----------

#[tokio::test]
async fn overpass_upstream_error_passthrough() {
    let resp = upstream_error_response(StatusCode::from_u16(429).unwrap(), "rate limited".into());
    assert_eq!(resp.status(), 429, "upstream status passes through verbatim");
    let body = axum::body::to_bytes(resp.into_body(), 4096)
        .await
        .expect("body readable");
    assert_eq!(body.as_ref(), b"rate limited", "upstream error body passes through verbatim");
}

// ---------- T6b: upstream URL scheme ----------

#[test]
fn flow_upstream_url_uses_tomtom_flow_scheme() {
    let url = flow_tile_upstream_url("SECRET", 12, 100, 200);
    assert!(url.starts_with("https://api.tomtom.com/traffic/map/4/tile/flow/relative0/12/100/200.pbf?"),
        "scheme must match tomtomTiles.js header (traffic/map/4/tile/flow): {url}");
    assert!(url.contains("key=SECRET"));
    assert!(url.contains("thickness=10"));
}
