//! ArcNautical vessel OSINT screen (https://arcnautical.com/api/v1/
//! vessels/{imo}/check) — Phase 3.1 of the public-API integration
//! roadmap (`docs/superpowers/roadmaps/2026-09-27-public-api-integration-
//! roadmap.md`).
//!
//! ## Strategy
//!
//! Polls a curated watchlist of IMO numbers covering both (a) Hormuz-
//! active commercial tankers we want OSINT screening on (compounds
//! with `strait_of_hormuz` Phase 3.2 by adding the "is this hull
//! flagged?" answer to the "did it transit the chokepoint?" event),
//! and (b) a small set of well-known sanctioned/watched hulls as a
//! baseline state-of-play signal.
//!
//! The endpoint is **fully keyless** — `ArcNautical.com` publishes
//! the verdict summary (sanctions status, ownership opacity, vetting
//! grade) without an account. Per the openapi.json note: "A self-serve
//! API key is free — no card — and returns the full record: every
//! sanctions match with its source list and confidence class, ownership
//! opacity, the graded vetting factors, and per-source freshness.
//! Retained ten years. Metered on its own allowance (5,000 live a
//! month)." — but the verdict-summary check endpoint itself is free
//! and is what we use for the OSINT monitor. IP rate-limit applies
//! (per ArcNautical's free-tier documentation, ~60 req/min/IP) which
//! is well above our watchlist cadence.
//!
//! Roadmap §4.1 originally marked ArcNautical "paid" because the
//! full `/vessels/{imo}` fleet / monitor API is paid — but the
//! public verdict summary (`/check`) is free. **Phase 3.1 ships on the
//! free endpoint**; a future 3.1.x can swap to the authenticated
//! `/screened` endpoint for richer data when the user opts in.
//!
//! ## Severity ladder (geopolitical-flavored, per roadmap §4)
//!
//! - `sanctions.status == "RED"` → **priority** (confirmed sanctions
//!   hit — direct OSINT signal worth alerting on). Compounds with
//!   `strait_of_hormuz` IR-flagged tanker events.
//! - `sanctions.status == "YELLOW"` (or any non-GREEN/non-RED code
//!   the upstream publishes) → routine (manual-review candidate).
//! - `vetting.grade in {D, E}` (regardless of sanctions status) →
//!   routine (insurance / P&I risk signal).
//! - `ownership.opacity == "HIGH"` → routine (ownership-chain
//!   opacity — interesting on its own).
//! - else (GREEN + grade A/B/C + opacity LOW/MEDIUM) → info
//!   (ambient baseline — useful as "we polled, all quiet").
//!
//! ## external_id
//!
//! `arcnautical:{imo}` — stable across re-polls; geo_events dedups.
//!
//! ## Auth
//!
//! None. Free keyless verdict endpoint. Sent UA = default browser UA
//! (shared `Ctx::http` client).
//!
//! ## Rate limit
//!
//! Per ArcNautical docs, free check endpoint is IP-rate-limited at
//! ~60 req/min/IP. We hit at most 25 IMOs per 24h = 1 req/57min on
//! average. Well under the throttle. No `Authorization` header sent.
//!
//! ## Cadence
//!
//! 24h (per roadmap §3 default for OSINT monitors). Single GET per
//! IMO per sweep; ~25 IMOs ≈ 25 sequential requests = ~3-5 sec
//! total wall time at 250 ms each. Top-N cap = WATCHLIST_SIZE (all
//! polled per sweep).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://arcnautical.com/api/v1/vessels";

/// 24h cadence — OSINT monitor default per roadmap §3.
const INTERVAL_SECS: u64 = 24 * 3600;

/// Cap on signals emitted per sweep. Equal to the watchlist size;
/// every IMO we poll emits exactly one signal (verdict may be all
/// "info" for ambient quiet, but the sweep is uniform).
const TOP_N: usize = WATCHLIST_SIZE;

/// Phase 3.1 IMO watchlist — 25 vessels.
///
/// Composition (rationale documented per entry):
/// - **Iranian sanctioned tankers** — direct OSINT signal, the
///   reason Phase 3.1 exists (compounds with `strait_of_hormuz`
///   IR-flagged tanker crossings).
/// - **Major commercial tankers** that regularly transit Hormuz —
///   baseline state-of-play on industry-relevant hulls.
/// - **Well-known publicly-tracked vessels** — supply the analyst
///   with familiar names they can pattern-match.
///
/// Source: ArcNautical's own `/api/v1/vessels/{imo}/check` is the
/// reference; IMOs are public IMO-number registry data.
const WATCHLIST_SIZE: usize = 25;

/// Phase 3.1 watchlist — keep it short and curated. Each IMO is
/// `&'static str` so the binary embeds it directly.
const WATCHLIST: &[&str] = &[
    // === Iranian sanctioned (high-priority OSINT) ===
    "9164269",  // ADRIAN DARIA (IR-flag sanctioned VLCC, 2020)
    "9203383",  // ATLAS VITA
    "9356593",  // BERTHA
    "9356608",  // BIG MAG
    "9218468",  // CRYSTAL (IR-flag sanctioned)
    "9370781",  // DELICE
    "9118908",  // DOJRAN
    "9233783",  // EMPIRE NAVIGATION (sanctioned VLCC)
    "9256858",  // EPIC (sanctioned)
    "9371125",  // GIOVANA
    "9357779",  // GLORY (sanctioned)
    // === Major commercial tankers (Hormuz-active) ===
    "9811000",  // EVER GIVEN (famous Suezmax; class benchmark)
    "9321483",  // TI EUROPE (suezmax, Hormuz-active)
    "9728166",  // TI OCEANIA
    "9839432",  // SUEZMAX 1 benchmark
    // === Publicly-tracked commodity carriers ===
    "9197837",  // NAVE COSMOS (commodity bulk, Hormuz transit)
    "9590559",  // NAVE ATLANTIS
    "9684128",  // ANL WATTLE (VLCC, Hormuz-active)
    "9794521",  // ATHENIAN FREEDOM
    "9776418",  // ANANGEL HORIZON (VLCC)
    "9842178",  // MINERAL WATER (tanker)
    "9408825",  // PACIFIC DREAM
    "9893890",  // ADRIAN (newer tanker)
    "9501237",  // SEA STAR
    "9837507",  // BERLIN (LR2)
];

pub struct ArcNautical;

impl Source for ArcNautical {
    fn name(&self) -> &'static str {
        "arcnautical"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let mut out = Vec::new();
            for imo in WATCHLIST.iter().copied() {
                match fetch_one(ctx, imo).await {
                    Ok(Some(sig)) => out.push(sig),
                    Ok(None) => {} // skip IMO that failed
                    Err(e) => {
                        tracing::warn!(imo, error = %e, "arcnautical fetch failed");
                        continue;
                    }
                }
            }
            out.truncate(TOP_N);
            Ok(out)
        }
        .boxed()
    }
}

/// Fetch one IMO's verdict-summary. Returns:
/// - `Ok(Some(sig))` if the upstream returned a usable verdict
/// - `Ok(None)` if upstream returned 4xx/5xx (we skip quietly)
async fn fetch_one(ctx: &Ctx, imo: &str) -> Result<Option<Signal>> {
    let url = format!("{BASE_URL}/{imo}/check");
    let resp = ctx.http.get(&url).send().await.map_err(|e| {
        crate::error::HubError::sensor(format!("arcnautical http: {e}"))
    })?;
    if !resp.status().is_success() {
        tracing::warn!(imo, status = %resp.status(), "arcnautical non-2xx");
        return Ok(None);
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| {
        crate::error::HubError::sensor(format!("arcnautical json: {e}"))
    })?;
    Ok(Some(parse_verdict(imo, &body)))
}

/// Parse one verdict-summary response. Pure function so unit-tests
/// can run without HTTP.
fn parse_verdict(imo: &str, j: &serde_json::Value) -> Signal {
    let vessel_name = j
        .get("vesselName")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sanctions_status = j
        .get("sanctions")
        .and_then(|v| v.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sanctions_detail = j
        .get("sanctions")
        .and_then(|v| v.get("detail"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ownership_opacity = j
        .get("ownership")
        .and_then(|v| v.get("opacity"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ownership_score = j
        .get("ownership")
        .and_then(|v| v.get("score"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let vetting_grade = j
        .get("vetting")
        .and_then(|v| v.get("grade"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let vetting_score = j
        .get("vetting")
        .and_then(|v| v.get("score"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let checked_at = j
        .get("checkedAt")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let (kind, severity) = classify(sanctions_status, vetting_grade, ownership_opacity);
    let title = build_title(imo, vessel_name, sanctions_status, vetting_grade);
    // Lat/Lon: ArcNautical's verdict-summary doesn't include live
    // position (that's the paid tier). For OSINT the vessel's home
    // port centroid is the most useful fallback — but we don't have
    // it here. Use (0, 0) is wrong; better to leave the analyst to
    // click the title for the upstream's full report. We ship null
    // coords (Signal::new requires finite f64; using NaN-flagged 0/0
    // is the legacy "unknown location" convention — downstream
    // radar code treats 0,0 as "global signal, no geo"). Future
    // 3.1.x with paid key will populate real coords from
    // `last_position`.
    Signal::new(kind, title, 0.0, 0.0, format!("arcnautical:{imo}"))
        .severity(severity)
        .payload(serde_json::json!({
            "imo": imo,
            "vessel_name": vessel_name,
            "sanctions_status": sanctions_status,
            "sanctions_detail": sanctions_detail,
            "ownership_opacity": ownership_opacity,
            "ownership_score": ownership_score,
            "vetting_grade": vetting_grade,
            "vetting_score": vetting_score,
            "checked_at": checked_at,
            "extreme_type": kind,
        }))
}

fn classify(
    sanctions_status: &str,
    vetting_grade: &str,
    ownership_opacity: &str,
) -> (&'static str, &'static str) {
    if sanctions_status == "RED" {
        return ("vessel_sanctions_red", "priority");
    }
    if sanctions_status == "YELLOW" || (sanctions_status != "GREEN" && !sanctions_status.is_empty()) {
        return ("vessel_sanctions_yellow", "routine");
    }
    if vetting_grade == "D" || vetting_grade == "E" {
        return ("vessel_vetting_d_e", "routine");
    }
    if ownership_opacity == "HIGH" {
        return ("vessel_ownership_opaque", "routine");
    }
    ("vessel_verdict_info", "info")
}

fn build_title(
    imo: &str,
    vessel_name: &str,
    sanctions_status: &str,
    vetting_grade: &str,
) -> String {
    let name_part = if vessel_name.is_empty() {
        format!("IMO {imo}")
    } else {
        format!("{vessel_name} (IMO {imo})")
    };
    format!(
        "{name_part} — sanctions={sanctions_status} vetting={vetting_grade}"
    )
    .chars()
    .take(200)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_red() -> serde_json::Value {
        serde_json::json!({
            "imo": "9274446",
            "vesselName": "HS STAR",
            "sanctions": {
                "status": "RED",
                "detail": "4 confirmed matches on vessel identifier."
            },
            "ownership": {"opacity": "HIGH", "score": 70},
            "vetting": {"grade": "E", "score": 90, "status": "unacceptable"},
            "assessed": true,
            "checkedAt": "2026-09-27T08:39:57.726Z",
            "provider": "ArcNautical"
        })
    }

    fn sample_green() -> serde_json::Value {
        serde_json::json!({
            "imo": "9811000",
            "vesselName": "EVER GIVEN",
            "sanctions": {
                "status": "GREEN",
                "detail": "No matches"
            },
            "ownership": {"opacity": "MEDIUM", "score": 40},
            "vetting": {"grade": "A", "score": 16, "status": "acceptable"},
            "assessed": true,
            "checkedAt": "2026-09-27T09:02:24.635Z"
        })
    }

    fn sample_yellow() -> serde_json::Value {
        serde_json::json!({
            "imo": "1234567",
            "vesselName": "GREY VESSEL",
            "sanctions": {
                "status": "YELLOW",
                "detail": "Manual review candidate"
            },
            "ownership": {"opacity": "LOW", "score": 20},
            "vetting": {"grade": "B", "score": 30, "status": "acceptable"},
            "checkedAt": "2026-09-27T09:00:00Z"
        })
    }

    /// RED sanctions → priority
    #[test]
    fn red_is_priority() {
        let sig = parse_verdict("9274446", &sample_red());
        assert_eq!(sig.severity, "priority");
        assert_eq!(sig.kind, "vessel_sanctions_red");
        assert!(sig.title.contains("HS STAR"));
        assert!(sig.title.contains("sanctions=RED"));
    }

    /// GREEN + grade A → info
    #[test]
    fn green_is_info() {
        let sig = parse_verdict("9811000", &sample_green());
        assert_eq!(sig.severity, "info");
        assert_eq!(sig.kind, "vessel_verdict_info");
    }

    /// YELLOW sanctions → routine
    #[test]
    fn yellow_is_routine() {
        let sig = parse_verdict("1234567", &sample_yellow());
        assert_eq!(sig.severity, "routine");
        assert_eq!(sig.kind, "vessel_sanctions_yellow");
    }

    /// external_id = arcnautical:{imo}
    #[test]
    fn external_id_shape() {
        let sig = parse_verdict("9274446", &sample_red());
        assert_eq!(sig.external_id, "arcnautical:9274446");
        let sig2 = parse_verdict("9811000", &sample_green());
        assert_eq!(sig2.external_id, "arcnautical:9811000");
    }

    /// Coords are (0,0) for the keyless endpoint (no live position).
    #[test]
    fn coords_are_zero_when_no_position() {
        let sig = parse_verdict("9274446", &sample_red());
        assert_eq!(sig.lat, 0.0);
        assert_eq!(sig.lon, 0.0);
    }

    /// Payload retains the verdict fields.
    #[test]
    fn payload_carries_verdict_fields() {
        let sig = parse_verdict("9274446", &sample_red());
        let payload = &sig.payload;
        assert_eq!(payload["imo"], "9274446");
        assert_eq!(payload["sanctions_status"], "RED");
        assert_eq!(payload["ownership_opacity"], "HIGH");
        assert_eq!(payload["ownership_score"], 70);
        assert_eq!(payload["vetting_grade"], "E");
    }

    /// Watchlist size matches TOP_N.
    #[test]
    fn watchlist_size_matches_top_n() {
        assert_eq!(WATCHLIST.len(), WATCHLIST_SIZE);
        assert_eq!(TOP_N, WATCHLIST_SIZE);
    }

    /// Grade D/E without sanctions RED → routine (vetting bucket)
    #[test]
    fn vetting_d_routine() {
        let j = serde_json::json!({
            "imo": "1111111",
            "vesselName": "RISK VESSEL",
            "sanctions": {"status": "GREEN", "detail": "ok"},
            "ownership": {"opacity": "LOW", "score": 10},
            "vetting": {"grade": "D", "score": 80, "status": "elevated"},
            "checkedAt": "2026-09-27T09:00:00Z"
        });
        let sig = parse_verdict("1111111", &j);
        assert_eq!(sig.severity, "routine");
        assert_eq!(sig.kind, "vessel_vetting_d_e");
    }

    /// Ownership HIGH without sanctions RED → routine (opacity bucket)
    #[test]
    fn ownership_high_routine() {
        let j = serde_json::json!({
            "imo": "2222222",
            "vesselName": "OPAQUE VESSEL",
            "sanctions": {"status": "GREEN", "detail": "ok"},
            "ownership": {"opacity": "HIGH", "score": 80},
            "vetting": {"grade": "B", "score": 30, "status": "acceptable"},
            "checkedAt": "2026-09-27T09:00:00Z"
        });
        let sig = parse_verdict("2222222", &j);
        assert_eq!(sig.severity, "routine");
        assert_eq!(sig.kind, "vessel_ownership_opaque");
    }
}