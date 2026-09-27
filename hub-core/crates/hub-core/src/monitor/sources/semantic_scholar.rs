//! Semantic Scholar Graph API (https://api.semanticscholar.org —
//! requires free API key for sustained use, 100 req/sec with key).
//! Phase 2.3 of the public-API integration roadmap (`docs/superpowers/
//! roadmaps/2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Daily sweep: query `/graph/v1/paper/search` with OSINT-relevant
//! keywords (default: "geopolitics"), top 10 papers by relevance.
//! Each paper emits a Signal with title, authors, year, citation
//! count, and URL. Severity by citation count (high-citation =
//! priority, mid-citation = routine, low-citation = info).
//!
//! ## Why Semantic Scholar
//!
//! - Bias-balanced news (Helium, Currents) — same-day, high-volume
//! - Academic literature — multi-month lag, lower volume, but
//!   authoritative for state-of-the-art on AI safety, pandemic, etc.
//!
//! ## Future: enrich.rs integration (Phase 2.3.x)
//!
//! Roadmap §3.1 calls for wiring Semantic Scholar as a citation
//! **enrichment** stage on existing claims/findings, not as a monitor:
//!
//!     enriched.evidence[].source_kind = "semantic_scholar"
//!
//! New `enrich.rs` extension points: `enrich_with_semantic_scholar(claim)`,
//! `enrich_with_gitguardian(url)`. Both opt-in via env flag, not
//! always-on, to keep cost predictable.
//!
//! Phase 2.3 v1 emits as a normal monitor (gets signals flowing). Phase
//! 2.3.x will move the wiring into the enrich stage — out of scope here.
//!
//! ## Severity ladder
//!
//! By citation count (a paper's reach in academic discourse):
//!   - >= 50 citations → priority (well-established work)
//!   - >= 10 citations → routine (notable but newer)
//!   - < 10 citations   → info (recent work, low reach yet)
//!
//! ## external_id
//!
//! `semantic_scholar:{paperId}` — paperId is the canonical S2
//! identifier (stable across re-polls; geo_events dedups).
//!
//! ## Auth
//!
//! `HUB_SEMANTIC_SCHOLAR_API_KEY` (env-gated, mirrors open_aq/
//! currents). Without key → source NOT registered; sp6 reports
//! `shelved-by-design`. User signup at
//! https://www.semanticscholar.org/product/api#api-key-form.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.semanticscholar.org/graph/v1/paper/search";

/// Default search query. Single query per day (free tier budget).
/// Easy to extend to multi-query in Phase 2.3.x.
const DEFAULT_QUERY: &str = "geopolitics";

/// Cap on signals emitted per day. Academic literature moves slower
/// than news — 10/day matches Helium/Currents parity.
const TOP_N: usize = 10;

/// Citation count thresholds for severity ladder.
const CITATION_PRIORITY: usize = 50;
const CITATION_ROUTINE: usize = 10;

fn api_key() -> Option<String> {
    std::env::var("HUB_SEMANTIC_SCHOLAR_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
}

pub struct SemanticScholar;

impl Source for SemanticScholar {
    fn name(&self) -> &'static str {
        "semantic_scholar"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600) // 24h — roadmap §3
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(_key) = api_key() else {
                tracing::warn!(
                    "semantic_scholar: no HUB_SEMANTIC_SCHOLAR_API_KEY — collector not \
                     registered; sp6 will report 'shelved-by-design' until signup at \
                     https://www.semanticscholar.org/product/api#api-key-form"
                );
                return Ok(Vec::new());
            };
            // Use x-api-key header (matches Semantic Scholar docs).
            let mut req = ctx
                .http
                .get(BASE_URL)
                .query(&[
                    ("query", DEFAULT_QUERY),
                    ("limit", TOP_N.to_string().as_str()),
                    (
                        "fields",
                        "paperId,title,year,authors,abstract,citationCount,url,venue",
                    ),
                ]);
            req = req.header("x-api-key", &_key);
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "semantic_scholar fetch failed");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "semantic_scholar non-2xx");
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "semantic_scholar parse failed");
                    return Ok(Vec::new());
                }
            };
            Ok(parse_response(&body))
        }
        .boxed()
    }
}

/// Pure parser. Walks the `data[]` array, classifies by citation count.
fn parse_response(j: &serde_json::Value) -> Vec<Signal> {
    let mut out = Vec::new();
    let Some(data) = j.get("data").and_then(|v| v.as_array()) else {
        return out;
    };
    for (i, r) in data.iter().enumerate() {
        if i >= TOP_N {
            break;
        }
        let paper_id = r.get("paperId").and_then(|v| v.as_str()).unwrap_or("");
        if paper_id.is_empty() {
            continue; // skip papers with no id
        }
        let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let year = r.get("year").and_then(|v| v.as_i64()).unwrap_or(0);
        let abstract_text = r
            .get("abstract")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let citations = r
            .get("citationCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let venue = r.get("venue").and_then(|v| v.as_str()).unwrap_or("");
        let authors: Vec<String> = r
            .get("authors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        a.get("name")
                            .and_then(|n| n.as_str())
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (severity, kind) = if citations >= CITATION_PRIORITY {
            ("priority", "paper_priority")
        } else if citations >= CITATION_ROUTINE {
            ("routine", "paper_routine")
        } else {
            ("info", "paper_info")
        };
        out.push(
            Signal::new(kind, title.to_string(), 0.0, 0.0, format!("semantic_scholar:{paper_id}"))
                .severity(severity)
                .payload(serde_json::json!({
                    "paper_id": paper_id,
                    "title": title,
                    "year": year,
                    "authors": authors,
                    "abstract": abstract_text,
                    "citation_count": citations,
                    "url": url,
                    "venue": venue,
                    "extreme_type": kind,
                })),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_response() -> serde_json::Value {
        serde_json::json!({
            "total": 3,
            "offset": 0,
            "data": [
                {
                    "paperId": "abc123",
                    "title": "High-impact paper on geopolitics",
                    "year": 2023,
                    "authors": [{"authorId": "1", "name": "Alice"}, {"authorId": "2", "name": "Bob"}],
                    "abstract": "We study...",
                    "citationCount": 100,
                    "url": "https://semanticscholar.org/paper/abc123",
                    "venue": "Nature"
                },
                {
                    "paperId": "def456",
                    "title": "Mid-impact paper",
                    "year": 2024,
                    "authors": [{"authorId": "3", "name": "Charlie"}],
                    "abstract": "We analyze...",
                    "citationCount": 25,
                    "url": "https://semanticscholar.org/paper/def456",
                    "venue": "Science"
                },
                {
                    "paperId": "ghi789",
                    "title": "New low-citation paper",
                    "year": 2026,
                    "authors": [],
                    "abstract": "We propose...",
                    "citationCount": 2,
                    "url": "",
                    "venue": ""
                }
            ]
        })
    }

    /// Happy path: 3 results, severity by citation count.
    #[test]
    fn detects_severity_by_citations() {
        let sigs = parse_response(&sample_response());
        assert_eq!(sigs.len(), 3);
        // 100 citations → priority
        assert_eq!(sigs[0].severity, "priority");
        assert_eq!(sigs[0].kind, "paper_priority");
        // 25 citations → routine
        assert_eq!(sigs[1].severity, "routine");
        assert_eq!(sigs[1].kind, "paper_routine");
        // 2 citations → info
        assert_eq!(sigs[2].severity, "info");
        assert_eq!(sigs[2].kind, "paper_info");
    }

    /// external_id format: `semantic_scholar:{paperId}`.
    #[test]
    fn external_id_shape() {
        let sigs = parse_response(&sample_response());
        assert_eq!(sigs[0].external_id, "semantic_scholar:abc123");
        assert_eq!(sigs[1].external_id, "semantic_scholar:def456");
        assert_eq!(sigs[2].external_id, "semantic_scholar:ghi789");
    }

    /// Defensive: missing data array → empty.
    #[test]
    fn missing_data_returns_empty() {
        assert!(parse_response(&serde_json::json!({})).is_empty());
    }

    /// Defensive: empty data → empty.
    #[test]
    fn empty_data_returns_empty() {
        let sigs = parse_response(&serde_json::json!({"data": []}));
        assert!(sigs.is_empty());
    }

    /// Defensive: papers with empty paperId are skipped.
    #[test]
    fn skips_papers_with_empty_id() {
        let j = serde_json::json!({
            "data": [
                {"paperId": "", "title": "Should be skipped", "citationCount": 100},
                {"paperId": "real-id", "title": "Should be kept", "citationCount": 5}
            ]
        });
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].external_id, "semantic_scholar:real-id");
    }

    /// Threshold inclusivity: exactly at threshold.
    #[test]
    fn threshold_inclusive() {
        let mut j = serde_json::json!({"data": []});
        for (id, cites) in [("a", 50), ("b", 10), ("c", 0)] {
            j["data"].as_array_mut().unwrap().push(serde_json::json!({
                "paperId": id, "title": id, "citationCount": cites
            }));
        }
        let sigs = parse_response(&j);
        // 50 → priority, 10 → routine, 0 → info
        assert_eq!(sigs[0].kind, "paper_priority");
        assert_eq!(sigs[1].kind, "paper_routine");
        assert_eq!(sigs[2].kind, "paper_info");
    }

    /// Defensive: missing fields default safe (citationCount=0 → info).
    #[test]
    fn missing_fields_default_safe() {
        let j = serde_json::json!({
            "data": [{
                "paperId": "p1",
                "title": "Minimal paper"
            }]
        });
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].severity, "info");
    }

    /// Top-N cap.
    #[test]
    fn caps_at_top_n() {
        let mut j = serde_json::json!({"data": []});
        for i in 0..(TOP_N + 5) {
            j["data"].as_array_mut().unwrap().push(serde_json::json!({
                "paperId": format!("p{i}"), "title": format!("Title {i}"),
                "citationCount": 100
            }));
        }
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), TOP_N);
    }

    /// Authors array preserved.
    #[test]
    fn authors_array_preserved() {
        let sigs = parse_response(&sample_response());
        assert_eq!(sigs[0].payload["authors"].as_array().unwrap().len(), 2);
        assert_eq!(sigs[0].payload["authors"][0], "Alice");
        assert_eq!(sigs[0].payload["authors"][1], "Bob");
    }
}