//! Nager.Date Public Holidays API (https://date.nager.at/ — keyless, 204
//! country coverage). Phase 1.5 of the public-API integration roadmap
//! (`docs/superpowers/roadmaps/2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Daily sweep: for each of 34 OSINT-relevant countries in Nager.Date's
//! coverage, fetch `PublicHolidays/{year}/{cc}` and filter to entries
//! whose `date` equals today (UTC). Emit one Signal per matching holiday.
//!
//! 34 countries × ~2 KB JSON each ≈ 70 KB total per sweep. 24h cadence
//! matches humanitarian data rhythm; holidays rarely change intra-year
//! but governments do declare new ones (e.g., snap mourning days).
//!
//! ## Nager.Date coverage reality
//!
//! Nager.Date has 204 countries but NOT all OSINT-critical ones.
//! Verified missing (HTTP 204) at sweep time: IR, IL, IN, PK, AF, MM,
//! SA, AE, TH, MY. The 34 below are what Nager.Date actually serves
//! AND matter for IntelHub's mission. Countries missing from Nager
//! are NOT a bug — Nager.Date is a community-maintained project and
//! many governments don't publish machine-readable holiday lists.
//! (Phase 1.6+ candidates: government APIs for IR/IL holidays, or
//! scrape the official gazette for IL/IR — but out of Phase 1.5 scope.)
//!
//! ## Severity
//!
//! - global + Public/Bank → routine (real country-wide holiday)
//! - regional (global=false) → info
//! - Observance / Optional / School / Authorities → info
//!
//! Holidays are CONTEXTUAL signals, not alerts — the OSINT value is
//! "government offices closed in Y today" rather than "incident in
//! Y". Severity ladder reflects this: holidays never auto-page
//! operators; they show up in event timelines as anchor markers.
//!
//! ## external_id
//!
//! `nager:{cc}:{date}:{name-slugified}` — idempotent within a single
//! day (same holiday re-poll = same id, geo_events dedup). The date
//! component is the holiday's calendar date, not the polling date —
//! so a holiday on 2026-09-27 polled on 2026-09-28 emits the same id
//! and stays deduped against replay. (Nager.Date holidays move slowly
//! enough that this is desirable: you don't want a stale cache sweep
//! to re-alert on yesterday's holiday.)
//!
//! ## Auth
//!
//! Keyless. Nager.Date is a free public service hosted by the original
//! author (Tino Wetzig), no registration, no rate-limit published
//! (live testing: ~30 reqs/min observed before any 429 — we do 34
//! reqs/24h so well within bounds).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://date.nager.at/api/v3";

/// 34 OSINT-relevant countries that Nager.Date actually serves.
/// Cross-checked live against `/api/v3/AvailableCountries` 2026-09-27.
/// Missing: IR/IL/IN/PK/AF/MM/SA/AE/TH/MY (Nager doesn't have data).
/// Includes all major conflict zones, economic powers, and
/// humanitarian-crisis geographies that Nager covers.
const COUNTRIES: &[&str] = &[
    // ── Western / Europe ──
    "US", "GB", "DE", "FR", "IT", "ES", "NL", "PL",
    // ── Eastern Europe / Russia ──
    "UA", "RU",
    // ── Latin America ──
    "BR", "MX", "AR", "CL", "CO", "PE",
    // ── Africa ──
    "EG", "NG", "ZA", "KE", "ET", "SD",
    // ── East / Southeast Asia / Pacific ──
    "CN", "JP", "KR", "VN", "PH", "ID", "SG", "BD",
    "AU", "CA", "NZ",
    // ── Middle East (limited to Nager-coverage subset) ──
    "TR",
];

/// Country capital centroid table (alpha-2 → lat/lon). Mirrors the
/// HDX centroid table but in alpha-2 form for Nager.Date's country
/// code format. UN HQ Geneva fallback for any country in COUNTRIES
/// that doesn't have an entry here (none currently, but defensive).
const COUNTRY_COORDS: &[(&str, f64, f64)] = &[
    // Western / Europe
    ("US", 38.9072, -77.0369),  // Washington DC
    ("GB", 51.5074, -0.1278),   // London
    ("DE", 52.5200, 13.4050),   // Berlin
    ("FR", 48.8566, 2.3522),    // Paris
    ("IT", 41.9028, 12.4964),   // Rome
    ("ES", 40.4168, -3.7038),   // Madrid
    ("NL", 52.3676, 4.9041),    // Amsterdam
    ("PL", 52.2297, 21.0122),   // Warsaw
    // Eastern Europe
    ("UA", 50.4501, 30.5234),   // Kyiv
    ("RU", 55.7558, 37.6173),   // Moscow
    // Latin America
    ("BR", -15.7942, -47.8822), // Brasília
    ("MX", 19.4326, -99.1332),  // Mexico City
    ("AR", -34.6037, -58.3816), // Buenos Aires
    ("CL", -33.4489, -70.6693), // Santiago
    ("CO", 4.7110, -74.0721),   // Bogotá
    ("PE", -12.0464, -77.0428), // Lima
    // Africa
    ("EG", 30.0444, 31.2357),   // Cairo
    ("NG", 9.0765, 7.3986),     // Abuja
    ("ZA", -25.7479, 28.2293),  // Pretoria
    ("KE", -1.2921, 36.8219),   // Nairobi
    ("ET", 9.1450, 40.4897),    // Addis Ababa
    ("SD", 15.5007, 32.5599),   // Khartoum
    // East / Southeast Asia / Pacific
    ("CN", 39.9042, 116.4074),  // Beijing
    ("JP", 35.6762, 139.6503),  // Tokyo
    ("KR", 37.5665, 126.9780),  // Seoul
    ("VN", 21.0285, 105.8542),  // Hanoi
    ("PH", 14.5995, 120.9842),  // Manila
    ("ID", -6.2088, 106.8456),  // Jakarta
    ("SG", 1.3521, 103.8198),   // Singapore
    ("BD", 23.8103, 90.4125),   // Dhaka
    ("AU", -35.2809, 149.1300), // Canberra
    ("CA", 45.4215, -75.6972),  // Ottawa
    ("NZ", -41.2865, 174.7762), // Wellington
    // Middle East
    ("TR", 39.9334, 32.8597),   // Ankara
];

/// UN HQ Geneva fallback for countries without an explicit coord
/// entry. None of COUNTRIES are missing today, but kept defensive
/// in case the table shrinks.
const FALLBACK_LAT: f64 = 46.2044;
const FALLBACK_LON: f64 = 6.1432;

pub struct NagerDate;

impl Source for NagerDate {
    fn name(&self) -> &'static str {
        "nager_date"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600) // 24h — roadmap §2.1
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let now = chrono::Utc::now();
            let today = now.format("%Y-%m-%d").to_string();
            let year = now.format("%Y").to_string();
            let mut out = Vec::new();
            for &cc in COUNTRIES {
                let url = format!("{BASE_URL}/PublicHolidays/{year}/{cc}");
                let resp = match ctx.http.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        // Per-country isolation: one bad fetch ≠ source failure
                        tracing::warn!(country = cc, error = %e, "nager_date fetch failed");
                        continue;
                    }
                };
                // Nager.Date returns 204 No Content for countries it
                // doesn't have data for. Treat as empty (no holidays),
                // not as an error.
                if resp.status() == 204 {
                    continue;
                }
                if !resp.status().is_success() {
                    tracing::warn!(country = cc, status = %resp.status(), "nager_date non-2xx");
                    continue;
                }
                let body: serde_json::Value = match resp.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::warn!(country = cc, error = %e, "nager_date parse failed");
                        continue;
                    }
                };
                out.extend(parse_country(&body, cc, &today));
            }
            Ok(out)
        }
        .boxed()
    }
}

/// Pure parser: walk one country's holiday list, emit a Signal for
/// each entry whose `date` equals today. Defensive against missing
/// fields and unexpected types.
fn parse_country(j: &serde_json::Value, cc: &str, today: &str) -> Vec<Signal> {
    let mut out = Vec::new();
    let Some(arr) = j.as_array() else {
        return out;
    };
    let (lat, lon) = COUNTRY_COORDS
        .iter()
        .find(|(c, _, _)| *c == cc)
        .map(|(_, lat, lon)| (*lat, *lon))
        .unwrap_or((FALLBACK_LAT, FALLBACK_LON));
    for h in arr {
        let Some(date) = h.get("date").and_then(|v| v.as_str()) else {
            continue;
        };
        if date != today {
            continue; // not today
        }
        let name = h
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Public Holiday");
        let local_name = h
            .get("localName")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let global = h
            .get("global")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let types: Vec<String> = h
            .get("types")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // Severity: country-wide Public/Bank = routine. Regional or
        // Observance/Optional = info. The "is_public" check below
        // mirrors the OpenAQ precedent: meaningful events get a tier,
        // ancillary events are informational.
        let is_public = types.iter().any(|t| t == "Public" || t == "Bank");
        let severity = match (global, is_public) {
            (true, true) => "routine",
            _ => "info",
        };
        out.push(
            Signal::new(
                "holiday",
                if local_name.is_empty() || local_name == name {
                    format!("{cc} — {name}")
                } else {
                    format!("{cc} — {name} ({local_name})")
                },
                lat,
                lon,
                format!("nager:{cc}:{date}:{}", slugify(name)),
            )
            .severity(severity)
            .payload(serde_json::json!({
                "country_code": cc,
                "date": date,
                "name": name,
                "local_name": local_name,
                "global": global,
                "types": types,
                "extreme_type": "public_holiday",
            })),
        );
    }
    out
}

/// Slugify a holiday name for use in external_id. Lowercase,
/// non-alphanumeric replaced with `_`, leading/trailing `_` trimmed.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_us = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_us = false;
        } else if !prev_us {
            out.push('_');
            prev_us = true;
        }
    }
    out.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_holiday(date: &str, name: &str, global: bool, types: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "date": date,
            "localName": name,
            "name": name,
            "countryCode": "US",
            "global": global,
            "types": types,
            "counties": null,
            "launchYear": null,
            "fixed": false
        })
    }

    /// Happy path: 3 holidays, 1 matches today → 1 signal emitted.
    #[test]
    fn detects_todays_holiday() {
        let today = "2026-09-27";
        let body = serde_json::json!([
            sample_holiday("2026-01-01", "New Year's Day", true, &["Public", "Bank"]),
            sample_holiday("2026-09-27", "World Tourism Day", true, &["Observance"]),
            sample_holiday("2026-12-25", "Christmas", true, &["Public", "Bank"]),
        ]);
        let sigs = parse_country(&body, "US", today);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "info"); // Observance, not Public
        assert_eq!(sigs[0].kind, "holiday");
        assert!(sigs[0].title.contains("World Tourism Day"));
    }

    /// global + Public = routine severity.
    #[test]
    fn public_global_holiday_is_routine() {
        let body = serde_json::json!([
            sample_holiday("2026-09-27", "National Day", true, &["Public"])
        ]);
        let sigs = parse_country(&body, "US", "2026-09-27");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "routine");
    }

    /// global + Bank = routine (banks closed = economically significant).
    #[test]
    fn bank_holiday_is_routine() {
        let body = serde_json::json!([
            sample_holiday("2026-09-27", "Bank Holiday", true, &["Bank"])
        ]);
        let sigs = parse_country(&body, "US", "2026-09-27");
        assert_eq!(sigs[0].severity, "routine");
    }

    /// Regional (global=false) = info even if Public type.
    #[test]
    fn regional_holiday_is_info() {
        let body = serde_json::json!([
            sample_holiday("2026-09-27", "State Day", false, &["Public"])
        ]);
        let sigs = parse_country(&body, "US", "2026-09-27");
        assert_eq!(sigs[0].severity, "info");
    }

    /// No match today → zero signals.
    #[test]
    fn no_match_returns_empty() {
        let body = serde_json::json!([
            sample_holiday("2026-01-01", "New Year", true, &["Public"]),
            sample_holiday("2026-12-25", "Christmas", true, &["Public"]),
        ]);
        let sigs = parse_country(&body, "US", "2026-09-27");
        assert!(sigs.is_empty());
    }

    /// external_id format: `nager:{cc}:{date}:{slug}`.
    #[test]
    fn external_id_shape() {
        let body = serde_json::json!([
            sample_holiday("2026-09-27", "Independence Day", true, &["Public"])
        ]);
        let sigs = parse_country(&body, "BR", "2026-09-27");
        assert_eq!(sigs[0].external_id, "nager:BR:2026-09-27:independence_day");
    }

    /// localName differs from name → shown in title.
    #[test]
    fn local_name_appears_in_title() {
        let mut h = sample_holiday("2026-09-27", "Independence Day", true, &["Public"]);
        h.as_object_mut().unwrap().insert(
            "localName".into(),
            serde_json::json!("Dia da Independência"),
        );
        let sigs = parse_country(&serde_json::json!([h]), "BR", "2026-09-27");
        assert!(sigs[0].title.contains("Dia da Independência"));
    }

    /// localName equals name → no parens in title.
    #[test]
    fn no_local_name_when_equals_name() {
        let body = serde_json::json!([
            sample_holiday("2026-09-27", "Thanksgiving", true, &["Public"])
        ]);
        let sigs = parse_country(&body, "US", "2026-09-27");
        assert!(!sigs[0].title.contains("()"));
    }

    /// Defensive: empty body → zero signals, no panic.
    #[test]
    fn empty_body_returns_empty() {
        let sigs = parse_country(&serde_json::json!([]), "US", "2026-09-27");
        assert!(sigs.is_empty());
    }

    /// Defensive: non-array body → zero signals, no panic.
    #[test]
    fn non_array_body_returns_empty() {
        let sigs = parse_country(&serde_json::json!({"foo": "bar"}), "US", "2026-09-27");
        assert!(sigs.is_empty());
    }

    /// Defensive: missing-date item silently skipped; missing-name/-
/// global/-types fall through with unwrap_or defaults (severity=info).
/// Only entries that have BOTH date AND match today emit; others
/// silently skipped. This is the Open-Meteo precedent — defensive
/// parsing wins over strict skip.
    #[test]
    fn malformed_item_handling() {
        let body = serde_json::json!([
            {"date": "2026-09-27"},                                          // no name/types/global → emit with defaults (info)
            {"name": "Foo"},                                                  // no date → skipped
            {"date": "2026-12-25", "name": "Christmas", "global": true, "types": ["Public"]}, // not today → skipped
            {"date": "2026-09-27", "name": "Valid Holiday", "global": true, "types": ["Public"]}  // emit routine
        ]);
        let sigs = parse_country(&body, "US", "2026-09-27");
        assert_eq!(sigs.len(), 2);
        // First emits as info (defaults: name=Public Holiday, global=false, types=[])
        assert_eq!(sigs[0].severity, "info");
        assert_eq!(sigs[0].title, "US — Public Holiday");
        // Second emits as routine
        assert_eq!(sigs[1].severity, "routine");
        assert!(sigs[1].title.contains("Valid Holiday"));
    }

    /// Slugify handles unicode, punctuation, multi-spaces.
    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Independence Day"), "independence_day");
        assert_eq!(slugify("New Year's Day"), "new_year_s_day");
        assert_eq!(slugify("Día de la Raza"), "d_a_de_la_raza");
        assert_eq!(slugify("  trim  "), "trim");
    }

    /// Coords lookup: country in table uses capital centroid.
    #[test]
    fn coords_lookup_known() {
        let body = serde_json::json!([
            sample_holiday("2026-09-27", "Test", true, &["Public"])
        ]);
        let sigs = parse_country(&body, "US", "2026-09-27");
        assert!((sigs[0].lat - 38.9072).abs() < 1e-3);
        assert!((sigs[0].lon + 77.0369).abs() < 1e-3);
    }

    /// Coords lookup: unknown country → UN HQ fallback.
    #[test]
    fn coords_lookup_fallback() {
        let body = serde_json::json!([
            sample_holiday("2026-09-27", "Test", true, &["Public"])
        ]);
        let sigs = parse_country(&body, "XX", "2026-09-27");
        assert!((sigs[0].lat - FALLBACK_LAT).abs() < 1e-3);
        assert!((sigs[0].lon - FALLBACK_LON).abs() < 1e-3);
    }

    /// Watchlist size guard: drift below 25 or above 45 is a regression.
    #[test]
    fn watchlist_size() {
        assert!(COUNTRIES.len() >= 25, "watchlist too small: {}", COUNTRIES.len());
        assert!(COUNTRIES.len() <= 45, "watchlist too large: {}", COUNTRIES.len());
    }

    /// Coords table consistency: every country in COUNTRIES has a
    /// matching coord entry. (No silent fallback to UN HQ in
    /// production — that hides coverage gaps.)
    #[test]
    fn every_country_has_coords() {
        for &cc in COUNTRIES {
            assert!(
                COUNTRY_COORDS.iter().any(|(c, _, _)| *c == cc),
                "country {cc} in COUNTRIES but missing from COUNTRY_COORDS"
            );
        }
    }
}