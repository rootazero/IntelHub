//! Strait of Hormuz Ship Monitor (https://hormuz.data-tracking.net/) —
//! Phase 3.2 of the public-API integration roadmap (`docs/superpowers/
//! roadmaps/2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Why Phase 3.2 (chokepoint OSINT)
//!
//! The Strait of Hormuz is the single most important maritime
//! chokepoint on Earth — ~20% of global oil supply transits this
//! 21-nautical-mile-wide strait between the Persian Gulf and the Gulf
//! of Oman. data-tracking.net publishes a CC-BY-4.0 licensed AIS-based
//! crossing feed (`/api/crossings`); keyless, free for any use, no
//! registration. **Phase 3.2 is the FIRST Phase 3 source shipped** per
//! roadmap §4.1 sequencing — it proves the chokepoint-alert pattern
//! (a source whose data is intrinsically geopolitical — flag + dwt +
//! transit-path together encode "is this a sanctioned tanker?"), and
//! the alert-rule scaffolding it ships with will plug into ArcNautical
//! (3.1) and CompliAPI (3.3) when those paid APIs land.
//!
//! ## Strategy
//!
//! - 30-min cadence (matches upstream "updated every 30 min")
//! - Single GET to `https://hormuz.data-tracking.net/api/crossings`
//!   returns the most-recent N crossings (keyless, CC-BY-4.0).
//! - Each crossing has a stable `crossing_id` (server-assigned
//!   auto-increment). external_id = `hormuz:{crossing_id}`.
//! - Geo: every crossing is at the Strait chokepoint (~26.56°N,
//!   56.25°E); we don't ship the upstream's zone enum as a geo coord
//!   because zone centroids are at sea and 100s of km apart — for an
//!   analyst the chokepoint pin is the right grain.
//!
//! ## Severity ladder (geopolitical-flavored, per roadmap §4)
//!
//! - IR-flagged Oil/Chemical Tanker with `dwt >= 50000` → **priority**.
//!   Sanctioned-flagged tanker at the chokepoint = direct OSINT
//!   signal worth the analyst's attention.
//! - Oil/Chemical Tanker (any flag) with `dwt >= 50000` → routine
//!   (big-oil-mover chatter — useful context, not alarming on its own).
//! - Any Tanker or Bulker with `dwt >= 25000` → routine
//!   (medium commercial flow).
//! - All others (Yachts, General Cargo, Military, etc.) → info
//!   (ambient traffic — useful as the chokepoint pulse, but not an
//!   alert in its own right).
//!
//! Per-category watchlist routing (per roadmap §4.1: "proves the
//! chokepoint-alert pattern"): future phases can compose the
//! `flag=IR & category=Tanker` subset with OFAC / ArcNautical / EU
//! sanctions lists in Phase 3.1 + 3.3. For Phase 3.2 v1 we don't
//! query the segmenter; we just emit signals and let downstream
//! alert-rules correlate.
//!
//! ## external_id
//!
//! `hormuz:{crossing_id}` — server-assigned. Same id re-delivered next
//! sweep → `geo_events` idempotent dedup (the rows already exist).
//!
//! ## Auth
//!
//! None. Public CC-BY-4.0 dataset, Cloudflare-fronted. We send the
//! default browser UA (the shared `Ctx::http` client), which
//! Cloudflare accepts.
//!
//! ## Rate limits
//!
//! No published limits; we hit `/api/crossings` once per 30-min sweep.
//! 48 req/day. Well under any throttle.
use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://hormuz.data-tracking.net/api/crossings";

/// 30-min cadence (matches upstream's own update schedule — polling
/// faster wastes HOUR and risks being throttled).
const INTERVAL_SECS: u64 = 30 * 60;

/// Geo center of the Strait of Hormuz chokepoint. JSON-LD on the
/// upstream homepage advertises 26.56 / 56.25 — every crossing is
/// at the strait regardless of first/last seen zone.
const STRAIT_LAT: f64 = 26.56;
const STRAIT_LON: f64 = 56.25;

/// Cap on signals emitted per sweep. The upstream returns a
/// sliding window of recent crossings (typically 30-60 over 24h);
/// we cap at 30 to keep geo_events flowing smoothly without flooding
/// the bus during a surge (e.g. a military convoy).
const TOP_N: usize = 30;

/// Oil/Chemical tanker category string (upstream taxonomy).
const CATEGORY_TANKER: &str = "Oil/Chemical Tanker";

/// Bulk carrier category (upstream taxonomy — these carry grain, ore,
/// and occasionally crude on long-haul routes through Hormuz).
const CATEGORY_BULKER: &str = "Bulk Carrier";

/// Sanctioned/elevated flags for priority elevation. Keep this list
/// conservative — only clear, currently-sanctioned maritime flags.
/// Additions require Phase-3.1 ArcNautical cross-validation, so v1
/// keeps it to IR (most-frequent chokepoint actor with documented
/// sanctioned-flag crossings).
const PRIORITY_FLAGS: &[&str] = &["IR"];

pub struct StraitOfHormuz;

impl Source for StraitOfHormuz {
    fn name(&self) -> &'static str {
        "strait_of_hormuz"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let resp = match ctx.http.get(BASE_URL).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "strait_of_hormuz fetch failed");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "strait_of_hormuz non-2xx");
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "strait_of_hormuz parse failed");
                    return Ok(Vec::new());
                }
            };
            let mut out = parse_crossings(&body);
            out.truncate(TOP_N);
            Ok(out)
        }
        .boxed()
    }
}

/// Parse the crossings JSON array and build Signals. Pure function so
/// it can be unit-tested with fixture data without HTTP.
fn parse_crossings(j: &serde_json::Value) -> Vec<Signal> {
    let Some(arr) = j.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for r in arr {
        let crossing_id = r.get("crossing_id").and_then(|v| v.as_i64()).unwrap_or(0);
        if crossing_id == 0 {
            continue; // skip records missing the stable id
        }
        let ship_name = r.get("ship_name").and_then(|v| v.as_str()).unwrap_or("");
        let mmsi = r.get("mmsi").and_then(|v| v.as_str()).unwrap_or("");
        let flag = r.get("flag").and_then(|v| v.as_str()).unwrap_or("--");
        let category = r
            .get("ship_category")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let dwt = r.get("dwt").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let direction = r.get("direction").and_then(|v| v.as_str()).unwrap_or("");
        let first_zone = r
            .get("first_seen_zone")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let last_zone = r
            .get("last_seen_zone")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let transit_path = r
            .get("transit_path")
            .and_then(|v| v.as_str())
            .unwrap_or("unidentified");
        let detected_at = r.get("detected_at").and_then(|v| v.as_str()).unwrap_or("");
        let destination = r
            .get("destination")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let (kind, severity) = classify(category, flag, dwt);
        let title = build_title(ship_name, flag, category, direction);
        out.push(
            Signal::new(kind, title, STRAIT_LAT, STRAIT_LON, format!("hormuz:{crossing_id}"))
                .severity(severity)
                .payload(serde_json::json!({
                    "crossing_id": crossing_id,
                    "mmsi": mmsi,
                    "ship_name": ship_name,
                    "flag": flag,
                    "ship_category": category,
                    "dwt": dwt,
                    "direction": direction,
                    "first_seen_zone": first_zone,
                    "last_seen_zone": last_zone,
                    "transit_path": transit_path,
                    "destination": destination,
                    "detected_at": detected_at,
                    "extreme_type": kind,
                })),
        );
    }
    out
}

/// Map (category, flag, dwt) → (kind, severity) per the docstring
/// severity ladder.
fn classify(category: &str, flag: &str, dwt: f64) -> (&'static str, &'static str) {
    let is_tanker = category == CATEGORY_TANKER;
    let is_bulker = category == CATEGORY_BULKER;
    let is_priority_flag = PRIORITY_FLAGS.iter().any(|f| *f == flag);

    if is_tanker && is_priority_flag && dwt >= 50_000.0 {
        return ("chokepoint_tanker_priority", "priority");
    }
    if is_tanker && dwt >= 50_000.0 {
        return ("chokepoint_tanker_routine", "routine");
    }
    if (is_tanker || is_bulker) && dwt >= 25_000.0 {
        return ("chokepoint_bulker_routine", "routine");
    }
    ("chokepoint_info", "info")
}

fn build_title(ship_name: &str, flag: &str, category: &str, direction: &str) -> String {
    let name_part = if ship_name.is_empty() {
        "Unknown vessel".to_string()
    } else {
        format!("{ship_name} ({flag})")
    };
    format!("{name_part} — {category} {direction} via Strait of Hormuz")
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_crossings() -> serde_json::Value {
        serde_json::json!([
            {
                "crossing_id": 6701,
                "destination": "B ABBAS KHORAMSHAHR",
                "detected_at": "2026-09-27T00:31:27.594095+00:00",
                "direction": "outbound",
                "dwt": 64_400.0,
                "first_seen_zone": "persian_gulf",
                "flag": "IR",
                "gap_hours": 208.5,
                "last_seen_zone": "gulf_of_oman",
                "length": 59.0,
                "mmsi": "2408358",
                "ship_category": "Oil/Chemical Tanker",
                "ship_name": "IRAN TANKER 1",
                "ship_type": 7,
                "transit_path": "iran"
            },
            {
                "crossing_id": 6719,
                "destination": "FOR ORDERS",
                "detected_at": "2026-09-26T21:01:34.411025+00:00",
                "direction": "inbound",
                "dwt": 60_000.0,
                "first_seen_zone": "outside",
                "flag": "MH",
                "last_seen_zone": "persian_gulf",
                "ship_category": "Oil/Chemical Tanker",
                "ship_name": "MARSHALL TANKER",
                "transit_path": "unidentified"
            },
            {
                "crossing_id": 6720,
                "destination": "CLASS B",
                "detected_at": "2026-09-26T22:01:34.349793+00:00",
                "direction": "outbound",
                "dwt": null,
                "first_seen_zone": "persian_gulf",
                "flag": "--",
                "last_seen_zone": "gulf_of_oman",
                "ship_category": "Yacht",
                "ship_name": "YACHT X",
                "transit_path": "unidentified"
            },
            {
                "crossing_id": 0, // missing id → skip
                "ship_name": "NOID",
                "ship_category": "Yacht",
                "flag": "--",
                "dwt": 0.0,
                "direction": "outbound",
                "transit_path": "unidentified"
            }
        ])
    }

    /// IR-flagged tanker dwt>=50k → chokepoint_tanker_priority
    #[test]
    fn ir_tanker_priority() {
        let mut out = parse_crossings(&sample_crossings());
        assert_eq!(out.len(), 3); // 4 records, 1 dropped (crossing_id=0)
        assert_eq!(out[0].severity, "priority");
        assert_eq!(out[0].kind, "chokepoint_tanker_priority");
        assert!(out[0].title.contains("IRAN TANKER 1"));
        assert!(out[0].title.contains("(IR)"));
    }

    /// Non-IR tanker dwt>=50k → routine
    #[test]
    fn non_ir_tanker_routine() {
        let mut out = parse_crossings(&sample_crossings());
        assert_eq!(out[1].severity, "routine");
        assert_eq!(out[1].kind, "chokepoint_tanker_routine");
    }

    /// Yacht / non-tanker / dwt<25k → info
    #[test]
    fn yacht_info() {
        let mut out = parse_crossings(&sample_crossings());
        assert_eq!(out[2].severity, "info");
        assert_eq!(out[2].kind, "chokepoint_info");
    }

    /// external_id = hormuz:{crossing_id}
    #[test]
    fn external_id_shape() {
        let out = parse_crossings(&sample_crossings());
        assert_eq!(out[0].external_id, "hormuz:6701");
        assert_eq!(out[1].external_id, "hormuz:6719");
    }

    /// Coords pinned to the Strait center
    #[test]
    fn coords_pinned_to_strait() {
        let out = parse_crossings(&sample_crossings());
        for s in &out {
            assert!((s.lat - 26.56).abs() < 1e-9);
            assert!((s.lon - 56.25).abs() < 1e-9);
        }
    }

    /// Bulk carrier dwt>=25k → routine (bulker lane)
    #[test]
    fn bulker_routine() {
        let j = serde_json::json!([{
            "crossing_id": 100,
            "ship_category": "Bulk Carrier",
            "ship_name": "BULK",
            "flag": "PA",
            "dwt": 30_000.0,
            "direction": "inbound",
            "transit_path": "unidentified",
            "first_seen_zone": "gulf_of_oman",
            "last_seen_zone": "persian_gulf",
            "detected_at": "2026-09-27T01:00:00+00:00",
            "destination": ""
        }]);
        let out = parse_crossings(&j);
        assert_eq!(out[0].severity, "routine");
        assert_eq!(out[0].kind, "chokepoint_bulker_routine");
    }

    /// Records missing the crossing_id are skipped
    #[test]
    fn skips_zero_crossing_id() {
        let out = parse_crossings(&sample_crossings());
        assert!(out.iter().all(|s| !s.external_id.ends_with(":0")));
    }

    /// JSON body that isn't an array → empty output, no panic
    #[test]
    fn handles_non_array_body() {
        let j = serde_json::json!({"error": "upstream boom"});
        let out = parse_crossings(&j);
        assert!(out.is_empty());
    }
}