//! Ahmia — Tor hidden-service search results (OSINT Framework "Dark Web"
//! category). Polls ahmia.fi for a curated set of search terms and emits
//! a Signal per result with the .onion URL in the payload.
//!
//! Ahmia.fi has **no official JSON API** (verified 2026-09: /api/search,
//! /search.json, /api/v1/search all 404). The clearnet search page
//! (`/search/?q=<term>`) is JS-rendered; the static HTML response we get
//! via `reqwest` is the pre-render skeleton only and contains no results.
//!
//! To avoid silently scraping nothing, we use Ahmia's **documented
//! filterlist endpoint** at `/search/API?q=<term>` (referenced in
//! Ahmia GitHub issue #29 and used by the `digse` Rust crate) — it
//! returns a plain-text newline-delimited list of .onion URLs that match
//! the term. This endpoint is experimental and may change without
//! notice. If it 404s in the future, the collector degrades gracefully
//! (visible on health board, 0 new events) and self-heals when the
//! endpoint comes back.
//!
//! - **KEYLESS**: no auth required. Clearnet only (no Tor connectivity
//!   required on the hub side).
//! - **KIND**: "cyber" (matches otx/nvd/cisakev visual cluster on radar).
//! - **ANCHOR**: Ahmia HQ (Helsinki). Results have no real geo — Ahmia
//!   indexes Tor hidden services worldwide and onion-host geolocation
//!   is unreliable by design. The Helsinki anchor keeps them visually
//!   clustered as "dark web" on the radar.
//! - **WATCHLIST**: HUB_AHMIA_QUERY env var (CSV). Default ships with 5
//!   high-signal terms (ransomware, APT names, leak site, marketplace).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const AHMIA_BASE: &str = "https://ahmia.fi";
const AHMIA_HQ: (f64, f64) = (60.1699, 24.9384); // Helsinki, FI
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const INTER_QUERY_GAP: Duration = Duration::from_secs(4);
const MAX_RESULTS_PER_QUERY: usize = 25;

/// Default watchlist: 5 high-signal dark-web terms. Conservative on
/// volume — each term can return 100s of hits and we cap per-query.
const DEFAULT_QUERY: &[&str] = &[
    "ransomware",
    "APT29",
    "leak site",
    "marketplace",
    "phishing kit",
];

pub struct Ahmia;

impl Source for Ahmia {
    fn name(&self) -> &'static str {
        "ahmia"
    }
    fn interval(&self) -> Duration {
        // 12h. Dark-web index churns slowly; daily would miss active
        // campaigns, hourly would over-poll a free endpoint.
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let queries: Vec<&str> = if !ctx.config.monitor_ahmia_query.is_empty() {
                ctx.config.monitor_ahmia_query.iter().map(|s| s.as_str()).collect()
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
                        tracing::warn!(target: "monitor::ahmia", term = term, error = %e, "query failed");
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
    // Ahmia's documented filterlist endpoint: returns newline-delimited
    // .onion URLs that match the term. May 404 if Ahmia changes its URL
    // shape — see module docstring.
    let url = format!("{AHMIA_BASE}/search/API?q={}", url_encode(term));

    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (IntelHub research; contact: ops@intelhub.local)")
            .send(),
    )
    .await
    .map_err(|_| crate::error::HubError::sensor(format!("ahmia: request timed out")))?
    .map_err(|e| crate::error::HubError::sensor(format!("ahmia: {}", e.to_string())))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND
        || resp.status() == reqwest::StatusCode::GONE
    {
        // Endpoint moved or deprecated. Warn + degrade gracefully.
        tracing::warn!(target: "monitor::ahmia", term = term, "filterlist endpoint not found — site shape may have changed");
        return Ok(Vec::new());
    }
    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(crate::error::HubError::sensor(format!(
            "ahmia: HTTP {status}: {snippet}"
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("ahmia: body: {e}")))?;

    // Parse newline-delimited .onion URLs. Ahmia's filterlist endpoint
    // returns one URL per line; empty lines and non-onion tokens are
    // skipped silently.
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.contains(".onion") {
            continue;
        }
        if !seen.insert(trimmed.to_string()) {
            continue;
        }
        out.push(
            Signal::new(
                "cyber",
                format!("Tor hidden service: {trimmed} (matched: {term})"),
                AHMIA_HQ.0,
                AHMIA_HQ.1,
                format!("ahmia:onion:{trimmed}"),
            )
            .severity("routine")
            .payload(serde_json::json!({
                "onion_url": trimmed,
                "matched_term": term,
                "source_endpoint": "/search/API",
            })),
        );
        if out.len() >= MAX_RESULTS_PER_QUERY {
            break;
        }
    }

    Ok(out)
}

/// Minimal URL-encoding for query parameter values. Avoids pulling in
/// the `urlencoding` crate for a single call site.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_basic() {
        assert_eq!(url_encode("ransomware"), "ransomware");
        assert_eq!(url_encode("APT29"), "APT29");
        assert_eq!(url_encode("leak site"), "leak%20site");
        assert_eq!(url_encode("market&place"), "market%26place");
    }

    #[test]
    fn parses_onion_lines() {
        let body = "abcxyz.onion\nnot-an-onion\nfoo.onion\n";
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for line in body.lines() {
            let t = line.trim();
            if t.is_empty() || !t.contains(".onion") { continue; }
            if !seen.insert(t.to_string()) { continue; }
            out.push(t.to_string());
        }
        assert_eq!(out, vec!["abcxyz.onion", "foo.onion"]);
    }
}
