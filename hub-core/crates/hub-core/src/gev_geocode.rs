//! GEV P7: same-origin geocode proxy for the globe location search.
//!
//! Upstream: photon.komoot.io (keyless). Response normalized to the shape
//! the vendored engine's searchAndFlyTo consumes directly (locations.js:795):
//! viewport as {southwest:{lat,lng}, northeast:{lat,lng}}, types as the
//! engine's navigation-mode tokens (country/administrative/locality/...).
//!
//! Cache: Redis hub:gev:geocode:<sha256(norm-q)[:16]hex>, TTL 1h. The read
//! MUST stay Option<Option<Vec<u8>>> — redis-rs maps Nil → Ok(vec![]) for
//! Vec<u8>, so single-Option reads every miss as hit+empty-body (GEV P3).

use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::state::AppState;

const REDIS_BUDGET_MS: u64 = 2000;
const GEOCODE_CACHE_TTL_SECS: u64 = 3600;
const PHOTON_ENDPOINT: &str = "https://photon.komoot.io/api/";
const MAX_QUERY_LEN: usize = 200;
const MAX_RESULTS: usize = 5;

#[derive(Deserialize)]
pub struct GeocodeParams {
    q: String,
}

fn geocode_http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("intelhub-gev-geocode/1.0")
            .build()
            .expect("geocode http client build")
    })
}

fn gev_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn cache_key(query: &str) -> String {
    let norm = query.trim().to_lowercase();
    let digest = Sha256::digest(norm.as_bytes());
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("hub:gev:geocode:{hex}")
}

/// photon osm_key/osm_value → engine navigation-mode tokens
/// (locations.js geocodeNavigationMode: country/administrative →
/// region-overview, locality → city-overview, route/park/airport → …).
fn map_types(osm_key: &str, osm_value: &str) -> Vec<&'static str> {
    match (osm_key, osm_value) {
        ("place", "country") => vec!["country"],
        ("place", "state") | ("boundary", "administrative") => vec!["administrative"],
        ("place", "city") | ("place", "town") | ("place", "village") | ("place", "hamlet") => {
            vec!["locality"]
        }
        ("highway", _) => vec!["route"],
        ("aeroway", _) => vec!["airport"],
        ("leisure", "park") | ("boundary", "national_park") => vec!["park"],
        _ => vec!["place"],
    }
}

fn normalize_features(body: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for feat in body.get("features").and_then(Value::as_array).into_iter().flatten().take(MAX_RESULTS) {
        let coords = feat
            .get("geometry")
            .and_then(|g| g.get("coordinates"))
            .and_then(Value::as_array);
        let (lng, lat) = match coords {
            Some(c) if c.len() >= 2 => (c[0].as_f64(), c[1].as_f64()),
            _ => (None, None),
        };
        let (Some(lat), Some(lng)) = (lat, lng) else { continue };
        let props = feat.get("properties").cloned().unwrap_or(Value::Null);
        let s = |k: &str| props.get(k).and_then(Value::as_str).unwrap_or("");
        let label = [s("name"), s("district"), s("city"), s("state"), s("country")]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let viewport = props.get("extent").and_then(Value::as_array).and_then(|e| {
            // photon extent: [minLng, maxLat, maxLng, minLat]
            if e.len() == 4 {
                Some(json!({
                    "southwest": { "lat": e[3].as_f64()?, "lng": e[0].as_f64()? },
                    "northeast": { "lat": e[1].as_f64()?, "lng": e[2].as_f64()? },
                }))
            } else {
                None
            }
        });
        out.push(json!({
            "lat": lat,
            "lng": lng,
            "name": s("name"),
            "label": if label.is_empty() { s("name") } else { &label },
            "types": map_types(s("osm_key"), s("osm_value")),
            "viewport": viewport,
        }));
    }
    out
}

pub async fn gev_geocode(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GeocodeParams>,
) -> Result<Response, Response> {
    let q = params.q.trim();
    if q.is_empty() || q.chars().count() > MAX_QUERY_LEN {
        return Err(gev_err(StatusCode::BAD_REQUEST, "invalid query"));
    }
    let key = cache_key(q);
    // Double Option: Nil → Some(None) → miss (see module doc).
    let cached: Option<Option<Vec<u8>>> = state
        .redis_timed(redis::cmd("GET").arg(&key).clone(), REDIS_BUDGET_MS)
        .await;
    if let Some(Some(bytes)) = cached {
        return Ok(json_response(bytes, "hit"));
    }
    let resp = match geocode_http()
        .get(PHOTON_ENDPOINT)
        .query(&[("q", q), ("limit", "5")])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "photon upstream fetch failed");
            return Err(gev_err(StatusCode::BAD_GATEWAY, "geocode upstream failed"));
        }
    };
    if !resp.status().is_success() {
        tracing::warn!(status = %resp.status(), "photon upstream returned error");
        return Err(gev_err(StatusCode::BAD_GATEWAY, "geocode upstream failed"));
    }
    let body: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "photon body parse failed");
            return Err(gev_err(StatusCode::BAD_GATEWAY, "geocode upstream failed"));
        }
    };
    let payload = json!({ "results": normalize_features(&body) });
    let bytes = serde_json::to_vec(&payload)
        .map_err(|e| gev_err(StatusCode::INTERNAL_SERVER_ERROR, &format!("encode: {e}")))?;
    let _: Option<String> = state
        .redis_timed(
            redis::cmd("SETEX")
                .arg(&key)
                .arg(GEOCODE_CACHE_TTL_SECS)
                .arg(bytes.as_slice())
                .clone(),
            REDIS_BUDGET_MS,
        )
        .await;
    Ok(json_response(bytes, "miss"))
}

fn json_response(bytes: Vec<u8>, cache: &'static str) -> Response {
    let mut hm = HeaderMap::new();
    hm.insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    hm.insert("x-geocode-cache", HeaderValue::from_static(cache));
    (hm, bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_normalizes_case_and_whitespace() {
        assert_eq!(cache_key("  Paris "), cache_key("paris"));
        assert_ne!(cache_key("paris"), cache_key("london"));
    }

    #[test]
    fn normalize_features_maps_photon_geojson() {
        let body = json!({
            "features": [{
                "geometry": { "coordinates": [2.3522, 48.8566] },
                "properties": {
                    "name": "Paris", "city": "Paris", "country": "France",
                    "osm_key": "place", "osm_value": "city",
                    "extent": [2.22, 48.90, 2.47, 48.81]
                }
            }]
        });
        let out = normalize_features(&body);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["lat"], 48.8566);
        assert_eq!(out[0]["lng"], 2.3522);
        assert_eq!(out[0]["types"], json!(["locality"]));
        assert_eq!(out[0]["viewport"]["southwest"], json!({"lat": 48.81, "lng": 2.22}));
        assert_eq!(out[0]["viewport"]["northeast"], json!({"lat": 48.90, "lng": 2.47}));
    }

    #[test]
    fn normalize_features_skips_malformed_and_caps_results() {
        let body = json!({ "features": [
            { "geometry": { "coordinates": [] }, "properties": {} },
            { "geometry": { "coordinates": [0.0, 0.0] }, "properties": { "name": "Null Island", "osm_key": "place", "osm_value": "locality" } }
        ]});
        let out = normalize_features(&body);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["viewport"], Value::Null);
    }

    #[test]
    fn map_types_tokens_match_engine_modes() {
        assert_eq!(map_types("place", "country"), vec!["country"]);
        assert_eq!(map_types("boundary", "administrative"), vec!["administrative"]);
        assert_eq!(map_types("place", "town"), vec!["locality"]);
        assert_eq!(map_types("highway", "residential"), vec!["route"]);
        assert_eq!(map_types("amenity", "cafe"), vec!["place"]);
    }
}
