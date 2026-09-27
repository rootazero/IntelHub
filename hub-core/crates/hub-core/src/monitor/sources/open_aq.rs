//! OpenAQ global air quality (https://openaq.org/ — free for non-commercial
//! use, requires API key signup at https://explore.openaq.org/register).
//! Phase 1.3 of the public-API integration roadmap (`docs/superpowers/
//! roadmaps/2026-09-27-public-api-integration-roadmap.md`).
//!
//! Why dynamic location_id resolution (not hardcoded IDs): OpenAQ's
//! internal location IDs are subject to re-numbering when sensors are
//! re-cataloged. Hardcoding them creates ongoing maintenance debt for a
//! data layer that's not IntelHub-specific. Instead, on first sweep with
//! a valid API key we resolve `{city, country}` → `location_id` via
//! `GET /v3/locations?city=X&country=Y&limit=1` and cache in a process-
//! wide `OnceLock<HashMap>` (TTL = process lifetime; hub-core restart
//! pays a one-time 30-request bootstrap). Subsequent sweeps reuse the
//! cache and only issue the per-city `/v3/latest` call.
//!
//! Cadence: 30 min. Per-city sequential fetch (mirrors `open_meteo.rs`
//! + `firms.rs` precedent) — 30 cities × 2 requests (resolve + latest)
//! on first sweep ≈ 6s; subsequent sweeps ≈ 3s. Both well under the
//! 25s per-source ctx timeout and the 74-source stampede ceiling.
//!
//! Per-city isolation: one bad city ≠ source failure (matched pattern).
//!
//! Severity ladder (simplified WHO 2021 guideline; 24-hour-mean PM2.5
//! reference = 15 µg/m³, PM10 = 45 µg/m³, NO2 = 25 µg/m³, SO2 = 40 µg/m³,
//! O3 = 60 µg/m³ — we use ~5× these as priority thresholds so a single
//! peak reading can wake an alert; routine threshold = ~2× guideline):
//!   value >= 5× guideline  → priority + extreme_type={pollutant}_hazard
//!   value >= 2× guideline  → routine  + extreme_type={pollutant}_warn
//!   otherwise              → no signal
//!
//! external_id = `"{city_slug}:{reading_date}:{extreme_type}"` — fixed
//! per (city, date, pollutant+kind). Idempotent re-ingest across the
//! 30-min cadence: same city + same reading date + same kind = same id,
//! geo_events silently dedups.
//!
//! Auth: free API key (signup at https://explore.openaq.org/register,
//! free tier = 60 req/min, 2000 req/hour). Env-gated via `api_key()` —
//! `HUB_OPENAQ_API_KEY` wins, bare `OPENAQ_API_KEY` fallback, missing
//! → source returns 0 signals + sp6 reports `shelved`. The source is
//! NOT registered in `monitor::registry()` when the key is absent
//! (AIS / opensky env-gated precedent).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.openaq.org/v3";

/// WHO 2021 24h-mean guideline × multipliers for severity ladder.
/// Format: (param_name, guideline, routine_mult, priority_mult)
/// result_µg_m³ at routine = guideline * 2, priority = guideline * 5.
const POLLUTANT_THRESHOLDS: &[(&str, f64, f64, f64)] = &[
    ("pm25", 15.0, 2.0, 5.0),
    ("pm10", 45.0, 2.0, 5.0),
    ("no2", 25.0, 2.0, 5.0),
    ("so2", 40.0, 2.0, 5.0),
    ("o3", 60.0, 2.0, 5.0),
];

/// 30 OSINT strategic cities. (lat, lon, city, country_code, slug).
/// Slug becomes part of `external_id` so re-enumerated by accident are
/// caught in code review. OpenAQ's own `/v3/locations?city=X&country=Y`
/// endpoint resolves these to their internal `location_id`s on first
/// sweep with a valid API key.
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
    (40.7128, -74.0060, "New York", "US", "new_york"),
    (34.0522, -118.2437, "Los Angeles", "US", "los_angeles"),
    // ── Oceania ──
    (-33.8688, 151.2093, "Sydney", "AU", "sydney"),
];

/// Process-wide location_id cache. Populated lazily on first sweep with
/// a valid API key; reused thereafter. Restarts pay a one-time 30-req
/// bootstrap cost (well within OpenAQ's free-tier 2000 req/hour).
///
/// `OnceLock<Mutex<HashMap>>` is the safe pattern for a single-writer
/// multi-reader cache: writers hold the mutex briefly during a sweep,
/// readers do the same. No contention because the cache only changes
/// when a new city is added or OpenAQ re-numbers a sensor (rare).
fn id_cache() -> &'static Mutex<HashMap<String, u32>> {
    static CACHE: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve the OpenAQ API key with the standard HUB_/bare priority
/// pattern (mirrors `opensky.rs` + `ais.rs` precedent). Empty values
/// fall through (the same pattern every other env-gated source uses —
/// see `noaa.rs` precedent in tests).
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
                // No key → graceful degrade, zero output. The sp6 shelved
                // check below picks this up as "shelved by design", not
                // a regression. Per-city loops below never execute.
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for &(lat, lon, city, country, slug) in CITIES {
                match fetch_city(ctx, &key, city, country, slug, lat, lon).await {
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
/// latest readings + emit Signals for hazardous values. Errors are
/// returned to the caller for logging; per-city `continue` is in fetch().
async fn fetch_city(
    ctx: &Ctx,
    key: &str,
    city: &str,
    country: &str,
    slug: &str,
    lat: f64,
    lon: f64,
) -> Result<Vec<Signal>> {
    // Scope the MutexGuard to a block so it drops BEFORE the .await on
    // cache miss. std::sync::MutexGuard is !Send; holding it across an
    // await point poisons the future. (Tokio's Mutex would be Send but
    // adds an async dependency we don't need for an in-process cache.)
    let cached_id = {
        let cache = id_cache().lock().unwrap();
        cache.get(slug).copied()
    };
    let loc_id = match cached_id {
        Some(id) => id,
        None => resolve_location_id(ctx, key, city, country).await?,
    };
    let url = format!("{BASE_URL}/latest?locations_id={loc_id}");
    let resp = ctx.http.get(&url).header("X-API-Key", key).send().await?;
    if !resp.status().is_success() {
        tracing::warn!(city = slug, status = %resp.status(), "open_aq latest non-2xx");
        return Ok(Vec::new());
    }
    let body: serde_json::Value = resp.json().await?;
    Ok(parse_latest(&body, slug, city, lat, lon))
}

/// Resolve `{city, country}` → first matching OpenAQ `location_id`.
/// Caches result in the process-wide map; subsequent calls are O(1).
async fn resolve_location_id(
    ctx: &Ctx,
    key: &str,
    city: &str,
    country: &str,
) -> Result<u32> {
    let url = format!("{BASE_URL}/locations?city={city}&country={country}&limit=1");
    let resp = ctx.http.get(&url).header("X-API-Key", key).send().await?;
    if !resp.status().is_success() {
        tracing::warn!(city, status = %resp.status(), "open_aq resolve non-2xx");
        // Don't cache failures — let next sweep retry.
        return Err(crate::error::HubError::sensor(format!(
            "open_aq resolve failed for {city}/{country}: {}",
            resp.status()
        )));
    }
    let body: serde_json::Value = resp.json().await?;
    let id = body
        .pointer("/results/0/id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            crate::error::HubError::sensor(format!(
                "open_aq resolve empty result for {city}/{country}"
            ))
        })? as u32;
    id_cache().lock().unwrap().insert(city.to_string(), id);
    tracing::info!(city, country, id, "open_aq location_id resolved");
    Ok(id)
}

/// Pure parser: walk OpenAQ's `/v3/latest` response (array of
/// measurement dicts, one per pollutant) and emit 0..N Signals based on
/// each pollutant's severity ladder. Defensive against missing fields
/// and unknown pollutant names.
fn parse_latest(
    j: &serde_json::Value,
    slug: &str,
    city: &str,
    lat: f64,
    lon: f64,
) -> Vec<Signal> {
    let mut out = Vec::new();
    let Some(results) = j.get("results").and_then(|v| v.as_array()) else {
        return out;
    };
    // /v3/latest returns a flat list — each item has `parameters` array
    // (one entry per pollutant). Iterate outer→inner to keep the parser
    // simple: we only care about the per-pollutant latest values.
    for item in results {
        let Some(parameters) = item.get("parameters").and_then(|v| v.as_array()) else {
            continue;
        };
        let date = item
            .get("datetime")
            .and_then(|d| d.get("utc"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.get(..10))
            .unwrap_or("0000-00-00");
        for param in parameters {
            let Some(param_id) = param.get("parameter").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(value) = param.get("value").and_then(|v| v.as_f64()) else {
                continue;
            };
            // Find matching threshold ladder
            let Some(&(_, guideline, routine_mult, priority_mult)) = POLLUTANT_THRESHOLDS
                .iter()
                .find(|(name, _, _, _)| *name == param_id)
            else {
                continue; // unknown pollutant — skip silently
            };
            let routine_threshold = guideline * routine_mult;
            let priority_threshold = guideline * priority_mult;
            let (severity, kind, threshold) = if value >= priority_threshold {
                ("priority", "hazard", priority_threshold)
            } else if value >= routine_threshold {
                ("routine", "warn", routine_threshold)
            } else {
                continue;
            };
            out.push(
                Signal::new(
                    "air_quality",
                    format!("{city} — {param_id} {severity} ({value:.1} ≥ {threshold:.0} guideline×{priority_mult:.0})"),
                    lat,
                    lon,
                    format!("{slug}:{date}:{param_id}_{kind}"),
                )
                .severity(severity)
                .payload(serde_json::json!({
                    "city_slug": slug,
                    "city": city,
                    "pollutant": param_id,
                    "value": value,
                    "guideline": guideline,
                    "threshold": threshold,
                    "extreme_type": format!("{param_id}_{kind}"),
                    "reading_date": date,
                })),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_latest_response() -> serde_json::Value {
        serde_json::json!({
            "results": [{
                "location_id": 8118,
                "datetime": {"utc": "2026-09-27T10:00:00.000Z"},
                "parameters": [
                    {"parameter": "pm25", "value": 80.0},   // ≥ 75 (priority)
                    {"parameter": "pm10", "value": 100.0},  // ≥ 90 (routine)
                    {"parameter": "no2",  "value": 30.0},   // < 50 (no signal)
                    {"parameter": "so2",  "value": 0.0},    // (no signal)
                    {"parameter": "co",   "value": 300.0},   // unknown — skip
                ]
            }]
        })
    }

    /// Happy path: pm25 (priority) + pm10 (routine) fire; no2/so2/co silent.
    #[test]
    fn detects_priority_and_routine() {
        let sigs = parse_latest(&sample_latest_response(), "delhi", "Delhi", 28.62, 77.21);
        assert_eq!(sigs.len(), 2, "expected 2 signals, got len={}", sigs.len());
        let priority: Vec<&Signal> = sigs.iter().filter(|s| s.severity == "priority").collect();
        let routine: Vec<&Signal> = sigs.iter().filter(|s| s.severity == "routine").collect();
        assert_eq!(priority.len(), 1);
        assert_eq!(routine.len(), 1);
        assert!(priority[0].title.contains("pm25"));
        assert!(routine[0].title.contains("pm10"));
    }

    /// external_id format = `{slug}:{date}:{pollutant}_{kind}`.
    #[test]
    fn external_id_shape() {
        let sigs = parse_latest(&sample_latest_response(), "delhi", "Delhi", 0.0, 0.0);
        assert_eq!(sigs[0].external_id, "delhi:2026-09-27:pm25_hazard");
        assert_eq!(sigs[1].external_id, "delhi:2026-09-27:pm10_warn");
    }

    /// Boundary: pm25 exactly at priority threshold fires.
    #[test]
    fn threshold_inclusive() {
        let j = serde_json::json!({
            "results": [{
                "datetime": {"utc": "2026-09-27T10:00:00.000Z"},
                "parameters": [{"parameter": "pm25", "value": 75.0}] // exactly 5×15
            }]
        });
        let sigs = parse_latest(&j, "x", "X", 0.0, 0.0);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "priority");
    }

    /// Threshold exclusivity: 0.1 below priority fires routine (not priority).
    #[test]
    fn just_below_priority_is_routine() {
        let j = serde_json::json!({
            "results": [{
                "datetime": {"utc": "2026-09-27T10:00:00.000Z"},
                "parameters": [{"parameter": "pm25", "value": 74.9}]
            }]
        });
        let sigs = parse_latest(&j, "x", "X", 0.0, 0.0);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "routine");
    }

    /// Defensive: empty results array → zero signals, no panic.
    #[test]
    fn empty_results_returns_zero_signals() {
        let j = serde_json::json!({"results": []});
        assert!(parse_latest(&j, "x", "X", 0.0, 0.0).is_empty());
    }

    /// Defensive: missing `results` key → zero signals, no panic.
    #[test]
    fn missing_results_key() {
        assert!(parse_latest(&serde_json::json!({}), "x", "X", 0.0, 0.0).is_empty());
    }

    /// Defensive: missing datetime → falls back to "0000-00-00" sentinel.
    /// (Re-poll at unknown date would still produce a different id, but
    /// the parser doesn't crash on missing fields.)
    #[test]
    fn missing_datetime_uses_sentinel_date() {
        let j = serde_json::json!({
            "results": [{
                "parameters": [{"parameter": "pm25", "value": 80.0}]
            }]
        });
        let sigs = parse_latest(&j, "x", "X", 0.0, 0.0);
        assert_eq!(sigs.len(), 1);
        assert!(sigs[0].external_id.contains(":0000-00-00:"));
    }

    /// Unknown pollutant is silently skipped (no panic).
    #[test]
    fn unknown_pollutant_skipped() {
        let j = serde_json::json!({
            "results": [{
                "datetime": {"utc": "2026-09-27T10:00:00.000Z"},
                "parameters": [
                    {"parameter": "radon", "value": 999.0},  // not in ladder
                    {"parameter": "pm25",  "value": 80.0}    // fires priority
                ]
            }]
        });
        let sigs = parse_latest(&j, "x", "X", 0.0, 0.0);
        assert_eq!(sigs.len(), 1);
        assert!(sigs[0].title.contains("pm25"));
    }

    /// api_key() helper: HUB_ wins, empty HUB_ falls through, both empty → None.
    #[test]
    fn api_key_resolution() {
        // Set both with HUB_ winning
        std::env::set_var("HUB_OPENAQ_API_KEY", "hub-key");
        std::env::set_var("OPENAQ_API_KEY", "bare-key");
        assert_eq!(api_key().as_deref(), Some("hub-key"));

        // Empty HUB_ falls through
        std::env::set_var("HUB_OPENAQ_API_KEY", "");
        assert_eq!(api_key().as_deref(), Some("bare-key"));

        // Both empty → None
        std::env::set_var("OPENAQ_API_KEY", "");
        assert_eq!(api_key(), None);

        // Cleanup so subsequent tests don't inherit env state
        std::env::remove_var("HUB_OPENAQ_API_KEY");
        std::env::remove_var("OPENAQ_API_KEY");
    }

    /// Watchlist size guard: drift below 20 or above 40 is a regression.
    #[test]
    fn city_watchlist_size() {
        assert!(CITIES.len() >= 20, "watchlist too small: {}", CITIES.len());
        assert!(CITIES.len() <= 40, "watchlist too large: {}", CITIES.len());
    }

    /// Pollutant ladder sanity: every entry has guideline > 0 and
    /// routine_mult < priority_mult.
    #[test]
    fn pollutant_ladder_monotonic() {
        for &(name, g, r, p) in POLLUTANT_THRESHOLDS {
            assert!(g > 0.0, "{name} guideline must be > 0");
            assert!(r > 0.0 && r < p, "{name} routine ({r}) < priority ({p})");
        }
    }
}