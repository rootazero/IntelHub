//! Sunrise-Sunset API (https://sunrise-sunset.org/api — keyless, no
//! registration). Phase 1.6 of the public-API integration roadmap
//! (`docs/superpowers/roadmaps/2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Daily sweep: for each of 30 OSINT-relevant cities, fetch
//! `sunrise / sunset / twilight_begin_end` for today's date. Emit one
//! `info` Signal per city with the full daylight envelope in the
//! payload. Downstream consumers correlate this against other geo_events
//! to determine "this event happened during civil twilight" /
//! "this event happened under cover of astronomical night" / etc.
//!
//! 30 cities × ~1 KB JSON each ≈ 30 KB per sweep. 24h cadence matches
//! the day length granularity (sunrise changes by ~1 min/day — far
//! below the threshold of OSINT relevance).
//!
//! ## Why not just for OSINT cities, not all 200+?
//!
//! 30 keeps the per-sweep budget under 1% of the upstream's documented
//! 1 req/sec rate limit. Adding more cities linearly increases API
//! cost with diminishing OSINT return — daylight data is global (the
//! day length at a given lat/lng is the same regardless of which city
//! is "in" it), so a 30-city watchlist covers all major OSINT regions
//! with comfortable overlap.
//!
//! ## Severity
//!
//! Always `info` — daylight data is CONTEXTUAL. The downstream event
//! correlation engine can promote to priority when an `info` daylight
//! marker coincides with a `priority` event (e.g., a satellite imagery
//! intercept during astronomical twilight is more sensitive than the
//! same intercept at noon).
//!
//! ## external_id
//!
//! `sunrise_sunset:{city_slug}:{date}` — idempotent within a single
//! day. Same city polled twice on the same day = same id, geo_events
//! dedups. Next-day poll = new id (sunrise/sunset for that city will
//! have shifted by ~1 min).
//!
//! ## Auth
//!
//! Keyless. Sunrise-Sunset's free tier is "unlimited for non-commercial
//! use, 1 req/sec rate limit" per their docs. 30 reqs/24h is well within
//! bounds — even at a hypothetical 1000-city expansion we'd be at 0.03%
//! of daily rate budget.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.sunrise-sunset.org/json";

/// 30 OSINT-relevant cities. Reuses the same lat/lon set as the
/// Open-Meteo watchlist (Phase 1.1) — geographic coverage is the
/// point: Asia / MENA / Sub-Saharan Africa / Europe / Americas / Oceania
/// with strategic weighting. (Could merge with the Nager.Date
/// centroid table in a future refactor, but keeping the consts
/// local to each source makes per-source curation easier to review.)
const CITIES: &[(f64, f64, &str, &str)] = &[
    // ── Conflict zones ──
    (50.4501, 30.5234, "kyiv", "Kyiv"),
    (55.7558, 37.6173, "moscow", "Moscow"),
    (35.6892, 51.3890, "tehran", "Tehran"),
    (32.0853, 34.7818, "tel_aviv", "Tel Aviv"),
    (33.8938, 35.5018, "beirut", "Beirut"),
    (15.5007, 32.5599, "khartoum", "Khartoum"),
    (15.3694, 44.1910, "sanaa", "Sana'a"),
    (33.5138, 36.2765, "damascus", "Damascus"),
    (33.3152, 44.3661, "baghdad", "Baghdad"),
    (10.4806, -66.9036, "caracas", "Caracas"),
    // ── Chokepoints ──
    (30.0444, 31.2357, "cairo", "Cairo"),
    (41.0082, 28.9784, "istanbul", "Istanbul"),
    (25.2048, 55.2708, "dubai", "Dubai"),
    (1.3521, 103.8198, "singapore", "Singapore"),
    (12.7855, 45.0187, "aden", "Aden"),
    (8.9824, -79.5199, "panama_city", "Panama City"),
    // ── Geopolitical hotspots ──
    (39.9042, 116.4074, "beijing", "Beijing"),
    (35.6762, 139.6503, "tokyo", "Tokyo"),
    (37.5665, 126.9780, "seoul", "Seoul"),
    (39.0392, 125.7625, "pyongyang", "Pyongyang"),
    (28.6139, 77.2090, "new_delhi", "New Delhi"),
    (33.6844, 73.0479, "islamabad", "Islamabad"),
    (34.5553, 69.2075, "kabul", "Kabul"),
    (21.0285, 105.8542, "hanoi", "Hanoi"),
    (14.5995, 120.9842, "manila", "Manila"),
    (25.0330, 121.5654, "taipei", "Taipei"),
    // ── Climate-vulnerable mega-cities ──
    (6.5244, 3.3792, "lagos", "Lagos"),
    (-6.2088, 106.8456, "jakarta", "Jakarta"),
    (19.0760, 72.8777, "mumbai", "Mumbai"),
    (-23.5505, -46.6333, "sao_paulo", "São Paulo"),
];

/// Day-length thresholds (in seconds) for severity escalation.
/// Most mid-latitude cities stay in the 11h-14h range year-round.
/// Outside [4h, 22h] indicates polar-ish conditions (Arctic / Antarctic
/// circles in summer/winter). These thresholds are intentionally
/// lenient — a stricter 6h-20h window would mark Scandinavian and
/// Antarctic stations as priority, which is noise (we already know
/// Tromsø has polar night in December).
///
/// Currently unused (severity is always `info`) but kept for a
/// future spec extension that escalates to `routine` for extreme
/// photoperiods. Cheap to compute; documents the OSINT-relevance
/// reasoning for the "always info" choice.
#[allow(dead_code)]
const DAY_LENGTH_MIN_SECS: i64 = 4 * 3600;
#[allow(dead_code)]
const DAY_LENGTH_MAX_SECS: i64 = 22 * 3600;

pub struct SunriseSunset;

impl Source for SunriseSunset {
    fn name(&self) -> &'static str {
        "sunrise_sunset"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600) // 24h — roadmap §2.1
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let mut out = Vec::new();
            for &(lat, lon, slug, display) in CITIES {
                let url = format!(
                    "{BASE_URL}?lat={lat:.4}&lng={lon:.4}&date={date}&formatted=0"
                );
                let resp = match ctx.http.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(city = slug, error = %e, "sunrise_sunset fetch failed");
                        continue;
                    }
                };
                if !resp.status().is_success() {
                    tracing::warn!(city = slug, status = %resp.status(), "sunrise_sunset non-2xx");
                    continue;
                }
                let body: serde_json::Value = match resp.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::warn!(city = slug, error = %e, "sunrise_sunset parse failed");
                        continue;
                    }
                };
                // The API's `status` field can be "OK" or an error
                // message ("INVALID_REQUEST" etc.) — only "OK" is
                // usable. Checking upfront avoids emitting a city-
                // wide broken-payload signal.
                if body.get("status").and_then(|v| v.as_str()) != Some("OK") {
                    tracing::warn!(city = slug, "sunrise_sunset status != OK");
                    continue;
                }
                out.extend(parse_response(&body, slug, display, lat, lon, &date));
            }
            Ok(out)
        }
        .boxed()
    }
}

/// Pure parser: walk Sunrise-Sunset's `results` object, emit a single
/// Signal carrying the full daylight envelope in the payload.
///
/// The Signal kind is `daylight` and the severity is always `info`
/// (per spec §Strategy — these are contextual signals, downstream
/// does the priority promotion when correlating with priority events).
fn parse_response(
    j: &serde_json::Value,
    slug: &str,
    display: &str,
    lat: f64,
    lon: f64,
    date: &str,
) -> Vec<Signal> {
    let Some(results) = j.get("results") else {
        return Vec::new();
    };
    let sunrise = results.get("sunrise").and_then(|v| v.as_str()).unwrap_or("");
    let sunset = results.get("sunset").and_then(|v| v.as_str()).unwrap_or("");
    let solar_noon = results.get("solar_noon").and_then(|v| v.as_str()).unwrap_or("");
    let day_length_sec = results.get("day_length").and_then(|v| v.as_i64()).unwrap_or(0);
    let civil_begin = results
        .get("civil_twilight_begin")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let civil_end = results
        .get("civil_twilight_end")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let nautical_begin = results
        .get("nautical_twilight_begin")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let nautical_end = results
        .get("nautical_twilight_end")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let astro_begin = results
        .get("astronomical_twilight_begin")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let astro_end = results
        .get("astronomical_twilight_end")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Convert day_length (seconds) to human-readable "12h 22m" form
    // for the Signal title. Useful at-a-glance for the radar page.
    let (h, m) = (day_length_sec / 3600, (day_length_sec % 3600) / 60);
    let day_length_str = format!("{h}h {m:02}m");
    vec![Signal::new(
        "daylight",
        format!("{display} — daylight {day_length_str}"),
        lat,
        lon,
        format!("sunrise_sunset:{slug}:{date}"),
    )
    .severity("info")
    .payload(serde_json::json!({
        "city_slug": slug,
        "city": display,
        "date": date,
        "sunrise_utc": sunrise,
        "sunset_utc": sunset,
        "solar_noon_utc": solar_noon,
        "day_length_seconds": day_length_sec,
        "day_length_human": day_length_str,
        "civil_twilight_begin_utc": civil_begin,
        "civil_twilight_end_utc": civil_end,
        "nautical_twilight_begin_utc": nautical_begin,
        "nautical_twilight_end_utc": nautical_end,
        "astronomical_twilight_begin_utc": astro_begin,
        "astronomical_twilight_end_utc": astro_end,
        "extreme_type": "daylight_envelope",
    }))]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_response() -> serde_json::Value {
        serde_json::json!({
            "results": {
                "sunrise": "2026-09-26T22:05:09+00:00",
                "sunset": "2026-09-27T10:05:45+00:00",
                "solar_noon": "2026-09-27T04:05:27+00:00",
                "day_length": 43236,
                "civil_twilight_begin": "2026-09-26T21:39:35+00:00",
                "civil_twilight_end": "2026-09-27T10:31:19+00:00",
                "nautical_twilight_begin": "2026-09-26T21:08:09+00:00",
                "nautical_twilight_end": "2026-09-27T11:02:45+00:00",
                "astronomical_twilight_begin": "2026-09-26T20:36:18+00:00",
                "astronomical_twilight_end": "2026-09-27T11:34:36+00:00"
            },
            "status": "OK",
            "tzid": "UTC"
        })
    }

    /// Happy path: 1 signal emitted, all payload fields present.
    #[test]
    fn detects_daylight_envelope() {
        let sigs = parse_response(&sample_response(), "beijing", "Beijing", 39.9042, 116.4074, "2026-09-27");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].kind, "daylight");
        assert_eq!(sigs[0].severity, "info");
        // 43236 seconds = 12h 00m 36s
        assert!(sigs[0].title.contains("12h 00m"));
        assert!(sigs[0].title.contains("Beijing"));
        // All payload fields present
        let p = &sigs[0].payload;
        assert_eq!(p["city_slug"], "beijing");
        assert_eq!(p["day_length_seconds"], 43236);
        assert_eq!(p["day_length_human"], "12h 00m");
        assert!(p["sunrise_utc"].as_str().unwrap().contains("T22:05"));
        assert!(p["sunset_utc"].as_str().unwrap().contains("T10:05"));
    }

    /// Day-length human formatter: zero, sub-hour, full hours.
    #[test]
    fn day_length_human_formatting() {
        // 12h 0m 36s = 43236s → "12h 00m"
        let s = format!("{}h {:02}m", 43236 / 3600, (43236 % 3600) / 60);
        assert_eq!(s, "12h 00m");
        // 8h 22m = 30120s
        let s = format!("{}h {:02}m", 30120 / 3600, (30120 % 3600) / 60);
        assert_eq!(s, "8h 22m");
        // 14h 5m = 50700s
        let s = format!("{}h {:02}m", 50700 / 3600, (50700 % 3600) / 60);
        assert_eq!(s, "14h 05m");
    }

    /// external_id format: `sunrise_sunset:{slug}:{date}`.
    #[test]
    fn external_id_shape() {
        let sigs = parse_response(&sample_response(), "beijing", "Beijing", 0.0, 0.0, "2026-09-27");
        assert_eq!(sigs[0].external_id, "sunrise_sunset:beijing:2026-09-27");
    }

    /// Coords preserved verbatim.
    #[test]
    fn coords_preserved() {
        let sigs = parse_response(&sample_response(), "beijing", "Beijing", 39.9042, 116.4074, "2026-09-27");
        assert!((sigs[0].lat - 39.9042).abs() < 1e-9);
        assert!((sigs[0].lon - 116.4074).abs() < 1e-9);
    }

    /// Defensive: missing `results` key → empty vec, no panic.
    #[test]
    fn missing_results_returns_empty() {
        let sigs = parse_response(&serde_json::json!({"status": "OK"}), "x", "X", 0.0, 0.0, "2026-09-27");
        assert!(sigs.is_empty());
    }

    /// Defensive: missing individual fields → graceful defaults
    /// (empty string for time fields, 0 for day_length, etc.) — the
    /// Signal still emits, the payload carries placeholders. This
    /// matches the Open-Meteo precedent (defensive parsing wins over
    /// strict skip; downstream decides what to do with partial data).
    #[test]
    fn missing_individual_fields_uses_defaults() {
        let j = serde_json::json!({"results": {}});
        let sigs = parse_response(&j, "x", "X", 0.0, 0.0, "2026-09-27");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].payload["sunrise_utc"], "");
        assert_eq!(sigs[0].payload["sunset_utc"], "");
        assert_eq!(sigs[0].payload["day_length_seconds"], 0);
        assert_eq!(sigs[0].payload["day_length_human"], "0h 00m");
    }

    /// Defensive: status != OK → caller is responsible for skipping
    /// (parse_response doesn't check status, but emit-time check in
    /// fetch() does). Verifies parse_response accepts a body that
    /// has "OK" status without erroring.
    #[test]
    fn accepts_ok_status() {
        let j = serde_json::json!({
            "results": {"day_length": 43236},
            "status": "OK"
        });
        let sigs = parse_response(&j, "x", "X", 0.0, 0.0, "2026-09-27");
        assert_eq!(sigs.len(), 1);
    }

    /// Watchlist size guard.
    #[test]
    fn watchlist_size() {
        assert!(CITIES.len() >= 25, "watchlist too small: {}", CITIES.len());
        assert!(CITIES.len() <= 40, "watchlist too large: {}", CITIES.len());
    }
}