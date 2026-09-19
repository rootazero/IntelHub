//! GEV P9 T2: weather brief endpoint — dual-source fallback + in-memory cache
//! integration tests.
//!
//! No real API calls: NOAA + Open-Meteo are mocked with wiremock. The fetch
//! functions take injectable base URLs, so no env vars are mutated (and no
//! parallel-test env races). Each test uses distinct coordinates so the
//! process-global in-memory cache never collides across tests.

use hub_core::gev_weather::{
    fetch_weather, open_meteo_url, parse_noaa_grid, parse_open_meteo, weather_lookup,
    weather_response, WeatherData, WeatherSource,
};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_ua() -> String {
    "IntelHub/test".to_string()
}

fn noaa_grid_body() -> serde_json::Value {
    json!({
        "properties": {
            "temperature":   { "uom": "wmoUnit:degC", "values": [{ "validTime": "t", "value": 21.0 }] },
            "windSpeed":     { "uom": "wmoUnit:km_h-1", "values": [{ "validTime": "t", "value": 15.0 }] },
            "windDirection": { "uom": "wmoUnit:degree_(angle)", "values": [{ "validTime": "t", "value": 230.0 }] },
            "quantitativePrecipitation": { "uom": "wmoUnit:mm", "values": [{ "validTime": "t", "value": 2.5 }] },
            "skyCover":      { "uom": "wmoUnit:percent", "values": [{ "validTime": "t", "value": 45.0 }] },
            "visibility":    { "uom": "wmoUnit:m", "values": [{ "validTime": "t", "value": 16000.0 }] }
        }
    })
}

fn open_meteo_body() -> serde_json::Value {
    json!({
        "latitude": 40.0,
        "longitude": -105.0,
        "current": {
            "time": "2026-09-18T12:00",
            "temperature_2m": 18.5,
            "wind_speed_10m": 36.0,
            "wind_direction_10m": 200.0,
            "precipitation": 0.2,
            "cloud_cover": 70.0,
            "visibility": 25000.0,
            "surface_pressure": 1013.2
        }
    })
}

/// Mount NOAA points + grid mocks (grid returns `noaa_grid_body()`).
async fn mount_noaa(server: &MockServer, grid_path: &str) -> String {
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/points/39.7500,-104.8700"))
        .and(header("user-agent", "IntelHub/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "properties": { "forecastGridData": format!("{base}{grid_path}") }
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(grid_path))
        .respond_with(ResponseTemplate::new(200).set_body_json(noaa_grid_body()))
        .mount(server)
        .await;
    base
}

// ---------- (a) NOAA success → noaa source ----------

#[tokio::test]
async fn noaa_success_returns_noaa_source() {
    let server = MockServer::start().await;
    let base = mount_noaa(&server, "/gridpoints/BOU/62,61").await;

    let client = reqwest::Client::new();
    let (source, data) = fetch_weather(&client, &test_ua(), 39.75, -104.87, &base, &base)
        .await
        .expect("noaa success");

    assert_eq!(source, WeatherSource::Noaa);
    assert_eq!(data.temperature_c, Some(21.0));
    assert!((data.wind_speed_kts.unwrap() - 15.0 * hub_core::gev_weather::KMH_TO_KTS).abs() < 1e-9);
    assert_eq!(data.visibility_m, Some(16000.0));
    // NOAA grid has no surface pressure → null (Open-Meteo fills it).
    assert_eq!(data.pressure_hpa, None);
}

// ---------- (b) NOAA fails → Open-Meteo fallback ----------

#[tokio::test]
async fn noaa_failure_falls_back_to_open_meteo() {
    let server = MockServer::start().await;
    let base = server.uri();

    Mock::given(method("GET"))
        .and(path("/points/40.0000,-105.0000"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/forecast"))
        .respond_with(ResponseTemplate::new(200).set_body_json(open_meteo_body()))
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let (source, data) = fetch_weather(&client, &test_ua(), 40.0, -105.0, &base, &base)
        .await
        .expect("open-meteo fallback");

    assert_eq!(source, WeatherSource::OpenMeteo);
    assert_eq!(data.temperature_c, Some(18.5));
    assert_eq!(data.pressure_hpa, Some(1013.2));
    assert_eq!(data.visibility_m, Some(25000.0));
}

// ---------- (c) both fail → Err with sources_tried ----------

#[tokio::test]
async fn both_sources_fail_returns_sources_tried() {
    let server = MockServer::start().await;
    let base = server.uri();

    Mock::given(method("GET"))
        .and(path("/points/0.0000,0.0000"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/forecast"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let err = fetch_weather(&client, &test_ua(), 0.0, 0.0, &base, &base)
        .await
        .expect_err("both sources fail");

    assert_eq!(err.0, vec!["noaa", "open-meteo"], "sources_tried in order");
    assert!(!err.1.is_empty(), "error message present");
}

// ---------- (d) cache hit on second call (same fetched_at, one grid hit) ----------

#[tokio::test]
async fn second_call_within_ttl_serves_from_cache() {
    let server = MockServer::start().await;
    let base = server.uri();

    Mock::given(method("GET"))
        .and(path("/points/10.0000,20.0000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "properties": { "forecastGridData": format!("{base}/gridpoints/X/1,1") }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gridpoints/X/1,1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(noaa_grid_body()))
        .expect(1) // exactly one upstream grid fetch — second call is a cache hit
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let (bytes1, hit1) = weather_lookup(&client, &test_ua(), 10.0, 20.0, &base, &base)
        .await
        .expect("first fetch");
    assert!(!hit1, "first call is a miss");

    let (bytes2, hit2) = weather_lookup(&client, &test_ua(), 10.0, 20.0, &base, &base)
        .await
        .expect("second fetch");
    assert!(hit2, "second call is a cache hit");
    assert_eq!(bytes1, bytes2, "cache returns byte-identical payload");

    let v1: serde_json::Value = serde_json::from_slice(&bytes1).unwrap();
    let v2: serde_json::Value = serde_json::from_slice(&bytes2).unwrap();
    assert_eq!(
        v1["fetched_at"], v2["fetched_at"],
        "cache hit preserves the original fetched_at (no refetch)"
    );
}

// ---------- pure-contract regressions (shared with unit tests) ----------

#[test]
fn parse_noaa_grid_rejects_unexpected_uom() {
    // D1 gate: windSpeed reported in m/s (NOT the expected km/h) must be
    // null — never a silently-wrong kts conversion. The correctly-united
    // temperature is still accepted.
    let body = json!({
        "properties": {
            "windSpeed": { "uom": "wmoUnit:m_s-1", "values": [{ "validTime": "t", "value": 5.0 }] },
            "temperature": { "uom": "wmoUnit:degC", "values": [{ "validTime": "t", "value": 21.0 }] }
        }
    });
    let d = parse_noaa_grid(&body);
    assert_eq!(d.wind_speed_kts, None, "m/s wind must not be converted to kts");
    assert_eq!(d.temperature_c, Some(21.0), "degC temperature still accepted");
}

#[test]
fn open_meteo_url_carries_contract_vars() {
    let url = open_meteo_url("https://api.open-meteo.com", 40.0, -105.0);
    for v in [
        "temperature_2m",
        "wind_speed_10m",
        "wind_direction_10m",
        "precipitation",
        "cloud_cover",
        "visibility",
        "surface_pressure",
    ] {
        assert!(url.contains(v), "missing current var {v}");
    }
}

#[test]
fn parsers_produce_null_for_missing_fields() {
    assert_eq!(parse_noaa_grid(&json!({ "properties": {} })), WeatherData::default());
    assert!(parse_open_meteo(&json!({})).is_none());
}

#[test]
fn response_shape_is_contract_stable() {
    let v = weather_response(WeatherSource::OpenMeteo, &WeatherData::default(), "t");
    assert_eq!(v["source"], "open-meteo");
    for field in [
        "temperature_c",
        "wind_speed_kts",
        "wind_direction_deg",
        "precipitation_mm",
        "cloud_cover_pct",
        "visibility_m",
        "pressure_hpa",
    ] {
        assert!(v.get(field).is_some(), "missing contract field {field}");
    }
}
