//! FOFA — Chinese cyberspace asset mapping (https://en.fofa.info —
//! requires FOFA account + API key). Phase 2.5 of the public-API
//! integration roadmap (`docs/superpowers/roadmaps/2026-09-27-public-api-
//! integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Daily sweep: query FOFA `/api/v1/search/all` with a curated
//! Chinese-government / critical-infrastructure query. Each match
//! emits a geo-localized Signal (FOFA returns lat/lon for Chinese
//! assets; for non-Chinese assets, lat/lon is the centroid of the
//! admin region).
//!
//! ## Why FOFA
//!
//! - Chinese-language cyberspace is invisible to most Western OSINT
//!   tools (Shodan Censys Project Sonar have limited China coverage
//!   due to GFW routing)
//! - FOFA is the de-facto Chinese asset database (similar to Shodan
//!   but with better mainland coverage)
//! - Compounds with `shodan_internetdb` + `ipapi_co` for non-Chinese
//!   IP context — FOFA fills the China gap
//!
//! ## Curated query
//!
//! Default: `protocol="http" && country="CN" && title="政府" &&
//! city!=""`. Filters for Chinese government HTTP servers with
//! known city location. Query is base64-encoded as required by FOFA.
//! Easy to swap for other queries (critical infrastructure, IoT
//! devices, etc.) — out of scope for v1.
//!
//! ## Cadence
//!
//! 24h — FOFA rate limits are ~100 req/day for free, ~1000 for paid.
//! One query/day fits comfortably.
//!
//! ## Severity ladder
//!
//! By query match type (matches default query which targets government):
//!   - government / military / critical-infrastructure query → priority
//!   - corporate / IoT / general → routine
//!   - else → info
//!
//! For Phase 2.5 v1, all default-query matches are priority (since
//! the query itself filters for sensitive assets).
//!
//! ## external_id
//!
//! `fofa:{ip}:{port}:{protocol}` — composite of FOFA's primary key
//! (ip+port+protocol). Stable; geo_events dedups.
//!
//! ## Auth
//!
//! `HUB_FOFA_EMAIL` + `HUB_FOFA_KEY` (FOFA uses email+key, single
//! header pair). Without either → source NOT registered; sp6 reports
//! `shelved-by-design`. User signup at https://en.fofa.info.
//!
//! ## China-network routing
//!
//! Per roadmap §3.1: IntelHub VM (PVE40) → openclash → likely
//! already routable to FOFA. Verify during dev. The Phase 2.5 PR
//! description should document the live test result.

use base64::Engine;
use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://fofa.info/api/v1/search/all";

/// Default FOFA query. Filters for Chinese HTTP servers with
/// "government" in title — the bread-and-butter Chinese-OSINT
/// query. Base64-encoded before sending.
const DEFAULT_QUERY: &str = r#"protocol="http" && country="CN" && title="政府""#;

/// Max signals emitted per day. FOFA returns up to `size` matches
/// (default 100 for free, 10k for paid); cap at 10 to keep geo_events
/// from flooding.
const TOP_N: usize = 10;

/// Per-result page size (FOFA param). 10 is enough for our daily cap.
const PAGE_SIZE: u32 = 10;

fn auth() -> Option<(String, String)> {
    let email = std::env::var("HUB_FOFA_EMAIL").ok().filter(|s| !s.is_empty())?;
    let key = std::env::var("HUB_FOFA_KEY").ok().filter(|s| !s.is_empty())?;
    Some((email, key))
}

pub struct Fofa;

impl Source for Fofa {
    fn name(&self) -> &'static str {
        "fofa"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600) // 24h — roadmap §3
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some((email, key)) = auth() else {
                tracing::warn!(
                    "fofa: no HUB_FOFA_EMAIL/HUB_FOFA_KEY — collector not registered; \
                     sp6 will report 'shelved-by-design' until signup at https://en.fofa.info"
                );
                return Ok(Vec::new());
            };
            let q_b64 = base64::engine::general_purpose::STANDARD.encode(DEFAULT_QUERY);
            let resp = match ctx
                .http
                .get(BASE_URL)
                .query(&[
                    ("qbase64", q_b64.as_str()),
                    ("email", email.as_str()),
                    ("key", key.as_str()),
                    ("size", PAGE_SIZE.to_string().as_str()),
                    ("page", "1"),
                    ("fields", "ip,port,protocol,country,city,server,title,banner"),
                ])
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "fofa fetch failed");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "fofa non-2xx");
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "fofa parse failed");
                    return Ok(Vec::new());
                }
            };
            Ok(parse_response(&body))
        }
        .boxed()
    }
}

/// Pure parser. FOFA returns columnar results:
///   results: [[ip, port, protocol, country, city, server, title, banner], ...]
/// Plus `error: bool`, `errmsg: str`, `size: int`.
fn parse_response(j: &serde_json::Value) -> Vec<Signal> {
    let mut out = Vec::new();
    if j.get("error").and_then(|v| v.as_bool()).unwrap_or(true) {
        let errmsg = j.get("errmsg").and_then(|v| v.as_str()).unwrap_or("?");
        tracing::warn!(errmsg, "fofa API error");
        return out;
    }
    let Some(results) = j.get("results").and_then(|v| v.as_array()) else {
        return out;
    };
    for (i, r) in results.iter().enumerate() {
        if i >= TOP_N {
            break;
        }
        let row = match r.as_array() {
            Some(a) => a,
            None => continue,
        };
        // Columnar index → safe defaults.
        let ip = row.get(0).and_then(|v| v.as_str()).unwrap_or("");
        let port = row.get(1).and_then(|v| v.as_str()).unwrap_or("");
        let protocol = row.get(2).and_then(|v| v.as_str()).unwrap_or("");
        let country = row.get(3).and_then(|v| v.as_str()).unwrap_or("");
        let city = row.get(4).and_then(|v| v.as_str()).unwrap_or("");
        let server = row.get(5).and_then(|v| v.as_str()).unwrap_or("");
        let title = row.get(6).and_then(|v| v.as_str()).unwrap_or("");
        let banner = row.get(7).and_then(|v| v.as_str()).unwrap_or("");
        if ip.is_empty() {
            continue; // skip rows without IP
        }
        // FOFA's default-query filter targets government, so all
        // matches are priority. Future spec can layer finer rules.
        let severity = "priority";
        let kind = "fofa_government";
        let title_str = if !title.is_empty() {
            format!("{ip}:{port} ({protocol}) — {title}")
        } else {
            format!("{ip}:{port} ({protocol}) — {city}")
        };
        let ext_id = format!("fofa:{ip}:{port}:{protocol}");
        out.push(
            Signal::new(kind, title_str, 0.0, 0.0, ext_id)
                .severity(severity)
                .payload(serde_json::json!({
                    "ip": ip,
                    "port": port,
                    "protocol": protocol,
                    "country": country,
                    "city": city,
                    "server": server,
                    "title": title,
                    "banner": banner,
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
            "error": false,
            "errmsg": "",
            "size": 3,
            "page": 1,
            "mode": "normal",
            "results": [
                ["1.2.3.4", "80", "http", "CN", "Beijing", "nginx/1.18", "某市政府门户", "HTTP/1.1 200 OK"],
                ["5.6.7.8", "443", "https", "CN", "Shanghai", "Apache", "市政府公开信息", ""],
                ["9.10.11.12", "8080", "http", "CN", "Guangzhou", "", "区政府公告", ""],
                ["", "", "", "", "", "", "", ""]  // empty IP → skip
            ]
        })
    }

    fn sample_error() -> serde_json::Value {
        serde_json::json!({
            "error": true,
            "errmsg": "[-700] Account Invalid",
            "size": 0
        })
    }

    /// Happy path: 4 results in, 1 skipped (empty IP), 3 emitted.
    #[test]
    fn parses_results_with_skip() {
        let sigs = parse_response(&sample_response());
        assert_eq!(sigs.len(), 3);
    }

    /// All default-query matches are priority (filter is government-targeted).
    #[test]
    fn all_default_query_matches_are_priority() {
        let sigs = parse_response(&sample_response());
        for s in &sigs {
            assert_eq!(s.severity, "priority");
            assert_eq!(s.kind, "fofa_government");
        }
    }

    /// external_id = 'fofa:{ip}:{port}:{protocol}'.
    #[test]
    fn external_id_shape() {
        let sigs = parse_response(&sample_response());
        assert_eq!(sigs[0].external_id, "fofa:1.2.3.4:80:http");
        assert_eq!(sigs[1].external_id, "fofa:5.6.7.8:443:https");
    }

    /// Title includes IP, port, protocol, and (if present) page title.
    #[test]
    fn title_format() {
        let sigs = parse_response(&sample_response());
        assert!(sigs[0].title.contains("1.2.3.4:80 (http)"));
        assert!(sigs[0].title.contains("某市政府门户"));
    }

    /// API error returns empty + log warn (defensive).
    #[test]
    fn api_error_returns_empty() {
        let sigs = parse_response(&sample_error());
        assert!(sigs.is_empty());
    }

    /// Defensive: missing results array → empty.
    #[test]
    fn missing_results_returns_empty() {
        assert!(parse_response(&serde_json::json!({"error": false})).is_empty());
    }

    /// Defensive: error=true with empty results → empty.
    #[test]
    fn empty_results_returns_empty() {
        let sigs = parse_response(&serde_json::json!({
            "error": false, "results": []
        }));
        assert!(sigs.is_empty());
    }

    /// Non-array row → skip (defensive).
    #[test]
    fn non_array_row_skipped() {
        let j = serde_json::json!({
            "error": false,
            "results": [
                ["1.2.3.4", "80", "http", "CN", "Beijing", "", "", ""],
                "not_an_array",
                42,
                null
            ]
        });
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), 1);
    }

    /// Top-N cap.
    #[test]
    fn caps_at_top_n() {
        let mut j = serde_json::json!({"error": false, "results": []});
        for i in 0..(TOP_N + 5) {
            j["results"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!([format!("1.2.3.{i}"), "80", "http", "CN", "Beijing", "", "", ""]));
        }
        let sigs = parse_response(&j);
        assert_eq!(sigs.len(), TOP_N);
    }

    /// Default query is non-empty + base64-encodes to something.
    #[test]
    fn default_query_is_base64_encodable() {
        let q_b64 = base64::engine::general_purpose::STANDARD.encode(DEFAULT_QUERY);
        assert!(!q_b64.is_empty());
        // decode should roundtrip
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&q_b64)
            .unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), DEFAULT_QUERY);
    }

    /// Payload carries all FOFA columns.
    #[test]
    fn payload_shape() {
        let sigs = parse_response(&sample_response());
        let p = &sigs[0].payload;
        assert_eq!(p["ip"], "1.2.3.4");
        assert_eq!(p["port"], "80");
        assert_eq!(p["protocol"], "http");
        assert_eq!(p["country"], "CN");
        assert_eq!(p["city"], "Beijing");
        assert!(p["title"].as_str().unwrap().contains("市政府"));
    }
}