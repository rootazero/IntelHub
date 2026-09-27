//! Currents API (https://currentsapi.services — free 250 req/day with
//! API key). Phase 2.2 of the public-API integration roadmap (`docs/
//! superpowers/roadmaps/2026-09-27-public-api-integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Daily sweep: query `/v1/latest-news?language=en&page_size=10` for
//! fresh multi-language news (English v1, easy to extend). Each result
//! is a fresh news item with title, description, source URL, category,
//! and publish timestamp. Severity by category (politics/crime =
//! priority, tech/science = routine, lifestyle = info).
//!
//! ## Why Currents (vs Helium, RSS)
//!
//! - Helium (Phase 1.8): bias-balanced synthesis of 5000+ sources,
//   but ~free 50 queries/week and only 1 query/day practical
//! - RSS (existing): curated OSINT feeds but limited to subscribed
//!   sources, no language breadth
//! - Currents: free 250 req/day, multi-language, broad source coverage,
//   fresh latest-news filter → complementary news signal
//!
//! ## Future: GDELT 429 fallback wiring
//!
//! Roadmap §3.1 calls for wiring Currents as a GDELT fallback: when
//! gdelt health cell reports rate-limit (429) in last 10 min, raise
//! Currents weight; otherwise stay quiet. Phase 2.2 v1 emits as a
//! normal monitor (regardless of GDELT state). Phase 2.2.x adds the
//! conditional weight logic — out of scope here.
//!
//! ## Severity ladder
//!
//! By category (matches GDELT's primary categories):
//!   - politics_government / crime_law_justice → priority
//!   - economy_business_finance / science_technology / environment
//!     → routine
//!   - sport / lifestyle / arts_culture_entertainment / general → info
//!
//! ## external_id
//!
//! `currents:{uuid_first16}` — Currents returns a UUID per article
//! (stable across re-polls; same article = same UUID = geo_events
//! dedups).
//!
//! ## Auth
//!
//! `HUB_CURRENTS_API_KEY` (env-gated, mirrors open_aq). Without key →
//! source NOT registered; sp6 reports `shelved-by-design`. User signup
//! at https://currentsapi.services/en/register.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.currentsapi.services/v1/latest-news";

/// Default language filter. Free tier is best for English (broadest
/// coverage). Easy to switch to multi-language via comma-separated
/// (e.g. "en,fr,de,es") in a future spec.
const DEFAULT_LANGUAGE: &str = "en";

/// Cap on signals emitted per day. Currents free tier returns ~10-20
/// per `page_size`; cap at 10 to match Helium parity (similar news
/// ingest role). Easy to bump.
const TOP_N: usize = 10;

/// Categories that flag priority-level events (newsworthy political /
/// criminal activity).
const PRIORITY_CATEGORIES: &[&str] = &[
    "politics_government",
    "crime_law_justice",
    "world",
];

/// Categories that flag routine events (significant but secondary).
const ROUTINE_CATEGORIES: &[&str] = &[
    "economy_business_finance",
    "science_technology",
    "environment",
    "labour",
    "health",
];

fn api_key() -> Option<String> {
    std::env::var("HUB_CURRENTS_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
}

pub struct Currents;

impl Source for Currents {
    fn name(&self) -> &'static str {
        "currents"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600) // 24h — roadmap §3
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(key) = api_key() else {
                tracing::warn!(
                    "currents: no HUB_CURRENTS_API_KEY — collector not registered; \
                     sp6 will report 'shelved-by-design' until signup at \
                     https://currentsapi.services/en/register"
                );
                return Ok(Vec::new());
            };
            let resp = match ctx
                .http
                .get(BASE_URL)
                .header("Authorization", format!("Bearer {key}"))
                .query(&[("language", DEFAULT_LANGUAGE), ("page_size", &TOP_N.to_string())])
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "currents fetch failed");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "currents non-2xx");
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "currents parse failed");
                    return Ok(Vec::new());
                }
            };
            Ok(parse_response(&body))
        }
        .boxed()
    }
}

/// Pure parser. Walks the `news[]` array, classifies each by category,
/// emits the top N as Signals. Defensive against missing fields.
fn parse_response(j: &serde_json::Value) -> Vec<Signal> {
    let mut out = Vec::new();
    if j.get("status").and_then(|v| v.as_str()) != Some("ok") {
        return out;
    }
    let Some(news) = j.get("news").and_then(|v| v.as_array()) else {
        return out;
    };
    for (i, r) in news.iter().enumerate() {
        if i >= TOP_N {
            break;
        }
        let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let description = r
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let author = r.get("author").and_then(|v| v.as_str()).unwrap_or("");
        let image = r.get("image").and_then(|v| v.as_str()).unwrap_or("");
        let language = r.get("language").and_then(|v| v.as_str()).unwrap_or("");
        let categories: Vec<String> = r
            .get("category")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let published = r.get("published").and_then(|v| v.as_str()).unwrap_or("");
        // Severity by category priority.
        let (severity, kind) = if categories
            .iter()
            .any(|c| PRIORITY_CATEGORIES.contains(&c.as_str()))
        {
            ("priority", "news_priority")
        } else if categories
            .iter()
            .any(|c| ROUTINE_CATEGORIES.contains(&c.as_str()))
        {
            ("routine", "news_routine")
        } else {
            ("info", "news_info")
        };
        // External ID: UUID first 16 chars (Currents returns a UUID).
        // Falls back to URL hash if UUID missing (defensive).
        let ext_id = if !id.is_empty() {
            format!("currents:{}", &id[..id.len().min(16)])
        } else if !url.is_empty() {
            format!("currents:{}", short_hash(url))
        } else {
            format!("currents:{}", short_hash(title))
        };
        out.push(
            Signal::new(kind, title.to_string(), 0.0, 0.0, ext_id)
                .severity(severity)
                .payload(serde_json::json!({
                    "id": id,
                    "title": title,
                    "description": description,
                    "url": url,
                    "author": author,
                    "image": image,
                    "language": language,
                    "category": categories,
                    "published": published,
                    "extreme_type": kind,
                })),
        );
    }
    out
}

/// Truncated sha256 hex (16 chars = 64 bits of entropy).
fn short_hash(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(s.as_bytes());
    let hex = format!("{:x}", h);
    hex.chars().take(16).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_response() -> serde_json::Value {
        serde_json::json!({
            "status": "ok",
            "news": [
                {
                    "id": "abc-12345-uuid-67890",
                    "title": "Election results announced",
                    "description": "Voting concluded...",
                    "url": "https://example.com/election",
                    "author": "Jane Doe",
                    "image": "https://example.com/img.jpg",
                    "language": "en",
                    "category": ["politics_government", "world"],
                    "published": "2026-09-27T10:00:00+00:00"
                },
                {
                    "id": "def-67890-uuid-12345",
                    "title": "New AI model released",
                    "description": "AI breakthrough...",
                    "url": "https://example.com/ai",
                    "language": "en",
                    "category": ["science_technology"],
                    "published": "2026-09-27T11:00:00+00:00"
                },
                {
                    "id": "ghi-99999-uuid-33333",
                    "title": "Sports championship finals",
                    "description": "Game results...",
                    "url": "https://example.com/sports",
                    "language": "en",
                    "category": ["sport"],
                    "published": "2026-09-27T12:00:00+00:00"
                }
            ]
        })
    }

    /// Happy path: 3 results, severity by category.
    #[test]
    fn detects_severity_by_category() {
        let sigs = parse_response(&sample_response());
        assert_eq!(sigs.len(), 3);
        // politics → priority
        assert_eq!(sigs[0].severity, "priority");
        assert_eq!(sigs[0].kind, "news_priority");
        // science_technology → routine
        assert_eq!(sigs[1].severity, "routine");
        assert_eq!(sigs[1].kind, "news_routine");
        // sport → info
        assert_eq!(sigs[2].severity, "info");
        assert_eq!(sigs[2].kind, "news_info");
    }

    /// external_id format: `currents:{uuid_first16}`.
    #[test]
    fn external_id_shape() {
        let sigs = parse_response(&sample_response());
        assert!(sigs[0].external_id.starts_with("currents:"));
        // "abc-12345-uuid-67890".len() = 21, truncated to 16 = "abc-12345-uuid-6"
        assert_eq!(sigs[0].external_id, "currents:abc-12345-uuid-6");
    }

    /// Defensive: status != ok → empty.
    #[test]
    fn non_ok_status_returns_empty() {
        let j = serde_json::json!({"status": "error", "message": "something"});
        assert!(parse_response(&j).is_empty());
    }

    /// Defensive: missing news array → empty.
    #[test]
    fn missing_news_returns_empty() {
        let j = serde_json::json!({"status": "ok"});
        assert!(parse_response(&j).is_empty());
    }

    /// Defensive: empty news array → empty.
    #[test]
    fn empty_news_returns_empty() {
        let j = serde_json::json!({"status": "ok", "news": []});
        assert!(parse_response(&j).is_empty());
    }

    /// Defensive: missing UUID → falls back to URL hash.
    #[test]
    fn missing_uuid_falls_back_to_url_hash() {
        let j = serde_json::json!({
            "status": "ok",
            "news": [{
                "title": "Test article",
                "url": "https://example.com/test",
                "category": ["general"],
                "language": "en"
            }]
        });
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 1);
        // hash of URL, 16 chars
        assert!(sigs[0].external_id.starts_with("currents:"));
        assert_eq!(sigs[0].external_id.len(), "currents:".len() + 16);
    }

    /// Top-N cap: more than TOP_N results → only TOP_N emitted.
    #[test]
    fn caps_at_top_n() {
        let mut j = serde_json::json!({"status": "ok", "news": []});
        for i in 0..(TOP_N + 5) {
            j["news"].as_array_mut().unwrap().push(serde_json::json!({
                "id": format!("id{i}"),
                "title": format!("Title {i}"),
                "url": format!("https://example.com/{i}"),
                "category": ["general"],
                "language": "en"
            }));
        }
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), TOP_N);
    }

    /// Priority categories from spec list.
    #[test]
    fn priority_categories_list() {
        assert!(PRIORITY_CATEGORIES.contains(&"politics_government"));
        assert!(PRIORITY_CATEGORIES.contains(&"crime_law_justice"));
    }

    /// Routine categories from spec list.
    #[test]
    fn routine_categories_list() {
        assert!(ROUTINE_CATEGORIES.contains(&"economy_business_finance"));
        assert!(ROUTINE_CATEGORIES.contains(&"science_technology"));
    }

    /// short_hash format and determinism.
    #[test]
    fn short_hash_format() {
        let h = short_hash("https://example.com/foo");
        assert_eq!(h.len(), 16);
        assert_eq!(h, short_hash("https://example.com/foo"));
        assert_ne!(h, short_hash("https://example.com/bar"));
    }
}