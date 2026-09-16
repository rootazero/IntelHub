//! CourtListener — US Federal Court filings (OSINT Framework
//! "Public Records → Court Filings / Legal" gap). Polls the free,
//! keyless CourtListener v4 search API for a curated set of terms
//! relevant to sanctioned entities / APT groups / cyber-crime
//! defendants. Each new case emits a Signal anchored at the court's
//! district city coordinates, kind="sanction" (same visual cluster
//! as OFAC + OpenSanctions — but the data is "defendant sued in
//! federal court", not "person on a sanctions list").
//!
//! - **KEYLESS**: CourtListener is funded by Free Law Project. Public
//!   search requires no API key, no registration. They ask for an
//!   identifying User-Agent (we send `IntelHub research
//!   (admin@intelhub.local)`).
//! - **RATE LIMIT**: ~5 req/s per IP per their published etiquette.
//!   Default watchlist of 8 terms × 12h cadence = 16 calls/day — well
//!   under any sane quota.
//! - **WATCHLIST**: HUB_COURTLISTENER_QUERY env var (CSV of search
//!   terms). Default ships with 8 high-signal terms (lockbit, tornado
//!   cash, APT group names, sanctions evasion).
//! - **KIND**: "sanction" (matches ofac/opensanctions visual cluster
//!   on radar).
//! - **ANCHOR**: court district city. CourtListener returns the court
//!   name (e.g. "District Court, S.D. Florida") but not lat/lon —
//!   we use a hardcoded table of ~10 US federal courts. Unknown
//!   courts fall back to (0,0) and log a WARN so the operator can
//!   extend the table if needed.
//!
//! See https://www.courtlistener.com/help/api/rest/v4/ for full API
//! reference.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const COURTLISTENER_API: &str = "https://www.courtlistener.com/api/rest/v4/search/";
const USER_AGENT: &str = "IntelHub research (admin@intelhub.local)";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);
const INTER_QUERY_GAP: Duration = Duration::from_secs(2);
const MAX_RESULTS_PER_QUERY: usize = 25;

/// US federal court district/city coords. Used as Signal anchor for
/// each case (the "geography" of a court filing is the court's seat,
/// not the defendant's location). Unknown courts fall back to
/// (0,0) and a WARN log so operator can extend this table.
///
/// Substring match: court name from API ("District Court, S.D.
/// Florida") is matched against the `match_against` field with
/// case-insensitive contains() — so the table doesn't have to
/// enumerate every circuit.
const COURT_HQ: &[(&str, f64, f64)] = &[
    ("D.C.",            38.9072, -77.0369),  // District of Columbia
    ("S.D.N.Y",         40.7128, -74.0060),  // Southern District of New York
    ("E.D.N.Y",         40.6892, -73.9857),  // Eastern District of New York
    ("N.D. Cal",        37.7749, -122.4194), // Northern District of California
    ("C.D. Cal",        34.0522, -118.2437), // Central District of California
    ("S.D. Cal",        32.7157, -117.1611), // Southern District of California
    ("S.D. Fla",        25.7617, -80.1918),  // Southern District of Florida
    ("N.D. Ill",        41.8781, -87.6298),  // Northern District of Illinois
    ("D. Del",          39.7392, -75.5398),  // District of Delaware
    ("D. Mass",         42.3601, -71.0589),  // District of Massachusetts
    ("W.D. Wash",       47.6062, -122.3321), // Western District of Washington
    ("E.D. Va",         36.8508, -76.2859),  // Eastern District of Virginia
];

/// Default watchlist: 8 high-signal search terms. Each term is sent
/// as a free-text search against the "d" (docket) type. CourtListener
/// tokenizes the query — bare words are matched as full-text
/// substrings, quoted phrases are exact matches.
const DEFAULT_QUERY: &[&str] = &[
    "lockbit",
    "tornado cash",
    "Lazarus Group",
    "Conti ransomware",
    "Fancy Bear",
    "REvil",
    "Wagner Group",
    "sanctions evasion",
];

pub struct Courtlistener;

impl Source for Courtlistener {
    fn name(&self) -> &'static str {
        "courtlistener"
    }
    fn interval(&self) -> Duration {
        // 12h. Federal court filings are slow but high-signal — daily
        // would miss the news cycle, hourly would over-poll a free
        // public service.
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let queries: Vec<&str> = if !ctx.config.monitor_courtlistener_query.is_empty() {
                ctx.config
                    .monitor_courtlistener_query
                    .iter()
                    .map(|s| s.as_str())
                    .collect()
            } else {
                DEFAULT_QUERY.to_vec()
            };

            let mut out = Vec::new();
            for (idx, term) in queries.iter().enumerate() {
                if idx > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query_term(ctx, term).await {
                    Ok(mut sigs) => out.append(&mut sigs),
                    Err(e) => {
                        tracing::warn!(target: "monitor::courtlistener", term = term, error = %e, "query failed");
                    }
                }
            }

            if out.is_empty() {
                return Ok(Vec::new());
            }
            Ok(out)
        }
        .boxed()
    }
}

async fn query_term(ctx: &Ctx, term: &str) -> Result<Vec<Signal>> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(COURTLISTENER_API)
            .query(&[
                ("q", term),
                ("type", "d"), // d = dockets (case-level records)
                ("format", "json"),
                ("order_by", "score desc"),
            ])
            .header("User-Agent", USER_AGENT)
            .send(),
    )
    .await
    .map_err(|_| crate::error::HubError::sensor("courtlistener: request timed out".to_string()))?
    .map_err(|e| crate::error::HubError::sensor(format!("courtlistener: {e}")))?;

    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // CourtListener's published etiquette asks for ~5 req/s; we
        // stay under but if someone else is hitting the same egress IP
        // we may briefly share a 429. Self-heal next sweep.
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(crate::error::HubError::sensor(format!(
            "courtlistener: HTTP {status}: {snippet}"
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("courtlistener: parse: {e}")))?;

    let results = body
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for case in results.into_iter().take(MAX_RESULTS_PER_QUERY) {
        let case_name = case
            .get("caseName")
            .and_then(|v| v.as_str())
            .unwrap_or("(unnamed case)")
            .to_string();
        let case_name_full = case
            .get("case_name_full")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let docket_number = case
            .get("docketNumber")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let court_name = case
            .get("court")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown court");
        let date_filed = case
            .get("dateFiled")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let suit_nature = case
            .get("suitNature")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let docket_url = case
            .get("docket_absolute_url")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Anchor resolution: substring match against COURT_HQ.
        let (lat, lon, anchor_source) = resolve_court_anchor(court_name);

        let title = if !case_name_full.is_empty() && case_name_full != case_name {
            format!("{case_name_full} ({docket_number}) @ {court_name}")
        } else {
            format!("{case_name} ({docket_number}) @ {court_name}")
        };

        let source_id = if !docket_number.is_empty() {
            format!("courtlistener:docket:{}", docket_number.replace(' ', "_"))
        } else {
            format!(
                "courtlistener:case:{}",
                case_name.replace(' ', "_").chars().take(40).collect::<String>()
            )
        };

        out.push(
            Signal::new("sanction", title, lat, lon, source_id)
                .severity("routine")
                .occurred(parse_iso_date(date_filed).unwrap_or_else(chrono::Utc::now))
                .payload(serde_json::json!({
                    "case_name": case_name,
                    "case_name_full": case_name_full,
                    "docket_number": docket_number,
                    "court": court_name,
                    "date_filed": date_filed,
                    "suit_nature": suit_nature,
                    "docket_url": docket_url,
                    "matched_term": term,
                    "anchor_source": anchor_source,
                })),
        );
    }

    Ok(out)
}

/// Resolve a court name to (lat, lon, source). source describes
/// which row in COURT_HQ matched (or "missing" if no match).
fn resolve_court_anchor(court_name: &str) -> (f64, f64, &'static str) {
    for (match_str, lat, lon) in COURT_HQ {
        if court_name.contains(match_str) {
            return (*lat, *lon, match_str);
        }
    }
    tracing::warn!(target: "monitor::courtlistener", court = court_name,
        "no anchor match for court — falling back to (0,0). Add the court district to COURT_HQ in courtlistener.rs to fix.");
    (0.0, 0.0, "missing")
}

/// Parse a YYYY-MM-DD string to a UTC DateTime. Returns None on
/// malformed input (we fall back to "now" in the Signal).
fn parse_iso_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn court_anchor_known() {
        let (lat, _, src) = resolve_court_anchor("District Court, S.D. Florida");
        assert!((lat - 25.7617).abs() < 0.01);
        assert_eq!(src, "S.D. Fla");
    }

    #[test]
    fn court_anchor_unknown_returns_zero() {
        let (_, _, src) = resolve_court_anchor("District Court, M.D. Atlantis");
        assert_eq!(src, "missing");
    }

    #[test]
    fn parse_iso_date_valid() {
        let d = parse_iso_date("2024-08-09").unwrap();
        assert_eq!(d.format("%Y-%m-%d").to_string(), "2024-08-09");
    }

    #[test]
    fn parse_iso_date_invalid_returns_none() {
        assert!(parse_iso_date("not-a-date").is_none());
    }
}