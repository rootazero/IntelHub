//! OpenAQ global air quality (https://openaq.org/ — free for non-commercial
//! use, requires API key signup at https://explore.openaq.org/register).
//! Phase 1.3 of the public-API integration roadmap (`docs/superpowers/
//! roadmaps/2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## API shape (verified 2026-09-27 against live OpenAQ v3)
//!
//! The `/v3/latest?locations_id=X` path documented in some tutorials does
//! NOT exist — verified by 404 against the live OpenAPI spec
//! (https://api.openaq.org/openapi.json). The actual three-step flow:
//!
//!   1. `GET /v3/locations?coordinates={lat},{lon}&radius=25000&limit=1`
//!      → resolve `{lat,lon}` to the nearest `location_id`. The
//!      `?city=X&country=Y` filter is unreliable (returns fuzzy matches
//!      from unrelated regions — observed Accra, Ghana results when
//!      searching Beijing). Coordinates-based radius search is the only
//!      deterministic path.
//!
//!   2. `GET /v3/locations/{id}` → get the location's sensors[] with
//!      each sensor's `parameter` + `units`. Build a sensor_id →
//!      parameter map for value interpretation. (The `/latest` endpoint
//!      doesn't include parameter metadata — verified empty.)
//!
//!   3. `GET /v3/locations/{id}/latest` → array of {value, datetime,
//!      coordinates, sensorsId} entries. One entry per active sensor
//!      at the location.
//!
//! ## Data freshness reality
//!
//! OpenAQ v3's "latest" varies wildly by location:
//!   - SF US Embassy (loc 2009):  TODAY (EPA AirNow live)
//!   - Lagos (loc 404479):        ~7 months stale
//!   - Shanghai (loc 143):        ~3 years stale + sentinel -9999
//!   - Beijing US Embassy (21):   ~3 years stale
//!   - New Delhi (loc 13):        ~8 years stale
//!
//! Some sensors are permanently offline (sentinel -9999); some locations
//! haven't reported since 2018. This is the honest OSINT reality of
//! OpenAQ — we surface what is current, silently skip what isn't, and
//! don't pretend stale data is fresh. A `max_age_hours` guard (default
//! 7 days) prevents emitting "current air-quality alert" on decade-old
//! measurements.
//!
//! ## Units
//!
//! OpenAQ v3 reports gases (co, no, no2, nox, o3, so2) in ppm and
//! particles (pm25, pm10) in µg/m³. The thresholds below match the
//! unit OpenAQ actually returns — no conversion math needed.
//!
//! ## Cadence, isolation, severity, external_id
//!
//! - 30 min (matches Open-Meteo)
//! - Per-city isolation: one bad location ≠ source failure
//! - Three tiers: WHO-guideline-bounded multipliers
//! - external_id = `"{city_slug}:{reading_date}:{pollutant}_{kind}"`
//!   → idempotent across 30-min cadence (geo_events dedups)
//!
//! ## Auth
//!
//! Env-gated via `api_key()`: HUB_OPENAQ_API_KEY wins, OPENAQ_API_KEY
//! fallback, empty values fall through. Missing → source NOT registered
//! (AIS / opensky precedent); sp6 reports `shelved-by-design`.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.openaq.org/v3";

/// Max age for a reading to be considered "current enough" to emit.
/// OpenAQ's stale-data reality (some locations years behind) means we
/// must not surface decade-old readings as live alerts. 7 days is a
/// pragmatic ceiling: covers weekend gaps, normal sensor downtime, and
/// pipeline delays without diluting freshness to "whatever's archived".
const MAX_AGE_HOURS: i64 = 7 * 24;

/// Sentinel values OpenAQ uses for failed / offline sensors. Verified
/// against the live API: Shanghai's pm25 returns -9999; some others
/// return -999. We skip any value ≤ -9000 to be safe.
const SENTINEL_THRESHOLD: f64 = -9000.0;

/// Pollutant thresholds in the **units OpenAQ actually returns**:
/// gases in ppm, particles in µg/m³. Values verified against the live
/// API for SF (2026-09-27) which reported pm25=9 µg/m³, o3=0.026 ppm,
/// no2=0.0021 ppm — all well below our routine thresholds, so SF on a
/// clean day emits no signals. Hazard thresholds are roughly 5× WHO 24h
/// guidelines converted to those units; routine is roughly 2×.
///
/// `param_name` matches OpenAQ's `parameter.name` field exactly.
/// To add a new pollutant: append here + add to the watchlist (no code
/// change to the parser — it walks POLLUTANT_THRESHOLDS dynamically).
const POLLUTANT_THRESHOLDS: &[(&str, f64, f64)] = &[
    // (param_name, routine_threshold, priority_threshold)
    ("pm25", 30.0, 75.0),    // µg/m³ — WHO 24h=15; 2×=30, 5×=75
    ("pm10", 90.0, 225.0),   // µg/m³ — WHO 24h=45; 2×=90, 5×=225
    ("no2", 0.05, 0.125),    // ppm — WHO 24h≈25 µg/m³ ≈ 0.011 ppm; 5×≈0.05, 25×≈0.125
    ("so2", 0.013, 0.040),   // ppm — WHO 24h=40 µg/m³ ≈ 0.013 ppm; 3× ≈ 0.040
    ("o3",  0.050, 0.150),   // ppm — 8h peak ≈ 0.050 ppm, 3× ≈ 0.150
    ("co",  4.0,  9.0),      // ppm — WHO 24h ≈ 4 ppm; ~2× ≈ 9 ppm
];

/// 30 OSINT strategic cities. (lat, lon, city, country_code, slug).
/// Slug becomes part of `external_id`. Coordinates-based lookup is
/// used because city+country search returns fuzzy unrelated matches.
const CITIES: &[(f64, f64, &str, &str, &str)] = &[
    // ── East / South Asia (highest air-pollution health burden) ──
    (39.9042, 116.4074, "Beijing", "CN", "beijing"),
    (31.2304, 121.4737, "Shanghai", "CN", "shanghai"),
    (28.6139, 77.2090, "New Delhi", "IN", "new_delhi"),
    (19.0760, 72.8777, "Mumbai", "IN", "mumbai"),
    (23.8103, 90.4125, "Dhaka", "BD", "dhaka"),
    (24.8607, 67.0011, "Karachi", "PK", "karachi"),
    (13.7563, 100.5018, "Bangkok", "TH", "bangkok"),
    (14.5995, 120.9842, "Manila", "PH", "manila"),
    (21.0285, 105.8542, "Hanoi", "VN", "hanoi"),
    // ── Middle East / North Africa (dust + industrial) ──
    (30.0444, 31.2357, "Cairo", "EG", "cairo"),
    (24.7136, 46.6753, "Riyadh", "SA", "riyadh"),
    (25.2048, 55.2708, "Dubai", "AE", "dubai"),
    (35.6892, 51.3890, "Tehran", "IR", "tehran"),
    (33.8938, 35.5018, "Beirut", "LB", "beirut"),
    // ── Sub-Saharan Africa ──
    (6.5244, 3.3792, "Lagos", "NG", "lagos"),
    (-1.2921, 36.8219, "Nairobi", "KE", "nairobi"),
    (-26.2041, 28.0473, "Johannesburg", "ZA", "johannesburg"),
    // ── Europe ──
    (52.5200, 13.4050, "Berlin", "DE", "berlin"),
    (48.8566, 2.3522, "Paris", "FR", "paris"),
    (51.5074, -0.1278, "London", "GB", "london"),
    (41.9028, 12.4964, "Rome", "IT", "rome"),
    (40.4168, -3.7038, "Madrid", "ES", "madrid"),
    (50.4501, 30.5234, "Kyiv", "UA", "kyiv"),
    (55.7558, 37.6173, "Moscow", "RU", "moscow"),
    // ── Americas ──
    (19.4326, -99.1332, "Mexico City", "MX", "mexico_city"),
    (-23.5505, -46.6333, "São Paulo", "BR", "sao_paulo"),
    (37.7749, -122.4194, "San Francisco", "US", "san_francisco"),
    (34.0522, -118.2437, "Los Angeles", "US", "los_angeles"),
    // ── Oceania ──
    (-33.8688, 151.2093, "Sydney", "AU", "sydney"),
];

/// Process-wide location_id cache + sensor metadata cache. Populated
/// lazily on first sweep with a valid API key; reused thereafter.
/// Restarts pay a one-time 2×30 = 60-request bootstrap cost (well
/// within OpenAQ's free-tier 2000 req/hour).
///
/// Two caches:
/// - `id_cache`: `{city_slug}` → `location_id` (per-city)
/// - `sensors_cache`: `{location_id}` → `{sensor_id: parameter_name}`
///
/// `OnceLock<Mutex<HashMap>>` is the safe pattern for a single-writer
/// multi-reader cache. We scope the MutexGuard to a block so it drops
/// BEFORE any .await — std::sync::MutexGuard is !Send, and holding it
/// across an await point poisons the future.
fn id_cache() -> &'static Mutex<HashMap<String, u32>> {
    static CACHE: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}
fn sensors_cache() -> &'static Mutex<HashMap<u32, HashMap<u32, String>>> {
    static CACHE: OnceLock<Mutex<HashMap<u32, HashMap<u32, String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve the OpenAQ API key with the standard HUB_/bare priority
/// pattern (mirrors `opensky.rs` + `ais.rs` precedent).
pub(crate) fn api_key() -> Option<String> {
    let hub = std::env::var("HUB_OPENAQ_API_KEY").ok().filter(|s| !s.is_empty());
    let bare = std::env::var("OPENAQ_API_KEY").ok().filter(|s| !s.is_empty());
    hub.or(bare)
}

pub struct OpenAq;

impl Source for OpenAq {
    fn name(&self) -> &'static str {
        "open_aq"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(1800) // 30 min — roadmap §2.1
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(key) = api_key() else {
                // No key → graceful degrade. Per-city loops never execute.
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for &(lat, lon, city, _country, slug) in CITIES {
                match fetch_city(ctx, &key, lat, lon, city, slug).await {
                    Ok(sigs) => out.extend(sigs),
                    Err(e) => {
                        tracing::warn!(city = slug, error = %e, "open_aq city failed");
                        // continue — per-city isolation (open_meteo.rs precedent)
                    }
                }
            }
            Ok(out)
        }
        .boxed()
    }
}

/// One city's full sweep: resolve location_id (cache-first) + fetch
/// location metadata for sensor→parameter map (cache-first) + fetch
/// latest values + emit Signals for hazardous readings.
async fn fetch_city(
    ctx: &Ctx,
    key: &str,
    lat: f64,
    lon: f64,
    city: &str,
    slug: &str,
) -> Result<Vec<Signal>> {
    // Step 1: resolve location_id (cache-first)
    let cached_id = {
        let cache = id_cache().lock().unwrap();
        cache.get(slug).copied()
    };
    let loc_id = match cached_id {
        Some(id) => id,
        None => resolve_location_id(ctx, key, lat, lon, slug).await?,
    };

    // Step 2: sensor metadata (cache-first) — needed to interpret the
    // bare sensor_id + value pairs the /latest endpoint returns.
    let cached_sensors = {
        let cache = sensors_cache().lock().unwrap();
        cache.get(&loc_id).cloned()
    };
    let sensor_params = match cached_sensors {
        Some(m) => m,
        None => resolve_sensors(ctx, key, loc_id).await?,
    };

    // Step 3: latest values
    let url = format!("{BASE_URL}/locations/{loc_id}/latest");
    let resp = ctx.http.get(&url).header("X-API-Key", key).send().await?;
    if !resp.status().is_success() {
        tracing::warn!(city = slug, status = %resp.status(), "open_aq latest non-2xx");
        return Ok(Vec::new());
    }
    let body: serde_json::Value = resp.json().await?;
    Ok(parse_latest(&body, slug, city, lat, lon, &sensor_params))
}

/// Resolve `{lat, lon}` → nearest OpenAQ `location_id` via coords+radius.
/// Caches result; subsequent calls are O(1).
async fn resolve_location_id(
    ctx: &Ctx,
    key: &str,
    lat: f64,
    lon: f64,
    slug: &str,
) -> Result<u32> {
    let url = format!(
        "{BASE_URL}/locations?coordinates={lat},{lon}&radius=25000&limit=1"
    );
    let resp = ctx.http.get(&url).header("X-API-Key", key).send().await?;
    if !resp.status().is_success() {
        return Err(crate::error::HubError::sensor(format!(
            "open_aq resolve failed for {slug}: {}",
            resp.status()
        )));
    }
    let body: serde_json::Value = resp.json().await?;
    let id = body
        .pointer("/results/0/id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            crate::error::HubError::sensor(format!(
                "open_aq resolve empty result for {slug} (no sensor within 25km)"
            ))
        })? as u32;
    id_cache().lock().unwrap().insert(slug.to_string(), id);
    tracing::info!(slug, id, "open_aq location_id resolved");
    Ok(id)
}

/// Fetch sensor metadata for a location. Returns `sensor_id → param_name`
/// map (e.g., `{3569: "pm25", 25673: "no2"}`). Cached for the process
/// lifetime.
async fn resolve_sensors(
    ctx: &Ctx,
    key: &str,
    loc_id: u32,
) -> Result<HashMap<u32, String>> {
    let url = format!("{BASE_URL}/locations/{loc_id}");
    let resp = ctx.http.get(&url).header("X-API-Key", key).send().await?;
    if !resp.status().is_success() {
        return Err(crate::error::HubError::sensor(format!(
            "open_aq sensors fetch failed for {loc_id}: {}",
            resp.status()
        )));
    }
    let body: serde_json::Value = resp.json().await?;
    let mut map = HashMap::new();
    if let Some(sensors) = body
        .pointer("/results/0/sensors")
        .and_then(|v| v.as_array())
    {
        for s in sensors {
            if let (Some(id), Some(param)) = (
                s.get("id").and_then(|v| v.as_u64()),
                s.pointer("/parameter/name").and_then(|v| v.as_str()),
            ) {
                map.insert(id as u32, param.to_string());
            }
        }
    }
    sensors_cache().lock().unwrap().insert(loc_id, map.clone());
    Ok(map)
}

/// Pure parser: walk OpenAQ's `/v3/locations/{id}/latest` response and
/// emit 0..N Signals based on each value's severity ladder. Filters:
/// - sensor_id not in metadata map → unknown sensor, skip
/// - value ≤ SENTINEL_THRESHOLD → offline sensor, skip
/// - param not in POLLUTANT_THRESHOLDS → unknown pollutant, skip
/// - reading older than MAX_AGE_HOURS → stale data, skip (with debug log)
/// - value below routine threshold → no signal
fn parse_latest(
    j: &serde_json::Value,
    slug: &str,
    city: &str,
    lat: f64,
    lon: f64,
    sensor_params: &HashMap<u32, String>,
) -> Vec<Signal> {
    let mut out = Vec::new();
    let Some(results) = j.get("results").and_then(|v| v.as_array()) else {
        return out;
    };
    let now = chrono::Utc::now();
    for item in results {
        let Some(sensor_id) = item.get("sensorsId").and_then(|v| v.as_u64()) else {
            continue;
        };
        let Some(value) = item.get("value").and_then(|v| v.as_f64()) else {
            continue;
        };
        if value <= SENTINEL_THRESHOLD {
            // Sentinel — sensor offline or failed. Skip silently.
            continue;
        }
        let Some(param) = sensor_params.get(&(sensor_id as u32)) else {
            continue;
        };
        let Some(&(_, routine, priority)) = POLLUTANT_THRESHOLDS
            .iter()
            .find(|(name, _, _)| name == param)
        else {
            continue;
        };
        // Parse reading datetime; if missing or stale, skip with debug log.
        let date = item
            .get("datetime")
            .and_then(|d| d.get("utc"))
            .and_then(|v| v.as_str())
            .and_then(parse_iso_date);
        let Some(dt) = date else {
            tracing::debug!(city = slug, sensor_id, "open_aq reading missing/unparseable datetime");
            continue;
        };
        let age_hours = (now - dt).num_hours();
        if age_hours > MAX_AGE_HOURS {
            tracing::debug!(
                city = slug,
                sensor_id,
                age_h = age_hours,
                "open_aq reading too stale; skipping"
            );
            continue;
        }
        let date_str = dt.format("%Y-%m-%d").to_string();
        let (severity, kind, threshold) = if value >= priority {
            ("priority", "hazard", priority)
        } else if value >= routine {
            ("routine", "warn", routine)
        } else {
            continue;
        };
        out.push(
            Signal::new(
                "air_quality",
                format!("{city} — {param} {severity} ({value:.3} ≥ {threshold})"),
                lat,
                lon,
                format!("{slug}:{date_str}:{param}_{kind}"),
            )
            .severity(severity)
            .payload(serde_json::json!({
                "city_slug": slug,
                "city": city,
                "pollutant": param,
                "value": value,
                "threshold": threshold,
                "extreme_type": format!("{param}_{kind}"),
                "reading_date": date_str,
                "age_hours": age_hours,
            })),
        );
    }
    out
}

/// Parse OpenAQ's ISO 8601 datetime string into a `DateTime<Utc>`.
/// Returns None for unparseable strings (caller skips).
fn parse_iso_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sensor_map() -> HashMap<u32, String> {
        let mut m = HashMap::new();
        m.insert(3569, "pm25".to_string());
        m.insert(25673, "no2".to_string());
        m.insert(25672, "co".to_string());
        m.insert(3570, "o3".to_string());
        m
    }

    fn sample_response_with_date(date_str: &str) -> serde_json::Value {
        serde_json::json!({
            "results": [
                {"sensorsId": 3569,  "value": 80.0,  "datetime": {"utc": date_str}},  // pm25 priority
                {"sensorsId": 25673, "value": 0.06,  "datetime": {"utc": date_str}},  // no2 routine
                {"sensorsId": 25672, "value": 30.0,  "datetime": {"utc": date_str}},  // sentinel skip
                {"sensorsId": 99999, "value": 50.0,  "datetime": {"utc": date_str}},  // unknown sensor skip
                {"sensorsId": 3570,  "value": 0.020, "datetime": {"utc": date_str}}   // o3 below threshold skip
            ]
        })
    }

    /// Happy path: 5 entries → 3 signals (pm25 priority + no2 routine + co priority).
    #[test]
    fn detects_priority_routine_skips_others() {
        let date_str = "2026-09-27T10:00:00+00:00";
        let sigs = parse_latest(
            &sample_response_with_date(date_str),
            "san_francisco",
            "San Francisco",
            37.77,
            -122.42,
            &sensor_map(),
        );
        assert_eq!(sigs.len(), 3, "expected 3 signals, got len={}", sigs.len());
        let priority: Vec<&Signal> = sigs.iter().filter(|s| s.severity == "priority").collect();
        let routine: Vec<&Signal> = sigs.iter().filter(|s| s.severity == "routine").collect();
        assert_eq!(priority.len(), 2);  // pm25 + co
        assert_eq!(routine.len(), 1);   // no2
        assert!(priority.iter().any(|s| s.title.contains("pm25")));
        assert!(priority.iter().any(|s| s.title.contains("co")));
        assert!(routine[0].title.contains("no2"));
    }

    /// external_id format = `{slug}:{date}:{pollutant}_{kind}`.
    #[test]
    fn external_id_shape() {
        let date_str = "2026-09-27T10:00:00+00:00";
        let sigs = parse_latest(
            &sample_response_with_date(date_str),
            "san_francisco",
            "San Francisco",
            0.0, 0.0,
            &sensor_map(),
        );
        assert_eq!(sigs[0].external_id, "san_francisco:2026-09-27:pm25_hazard");
        assert_eq!(sigs[1].external_id, "san_francisco:2026-09-27:no2_warn");
    }

    /// Sentinel values (≤ -9000) are silently skipped.
    #[test]
    fn sentinel_values_skipped() {
        let date_str = "2026-09-27T10:00:00+00:00";
        let j = serde_json::json!({
            "results": [
                {"sensorsId": 3569, "value": -9999.0, "datetime": {"utc": date_str}}, // -9999
                {"sensorsId": 3569, "value": -999.0,  "datetime": {"utc": date_str}}, // -999
                {"sensorsId": 3569, "value": -10000.0, "datetime": {"utc": date_str}}, // below threshold
                {"sensorsId": 25672, "value": 4.5, "datetime": {"utc": date_str}} // co routine
            ]
        });
        let sigs = parse_latest(&j, "x", "X", 0.0, 0.0, &sensor_map());
        assert_eq!(sigs.len(), 1);
        assert!(sigs[0].title.contains("co"));
    }

    /// Stale readings (> MAX_AGE_HOURS) are skipped.
    #[test]
    fn stale_readings_skipped() {
        // 30 days ago
        let old_date = (chrono::Utc::now() - chrono::Duration::days(30))
            .format("%Y-%m-%dT%H:%M:%S+00:00")
            .to_string();
        let j = serde_json::json!({
            "results": [
                {"sensorsId": 3569, "value": 80.0, "datetime": {"utc": old_date}}
            ]
        });
        let sigs = parse_latest(&j, "x", "X", 0.0, 0.0, &sensor_map());
        assert!(sigs.is_empty(), "stale reading should not emit");
    }

    /// Boundary: pm25 exactly at priority threshold fires (inclusive).
    #[test]
    fn threshold_inclusive() {
        let date_str = "2026-09-27T10:00:00+00:00";
        let j = serde_json::json!({
            "results": [{"sensorsId": 3569, "value": 75.0, "datetime": {"utc": date_str}}]
        });
        let sigs = parse_latest(&j, "x", "X", 0.0, 0.0, &sensor_map());
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "priority");
    }

    /// Defensive: missing results array → zero signals, no panic.
    #[test]
    fn empty_results_returns_zero_signals() {
        let j = serde_json::json!({"results": []});
        assert!(parse_latest(&j, "x", "X", 0.0, 0.0, &sensor_map()).is_empty());
    }

    /// Defensive: missing `results` key → zero signals, no panic.
    #[test]
    fn missing_results_key() {
        assert!(parse_latest(&serde_json::json!({}), "x", "X", 0.0, 0.0, &sensor_map()).is_empty());
    }

    /// Defensive: unparseable datetime → silently skipped.
    #[test]
    fn unparseable_datetime_skipped() {
        let j = serde_json::json!({
            "results": [
                {"sensorsId": 3569, "value": 80.0, "datetime": {"utc": "not-a-date"}},
                {"sensorsId": 3569, "value": 80.0}  // no datetime
            ]
        });
        let sigs = parse_latest(&j, "x", "X", 0.0, 0.0, &sensor_map());
        assert!(sigs.is_empty());
    }

    /// Unknown sensor (not in metadata map) → skipped.
    #[test]
    fn unknown_sensor_id_skipped() {
        let date_str = "2026-09-27T10:00:00+00:00";
        let j = serde_json::json!({
            "results": [{"sensorsId": 12345, "value": 100.0, "datetime": {"utc": date_str}}]
        });
        let sigs = parse_latest(&j, "x", "X", 0.0, 0.0, &sensor_map());
        assert!(sigs.is_empty());
    }

    /// Unknown pollutant (in metadata but not in ladder) → skipped.
    #[test]
    fn unknown_pollutant_skipped() {
        let mut m = sensor_map();
        m.insert(99999, "radon".to_string());
        let date_str = "2026-09-27T10:00:00+00:00";
        let j = serde_json::json!({
            "results": [{"sensorsId": 99999, "value": 999.0, "datetime": {"utc": date_str}}]
        });
        let sigs = parse_latest(&j, "x", "X", 0.0, 0.0, &m);
        assert!(sigs.is_empty());
    }

    /// api_key() helper: HUB_ wins, empty HUB_ falls through, both empty → None.
    #[test]
    fn api_key_resolution() {
        std::env::set_var("HUB_OPENAQ_API_KEY", "hub-key");
        std::env::set_var("OPENAQ_API_KEY", "bare-key");
        assert_eq!(api_key().as_deref(), Some("hub-key"));

        std::env::set_var("HUB_OPENAQ_API_KEY", "");
        assert_eq!(api_key().as_deref(), Some("bare-key"));

        std::env::set_var("OPENAQ_API_KEY", "");
        assert_eq!(api_key(), None);

        std::env::remove_var("HUB_OPENAQ_API_KEY");
        std::env::remove_var("OPENAQ_API_KEY");
    }

    /// Watchlist + ladder size guards: drift is a regression.
    #[test]
    fn watchlist_and_ladder_size() {
        assert!(CITIES.len() >= 20, "watchlist too small: {}", CITIES.len());
        assert!(CITIES.len() <= 40, "watchlist too large: {}", CITIES.len());
        assert!(POLLUTANT_THRESHOLDS.len() >= 4, "ladder too small");
        for &(name, r, p) in POLLUTANT_THRESHOLDS {
            assert!(r > 0.0 && r < p, "{name} routine ({r}) must be < priority ({p})");
        }
    }

    /// Parse ISO date handles both `+00:00` and `Z` formats.
    #[test]
    fn parse_iso_date_formats() {
        assert!(parse_iso_date("2026-09-27T10:00:00+00:00").is_some());
        assert!(parse_iso_date("2026-09-27T10:00:00Z").is_some());
        assert!(parse_iso_date("2026-09-27T10:00:00-05:00").is_some());
        assert!(parse_iso_date("not-a-date").is_none());
        let dt = parse_iso_date("2026-09-27T10:00:00Z").unwrap();
        assert_eq!(
            dt,
            chrono::Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
        );
    }
}