//! GEV P12 T1: adsbdb enrichment — cache, parser, and handler tests.
//!
//! All upstream I/O is served by wiremock (the `EnrichmentService` takes a
//! base URL), so these run without network or live PG/Redis/Neo4j.
//!
//! Covers the design spec's T1 list plus an atomic-write check:
//! `cache_load_persist_roundtrip`, `cache_negative_404_stores_none`,
//! `cache_fresh_ttl_24h`, `invalid_hex_returns_400`, `inflight_coalesce`,
//! `upstream_5xx_returns_503`, `parse_route_minimal`, `parse_aircraft_minimal`,
//! `dirty_flush_atomic`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::http::StatusCode;
use hub_core::gev_enrichment::{
    handle_route, now_ms, parse_aircraft, parse_route, AircraftData, AirportData, CachedAircraft,
    CachedRoute, EnrichmentCache, EnrichmentService, RouteData,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------- fixtures ----------

fn route_fixture() -> RouteData {
    RouteData {
        airline: Some("United Airlines".to_string()),
        origin: AirportData {
            code: "KEWR".to_string(),
            name: "Newark".to_string(),
            lat: Some(40.69),
            lon: Some(-74.17),
        },
        destination: AirportData {
            code: "KSFO".to_string(),
            name: "San Francisco".to_string(),
            lat: Some(37.62),
            lon: Some(-122.38),
        },
    }
}

fn aircraft_fixture() -> AircraftData {
    AircraftData {
        type_code: Some("B738".to_string()),
        type_name: Some("Boeing 737-800".to_string()),
        registration: Some("EI-DCL".to_string()),
    }
}

/// Throwaway directory under the OS temp dir. Unique per call (name + ms).
fn tempdir_unique(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("intelhub-test-{}-{}", name, now_ms()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Service pointed at a mock upstream. No production code change needed —
/// the base URL is constructor-injectable.
fn test_service(base: &str) -> EnrichmentService {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    EnrichmentService::with_base_url(http, base)
}

fn aircraft_upstream_body() -> serde_json::Value {
    json!({
        "response": {
            "aircraft": {
                "icao_type": "B738",
                "manufacturer": "Boeing",
                "type": "737-800",
                "registration": "EI-DCL"
            }
        }
    })
}

// ---------- cache ----------

#[test]
fn cache_load_persist_roundtrip() {
    let tmp = tempdir_unique("adsbdb-cache");
    let file = tmp.join("adsbdb-cache.json");
    let mut cache = EnrichmentCache::new();
    cache.routes.insert(
        "UAL123".to_string(),
        CachedRoute { at: now_ms(), data: Some(route_fixture()) },
    );
    cache.aircraft.insert(
        "4ca9b1".to_string(),
        CachedAircraft { at: now_ms(), data: Some(aircraft_fixture()) },
    );
    cache.dirty.store(true, Ordering::Relaxed);
    cache.persist_to(&file).unwrap();

    let loaded = EnrichmentCache::load_from(&file).unwrap();
    assert_eq!(loaded.routes.len(), 1);
    assert_eq!(loaded.aircraft.len(), 1);
    assert_eq!(
        loaded.routes["UAL123"].data.as_ref().unwrap().airline.as_deref(),
        Some("United Airlines")
    );
    assert_eq!(
        loaded.aircraft["4ca9b1"].data.as_ref().unwrap().registration.as_deref(),
        Some("EI-DCL")
    );
}

#[test]
fn cache_negative_404_stores_none() {
    let mut cache = EnrichmentCache::new();
    cache
        .aircraft
        .insert("abc123".to_string(), CachedAircraft { at: now_ms(), data: None });
    let json = cache.aircraft_response("abc123");
    assert_eq!(json["found"], false);
    // A miss renders the same shape (unknown == unknown).
    assert_eq!(cache.aircraft_response("000000")["found"], false);
}

#[test]
fn cache_fresh_ttl_24h() {
    let mut cache = EnrichmentCache::new();
    // Within TTL (23h) is fresh.
    let recent = now_ms() - (23 * 3600 * 1000);
    cache.aircraft.insert(
        "4ca9b1".to_string(),
        CachedAircraft { at: recent, data: Some(aircraft_fixture()) },
    );
    assert!(cache.is_fresh(&cache.aircraft["4ca9b1"]), "23h-old entry must be fresh");
    // Past TTL (25h) is stale.
    let old_at = now_ms() - (25 * 3600 * 1000);
    cache.aircraft.insert(
        "4ca9b1".to_string(),
        CachedAircraft { at: old_at, data: Some(aircraft_fixture()) },
    );
    assert!(!cache.is_fresh(&cache.aircraft["4ca9b1"]), "25h-old entry must be stale");
}

#[test]
fn dirty_flush_atomic() {
    let dir = tempdir_unique("adsbdb-atomic");
    let file = dir.join("adsbdb-cache.json");
    let tmp = file.with_extension("json.tmp");
    let mut cache = EnrichmentCache::new();
    cache.aircraft.insert(
        "4ca9b1".to_string(),
        CachedAircraft { at: now_ms(), data: Some(aircraft_fixture()) },
    );
    cache.persist_to(&file).unwrap();
    assert!(file.exists(), "target cache file must exist");
    assert!(!tmp.exists(), "tmp file must be renamed away (atomic tmp+rename)");
    let loaded = EnrichmentCache::load_from(&file).unwrap();
    assert_eq!(loaded.aircraft.len(), 1);
}

// ---------- handlers ----------

#[tokio::test]
async fn invalid_hex_returns_400() {
    let svc = test_service("http://127.0.0.1:1");
    // Non-hex characters, and wrong length — both rejected before any fetch.
    let res = handle_route("/api/adsbdb/type/xyz", &svc).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let res = handle_route("/api/adsbdb/type/4ca9b1f", &svc).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(svc.upstream_calls().await, 0, "validation must not hit upstream");
}

#[tokio::test]
async fn inflight_coalesce() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/aircraft/4ca9b1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(aircraft_upstream_body()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let svc = Arc::new(test_service(&server.uri()));
    let reqs: Vec<_> = (0..10)
        .map(|_| {
            let svc = svc.clone();
            async move { svc.aircraft("4ca9b1").await }
        })
        .collect();
    let responses = futures::future::join_all(reqs).await;
    assert!(
        responses.iter().all(|r| r.status() == StatusCode::OK),
        "all coalesced callers must receive a 200"
    );
    assert_eq!(
        svc.upstream_calls().await,
        1,
        "10 concurrent misses on one key must coalesce to a single upstream call"
    );
}

#[tokio::test]
async fn upstream_5xx_returns_503() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v0/aircraft/4ca9b1"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let svc = test_service(&server.uri());
    let res = handle_route("/api/adsbdb/type/4ca9b1", &svc).await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    // A 5xx is transient — it must not be cached as a negative entry.
    let res2 = handle_route("/api/adsbdb/type/4ca9b1", &svc).await;
    assert_eq!(res2.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(svc.upstream_calls().await, 2, "5xx must not populate the cache");
}

// ---------- parsers ----------

#[test]
fn parse_route_minimal() {
    let json = json!({
        "response": {
            "flightroute": {
                "airline": { "name": "UA" },
                "origin": { "iata_code": "KEWR", "municipality": "Newark", "latitude": 40.69, "longitude": -74.17 },
                "destination": { "iata_code": "KSFO", "municipality": "San Francisco", "latitude": 37.62, "longitude": -122.38 }
            }
        }
    });
    let r = parse_route(&json).expect("route parses");
    assert_eq!(r.airline.as_deref(), Some("UA"));
    assert_eq!(r.origin.code, "KEWR");
    assert_eq!(r.destination.code, "KSFO");
    assert_eq!(r.origin.lat, Some(40.69));
}

#[test]
fn parse_aircraft_minimal() {
    let json = json!({
        "response": {
            "aircraft": {
                "icao_type": "B738",
                "manufacturer": "Boeing",
                "type": "737-800",
                "registration": "EI-DCL"
            }
        }
    });
    let a = parse_aircraft(&json).expect("aircraft parses");
    assert_eq!(a.type_code.as_deref(), Some("B738"));
    assert_eq!(a.type_name.as_deref(), Some("Boeing 737-800"));
    assert_eq!(a.registration.as_deref(), Some("EI-DCL"));

    // RF3: 200 with a null `aircraft` field must yield None (→ found:false),
    // never a panic.
    let null_body = json!({ "response": { "aircraft": null } });
    assert!(parse_aircraft(&null_body).is_none());
}
