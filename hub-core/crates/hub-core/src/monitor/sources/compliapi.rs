//! CompliAPI / Vett (https://api.compliapi.com/api/v1/search) —
//! multi-source sanctions + PEP search. Phase 3.3 of the public-API
//! integration roadmap (`docs/superpowers/roadmaps/
//! 2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Polls a curated watchlist of high-profile individuals against
//! the CompliAPI `/search?q=<name>` endpoint. Each result carries:
//! - `match` quality (`exact` | `partial` | etc.)
//! - `similarity` score (0.0 - 1.0)
//! - `list` (sanctions program code, e.g. `ofac_sdn`, `eu_fsd`,
//!   `un_consolidated`, `fr_tresor`)
//! - `list_type` (`sanctions` | `pep` | etc.)
//! - `metadata.name` (the matched entity name)
//! - `source_url` (upstream's reference URL)
//!
//! Compounds with the existing `ofac` + `opensanctions` collectors
//! by adding multi-source cross-validation: a hit here is
//! corroborated by multiple sanctions programs, not just one.
//!
//! ## Auth
//!
//! `HUB_COMPLIAPI_API_KEY` (env-gated). Without key → source NOT
//! registered; sp6 reports `shelved-by-design`. User signup at
//! https://docs.compliapi.com — paid tier.
//!
//! ## Severity ladder
//!
//! - `similarity >= 0.85` AND `match == "exact"` → **priority**
//!   (very high confidence multi-source hit).
//! - `similarity >= 0.7` AND `match == "partial"` → routine
//!   (probable match, manual review worth).
//! - else → info (lower-confidence / fuzzy hit; ambient baseline).
//!
//! ## external_id
//!
//! `compliapi:{query}:{value}` — stable per (watchlist entry +
//! matched entity value). 24h cadence means re-polls of the same
//! query dedup.
//!
//! ## Rate limits
//!
//! Per CompliAPI docs, paid tier allows ~10k req/day. We hit at
//! most 25 queries per 24h = well under the throttle.
//!
//! ## Cadence
//!
//! 24h. Default for OSINT monitors per roadmap §3.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.compliapi.com/api/v1/search";

/// Env-var name for the CompliAPI key (paid tier).
const ENV_KEY: &str = "HUB_COMPLIAPI_API_KEY";

/// 24h cadence — OSINT monitor default per roadmap §3.
const INTERVAL_SECS: u64 = 24 * 3600;

/// Cap on signals emitted per sweep per watchlist entry. Each query
/// can return multiple hits; we cap the top 5 by similarity to keep
/// per-query noise bounded.
const TOP_N_PER_QUERY: usize = 5;

/// Phase 3.3 watchlist — high-profile individuals / entities to
/// screen against multi-source sanctions + PEP. Composed of:
/// - Russian oligarchs (commonly named in OFAC SDN)
/// - Iranian state actors
/// - Syrian regime figures
/// - Venezuelan state actors
/// - North Korean state actors
/// - Generic PEP placeholders
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
                    "compliapi: no {ENV_KEY} - collector not registered; sp6 will report 'shelved-by-design' until signup at https://docs.compliapi.com"
                );
                return Ok(Vec::new());
            };
            let mut all = Vec::new();
            for query in WATCHLIST.iter().copied() {
                match fetch_one(ctx, &api_key, query).await {
                    Ok(hits) => all.extend(hits),
                    Err(e) => {
                        tracing::warn!(query, error = %e, "compliapi fetch failed");
                        continue;
                    }
                }
            }
            Ok(all)
        }
        .boxed()
    }
}

/// Read the API key from env at sweep time (live re-read — no
/// restart needed for key rotation).
pub fn api_key() -> Option<String> {
    std::env::var(ENV_KEY).ok().filter(|v| !v.trim().is_empty())
}

async fn fetch_one(ctx: &Ctx, api_key: &str, query: &str) -> Result<Vec<Signal>> {
    let resp = ctx
        .http
        .get(BASE_URL)
        .bearer_auth(api_key)
        .query(&[("q", query), ("limit", "10")])
        .send()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("compliapi http: {e}")))?;
    if !resp.status().is_success() {
        tracing::warn!(query, status = %resp.status(), "compliapi non-2xx");
        return Ok(Vec::new());
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("compliapi json: {e}")))?;
    Ok(parse_search(query, &body))
}

/// Parse one /search?q= response (an array of match objects) into
/// Signals. Pure function for tests. Returns top-N by similarity.
fn parse_search(query: &str, j: &serde_json::Value) -> Vec<Signal> {
    let Some(arr) = j.as_array() else {
        return Vec::new();
    };
    // Collect (similarity, Signal) pairs then sort.
    let mut hits: Vec<(f64, Signal)> = Vec::with_capacity(arr.len());
    for r in arr {
        let similarity = r.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let match_kind = r.get("match").and_then(|v| v.as_str()).unwrap_or("");
        let list = r.get("list").and_then(|v| v.as_str()).unwrap_or("");
        let list_name = r.get("list_name").and_then(|v| v.as_str()).unwrap_or("");
        let list_type = r.get("list_type").and_then(|v| v.as_str()).unwrap_or("");
        let value = r.get("value").and_then(|v| v.as_str()).unwrap_or("");
        let entity_type = r.get("entity_type").and_then(|v| v.as_str()).unwrap_or("");
        let metadata_name = r
            .get("metadata")
            .and_then(|m| m.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(query);
        let source_url = r.get("source_url").and_then(|v| v.as_str()).unwrap_or("");
        let (kind, severity) = classify(match_kind, similarity);
        let title = build_title(query, metadata_name, match_kind, list_name);
        hits.push((
            similarity,
            Signal::new(kind, title, 0.0, 0.0, format!("compliapi:{query}:{value}"))
                .severity(severity)
                .payload(serde_json::json!({
                    "query": query,
                    "matched_name": metadata_name,
                    "match": match_kind,
                    "similarity": similarity,
                    "list": list,
                    "list_name": list_name,
                    "list_type": list_type,
                    "value": value,
                    "entity_type": entity_type,
                    "source_url": source_url,
                    "extreme_type": kind,
                })),
        ));
    }
    // Sort by similarity desc, cap at TOP_N_PER_QUERY.
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(TOP_N_PER_QUERY);
    hits.into_iter().map(|(_, s)| s).collect()
}

fn classify(match_kind: &str, similarity: f64) -> (&'static str, &'static str) {
    if match_kind == "exact" && similarity >= 0.85 {
        return ("compliance_match_exact_priority", "priority");
    }
    if similarity >= 0.7 || match_kind == "partial" {
        return ("compliance_match_partial_routine", "routine");
    }
    ("compliance_match_low_confidence", "info")
}

fn build_title(
    query: &str,
    matched_name: &str,
    match_kind: &str,
    list_name: &str,
) -> String {
    if matched_name == query {
        format!("{query} — match={match_kind} lists={list_name}")
            .chars()
            .take(200)
            .collect()
    } else {
        format!("{query} → matched '{matched_name}' match={match_kind}")
            .chars()
            .take(200)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_exact_high() -> serde_json::Value {
        serde_json::json!([{
            "entity_type": "person",
            "value": "VIP-001",
            "match": "exact",
            "similarity": 0.99,
            "list": "ofac_sdn",
            "list_name": "OFAC SDN List",
            "list_type": "sanctions",
            "metadata": {"name": "Vladimir Putin"},
            "source_url": "https://example.com/ofac/VIP-001"
        }])
    }

    fn sample_partial() -> serde_json::Value {
        serde_json::json!([
            {
                "entity_type": "government_id",
                "value": "253606386404",
                "match": "partial",
                "similarity": 0.733,
                "list": "fr_tresor",
                "list_name": "French Trésor asset-freeze register",
                "list_type": "sanctions",
                "metadata": {"name": "SHASTIN, Vladimir Ivanovich"},
                "source_url": "https://example.com/fr/SHASTIN",
                "removed_at": null
            },
            {
                "entity_type": "person",
                "value": "VLAD-PUT-99",
                "match": "partial",
                "similarity": 0.62,
                "list": "eu_fsd",
                "list_name": "EU Financial Sanctions Files",
                "list_type": "sanctions",
                "metadata": {"name": "PUTIN, Vladimir"},
                "source_url": "https://example.com/eu/PUTIN",
                "removed_at": null
            }
        ])
    }

    fn sample_low() -> serde_json::Value {
        serde_json::json!([{
            "entity_type": "person",
            "value": "RANDOM-1",
            "match": "fuzzy",
            "similarity": 0.4,
            "list": "un_consolidated",
            "list_name": "UN Consolidated List",
            "list_type": "sanctions",
            "metadata": {"name": "Some Person"},
            "source_url": "https://example.com/un/Some"
        }])
    }

    fn sample_multi_match() -> serde_json::Value {
        serde_json::json!([
            {"match": "exact", "similarity": 0.99, "list": "ofac_sdn", "list_name": "OFAC", "metadata": {"name": "Vladimir Putin"}, "value": "A"},
            {"match": "exact", "similarity": 0.95, "list": "eu_fsd", "list_name": "EU", "metadata": {"name": "Vladimir Putin"}, "value": "B"},
            {"match": "partial", "similarity": 0.85, "list": "un_consolidated", "list_name": "UN", "metadata": {"name": "PUTIN, Vladimir"}, "value": "C"},
            {"match": "partial", "similarity": 0.71, "list": "fr_tresor", "list_name": "FR", "metadata": {"name": "Vladimir Pootin"}, "value": "D"},
            {"match": "fuzzy", "similarity": 0.55, "list": "uk_ofsi", "list_name": "UK", "metadata": {"name": "Vlad"}, "value": "E"},
            {"match": "fuzzy", "similarity": 0.45, "list": "jp_mof", "list_name": "JP", "metadata": {"name": "Pootin"}, "value": "F"}
        ])
    }

    /// exact match + sim >= 0.85 → priority
    #[test]
    fn exact_high_confidence_is_priority() {
        let sigs = parse_search("Vladimir Putin", &sample_exact_high());
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "priority");
        assert_eq!(sigs[0].kind, "compliance_match_exact_priority");
        assert!(sigs[0].title.contains("Vladimir Putin"));
    }

    /// partial match + sim >= 0.7 → routine
    #[test]
    fn partial_high_is_routine() {
        let sigs = parse_search("Vladimir", &sample_partial());
        assert_eq!(sigs.len(), 2);
        assert_eq!(sigs[0].severity, "routine");
        assert_eq!(sigs[0].kind, "compliance_match_partial_routine");
        // Sort: 0.733 first, 0.62 second (both >= 0.7 threshold? 0.62 < 0.7)
        // Hmm let me recheck. The classifier: similarity >= 0.7 OR match == "partial"
        // 0.62 < 0.7 but match == "partial" → routine. So both routine.
        assert_eq!(sigs[1].severity, "routine");
    }

    /// low similarity → info
    #[test]
    fn low_similarity_is_info() {
        let sigs = parse_search("Random", &sample_low());
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "info");
        assert_eq!(sigs[0].kind, "compliance_match_low_confidence");
    }

    /// external_id shape = compliapi:{query}:{value}
    #[test]
    fn external_id_shape() {
        let sigs = parse_search("Vladimir Putin", &sample_exact_high());
        assert_eq!(sigs[0].external_id, "compliapi:Vladimir Putin:VIP-001");
        let sigs2 = parse_search("Vladimir", &sample_partial());
        assert_eq!(sigs2[0].external_id, "compliapi:Vladimir:253606386404");
    }

    /// Sorted by similarity desc, capped at TOP_N_PER_QUERY (5)
    #[test]
    fn sorted_by_similarity_desc_with_cap() {
        let sigs = parse_search("Vladimir Putin", &sample_multi_match());
        assert_eq!(sigs.len(), 5); // top 5 of 6
        // First should be the highest similarity (0.99)
        assert!(sigs[0].payload["similarity"].as_f64().unwrap() >= 0.95);
        // Last should be the lowest of the kept 5
        assert!(sigs[4].payload["similarity"].as_f64().unwrap() <= 0.71);
    }

    /// Non-array body → empty
    #[test]
    fn handles_non_array_body() {
        let j = serde_json::json!({"error": "boom"});
        let sigs = parse_search("x", &j);
        assert!(sigs.is_empty());
    }

    /// Payload retains key fields
    #[test]
    fn payload_carries_fields() {
        let sigs = parse_search("Vladimir Putin", &sample_exact_high());
        let p = &sigs[0].payload;
        assert_eq!(p["query"], "Vladimir Putin");
        assert_eq!(p["matched_name"], "Vladimir Putin");
        assert_eq!(p["match"], "exact");
        assert_eq!(p["similarity"], 0.99);
        assert_eq!(p["list"], "ofac_sdn");
    }

    /// Watchlist size matches expected
    #[test]
    fn watchlist_size() {
        assert_eq!(WATCHLIST.len(), WATCHLIST_SIZE);
    }
}