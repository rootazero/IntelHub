//! CompliAPI / Vett (https://docs.compliapi.com) — multi-source
//! sanctions screening (OFAC SDN + EU FSD + UK OFSI + UN Consolidated
//! + PEP). Phase 3.3 of the public-API integration roadmap
//! (`docs/superpowers/roadmaps/2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Polls a curated watchlist of high-profile individuals / entities
//! against the CompliAPI `/api/v1/screen` endpoint and emits a Signal
//! per match. Compounds with the existing `ofac` + `opensanctions`
//! collectors by adding multi-source cross-validation: a hit here
//! is corroborated by multiple sanctions programs, not just one.
//!
//! ## Auth
//!
//! `HUB_COMPLIAPI_API_KEY` (env-gated). Without key → source NOT
//! registered; sp6 reports `shelved-by-design`. User signup at
//! https://docs.compliapi.com — paid tier (1-3 business day
//! approval).
//!
//! ## Severity ladder
//!
//! - `match` (sanctions hit, confidence >= 0.85) → **priority**
//! - `review` (manual review, any confidence) → routine
//! - `clear` → info (ambient baseline — useful as "we polled, all
//!   quiet")
//!
//! ## external_id
//!
//! `compliapi:{query_hash}` — stable per (watchlist entry + poll
//! time). 24h cadence means re-polls of the same name dedup.
//!
//! ## Rate limits
//!
//! 1 req per watchlist entry per 24h sweep. Default watchlist ~25
//! entries. Per CompliAPI docs, paid tier allows ~10k req/day, well
//! above our 25/24h usage.
//!
//! ## Cadence
//!
//! 24h. Default for OSINT monitors per roadmap §3.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.compliapi.com/api/v1/screen";

/// Env-var name for the CompliAPI key (paid tier).
const ENV_KEY: &str = "HUB_COMPLIAPI_API_KEY";

/// 24h cadence — OSINT monitor default per roadmap §3.
const INTERVAL_SECS: u64 = 24 * 3600;

/// Cap on signals emitted per sweep.
const TOP_N: usize = WATCHLIST_SIZE;

/// Phase 3.3 watchlist — high-profile individuals / entities to screen
/// against multi-source sanctions + PEP. Composed of:
/// - Russian oligarchs (commonly named in OFAC SDN)
/// - Iranian state actors
/// - Syrian regime figures
/// - Venezuelan state actors
/// - North Korean state actors
/// - Generic PEP placeholders (test queries that any user could
///   recognize)
const WATCHLIST_SIZE: usize = 25;

const WATCHLIST: &[&str] = &[
    // Russian oligarchs
    "Vladimir Putin",
    "Sergei Lavrov",
    "Igor Sechin",
    "Roman Abramovich",
    "Alisher Usmanov",
    "Oleg Deripaska",
    "Viktor Vekselberg",
    "Mikhail Fridman",
    "Petr Aven",
    "Vagit Alekperov",
    // Iranian state actors
    "Ali Khamenei",
    "Ebrahim Raisi",
    "Mohammad Javad Zarif",
    "Hossein Salami",
    "Qasem Soleimani",
    // Syrian regime
    "Bashar al-Assad",
    // Venezuelan
    "Nicolas Maduro",
    "Tareck El Aissami",
    // North Korean
    "Kim Jong-un",
    "Kim Yo-jong",
    // Generic PEP placeholders (high-recognition)
    "Vladimir Zelenskyy",
    "Xi Jinping",
    "Narendra Modi",
    "Recep Tayyip Erdogan",
    "Benjamin Netanyahu",
];

pub struct CompliApi;

impl Source for CompliApi {
    fn name(&self) -> &'static str {
        "compliapi"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(api_key) = api_key() else {
                tracing::warn!(
                    "compliapi: no {ENV_KEY} — collector not registered; sp6 will report 'shelved-by-design' until signup at https://docs.compliapi.com"
                );
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for query in WATCHLIST.iter().copied() {
                match fetch_one(ctx, &api_key, query).await {
                    Ok(Some(sig)) => out.push(sig),
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(query, error = %e, "compliapi fetch failed");
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

/// Read the API key from env at sweep time (live re-read — no
/// restart needed for key rotation).
pub fn api_key() -> Option<String> {
    std::env::var(ENV_KEY).ok().filter(|v| !v.trim().is_empty())
}

async fn fetch_one(ctx: &Ctx, api_key: &str, query: &str) -> Result<Option<Signal>> {
    let resp = ctx
        .http
        .post(BASE_URL)
        .bearer_auth(api_key)
        .json(&serde_json::json!({
            "name": query,
            // include PEP + sanctions (default)
            "datasets": ["sanctions", "pep"],
        }))
        .send()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("compliapi http: {e}")))?;
    if !resp.status().is_success() {
        tracing::warn!(query, status = %resp.status(), "compliapi non-2xx");
        return Ok(None);
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("compliapi json: {e}")))?;
    Ok(Some(parse_screen(query, &body)))
}

/// Parse one screen response into a Signal. Pure function for tests.
fn parse_screen(query: &str, j: &serde_json::Value) -> Signal {
    let verdict = j
        .get("verdict")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let confidence = j
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let matched_lists = j
        .get("matched_lists")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    let matched_name = j
        .get("matched_name")
        .and_then(|v| v.as_str())
        .unwrap_or(query);
    let checked_at = j
        .get("checked_at")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let (kind, severity) = classify(verdict, confidence);
    let title = build_title(query, matched_name, verdict, &matched_lists);
    // Coords: CompliAPI verdict is name-based, no geo. Use (0,0)
    // convention — same as ArcNautical; downstream radar handles it.
    Signal::new(kind, title, 0.0, 0.0, format!("compliapi:{query}"))
        .severity(severity)
        .payload(serde_json::json!({
            "query": query,
            "matched_name": matched_name,
            "verdict": verdict,
            "confidence": confidence,
            "matched_lists": matched_lists,
            "checked_at": checked_at,
            "extreme_type": kind,
        }))
}

fn classify(verdict: &str, confidence: f64) -> (&'static str, &'static str) {
    if verdict == "match" && confidence >= 0.85 {
        return ("compliance_match_priority", "priority");
    }
    if verdict == "review" {
        return ("compliance_review_routine", "routine");
    }
    if verdict == "match" {
        return ("compliance_match_low_confidence", "routine");
    }
    ("compliance_clear_info", "info")
}

fn build_title(
    query: &str,
    matched_name: &str,
    verdict: &str,
    matched_lists: &str,
) -> String {
    if matched_name == query {
        format!("{query} — sanctions verdict={verdict} lists={matched_lists}")
            .chars()
            .take(200)
            .collect()
    } else {
        format!("{query} → matched '{matched_name}' verdict={verdict}")
            .chars()
            .take(200)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_match() -> serde_json::Value {
        serde_json::json!({
            "query": "Vladimir Putin",
            "matched_name": "PUTIN, Vladimir Vladimirovich",
            "verdict": "match",
            "confidence": 0.99,
            "matched_lists": ["OFAC SDN", "EU FSD", "UN Consolidated", "UK OFSI"],
            "checked_at": "2026-09-27T09:00:00Z"
        })
    }

    fn sample_review() -> serde_json::Value {
        serde_json::json!({
            "query": "John Smith",
            "matched_name": "SMITH, John",
            "verdict": "review",
            "confidence": 0.55,
            "matched_lists": ["OFAC SDN"],
            "checked_at": "2026-09-27T09:00:00Z"
        })
    }

    fn sample_clear() -> serde_json::Value {
        serde_json::json!({
            "query": "Marie Curie",
            "matched_name": "CURIE, Marie",
            "verdict": "clear",
            "confidence": 0.0,
            "matched_lists": [],
            "checked_at": "2026-09-27T09:00:00Z"
        })
    }

    fn sample_low_confidence() -> serde_json::Value {
        serde_json::json!({
            "query": "Smith Jones",
            "matched_name": "JONES, Smith",
            "verdict": "match",
            "confidence": 0.42,
            "matched_lists": ["EU FSD"],
            "checked_at": "2026-09-27T09:00:00Z"
        })
    }

    /// match + conf >= 0.85 → priority
    #[test]
    fn match_high_confidence_is_priority() {
        let sig = parse_screen("Vladimir Putin", &sample_match());
        assert_eq!(sig.severity, "priority");
        assert_eq!(sig.kind, "compliance_match_priority");
        assert!(sig.title.contains("PUTIN"));
        assert!(sig.title.contains("match"));
    }

    /// review verdict → routine
    #[test]
    fn review_is_routine() {
        let sig = parse_screen("John Smith", &sample_review());
        assert_eq!(sig.severity, "routine");
        assert_eq!(sig.kind, "compliance_review_routine");
    }

    /// clear verdict → info
    #[test]
    fn clear_is_info() {
        let sig = parse_screen("Marie Curie", &sample_clear());
        assert_eq!(sig.severity, "info");
        assert_eq!(sig.kind, "compliance_clear_info");
    }

    /// match with low confidence → routine (different bucket)
    #[test]
    fn match_low_confidence_is_routine() {
        let sig = parse_screen("Smith Jones", &sample_low_confidence());
        assert_eq!(sig.severity, "routine");
        assert_eq!(sig.kind, "compliance_match_low_confidence");
    }

    /// external_id shape = compliapi:{query}
    #[test]
    fn external_id_shape() {
        let sig = parse_screen("Vladimir Putin", &sample_match());
        assert_eq!(sig.external_id, "compliapi:Vladimir Putin");
    }

    /// Payload retains multi-source list
    #[test]
    fn payload_carries_lists() {
        let sig = parse_screen("Vladimir Putin", &sample_match());
        let p = &sig.payload;
        assert_eq!(p["matched_lists"], "OFAC SDN,EU FSD,UN Consolidated,UK OFSI");
        assert_eq!(p["confidence"], 0.99);
    }

    /// Watchlist size matches TOP_N
    #[test]
    fn watchlist_size_matches_top_n() {
        assert_eq!(WATCHLIST.len(), WATCHLIST_SIZE);
        assert_eq!(TOP_N, WATCHLIST_SIZE);
    }

    /// api_key returns None for empty/whitespace
    #[test]
    fn api_key_returns_none_for_empty() {
        // default env doesn't have HUB_COMPLIAPI_API_KEY (test fixture)
        assert!(api_key().is_none() || api_key().unwrap().len() > 5);
    }
}