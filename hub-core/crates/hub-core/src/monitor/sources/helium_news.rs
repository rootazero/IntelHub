//! Helium News MCP (https://heliumtrades.com/mcp-page/ — keyless, no
//! signup, 50 free queries/window per docs). Phase 1.8 of the public-API
//! integration roadmap (`docs/superpowers/roadmaps/2026-09-27-public-api-
//! integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Daily sweep: hit `mcp_balanced_search` with a curated OSINT query
//! (default: "geopolitics"), parse the top results. Each result is a
//! bias-balanced news synthesis with full citation evidence — much
//! richer than a single-headline feed. Emits one Signal per result
//! with severity by rank (rank 1 = priority, 2-5 = routine, 6+ = info).
//!
//! Helium's editorial flow:
//! - User asks a question ("what's the geopolitical situation in X?")
//! - Helium searches 5000+ sources
//! - Returns synthesized answer with bias scores for each citation
//!   (left/center/right lean), supporting evidence, and potential
//!   outcomes
//!
//! For IntelHub: we treat each Helium result as a SIGNAL — a curated
//! "this is the news landscape today" snapshot with bias-aware context.
//! Downstream correlation can promote severity when bias-divergence is
//! high (signals the situation is contested/breaking).
//!
//! ## Query curation
//!
//! Single query "geopolitics" — broad OSINT coverage that surfaces
//! conflict / sanctions / diplomacy / elections in one sweep. A future
//! extension can run multiple queries (e.g. "ai regulation", "energy
//! markets") but each costs a free-tier query; staying at 1 query/day
//! keeps us well within budget (50/window = 50/week, and the budget
//! refills weekly per docs).
//!
//! ## Severity ladder
//!
//! Helium returns `rank` per result (integer, lower = higher editorial
//! importance). Mapping:
//!   rank = 1 → priority (headline synthesis of the day)
//!   rank 2-5 → routine (significant but secondary)
//!   rank 6+ → info (peripheral coverage)
//!
//! Note: the mapping is intentionally coarse — we don't want to spam
//! geo_events with priority on every Helium result.
//!
//! ## external_id
//!
//! We don't have an upstream ID, so we hash the `page_url` (stable,
//! URL-encoded page slug). `helium:{sha256_first16_hex(page_url)}` —
//! idempotent across re-polls. Same synthesis = same URL = same id,
//! geo_events dedups.
//!
//! ## Auth
//!
//! Keyless. Helium's docs are clear: "no SDK, no API key, no signup".
//! 50 free queries/window.

use futures::future::BoxFuture;
use futures::FutureExt;
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://heliumtrades.com/mcp_balanced_search/";

/// Curated query. Single query per day — Helium's "balanced search"
/// already cross-references 5000+ sources per result, so we don't
/// need multiple queries for breadth. Easy to tune via env if needed
/// later (Phase 1.8.x).
const DEFAULT_QUERY: &str = "geopolitics";

/// Cap on signals emitted per day. Helium returns ~10 results per
/// query; cap at 10 to match. (Easy to relax if Helium's response
/// shape changes.)
const TOP_N: usize = 10;

pub struct HeliumNews;

impl Source for HeliumNews {
    fn name(&self) -> &'static str {
        "helium_news"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600) // 24h — roadmap §2.1
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            // query param via URL encoding is fine for ASCII, but
            // for safety we use reqwest's .query() which percent-
            // encodes the value.
            let resp = match ctx
                .http
                .get(BASE_URL)
                .query(&[("q", DEFAULT_QUERY)])
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "helium_news fetch failed");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "helium_news non-2xx");
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "helium_news parse failed");
                    return Ok(Vec::new());
                }
            };
            Ok(parse_response(&body))
        }
        .boxed()
    }
}

/// Pure parser: walk the `results[]` array, map each to a Signal
/// with severity by `rank`. Defensive against missing fields.
fn parse_response(j: &serde_json::Value) -> Vec<Signal> {
    let mut out = Vec::new();
    let Some(results) = j.get("results").and_then(|v| v.as_array()) else {
        return out;
    };
    for (i, r) in results.iter().enumerate() {
        if i >= TOP_N {
            break;
        }
        let title = r
            .get("title")
            .or_else(|| r.get("simple_title"))
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled");
        let date_str = r.get("date").and_then(|v| v.as_str()).unwrap_or("");
        let category = r.get("category").and_then(|v| v.as_str()).unwrap_or("");
        let page_url = r
            .get("page_url")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let summary = r.get("summary").and_then(|v| v.as_str()).unwrap_or("");
        let takeaway = r.get("takeaway").and_then(|v| v.as_str()).unwrap_or("");
        let context = r.get("context").and_then(|v| v.as_str()).unwrap_or("");
        let num_sources = r
            .get("num_sources")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let rank = r.get("rank").and_then(|v| v.as_u64()).unwrap_or((i + 1) as u64);
        // Severity by rank: 1 = priority (today's headline), 2-5 =
        // routine (significant secondary), 6+ = info (peripheral).
        let (severity, kind) = match rank {
            1 => ("priority", "news_headline"),
            2..=5 => ("routine", "news_significant"),
            _ => ("info", "news_peripheral"),
        };
        // external_id = sha256(page_url) truncated to 16 hex chars.
        // page_url is the most stable identifier per Helium synthesis.
        let ext_id = if page_url.is_empty() {
            format!("helium:{}", &short_hash(title))
        } else {
            format!("helium:{}", short_hash(page_url))
        };
        out.push(
            Signal::new(kind, title.to_string(), 0.0, 0.0, ext_id)
                .severity(severity)
                .payload(serde_json::json!({
                    "title": title,
                    "simple_title": r.get("simple_title").and_then(|v| v.as_str()),
                    "date": date_str,
                    "category": category,
                    "page_url": page_url,
                    "summary": summary,
                    "takeaway": takeaway,
                    "context": context,
                    "num_sources": num_sources,
                    "rank": rank,
                    "extreme_type": kind,
                })),
        );
    }
    out
}

/// Truncated sha256 hex (first 16 chars = 64 bits of entropy, plenty
/// for an external_id dedup key).
fn short_hash(s: &str) -> String {
    let h = Sha256::digest(s.as_bytes());
    let hex = format!("{:x}", h);
    hex.chars().take(16).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_response() -> serde_json::Value {
        serde_json::json!({
            "results": [
                {
                    "title": "AI and geopolitics amplify cybersecurity risk",
                    "simple_title": "AI and geopolitics amplify cybersecurity risk",
                    "date": "2026-03-12 07:04:11+00:00",
                    "category": "technology",
                    "page_url": "https://heliumtrades.com/balanced-news/AI-and-cybersecurity/",
                    "image": "https://example.com/img.jpg",
                    "summary": "AI and geopolitics raise systemic cybersecurity risks.",
                    "takeaway": "Agentic AI widening attack surfaces.",
                    "context": "Digitalization, generative AI adoption, and geopolitical tension.",
                    "evidence": ["[3] China's CERT warned...", "[5] OpenClaw analysis..."],
                    "potential_outcomes": ["..."],
                    "relevant_tickers": ["NVDA", "MSFT"],
                    "num_sources": 24,
                    "rank": 1
                },
                {
                    "title": "Regional tensions reshape trade flows",
                    "simple_title": "Regional tensions reshape trade flows",
                    "date": "2026-03-11 18:00:00+00:00",
                    "category": "geopolitics",
                    "page_url": "https://heliumtrades.com/balanced-news/regional-trade/",
                    "summary": "Trade flows are reshuffling.",
                    "takeaway": "Tariff regimes are diversifying supply chains.",
                    "context": "Tariff regimes reshape trade.",
                    "evidence": ["[1] Bloomberg analysis"],
                    "num_sources": 12,
                    "rank": 3
                },
                {
                    "title": "Minor economic update",
                    "simple_title": "Minor economic update",
                    "date": "2026-03-10 12:00:00+00:00",
                    "category": "finance",
                    "page_url": "https://heliumtrades.com/balanced-news/minor/",
                    "summary": "Small news.",
                    "takeaway": "Minor takeaway.",
                    "context": "Small context.",
                    "evidence": [],
                    "num_sources": 3,
                    "rank": 7
                }
            ]
        })
    }

    /// Happy path: 3 results → 3 signals, severity by rank.
    #[test]
    fn detects_severity_by_rank() {
        let sigs = parse_response(&sample_response());
        assert_eq!(sigs.len(), 3);
        // rank 1 → priority
        assert_eq!(sigs[0].severity, "priority");
        assert_eq!(sigs[0].kind, "news_headline");
        // rank 3 → routine
        assert_eq!(sigs[1].severity, "routine");
        assert_eq!(sigs[1].kind, "news_significant");
        // rank 7 → info
        assert_eq!(sigs[2].severity, "info");
        assert_eq!(sigs[2].kind, "news_peripheral");
    }

    /// external_id format: `helium:{16-char-sha256-hex}`.
    #[test]
    fn external_id_shape() {
        let sigs = parse_response(&sample_response());
        for s in &sigs {
            assert!(s.external_id.starts_with("helium:"));
            assert_eq!(s.external_id.len(), "helium:".len() + 16);
        }
    }

    /// external_id stability across re-polls: same page_url = same id.
    #[test]
    fn external_id_stable() {
        let a = parse_response(&sample_response());
        let b = parse_response(&sample_response());
        assert_eq!(a[0].external_id, b[0].external_id);
        assert_eq!(a[1].external_id, b[1].external_id);
    }

    /// Top-N cap: more than TOP_N results → only TOP_N emitted.
    #[test]
    fn caps_at_top_n() {
        let mut j = serde_json::json!({"results": []});
        for i in 0..TOP_N + 5 {
            j["results"].as_array_mut().unwrap().push(serde_json::json!({
                "title": format!("Title {i}"),
                "page_url": format!("https://example.com/{i}"),
                "rank": (i + 1) as u64,
            }));
        }
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), TOP_N);
    }

    /// Defensive: missing `results` key → zero signals, no panic.
    #[test]
    fn missing_results_returns_empty() {
        assert!(parse_response(&serde_json::json!({})).is_empty());
    }

    /// Defensive: empty results → zero signals, no panic.
    #[test]
    fn empty_results_returns_empty() {
        let sigs = parse_response(&serde_json::json!({"results": []}));
        assert!(sigs.is_empty());
    }

    /// Defensive: missing `page_url` falls back to title hash.
    #[test]
    fn missing_page_url_uses_title_hash() {
        let j = serde_json::json!({"results": [{
            "title": "Test headline",
            "simple_title": "Test headline",
            "rank": 1
        }]});
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 1);
        // external_id still works (hash of title)
        assert!(sigs[0].external_id.starts_with("helium:"));
        assert_eq!(sigs[0].external_id.len(), "helium:".len() + 16);
    }

    /// Defensive: missing rank falls back to position-based rank.
    #[test]
    fn missing_rank_falls_back_to_index() {
        let j = serde_json::json!({"results": [
            {"title": "First", "page_url": "https://e.com/1"},
            {"title": "Second", "page_url": "https://e.com/2"}
        ]});
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 2);
        // First gets rank 1 → priority
        assert_eq!(sigs[0].severity, "priority");
        // Second gets rank 2 → routine
        assert_eq!(sigs[1].severity, "routine");
    }

    /// Payload carries all useful Helium fields.
    #[test]
    fn payload_shape() {
        let sigs = parse_response(&sample_response());
        let p = &sigs[0].payload;
        assert!(p["title"].as_str().unwrap().contains("cybersecurity"));
        assert_eq!(p["category"], "technology");
        assert_eq!(p["num_sources"], 24);
        assert_eq!(p["rank"], 1);
        assert!(p["page_url"].as_str().unwrap().contains("heliumtrades"));
    }

    /// short_hash is deterministic and 16 chars.
    #[test]
    fn short_hash_format() {
        let h = short_hash("https://example.com/foo");
        assert_eq!(h.len(), 16);
        // Same input → same output
        assert_eq!(h, short_hash("https://example.com/foo"));
        // Different input → different output
        assert_ne!(h, short_hash("https://example.com/bar"));
    }
}