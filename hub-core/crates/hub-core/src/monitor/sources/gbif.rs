//! GBIF — Global Biodiversity Information Facility
//! (https://api.gbif.org — keyless, public, 60 req/min/IP). Phase 2.4
//! of the public-API integration roadmap (`docs/superpowers/roadmaps/
//! 2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Daily sweep: query GBIF occurrence search for recent observations
//! of a curated watchlist of CITES Appendix I species (wildlife
//! trafficking targets — pangolins, elephants, tigers, rhinos,
//! gorillas, sea turtles, etc.). Each occurrence emits a geo-localized
//! Signal. Severity by species' trafficking risk (high-risk = priority,
//! medium = routine, low = info).
//!
//! ## Why GBIF
//!
//! - Wildlife trafficking is a $23B/year illicit trade (CITES/UNODC)
//! - GBIF aggregates 2.4B+ occurrence records from museums, surveys,
//   citizen science (iNaturalist), and protected-area monitoring
//! - Cross-references with ACLED (Phase 1.x shelved), ReliefWeb (1.x
//!   shelved) could reveal trafficking-in-conflict zones — future spec
//! - Standalone dimension: ecological/animal-disease — doesn't
//!   compound with existing Phase 1 sources
//!
//! ## Severity ladder
//!
//! By species' trafficking risk (CITES Appendix I + media coverage):
//!   - pangolins / rhinos / tigers → priority (most-trafficked
//!     vertebrates per UNODC 2024)
//!   - elephants / gorillas / orangutans / sea turtles → routine
//!   - other Appendix II species → info
//!
//! ## Cadence
//!
//! 24h — most GBIF observations are aggregated monthly, but the
//! occurrence_date filter (`eventDate >= now-30d`) keeps us on the
//! freshest 30-day window. 60 req/min unauthenticated rate limit
//! allows ~12 species × 1 query/day easily.
//!
//! ## external_id
//!
//! `gbif:{occurrence_key}` — GBIF's `key` is the canonical occurrence
//! ID (stable; geo_events dedups across re-polls).
//!
//! ## Auth
//!
//! Keyless. GBIF API is fully public (Creative Commons data).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.gbif.org/v1/occurrence/search";

/// Max signals emitted per day. 12 species × ~10 occurrences per
/// query = ~120 signals raw. Cap at 30 to keep geo_events from
/// flooding while surfacing the most newsworthy observations.
const TOP_N: usize = 30;

/// Curated watchlist of CITES-trafficking-relevant species with
/// their GBIF taxonKey and severity tier. Names are kept short for
/// display. Coverage: 12 high-trafficking vertebrate species.
///
/// `severity` is the *base* severity for any occurrence of this
/// species — info/routine/priority. Future spec can layer
/// location/context (e.g. "pangolin in protected area = priority",
/// "pangolin in zoo = info") — out of scope here.
const WATCHLIST: &[(u64, &str, Severity)] = &[
    // High-risk (priority)
    (5_219_586, "Manis spp. (pangolin)", Severity::Priority),
    (5_219_644, "Rhinocerotidae (rhino)", Severity::Priority),
    (5_219_625, "Panthera tigris (tiger)", Severity::Priority),
    // Medium-risk (routine)
    (5_219_521, "Loxodonta africana (African elephant)", Severity::Routine),
    (2_439_195, "Gorilla gorilla (gorilla)", Severity::Routine),
    (5_219_545, "Pongo (orangutan)", Severity::Routine),
    (5_221_347, "Cheloniidae (sea turtle)", Severity::Routine),
    // Low-risk / informational (info) — still worth tracking
    (5_219_570, "Hyaenidae (hyena)", Severity::Info),
    (5_219_588, "Panthera pardus (leopard)", Severity::Info),
    (5_219_604, "Accipitridae (eagle/hawk)", Severity::Info),
    (5_219_612, "Psittacidae (parrot)", Severity::Info),
    (5_222_421, "Orchidaceae (orchid)", Severity::Info),
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Severity {
    Priority,
    Routine,
    Info,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Priority => "priority",
            Severity::Routine => "routine",
            Severity::Info => "info",
        }
    }
}

pub struct Gbif;

impl Source for Gbif {
    fn name(&self) -> &'static str {
        "gbif"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600) // 24h — roadmap §3
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let mut all = Vec::new();
            for (taxon_key, species_name, base_severity) in WATCHLIST.iter().copied() {
                let url = format!(
                    "{BASE_URL}?taxonKey={taxon_key}&limit={limit}&fields=key,species,decimalLatitude,decimalLongitude,country,stateProvince,eventDate,basisOfRecord,datasetName,occurrenceStatus&eventDate={since}",
                    limit = 5, // per-species cap; 12 × 5 = 60 max raw
                    since = gbif_date_window()
                );
                let resp = match ctx.http.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(taxon_key, error = %e, "gbif fetch failed");
                        continue;
                    }
                };
                if !resp.status().is_success() {
                    tracing::warn!(taxon_key, status = %resp.status(), "gbif non-2xx");
                    continue;
                }
                let body: serde_json::Value = match resp.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::warn!(taxon_key, error = %e, "gbif parse failed");
                        continue;
                    }
                };
                parse_results(&body, species_name, base_severity, &mut all);
            }
            // Cap total emitted
            all.truncate(TOP_N);
            Ok(all)
        }
        .boxed()
    }
}

/// Date window for the eventDate filter: last 30 days (GBIF aggregates
/// ~monthly so 30d gives fresh observations without too much backlog).
/// Returns the GBIF ISO-8601 date prefix it expects for `eventDate >= ...`.
fn gbif_date_window() -> String {
    // 30 days ago in YYYY-MM-DD format. Cheaper than pulling chrono in here.
    let secs_in_day = 86400;
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let since_days = (now_secs - 30 * secs_in_day) / secs_in_day;
    // YYYY-MM-DD from Unix days. Approximate — DST/timezone ignored, GBIF
    // parses day-precision anyway.
    let secs = (since_days as u64) * (secs_in_day as u64);
    let dt = time_from_unix(secs);
    format!("{dt}")
}

/// Minimal epoch-seconds → "YYYY-MM-DD" without bringing in chrono.
fn time_from_unix(secs: u64) -> String {
    // Days from 1970-01-01
    let days = (secs / 86_400) as i64;
    // Convert via civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Parse one GBIF occurrence-search response. Pushes results into `out`.
fn parse_results(j: &serde_json::Value, species_name: &str, base_severity: Severity, out: &mut Vec<Signal>) {
    let Some(results) = j.get("results").and_then(|v| v.as_array()) else {
        return;
    };
    for r in results {
        let key = r.get("key").and_then(|v| v.as_i64()).unwrap_or(0);
        if key == 0 {
            continue;
        }
        let lat = r
            .get("decimalLatitude")
            .and_then(|v| v.as_f64());
        let lon = r
            .get("decimalLongitude")
            .and_then(|v| v.as_f64());
        let (lat, lon) = match (lat, lon) {
            (Some(a), Some(b)) if a.is_finite() && b.is_finite() => (a, b),
            _ => continue, // skip records without coords
        };
        let species = r
            .get("species")
            .and_then(|v| v.as_str())
            .unwrap_or(species_name);
        let country = r.get("country").and_then(|v| v.as_str()).unwrap_or("");
        let state = r
            .get("stateProvince")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let basis = r
            .get("basisOfRecord")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let event_date = r.get("eventDate").and_then(|v| v.as_str()).unwrap_or("");
        let dataset = r
            .get("datasetName")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let status = r
            .get("occurrenceStatus")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let (kind, severity) = match base_severity {
            Severity::Priority => ("wildlife_priority", "priority"),
            Severity::Routine => ("wildlife_routine", "routine"),
            Severity::Info => ("wildlife_info", "info"),
        };
        let title = format!(
            "{species} — {country}{}{basis}",
            if state.is_empty() {
                String::new()
            } else {
                format!("/{state}, ")
            },
        );
        out.push(
            Signal::new(kind, title, lat, lon, format!("gbif:{key}"))
                .severity(severity)
                .payload(serde_json::json!({
                    "key": key,
                    "species": species,
                    "taxon_name": species_name,
                    "country": country,
                    "state": state,
                    "basis_of_record": basis,
                    "event_date": event_date,
                    "dataset": dataset,
                    "occurrence_status": status,
                    "extreme_type": kind,
                })),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_response() -> serde_json::Value {
        serde_json::json!({
            "offset": 0,
            "limit": 5,
            "endOfRecords": false,
            "count": 3,
            "results": [
                {
                    "key": 1001,
                    "species": "Manis javanica",
                    "decimalLatitude": -6.2,
                    "decimalLongitude": 106.8,
                    "country": "Indonesia",
                    "stateProvince": "Java",
                    "eventDate": "2026-09-15T00:00",
                    "basisOfRecord": "HUMAN_OBSERVATION",
                    "datasetName": "iNaturalist research-grade observations",
                    "occurrenceStatus": "PRESENT"
                },
                {
                    "key": 1002,
                    "species": "Loxodonta africana",
                    "decimalLatitude": -1.4,
                    "decimalLongitude": 35.0,
                    "country": "Kenya",
                    "stateProvince": "Maasai Mara",
                    "eventDate": "2026-09-20T00:00",
                    "basisOfRecord": "MACHINE_OBSERVATION",
                    "datasetName": "Snapshot Safari",
                    "occurrenceStatus": "PRESENT"
                },
                {
                    "key": 0, // missing key → skip
                    "species": "Unknown",
                    "decimalLatitude": 0.0,
                    "decimalLongitude": 0.0
                }
            ]
        })
    }

    /// Happy path: 3 results in (one with key=0 → skipped), 2 emitted.
    #[test]
    fn parses_results_with_skips() {
        let mut out = Vec::new();
        parse_results(&sample_response(), "pangolin", Severity::Priority, &mut out);
        assert_eq!(out.len(), 2);
        // first (pangolin) → priority
        assert_eq!(out[0].severity, "priority");
        assert_eq!(out[0].kind, "wildlife_priority");
        // second (elephant) → routine (different base severity)
        // Note: sample uses pangolin severity for both — fix below
    }

    /// Severity depends on the species' base severity (passed in).
    #[test]
    fn severity_driven_by_base() {
        let mut out = Vec::new();
        parse_results(&sample_response(), "African elephant", Severity::Routine, &mut out);
        // Both records get routine (since base = Routine)
        assert_eq!(out[0].severity, "routine");
        assert_eq!(out[1].severity, "routine");
    }

    /// Coords preserved for geo-localization.
    #[test]
    fn coords_preserved() {
        let mut out = Vec::new();
        parse_results(&sample_response(), "pangolin", Severity::Priority, &mut out);
        // Pangolin: -6.2, 106.8
        assert!((out[0].lat + 6.2).abs() < 1e-9);
        assert!((out[0].lon - 106.8).abs() < 1e-9);
    }

    /// external_id = 'gbif:{key}'.
    #[test]
    fn external_id_shape() {
        let mut out = Vec::new();
        parse_results(&sample_response(), "pangolin", Severity::Priority, &mut out);
        assert_eq!(out[0].external_id, "gbif:1001");
        assert_eq!(out[1].external_id, "gbif:1002");
    }

    /// Records missing coords are skipped (defensive).
    #[test]
    fn skips_missing_coords() {
        let j = serde_json::json!({
            "results": [{
                "key": 1,
                "species": "Unknown",
                "decimalLatitude": null,
                "decimalLongitude": null
            }]
        });
        let mut out = Vec::new();
        parse_results(&j, "x", Severity::Info, &mut out);
        assert!(out.is_empty());
    }

    /// Records with key=0 are skipped (defensive).
    #[test]
    fn skips_zero_key() {
        let j = serde_json::json!({
            "results": [{
                "key": 0,
                "species": "Unknown",
                "decimalLatitude": 1.0,
                "decimalLongitude": 2.0
            }]
        });
        let mut out = Vec::new();
        parse_results(&j, "x", Severity::Info, &mut out);
        assert!(out.is_empty());
    }

    /// Defensive: missing results array → no panic.
    #[test]
    fn missing_results_returns_empty() {
        let mut out = Vec::new();
        parse_results(&serde_json::json!({}), "x", Severity::Info, &mut out);
        assert!(out.is_empty());
    }

    /// empty results array → no panic.
    #[test]
    fn empty_results_returns_empty() {
        let mut out = Vec::new();
        parse_results(&serde_json::json!({"results": []}), "x", Severity::Info, &mut out);
        assert!(out.is_empty());
    }

    /// Watchlist size sanity.
    #[test]
    fn watchlist_size() {
        assert!(WATCHLIST.len() >= 5);
        assert!(WATCHLIST.len() <= 20); // reasonable cap
    }

    /// All watchlist entries have unique taxonKeys.
    #[test]
    fn watchlist_unique_taxa() {
        let mut seen = std::collections::HashSet::new();
        for (k, _, _) in WATCHLIST {
            assert!(seen.insert(*k), "duplicate taxonKey: {k}");
        }
    }

    /// time_from_unix produces a valid YYYY-MM-DD.
    #[test]
    fn time_from_unix_format() {
        // 2026-01-01 = 1767225600 (approx)
        let s = time_from_unix(1_767_225_600);
        assert_eq!(s.len(), 10);
        assert!(s.starts_with("2026-01"));
    }

    /// gbif_date_window returns valid YYYY-MM-DD.
    #[test]
    fn date_window_format() {
        let s = gbif_date_window();
        assert_eq!(s.len(), 10);
        // Should be ~30 days before today — just sanity check format
        let year: u32 = s[..4].parse().unwrap();
        assert!(year >= 2024 && year <= 2030);
    }

    /// Severity enum maps correctly.
    #[test]
    fn severity_as_str() {
        assert_eq!(Severity::Priority.as_str(), "priority");
        assert_eq!(Severity::Routine.as_str(), "routine");
        assert_eq!(Severity::Info.as_str(), "info");
    }
}