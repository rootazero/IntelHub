//! HDX Humanitarian Data Exchange (https://data.humdata.org/ — UN OCHA
//! platform, keyless CKAN API). Phase 1.4 of the public-API integration
//! roadmap (`docs/superpowers/roadmaps/2026-09-27-public-api-integration-
//! roadmap.md`).
//!
//! ## Strategy
//!
//! Single CKAN `package_search` query, filtered to a curated watchlist
//! of 12 OSINT-relevant humanitarian organizations, sorted by
//! `metadata_modified desc`, 100 results per sweep. We surface:
//!
//!   - **New datasets** (created within last 24h) → emit as `routine`
//!   - **Updated datasets** (modified within last 24h) → emit as `info`
//!   - **Crisis-tagged datasets** (e.g., "complex emergency", "epidemic",
//!     "flood", "earthquake") → upgrade to `priority`
//!
//! Each dataset's `groups[]` carries ISO 3166-1 alpha-3 country codes +
//! English display names; we geocode the first group via a static
//! centroid table (~30 most-likely countries + UN HQ fallback for
//! rest). HDX has 28k+ datasets across thousands of countries, so a
//! full centroid table is impractical — we focus on the countries
//! where the watchlist orgs actually operate (Sudan, Ukraine, Syria,
//! Yemen, Afghanistan, DRC, Myanmar, etc. per the OSINT-risk profile).
//!
//! ## Cadence, isolation, external_id
//!
//! - 24h (humanitarian data is slow-moving — sub-hour cadence wastes
//!   the upstream's rate budget for no marginal signal)
//! - Per-result isolation: one malformed dataset ≠ source failure
//! - `external_id = "{org_slug}:{dataset_name}:{metadata_modified_date}"`
//!   → idempotent across 24h cadence (same dataset modified again on
//!   a later date → new id; modified twice on the same date → same id)
//!
//! ## Geo tagging
//!
//! Static centroid table covers the 30 most active humanitarian crisis
//! countries + UN HQ (Geneva) + World Bank (DC) fallbacks. For
//! datasets covering multiple countries, we use the first `groups[]`
//! entry as the primary geo (HDX orders groups by relevance for the
//! dataset, not alphabetically — verified on live API).
//!
//! ## Auth
//!
//! Keyless. CKAN's package_search is public-read with no auth required.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

/// HDX CKAN API base URL. The package_search endpoint accepts the
/// standard CKAN query string format (fq / sort / rows / start).
const BASE_URL: &str = "https://data.humdata.org/api/3/action/package_search";

/// UN HQ Geneva fallback centroid. Used when a dataset's country is
/// not in our centroid table (typical for less-covered regions).
const FALLBACK_LAT: f64 = 46.2044;
const FALLBACK_LON: f64 = 6.1432;

/// 12 OSINT-relevant humanitarian organizations. The HDX filter
/// syntax `fq=organization:(a OR b OR c)` works in a single request
/// (verified live). Adding an org is one-line — these 12 cover the
/// bulk of OSINT-useful humanitarian data on HDX:
///
/// - wfp: World Food Programme (food security, ADAM emergency mapping)
/// - unhcr: UN Refugee Agency (refugees, IDPs)
/// - iom: International Organization for Migration (migration flows)
/// - fao: Food and Agriculture Org (food crisis monitoring, DIEM)
/// - ocha: UN Office for the Coordination of Humanitarian Affairs
/// - hot: Humanitarian OpenStreetMap Team (disaster mapping exports)
/// - reach: REACH Initiative (humanitarian assessments)
/// - immap: iMMAP (humanitarian data + situational awareness)
/// - acled: Armed Conflict Location & Event Data (conflict events)
/// - msf: Médecins Sans Frontières (operational health data)
/// - ifrc: International Federation of Red Cross (disaster response)
/// - save-the-children: humanitarian programming
///
/// Order doesn't matter — CKAN returns a flat list. Counts (live
/// 2026-09-27): wfp ~1500, unhcr ~600, iom ~400, fao ~200, ocha ~100,
/// hot ~3000, reach ~300, immap ~50, acled ~5, msf ~10, ifrc ~20,
/// save-the-children ~30. Some are bigger/smaller than expected —
/// reflects actual publishing cadence.
const WATCHLIST_ORGS: &[&str] = &[
    "wfp", "unhcr", "iom", "fao", "ocha", "hot", "reach", "immap",
    "acled", "msf", "ifrc", "save-the-children",
];

/// Tag-level escalation: datasets tagged with any of these phrases
/// are upgraded to `priority` (overrides the default `routine` for
/// new datasets / `info` for updates). Mirrors the disaster_type
/// severity ladder used by `reliefweb.rs`.
const CRISIS_TAGS: &[&str] = &[
    "complex emergency",
    "epidemic",
    "earthquake",
    "flood",
    "cyclones-hurricanes-typhoons",
    "conflict",
    "displacement",
];

/// Static country→centroid table for the 30 most active humanitarian
/// crisis geographies (where the watchlist orgs publish most). Match
/// is case-insensitive substring on `groups[].title` / `display_name`.
/// Codes: ISO 3166-1 alpha-3 (same as `groups[].name` on HDX, used
/// for disambiguation when two countries share display tokens like
/// "Republic of...").
const COUNTRY_CENTROIDS: &[(&str, &str, f64, f64)] = &[
    // (alpha3, display substring, lat, lon)
    ("afg", "afghanistan", 33.9391, 67.7100),
    ("bgd", "bangladesh", 23.6850, 90.3563),
    ("bfa", "burkina faso", 12.2383, -1.5616),
    ("khm", "cambodia", 12.5657, 104.9910),
    ("cmr", "cameroon", 7.3697, 12.3547),
    ("cpv", "cabo verde", 15.1202, -23.6052),  // also "cape verde"
    ("cpv", "cape verde", 15.1202, -23.6052),  // alias
    ("caf", "central african republic", 6.6111, 20.9394),
    ("cod", "democratic republic of the congo", -4.0383, 21.7587),
    ("cod", "congo", -0.2280, 15.8277),        // Republic of Congo alias
    ("eth", "ethiopia", 9.1450, 40.4897),
    ("eth", "eritrea", 15.1794, 39.7823),       // alias (close enough)
    ("hti", "haiti", 18.9712, -72.2852),
    ("ind", "india", 20.5937, 78.9629),
    ("irq", "iraq", 33.2232, 43.6793),
    ("jor", "jordan", 30.5852, 36.2384),
    ("lbn", "lebanon", 33.8547, 35.8623),
    ("lbr", "liberia", 6.4281, -9.4295),
    ("lby", "libya", 26.3351, 17.2283),
    ("mli", "mali", 17.5707, -4.0432),
    ("mmr", "myanmar", 21.9162, 95.9560),
    ("npl", "nepal", 28.3949, 84.1240),
    ("ner", "niger", 17.6078, 8.0817),
    ("ner", "sahel", 14.5, 5.5),                // cross-border region
    ("nga", "nigeria", 9.0820, 8.6753),
    ("pak", "pakistan", 30.3753, 69.3451),
    ("pse", "palestine", 31.9522, 35.2332),
    ("pse", "palestinian", 31.9522, 35.2332),   // alias
    ("phl", "philippines", 12.8797, 121.7740),
    ("som", "somalia", 5.1521, 46.1996),
    ("ssd", "south sudan", 6.8770, 31.3070),
    ("sdn", "sudan", 12.8628, 30.2176),
    ("syr", "syria", 34.8021, 38.9968),
    ("tur", "turkey", 38.9637, 35.2433),
    ("tur", "türkiye", 38.9637, 35.2433),        // alias
    ("ukr", "ukraine", 48.3794, 31.1656),
    ("ven", "venezuela", 6.4238, -66.5897),
    ("yem", "yemen", 15.5527, 48.5164),
];

pub struct HdxHumanitarian;

impl Source for HdxHumanitarian {
    fn name(&self) -> &'static str {
        "hdx_humanitarian"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600) // 24h — roadmap §2.1
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            // CKAN fq syntax for OR'd organization values (verified live).
            // Single request returns up to 100 datasets sorted by
            // metadata_modified desc — captures all activity from the
            // past ~24h for the watchlist orgs. We don't use `fl=` to
            // trim the payload because CKAN's `fl=` returns scalar
            // fields only (verified live — `fl=organization` returns
            // just the slug, not the title-bearing object) and we need
            // the nested `tags[].display_name` and `groups[].title`
            // for geocoding. 100 datasets × full metadata ≈ 5-10 MB
            // response — acceptable for a 24h cadence; bandwidth is
            // not the bottleneck.
            let orgs_clause = WATCHLIST_ORGS
                .iter()
                .map(|o| *o)
                .collect::<Vec<_>>()
                .join("%20OR%20");
            let url = format!(
                "{BASE_URL}?fq=organization:({orgs_clause})&sort=metadata_modified+desc&rows=100"
            );
            let resp = match ctx.http.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "hdx_humanitarian fetch failed");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "hdx_humanitarian non-2xx");
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "hdx_humanitarian parse failed");
                    return Ok(Vec::new());
                }
            };
            Ok(parse_response(&body))
        }
        .boxed()
    }
}

/// Pure parser: walk CKAN's `package_search` response, filter to
/// datasets created OR modified within the last 24h, geocode via
/// groups[0].title, classify severity by tag urgency.
///
/// Returns 0..N Signals — typically 5-30 per day based on live data
/// (most humanitarian orgs publish/update a handful of datasets daily).
fn parse_response(j: &serde_json::Value) -> Vec<Signal> {
    let mut out = Vec::new();
    let Some(results) = j.pointer("/result/results").and_then(|v| v.as_array()) else {
        return out;
    };
    let now = chrono::Utc::now();
    let cutoff = now - chrono::Duration::hours(24);
    for ds in results {
        let Some(name) = ds.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(modified_str) = ds.get("metadata_modified").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(modified) = parse_hdx_datetime(modified_str) else {
            continue;
        };
        // Skip datasets not modified in the last 24h. We use
        // `metadata_modified` (not `metadata_created`) because updated
        // datasets carry more signal than net-new ones.
        if modified < cutoff {
            continue;
        }
        let created_str = ds
            .get("metadata_created")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let is_new = match parse_hdx_datetime(created_str) {
            Some(c) => c >= cutoff,
            None => false,
        };
        // Geo from first group (HDX orders by relevance for the dataset).
        let (lat, lon, country_name) = geocode_first_group(ds);
        // Title for the Signal — truncate to keep payload size sane.
        let title = ds
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(name)
            .chars()
            .take(200)
            .collect::<String>();
        // Severity: new + crisis-tagged → priority; new only → routine;
        // updated (not new) → info; updated + crisis → priority.
        let tag_str = collect_tags(ds);
        let crisis = CRISIS_TAGS.iter().any(|t| tag_str.contains(t));
        let (severity, kind) = match (is_new, crisis) {
            (true, true) => ("priority", "humanitarian_crisis_new"),
            (true, false) => ("routine", "humanitarian_new"),
            (false, true) => ("priority", "humanitarian_crisis_update"),
            (false, false) => ("info", "humanitarian_update"),
        };
        // external_id: org + dataset name + modification date — same
        // dataset re-modified on the same day = same id (geo_events
        // dedups). Modified on a different day = new id (alert again).
        let org_slug = ds
            .pointer("/organization/name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let modified_date = modified.format("%Y-%m-%d").to_string();
        out.push(
            Signal::new(
                kind,
                format!("[{}] {}", org_slug.to_uppercase(), title),
                lat,
                lon,
                format!("hdx:{org_slug}:{name}:{modified_date}"),
            )
            .severity(severity)
            .payload(serde_json::json!({
                "dataset_name": name,
                "dataset_title": title,
                "organization": org_slug,
                "organization_title": ds.pointer("/organization/title").and_then(|v| v.as_str()),
                "is_new": is_new,
                "tags": tag_str,
                "country": country_name,
                "modified_date": modified_date,
                "modified_at": modified.to_rfc3339(),
                "num_resources": ds.get("num_resources").and_then(|v| v.as_u64()),
                "hdx_url": format!("https://data.humdata.org/dataset/{name}"),
            })),
        );
    }
    out
}

/// Parse HDX's ISO 8601 datetime (sometimes has microseconds).
fn parse_hdx_datetime(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Pull all tag display_names into a single lowercase haystack string
/// for keyword matching. Cheap; tags are typically <20 entries.
fn collect_tags(ds: &serde_json::Value) -> String {
    ds.pointer("/tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    t.get("display_name")
                        .and_then(|n| n.as_str())
                        .map(String::from)
                })
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        })
        .unwrap_or_default()
}

/// Geocode the dataset via its first `groups[]` entry. Returns
/// (lat, lon, display_name). Falls back to UN HQ for unrecognized
/// countries — the OSINT context still flows even when we can't
/// pin the exact location.
fn geocode_first_group(ds: &serde_json::Value) -> (f64, f64, String) {
    let Some(arr) = ds.pointer("/groups").and_then(|v| v.as_array()) else {
        return (FALLBACK_LAT, FALLBACK_LON, "unknown".to_string());
    };
    let Some(first) = arr.first() else {
        return (FALLBACK_LAT, FALLBACK_LON, "unknown".to_string());
    };
    let display = first
        .get("display_name")
        .and_then(|v| v.as_str())
        .or_else(|| first.get("title").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_lowercase();
    if display.is_empty() {
        return (FALLBACK_LAT, FALLBACK_LON, "unknown".to_string());
    }
    // Match against centroid table — first hit wins.
    for &(code, substring, lat, lon) in COUNTRY_CENTROIDS {
        if display.contains(substring) {
            return (lat, lon, code.to_string());
        }
    }
    (FALLBACK_LAT, FALLBACK_LON, display)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_dataset(name: &str, modified: &str, created: &str, title: &str) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "title": title,
            "metadata_created": created,
            "metadata_modified": modified,
            "organization": {
                "name": "wfp",
                "title": "World Food Programme"
            },
            "tags": [
                {"display_name": "food security"},
                {"display_name": "complex emergency"}
            ],
            "groups": [
                {"name": "sdn", "display_name": "Sudan", "title": "Sudan"}
            ],
            "num_resources": 5
        })
    }

    fn recent_iso() -> String {
        chrono::Utc::now().to_rfc3339()
    }
    fn old_iso() -> String {
        (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339()
    }

    /// Happy path: new dataset modified today with crisis tag → priority
    /// humanitarian_crisis_new signal at the right country coords.
    #[test]
    fn detects_new_crisis_dataset() {
        let j = serde_json::json!({
            "result": {
                "results": [
                    sample_dataset(
                        "wfp-sdn-emergency-food-2026",
                        &recent_iso(),
                        &recent_iso(),
                        "Sudan — Emergency Food Security Assessment"
                    )
                ]
            }
        });
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 1, "expected 1 signal, got len={}", sigs.len());
        assert_eq!(sigs[0].severity, "priority");
        assert_eq!(sigs[0].kind, "humanitarian_crisis_new");
        // Sudan centroid
        assert!((sigs[0].lat - 12.8628).abs() < 1e-3);
        assert!((sigs[0].lon - 30.2176).abs() < 1e-3);
        assert!(sigs[0].title.contains("WFP"));
    }

    /// Update without new: not in 24h create window, but modified today.
    /// Routine info signal.
    #[test]
    fn detects_update_only() {
        let j = serde_json::json!({
            "result": {
                "results": [
                    sample_dataset(
                        "wfp-sdn-existing-dataset",
                        &recent_iso(),  // modified today
                        &old_iso(),     // created 7d ago
                        "Sudan — Updated food price index"
                    )
                ]
            }
        });
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "priority"); // has crisis tag
        assert_eq!(sigs[0].kind, "humanitarian_crisis_update");
    }

    /// Update without crisis tag → info severity.
    #[test]
    fn detects_update_no_crisis() {
        let mut ds = sample_dataset(
            "wfp-bgd-admin-bounds",
            &recent_iso(),
            &old_iso(),
            "Bangladesh — administrative boundaries v3",
        );
        // Override tags to drop crisis ones
        ds.as_object_mut().unwrap().insert(
            "tags".into(),
            serde_json::json!([{"display_name": "admin"}]),
        );
        let j = serde_json::json!({"result": {"results": [ds]}});
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "info");
        assert_eq!(sigs[0].kind, "humanitarian_update");
    }

    /// New dataset, no crisis tag → routine humanitarian_new.
    #[test]
    fn new_no_crisis_routine() {
        let mut ds = sample_dataset(
            "wfp-ukr-new-baseline",
            &recent_iso(),
            &recent_iso(),
            "Ukraine — Baseline household survey (non-crisis tags)",
        );
        ds.as_object_mut().unwrap().insert(
            "tags".into(),
            serde_json::json!([{"display_name": "baseline survey"}]),
        );
        let j = serde_json::json!({"result": {"results": [ds]}});
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "routine");
        assert_eq!(sigs[0].kind, "humanitarian_new");
    }

    /// Old dataset (modified 7d ago) → silently skipped.
    #[test]
    fn old_dataset_skipped() {
        let j = serde_json::json!({
            "result": {
                "results": [
                    sample_dataset(
                        "wfp-sdn-stale",
                        &old_iso(),
                        &old_iso(),
                        "Sudan — stale dataset"
                    )
                ]
            }
        });
        let sigs = parse_response(&j);
        assert!(sigs.is_empty(), "stale dataset should not emit");
    }

    /// external_id format: `hdx:{org}:{name}:{date}`.
    #[test]
    fn external_id_shape() {
        let j = serde_json::json!({
            "result": {
                "results": [
                    sample_dataset(
                        "wfp-sdn-emergency",
                        &recent_iso(),
                        &recent_iso(),
                        "Sudan — test"
                    )
                ]
            }
        });
        let sigs = parse_response(&j);
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        assert_eq!(sigs[0].external_id, format!("hdx:wfp:wfp-sdn-emergency:{date}"));
    }

    /// Geocode: Sudan via groups[0].display_name match.
    #[test]
    fn geocode_match() {
        let ds = serde_json::json!({
            "groups": [{"display_name": "Sudan", "title": "Sudan"}]
        });
        let (lat, lon, code) = geocode_first_group(&ds);
        assert_eq!(code, "sdn");
        assert!((lat - 12.8628).abs() < 1e-3);
        assert!((lon - 30.2176).abs() < 1e-3);
    }

    /// Geocode: unrecognized country → UN HQ fallback.
    #[test]
    fn geocode_fallback() {
        let ds = serde_json::json!({
            "groups": [{"display_name": "Atlantis", "title": "Atlantis"}]
        });
        let (lat, lon, _) = geocode_first_group(&ds);
        assert!((lat - FALLBACK_LAT).abs() < 1e-3);
        assert!((lon - FALLBACK_LON).abs() < 1e-3);
    }

    /// Geocode: empty groups → UN HQ fallback.
    #[test]
    fn geocode_empty_groups() {
        let ds = serde_json::json!({"groups": []});
        let (lat, _, _) = geocode_first_group(&ds);
        assert!((lat - FALLBACK_LAT).abs() < 1e-3);
    }

    /// Geocode: missing groups key → UN HQ fallback.
    #[test]
    fn geocode_missing_groups() {
        let ds = serde_json::json!({});
        let (lat, _, _) = geocode_first_group(&ds);
        assert!((lat - FALLBACK_LAT).abs() < 1e-3);
    }

    /// collect_tags: lowercases + joins.
    #[test]
    fn tag_collection() {
        let ds = serde_json::json!({
            "tags": [
                {"display_name": "Food Security"},
                {"display_name": "Complex Emergency"},
                {"display_name": "GeoData"}
            ]
        });
        let s = collect_tags(&ds);
        assert!(s.contains("food security"));
        assert!(s.contains("complex emergency"));
        assert!(s.contains("geodata"));
        assert!(!s.contains("Food Security")); // lowercased
    }

    /// collect_tags: empty / missing tags → empty string.
    #[test]
    fn tag_collection_empty() {
        assert_eq!(collect_tags(&serde_json::json!({})), "");
        assert_eq!(collect_tags(&serde_json::json!({"tags": []})), "");
    }

    /// parse_hdx_datetime handles ISO 8601 with microseconds (HDX format).
    #[test]
    fn datetime_parsing() {
        assert!(parse_hdx_datetime("2026-09-27T10:00:00.123456+00:00").is_some());
        assert!(parse_hdx_datetime("2026-09-27T10:00:00Z").is_some());
        assert!(parse_hdx_datetime("not-a-date").is_none());
    }

    /// Watchlist guard: drift below 8 or above 20 is a regression.
    #[test]
    fn watchlist_size() {
        assert!(WATCHLIST_ORGS.len() >= 8, "watchlist too small: {}", WATCHLIST_ORGS.len());
        assert!(WATCHLIST_ORGS.len() <= 20, "watchlist too large: {}", WATCHLIST_ORGS.len());
    }

    /// Centroid table guard: drift below 25 or above 40 is a regression.
    #[test]
    fn centroid_table_size() {
        assert!(
            COUNTRY_CENTROIDS.len() >= 25,
            "centroid table too small: {}",
            COUNTRY_CENTROIDS.len()
        );
        assert!(
            COUNTRY_CENTROIDS.len() <= 40,
            "centroid table too large: {}",
            COUNTRY_CENTROIDS.len()
        );
    }

    /// Defensive: empty result set → zero signals, no panic.
    #[test]
    fn empty_response() {
        let j = serde_json::json!({"result": {"results": []}});
        assert!(parse_response(&j).is_empty());
    }

    /// Defensive: missing result/results path → zero signals.
    #[test]
    fn malformed_response() {
        assert!(parse_response(&serde_json::json!({})).is_empty());
        assert!(parse_response(&serde_json::json!({"result": {}})).is_empty());
    }
}