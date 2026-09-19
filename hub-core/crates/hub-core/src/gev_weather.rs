//! GEV P9 T2: regional weather brief endpoint — NOAA + Open-Meteo dual-source
//! fallback (spec §3.3 / plan Task 2).
//!
//! `GET /api/v1/gev/weather?lat&lon`
//!
//! - NOAA first (keyless, but api.weather.gov requires a `User-Agent` header
//!   — see `crate::secrets::noaa_user_agent()`). Two-step lookup:
//!   `/points/{lat},{lon}` → `properties.forecastGridData` URL → grid data.
//! - Open-Meteo fallback (keyless, no UA): one-step `/v1/forecast?current=…`.
//! - Both failures → 503 + `{error, sources_tried}` (spec §3.3 D3: the
//!   cockpit shows a grey banner, never a retry storm).
//! - 5-minute in-memory cache (quantized lat/lon) to absorb hot-spot
//!   cockpit refetches. No Redis dependency — the cache is per-process,
//!   which is fine for a single hub-core instance.
//!
//! Response shape (spec §3.3 WeatherResponse): all 7 metric fields are
//! always present, `null` when the source omits them. Units are the
//! contract's metric names (temperature °C, wind kts, precip mm, cloud %,
//! visibility m, pressure hPa). A `units` query param (if the console
//! sends one) is ignored — the contract is fixed-metric.
//!
//! Testability: base URLs and the UA are injectable (no env races in tests);
//! the handler resolves them from env following the overpass precedent.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::{
    extract::Query,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock;

// ---------- constants ----------

const NOAA_DEFAULT_BASE: &str = "https://api.weather.gov";
const OPEN_METEO_DEFAULT_BASE: &str = "https://api.open-meteo.com";
const WEATHER_TIMEOUT_SECS: u64 = 12;
/// In-memory cache TTL — 5 min per plan Task 2 (hot-spot guard).
const WEATHER_CACHE_TTL_SECS: u64 = 300;
/// Cache-key quantization: lat/lon rounded to 2 decimals (~1.1 km) — finer
/// than the 10 km regional-refresh throttle, so cockpit movement always
/// refetches but same-position re-renders hit the cache.
const QUANT: f64 = 100.0;
/// 1 km/h = 0.539956803 knots. NOAA `windSpeed` and Open-Meteo
/// `wind_speed_10m` both default to km/h.
pub const KMH_TO_KTS: f64 = 0.539_956_8;

/// Open-Meteo `current` variables — one per contract metric. Plan Task 2 URL
/// verbatim (default units: °C, km/h, mm, %, m, hPa).
const OPEN_METEO_CURRENT_VARS: &str = "temperature_2m,wind_speed_10m,wind_direction_10m,precipitation,cloud_cover,visibility,surface_pressure";

// ---------- params ----------

#[derive(Deserialize)]
pub struct WeatherParams {
    lat: f64,
    lon: f64,
}

// ---------- response model ----------

/// The 7 contract metrics (§3.3 WeatherResponse). Optional per-field because
/// a source may omit any given one (e.g. NOAA forecastGridData has no
/// surface pressure — Open-Meteo fills that slot).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeatherData {
    pub temperature_c: Option<f64>,
    pub wind_speed_kts: Option<f64>,
    pub wind_direction_deg: Option<f64>,
    pub precipitation_mm: Option<f64>,
    pub cloud_cover_pct: Option<f64>,
    pub visibility_m: Option<f64>,
    pub pressure_hpa: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherSource {
    Noaa,
    OpenMeteo,
}

impl WeatherSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            WeatherSource::Noaa => "noaa",
            WeatherSource::OpenMeteo => "open-meteo",
        }
    }
}

/// Build the contract `WeatherResponse` JSON. `fetched_at` is RFC3339.
pub fn weather_response(source: WeatherSource, data: &WeatherData, fetched_at: &str) -> Value {
    json!({
        "source": source.as_str(),
        "fetched_at": fetched_at,
        "temperature_c": data.temperature_c,
        "wind_speed_kts": data.wind_speed_kts,
        "wind_direction_deg": data.wind_direction_deg,
        "precipitation_mm": data.precipitation_mm,
        "cloud_cover_pct": data.cloud_cover_pct,
        "visibility_m": data.visibility_m,
        "pressure_hpa": data.pressure_hpa,
    })
}

// ---------- http client ----------

fn weather_http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(WEATHER_TIMEOUT_SECS))
            .build()
            .expect("weather http client build")
    })
}

fn gev_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn service_unavailable_response(sources: Vec<String>, error: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": error, "sources_tried": sources })),
    )
        .into_response()
}

fn json_response(bytes: Vec<u8>, cache: &'static str) -> Response {
    let mut hm = HeaderMap::new();
    hm.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    hm.insert("x-weather-cache", HeaderValue::from_static(cache));
    (hm, bytes).into_response()
}

// ---------- base URL resolution (env override — overpass precedent) ----------

pub fn noaa_base_url() -> String {
    for var in ["HUB_NOAA_BASE_URL", "NOAA_BASE_URL"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return v.trim_end_matches('/').to_string();
            }
        }
    }
    NOAA_DEFAULT_BASE.to_string()
}

pub fn open_meteo_base_url() -> String {
    for var in ["HUB_OPEN_METEO_BASE_URL", "OPEN_METEO_BASE_URL"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return v.trim_end_matches('/').to_string();
            }
        }
    }
    OPEN_METEO_DEFAULT_BASE.to_string()
}

// ---------- URL builders (base injectable for tests) ----------

pub fn noaa_points_url(base: &str, lat: f64, lon: f64) -> String {
    format!("{}/points/{lat:.4},{lon:.4}", base.trim_end_matches('/'))
}

pub fn open_meteo_url(base: &str, lat: f64, lon: f64) -> String {
    format!(
        "{}/v1/forecast?latitude={lat}&longitude={lon}&current={}",
        base.trim_end_matches('/'),
        OPEN_METEO_CURRENT_VARS
    )
}

// ---------- parsers (pure, unit-tested) ----------

/// NOAA forecastGridData values are `{uom, values: [{validTime, value}]}`.
/// Take the first (current) value. Field units are carried in the `uom`
/// string (NOT `unitCode` — live probe: `uom` is `"wmoUnit:km_h-1"` etc.
/// while `unitCode` is null). Expected units: temperature `wmoUnit:degC`,
/// windSpeed `wmoUnit:km_h-1` (→ kts), windDirection
/// `wmoUnit:degree_(angle)`, quantitativePrecipitation `wmoUnit:mm`,
/// skyCover `wmoUnit:percent`, visibility `wmoUnit:m`,
/// pressure/barometricPressure `wmoUnit:Pa` (→ hPa) when present.
///
/// D1 unit gate: a value is accepted only when its `uom` matches the
/// expected unit; otherwise the field is `null` — never a silently-wrong
/// conversion (e.g. windSpeed reported in m/s must NOT be read as kts).
///
/// NOTE: plan Task 2 listed `precipitationProbability` (a %, no contract
/// slot). The contract field is `precipitation_mm`, so the honest amount
/// source `quantitativePrecipitation` (mm) is used instead.
pub fn parse_noaa_grid(body: &Value) -> WeatherData {
    let props = body.get("properties");
    let uom = |key: &str| -> Option<&str> { props?.get(key)?.get("uom")?.as_str() };
    let current = |key: &str| -> Option<f64> {
        props?
            .get(key)?
            .get("values")?
            .as_array()?
            .first()?
            .get("value")?
            .as_f64()
    };
    // D1 gate: only return a value when its `uom` matches the expected unit.
    let gated = |key: &str, expected: &str| -> Option<f64> {
        if uom(key) == Some(expected) {
            current(key)
        } else {
            None
        }
    };
    WeatherData {
        temperature_c: gated("temperature", "wmoUnit:degC"),
        wind_speed_kts: gated("windSpeed", "wmoUnit:km_h-1").map(|v| v * KMH_TO_KTS),
        wind_direction_deg: current("windDirection"),
        precipitation_mm: gated("quantitativePrecipitation", "wmoUnit:mm"),
        cloud_cover_pct: gated("skyCover", "wmoUnit:percent"),
        visibility_m: gated("visibility", "wmoUnit:m"),
        pressure_hpa: gated("pressure", "wmoUnit:Pa")
            .or_else(|| gated("barometricPressure", "wmoUnit:Pa"))
            .map(|v| v / 100.0),
    }
}

/// Open-Meteo `current` block → contract metrics. `wind_speed_10m` is km/h
/// by default (plan URL omits `wind_speed_unit`), converted to kts here.
pub fn parse_open_meteo(body: &Value) -> Option<WeatherData> {
    let cur = body.get("current")?;
    let f = |k: &str| cur.get(k).and_then(Value::as_f64);
    Some(WeatherData {
        temperature_c: f("temperature_2m"),
        wind_speed_kts: f("wind_speed_10m").map(|v| v * KMH_TO_KTS),
        wind_direction_deg: f("wind_direction_10m"),
        precipitation_mm: f("precipitation"),
        cloud_cover_pct: f("cloud_cover"),
        visibility_m: f("visibility"),
        pressure_hpa: f("surface_pressure"),
    })
}

// ---------- coordinate validation ----------

pub fn valid_coords(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
}

// ---------- in-memory cache ----------

struct CacheEntry {
    bytes: Vec<u8>,
    fetched: Instant,
}

fn weather_cache() -> &'static RwLock<HashMap<(i64, i64), CacheEntry>> {
    static CACHE: OnceLock<RwLock<HashMap<(i64, i64), CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Quantize lat/lon to a stable integer key (f64 has no Eq/Hash).
pub fn quantize(lat: f64, lon: f64) -> (i64, i64) {
    ((lat * QUANT).round() as i64, (lon * QUANT).round() as i64)
}

// ---------- fetch (dual-source fallback) ----------

/// Dual-source fallback: NOAA then Open-Meteo. Returns
/// `Ok((source, data))` or `Err((sources_tried, last_error))`.
pub async fn fetch_weather(
    client: &reqwest::Client,
    ua: &str,
    lat: f64,
    lon: f64,
    noaa_base: &str,
    open_meteo_base: &str,
) -> Result<(WeatherSource, WeatherData), (Vec<String>, String)> {
    let mut tried: Vec<String> = Vec::new();

    match fetch_noaa(client, ua, lat, lon, noaa_base).await {
        Ok(data) => return Ok((WeatherSource::Noaa, data)),
        Err(e) => {
            tried.push("noaa".to_string());
            tracing::warn!(error = %e, lat, lon, "noaa weather fetch failed — falling back to open-meteo");
        }
    }

    match fetch_open_meteo(client, lat, lon, open_meteo_base).await {
        Ok(data) => Ok((WeatherSource::OpenMeteo, data)),
        Err(e) => {
            tried.push("open-meteo".to_string());
            Err((tried, e))
        }
    }
}

/// Step 1: `/points/{lat},{lon}` → `properties.forecastGridData` URL.
async fn noaa_grid_url(
    client: &reqwest::Client,
    ua: &str,
    lat: f64,
    lon: f64,
    base: &str,
) -> Result<String, String> {
    let url = noaa_points_url(base, lat, lon);
    let resp = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, ua)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if !resp.status().is_success() {
        return Err(format!("noaa points HTTP {}", resp.status().as_u16()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| e.without_url().to_string())?;
    body.pointer("/properties/forecastGridData")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| "noaa points missing forecastGridData".to_string())
}

/// Step 2: fetch grid data and parse the contract metrics.
async fn noaa_grid(
    client: &reqwest::Client,
    ua: &str,
    grid_url: &str,
) -> Result<WeatherData, String> {
    let resp = client
        .get(grid_url)
        .header(reqwest::header::USER_AGENT, ua)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if !resp.status().is_success() {
        return Err(format!("noaa grid HTTP {}", resp.status().as_u16()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| e.without_url().to_string())?;
    Ok(parse_noaa_grid(&body))
}

async fn fetch_noaa(
    client: &reqwest::Client,
    ua: &str,
    lat: f64,
    lon: f64,
    base: &str,
) -> Result<WeatherData, String> {
    let grid_url = noaa_grid_url(client, ua, lat, lon, base).await?;
    noaa_grid(client, ua, &grid_url).await
}

async fn fetch_open_meteo(
    client: &reqwest::Client,
    lat: f64,
    lon: f64,
    base: &str,
) -> Result<WeatherData, String> {
    let url = open_meteo_url(base, lat, lon);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if !resp.status().is_success() {
        return Err(format!("open-meteo HTTP {}", resp.status().as_u16()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| e.without_url().to_string())?;
    parse_open_meteo(&body).ok_or_else(|| "open-meteo missing current block".to_string())
}

// ---------- cache + fetch wrapper ----------

/// In-memory cache lookup, falling through to `fetch_weather` on a miss or
/// expired entry. Returns `(serialized_response_bytes, cache_hit)`.
pub async fn weather_lookup(
    client: &reqwest::Client,
    ua: &str,
    lat: f64,
    lon: f64,
    noaa_base: &str,
    open_meteo_base: &str,
) -> Result<(Vec<u8>, bool), (Vec<String>, String)> {
    let key = quantize(lat, lon);
    if let Some(entry) = weather_cache().read().await.get(&key) {
        if entry.fetched.elapsed().as_secs() < WEATHER_CACHE_TTL_SECS {
            return Ok((entry.bytes.clone(), true));
        }
    }

    let (source, data) = fetch_weather(client, ua, lat, lon, noaa_base, open_meteo_base).await?;
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let payload = weather_response(source, &data, &fetched_at);
    let bytes = serde_json::to_vec(&payload)
        .map_err(|e| (Vec::new(), format!("encode weather response: {e}")))?;
    weather_cache().write().await.insert(
        key,
        CacheEntry {
            bytes: bytes.clone(),
            fetched: Instant::now(),
        },
    );
    Ok((bytes, false))
}

// ---------- handler ----------

pub async fn gev_weather(Query(params): Query<WeatherParams>) -> Result<Response, Response> {
    if !valid_coords(params.lat, params.lon) {
        return Err(gev_err(StatusCode::BAD_REQUEST, "invalid lat/lon"));
    }
    let ua = crate::secrets::noaa_user_agent();
    let noaa_base = noaa_base_url();
    let om_base = open_meteo_base_url();
    match weather_lookup(weather_http(), &ua, params.lat, params.lon, &noaa_base, &om_base).await {
        Ok((bytes, hit)) => Ok(json_response(bytes, if hit { "hit" } else { "miss" })),
        Err((sources, err)) => Err(service_unavailable_response(sources, &err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_rounds_to_two_decimals() {
        assert_eq!(quantize(39.756, -104.874), (3976, -10487));
        assert_eq!(quantize(0.0, 0.0), (0, 0));
        assert_eq!(quantize(-90.0, 180.0), (-9000, 18000));
    }

    #[test]
    fn coords_bounds() {
        assert!(valid_coords(0.0, 0.0));
        assert!(valid_coords(90.0, 180.0));
        assert!(valid_coords(-90.0, -180.0));
        assert!(!valid_coords(90.1, 0.0));
        assert!(!valid_coords(-90.1, 0.0));
        assert!(!valid_coords(0.0, 180.1));
        assert!(!valid_coords(f64::NAN, 0.0));
        assert!(!valid_coords(0.0, f64::INFINITY));
    }

    #[test]
    fn open_meteo_url_includes_all_vars() {
        let url = open_meteo_url("https://api.open-meteo.com", 40.0, -105.0);
        assert!(url.starts_with("https://api.open-meteo.com/v1/forecast?"));
        assert!(url.contains("latitude=40"));
        assert!(url.contains("longitude=-105"));
        for v in OPEN_METEO_CURRENT_VARS.split(',') {
            assert!(url.contains(v), "missing var {v}");
        }
    }

    #[test]
    fn noaa_points_url_formats_four_decimals() {
        let url = noaa_points_url("https://api.weather.gov", 39.75, -104.87);
        assert_eq!(url, "https://api.weather.gov/points/39.7500,-104.8700");
    }

    #[test]
    fn parse_noaa_grid_maps_units() {
        let body = json!({
            "properties": {
                "temperature":   { "uom": "wmoUnit:degC", "values": [{ "validTime": "t", "value": 21.0 }] },
                "windSpeed":     { "uom": "wmoUnit:km_h-1", "values": [{ "validTime": "t", "value": 15.0 }] },
                "windDirection": { "uom": "wmoUnit:degree_(angle)", "values": [{ "validTime": "t", "value": 230.0 }] },
                "quantitativePrecipitation": { "uom": "wmoUnit:mm", "values": [{ "validTime": "t", "value": 2.5 }] },
                "skyCover":      { "uom": "wmoUnit:percent", "values": [{ "validTime": "t", "value": 45.0 }] },
                "visibility":    { "uom": "wmoUnit:m", "values": [{ "validTime": "t", "value": 16000.0 }] },
                "pressure":      { "uom": "wmoUnit:Pa", "values": [{ "validTime": "t", "value": 101300.0 }] }
            }
        });
        let d = parse_noaa_grid(&body);
        assert_eq!(d.temperature_c, Some(21.0));
        assert!((d.wind_speed_kts.unwrap() - 15.0 * KMH_TO_KTS).abs() < 1e-9);
        assert_eq!(d.wind_direction_deg, Some(230.0));
        assert_eq!(d.precipitation_mm, Some(2.5));
        assert_eq!(d.cloud_cover_pct, Some(45.0));
        assert_eq!(d.visibility_m, Some(16000.0));
        assert!((d.pressure_hpa.unwrap() - 1013.0).abs() < 1e-9);
    }

    #[test]
    fn parse_noaa_grid_omits_absent_fields() {
        let body = json!({ "properties": { "temperature": { "uom": "wmoUnit:degC", "values": [{ "value": 1.0 }] } } });
        let d = parse_noaa_grid(&body);
        assert_eq!(d.temperature_c, Some(1.0));
        assert_eq!(d.pressure_hpa, None);
        assert_eq!(d.visibility_m, None);
        // No pressure anywhere → null.
        let body2 = json!({ "properties": {} });
        assert_eq!(parse_noaa_grid(&body2), WeatherData::default());
    }

    #[test]
    fn parse_open_meteo_maps_units() {
        let body = json!({
            "current": {
                "temperature_2m": 18.5,
                "wind_speed_10m": 36.0,
                "wind_direction_10m": 200.0,
                "precipitation": 0.2,
                "cloud_cover": 70.0,
                "visibility": 25000.0,
                "surface_pressure": 1013.2
            }
        });
        let d = parse_open_meteo(&body).expect("current block present");
        assert_eq!(d.temperature_c, Some(18.5));
        assert!((d.wind_speed_kts.unwrap() - 36.0 * KMH_TO_KTS).abs() < 1e-9);
        assert_eq!(d.wind_direction_deg, Some(200.0));
        assert_eq!(d.precipitation_mm, Some(0.2));
        assert_eq!(d.cloud_cover_pct, Some(70.0));
        assert_eq!(d.visibility_m, Some(25000.0));
        assert_eq!(d.pressure_hpa, Some(1013.2));
        assert!(parse_open_meteo(&json!({})).is_none());
    }

    #[test]
    fn weather_response_shape_has_all_seven_fields() {
        let d = WeatherData::default();
        let v = weather_response(WeatherSource::Noaa, &d, "2026-09-18T00:00:00+00:00");
        assert_eq!(v["source"], "noaa");
        assert_eq!(v["fetched_at"], "2026-09-18T00:00:00+00:00");
        for field in [
            "temperature_c",
            "wind_speed_kts",
            "wind_direction_deg",
            "precipitation_mm",
            "cloud_cover_pct",
            "visibility_m",
            "pressure_hpa",
        ] {
            assert!(v.get(field).is_some(), "missing field {field}");
            assert!(v[field].is_null(), "{field} should be null for default");
        }
    }
}
