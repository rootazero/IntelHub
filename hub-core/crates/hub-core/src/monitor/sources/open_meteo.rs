//! Open-Meteo global weather forecast (https://open-meteo.com — free, no
//! key required, non-commercial use per their ToS). Phase 1.1 of the
//! public-API integration roadmap (`docs/superpowers/roadmaps/
//! 2026-09-27-public-api-integration-roadmap.md`).
//!
//! Strategy: poll 30 OSINT-relevant cities (conflict zones + chokepoints +
//! geopolitical hotspots + climate-vulnerable mega-cities), parse the next
//! 24h forecast, and emit a `Signal` for each notable extreme. Each city
//! uses one HTTP request — 30 sequential calls @ ~200ms each ≈ 6s sweep
//! wall time, well under the per-source 25s ctx timeout and far below the
//! 74-source stampede ceiling. Sequential mirrors `firms.rs` precedent;
//! parallel `join_all` would shave ~5s but complicate error isolation —
//! one bad city ≠ source failure today, isolated per-city `continue`.
//!
//! Severity ladder (matches SP6 geo_events alert thresholds):
//!   T_max >= 40 °C OR T_min <= -20 °C → "priority"
//!   precip_24h >= 50 mm OR wind_max >= 20 m/s → "routine"
//!
//! external_id format = `"{city_slug}:{forecast_date}:{extreme_type}"` —
//! fixed per (city, date, kind). Idempotent re-ingest across the 30-min
//! cadence: the second sweep finds the same forecast already in
//! geo_events and silently skips.
//!
//! Keyless (no env var). Open-Meteo's free tier is unlimited for
//! non-commercial use; we cite this in the OSINT-public mission under the
//! IntelHub charter. If they ever tighten ToS we flip to the env-gated
//! pattern (`select(HUB_OPEN_METEO_KEY, OPEN_METEO_KEY)`) and register
//! only when present (opensky/ais precedent).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

/// Open-Meteo forecast endpoint. Single base URL; per-city query string.
const URL: &str = "https://api.open-meteo.com/v1/forecast";

/// Extreme-weather thresholds. Tuned for OSINT-relevance (life-disrupting,
/// newsworthy) rather than meteorological strictness — a 38°C day in Cairo
/// is normal, in Seoul it's a heatwave. Phase 1 ships global-uniform; a
/// future per-climate-zone refinement is straightforward (city table grows
/// a `climate_zone` column, thresholds become a function of it).
const T_MAX_PRIORITY_C: f64 = 40.0;
const T_MIN_PRIORITY_C: f64 = -20.0;
const PRECIP_ROUTINE_MM: f64 = 50.0;
const WIND_ROUTINE_MS: f64 = 20.0;

/// OSINT strategic-city watchlist. (lat, lon, slug, display name). Slug
/// becomes part of `external_id` so re-enumerated by accident are caught
/// in code review (lowercase, no spaces, ≤ 24 chars).
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
    (30.0444, 31.2357, "cairo", "Cairo"),        // Suez
    (41.0082, 28.9784, "istanbul", "Istanbul"),  // Bosporus
    (25.2048, 55.2708, "dubai", "Dubai"),        // Hormuz
    (1.3521, 103.8198, "singapore", "Singapore"), // Malacca
    (12.7855, 45.0187, "aden", "Aden"),          // Bab-el-Mandeb
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
    (19.4326, -99.1332, "mexico_city", "Mexico City"),
    (24.8607, 67.0011, "karachi", "Karachi"),
    (23.8103, 90.4125, "dhaka", "Dhaka"),
];

pub struct OpenMeteo;

impl Source for OpenMeteo {
    fn name(&self) -> &'static str {
        "open_meteo"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(1800) // 30 min — roadmap §2.1
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let mut out = Vec::new();
            for &(lat, lon, slug, display) in CITIES {
                let url = format!(
                    "{URL}?latitude={lat:.4}&longitude={lon:.4}\
                     &daily=temperature_2m_max,temperature_2m_min,\
                     precipitation_sum,wind_speed_10m_max\
                     &forecast_days=1&timezone=UTC"
                );
                let resp = match ctx.http.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        // Per-city isolation: one network blip ≠ source failure.
                        tracing::warn!(city = slug, error = %e, "open_meteo fetch failed");
                        continue;
                    }
                };
                if !resp.status().is_success() {
                    tracing::warn!(
                        city = slug,
                        status = %resp.status(),
                        "open_meteo non-2xx"
                    );
                    continue;
                }
                let body: serde_json::Value = match resp.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::warn!(city = slug, error = %e, "open_meteo parse failed");
                        continue;
                    }
                };
                out.extend(parse_city(&body, slug, display, lat, lon));
            }
            Ok(out)
        }
        .boxed()
    }
}

/// Pure parser: extract all extreme-weather Signals from one city's
/// response. Returns 0..4 signals per city (priority heat, priority cold,
/// routine precip, routine wind). Defensive against missing fields — the
/// upstream occasionally drops a `daily` key on transient errors.
fn parse_city(
    j: &serde_json::Value,
    slug: &str,
    display: &str,
    lat: f64,
    lon: f64,
) -> Vec<Signal> {
    let mut out = Vec::new();
    let Some(daily) = j.get("daily") else {
        return out;
    };
    let Some(date) = daily
        .get("time")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
    else {
        return out;
    };

    // Open-Meteo returns arrays-of-one when forecast_days=1. Each helper
    // returns None on missing/non-array/empty/non-numeric, so a missing
    // field simply skips the corresponding extreme — never panics.
    let t_max = daily
        .get("temperature_2m_max")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_f64());
    let t_min = daily
        .get("temperature_2m_min")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_f64());
    let precip = daily
        .get("precipitation_sum")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_f64());
    let wind = daily
        .get("wind_speed_10m_max")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_f64());

    if let Some(t) = t_max {
        if t >= T_MAX_PRIORITY_C {
            out.push(extreme_signal(
                slug, display, lat, lon, date, "extreme_heat",
                "priority",
                format!("{display} — extreme heat {t:.0}°C"),
                serde_json::json!({ "t_max_c": t }),
            ));
        }
    }
    if let Some(t) = t_min {
        if t <= T_MIN_PRIORITY_C {
            out.push(extreme_signal(
                slug, display, lat, lon, date, "extreme_cold",
                "priority",
                format!("{display} — extreme cold {t:.0}°C"),
                serde_json::json!({ "t_min_c": t }),
            ));
        }
    }
    if let Some(p) = precip {
        if p >= PRECIP_ROUTINE_MM {
            out.push(extreme_signal(
                slug, display, lat, lon, date, "heavy_precip",
                "routine",
                format!("{display} — heavy precip {p:.0} mm/24h"),
                serde_json::json!({ "precip_mm": p }),
            ));
        }
    }
    if let Some(w) = wind {
        if w >= WIND_ROUTINE_MS {
            out.push(extreme_signal(
                slug, display, lat, lon, date, "storm_wind",
                "routine",
                format!("{display} — storm wind {w:.0} m/s"),
                serde_json::json!({ "wind_max_ms": w }),
            ));
        }
    }
    out
}

fn extreme_signal(
    slug: &str,
    display: &str,
    lat: f64,
    lon: f64,
    date: &str,
    kind: &str,
    severity: &'static str,
    title: String,
    payload: serde_json::Value,
) -> Signal {
    Signal::new(
        "weather_extreme",
        title,
        lat,
        lon,
        format!("{slug}:{date}:{kind}"),
    )
    .severity(severity)
    .payload(payload
        .as_object()
        .map(|m| {
            let mut o = m.clone();
            o.insert("city".into(), serde_json::json!(display));
            o.insert("city_slug".into(), serde_json::json!(slug));
            o.insert("forecast_date".into(), serde_json::json!(date));
            o.insert("extreme_type".into(), serde_json::json!(kind));
            serde_json::Value::Object(o)
        })
        .unwrap_or(payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Happy path: one city, all four extremes fire.
    #[test]
    fn detects_all_four_extremes() {
        let j = serde_json::json!({
            "latitude": 50.45,
            "longitude": 30.52,
            "daily": {
                "time": ["2026-09-27"],
                "temperature_2m_max": [42.5],
                "temperature_2m_min": [-25.3],
                "precipitation_sum": [78.0],
                "wind_speed_10m_max": [25.1]
            }
        });
        let sigs = parse_city(&j, "kyiv", "Kyiv", 50.45, 30.52);
        assert_eq!(sigs.len(), 4, "expected 4 extremes, got len={}", sigs.len());
        assert_eq!(sigs[0].kind, "weather_extreme");
        assert_eq!(sigs[0].severity, "priority"); // heat
        assert_eq!(sigs[1].severity, "priority"); // cold
        assert_eq!(sigs[2].severity, "routine"); // precip
        assert_eq!(sigs[3].severity, "routine"); // wind

        // external_id format: {slug}:{date}:{extreme_type}
        assert_eq!(sigs[0].external_id, "kyiv:2026-09-27:extreme_heat");
        assert_eq!(sigs[1].external_id, "kyiv:2026-09-27:extreme_cold");
        assert_eq!(sigs[2].external_id, "kyiv:2026-09-27:heavy_precip");
        assert_eq!(sigs[3].external_id, "kyiv:2026-09-27:storm_wind");

        // coords preserved
        assert!((sigs[0].lat - 50.45).abs() < 1e-9);
        assert!((sigs[0].lon - 30.52).abs() < 1e-9);

        // payload carries observed values
        assert_eq!(sigs[0].payload["t_max_c"].as_f64().unwrap(), 42.5);
        assert_eq!(sigs[2].payload["precip_mm"].as_f64().unwrap(), 78.0);
        assert_eq!(sigs[3].payload["wind_max_ms"].as_f64().unwrap(), 25.1);
        assert_eq!(sigs[0].payload["city"].as_str().unwrap(), "Kyiv");
        assert_eq!(sigs[0].payload["extreme_type"].as_str().unwrap(), "extreme_heat");
    }

    /// Boundary: exactly at threshold → still fires.
    #[test]
    fn threshold_inclusive() {
        let j = serde_json::json!({
            "daily": {
                "time": ["2026-09-27"],
                "temperature_2m_max": [40.0],
                "temperature_2m_min": [-20.0],
                "precipitation_sum": [50.0],
                "wind_speed_10m_max": [20.0]
            }
        });
        let sigs = parse_city(&j, "x", "X", 0.0, 0.0);
        assert_eq!(sigs.len(), 4, "inclusive thresholds must fire");
    }

    /// Boundary: 0.1 below/above threshold → no signal.
    #[test]
    fn threshold_exclusive_below() {
        let j = serde_json::json!({
            "daily": {
                "time": ["2026-09-27"],
                "temperature_2m_max": [39.9],
                "temperature_2m_min": [-19.9],
                "precipitation_sum": [49.9],
                "wind_speed_10m_max": [19.9]
            }
        });
        let sigs = parse_city(&j, "x", "X", 0.0, 0.0);
        assert_eq!(sigs.len(), 0, "just-below thresholds must NOT fire");
    }

    /// Defensive: empty body → empty vec (no panic).
    #[test]
    fn empty_body_returns_zero_signals() {
        assert!(parse_city(&serde_json::json!({}), "x", "X", 0.0, 0.0).is_empty());
    }

    /// Defensive: missing `daily` key → empty vec.
    #[test]
    fn missing_daily_key() {
        let j = serde_json::json!({"latitude": 1.0, "longitude": 2.0});
        assert!(parse_city(&j, "x", "X", 1.0, 2.0).is_empty());
    }

    /// Defensive: missing date → empty vec (can't build stable external_id).
    #[test]
    fn missing_date_skips_city() {
        let j = serde_json::json!({
            "daily": {
                "temperature_2m_max": [50.0]
            }
        });
        assert!(parse_city(&j, "x", "X", 0.0, 0.0).is_empty());
    }

    /// Defensive: `daily.time` is empty array → no signal.
    #[test]
    fn empty_time_array() {
        let j = serde_json::json!({
            "daily": {
                "time": [],
                "temperature_2m_max": [50.0]
            }
        });
        assert!(parse_city(&j, "x", "X", 0.0, 0.0).is_empty());
    }

    /// Selective: only heat fires, cold/precip/wind all calm.
    #[test]
    fn selective_only_heat() {
        let j = serde_json::json!({
            "daily": {
                "time": ["2026-09-27"],
                "temperature_2m_max": [41.0],
                "temperature_2m_min": [10.0],
                "precipitation_sum": [0.0],
                "wind_speed_10m_max": [3.0]
            }
        });
        let sigs = parse_city(&j, "lagos", "Lagos", 6.52, 3.38);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "priority");
        assert!(sigs[0].title.contains("extreme heat"));
    }

    /// external_id stability: same input twice → identical external_ids
    /// (idempotent ingest across the 30-min cadence).
    #[test]
    fn external_id_stable_across_calls() {
        let j = serde_json::json!({
            "daily": {
                "time": ["2026-09-27"],
                "temperature_2m_max": [45.0]
            }
        });
        let a = parse_city(&j, "lagos", "Lagos", 0.0, 0.0);
        let b = parse_city(&j, "lagos", "Lagos", 0.0, 0.0);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].external_id, b[0].external_id);
    }

    /// City count sanity: the watchlist has 33 strategic cities. Drops to
    /// 30 are a regression we want caught in code review.
    #[test]
    fn city_watchlist_size() {
        assert!(CITIES.len() >= 30, "watchlist too small: {}", CITIES.len());
        assert!(CITIES.len() <= 50, "watchlist too large (stampede): {}", CITIES.len());
    }
}