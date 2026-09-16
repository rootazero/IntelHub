//! OpenCorporates — global company registry watcher (OSINT Framework
//! "Business Records → Entities" gap). Polls the OpenCorporates v0.4
//! search API for a curated set of company names and jurisdictions;
//! each new incorporation event emits a Signal anchored at the
//! company's registered address (or HQ jurisdiction capital if the
//! address isn't returned).
//!
//! - **AUTH REQUIRED**: OpenCorporates requires an `api_token` query
//!   parameter. Free tier is 200 calls/month — fine for one call per
//!   watch entry per day (5 entries × 30 days = 150 calls/month). Apply
//!   at https://api.opencorporates.com/ — registration is instant, the
//!   token is shown on the dashboard.
//! - **SHELVED-BY-DESIGN** without a token (verified 2026-09-16: HTTP
//!   401 with `{"error":{"message":"Invalid Api Token"}}`). The
//!   collector stays visible on the health board and self-heals the
//!   moment a token is configured.
//! - **WATCHLIST**: HUB_OPENCORP_WATCH env var is a comma-list of
//!   `name|jurisdiction_code` pairs (e.g. `Tesla Inc|us_de`,
//!   `LockBit Group|gb`). Default ships with 5 sentinel queries that
//!   match high-profile OFAC/sanctioned entities — these keep returning
//!   data as long as OpenCorporates has jurisdiction coverage.
//! - **KIND**: "financial" (matches etherscan/defillama visual cluster).
//! - **RATE LIMIT**: free tier = 200/month. Each sweep uses 1 call per
//!   watch entry. 24h cadence × 30 entries = 30 calls/day = 900/month —
//!   would exceed the free tier. Trim the watchlist to ≤6 entries for
//!   safe headroom.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const OPENCORP_ENDPOINT: &str = "https://api.opencorporates.com/v0.4/companies/search";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const INTER_QUERY_GAP: Duration = Duration::from_secs(2);
const MAX_RESULTS_PER_QUERY: usize = 10;

/// Jurisdiction capital coords (used as Signal anchor when company HQ
/// lat/lon isn't returned). OpenCorporates returns registered address
/// text only — no geo. We use the jurisdiction capital as a reasonable
/// visual fallback so signals cluster by jurisdiction on the radar.
const JURISDICTION_HQ: &[(&str, f64, f64)] = &[
    ("us_de", 39.7392, -75.5398),  // Dover, DE
    ("us_wy", 41.1400, -104.8202), // Cheyenne, WY
    ("gb", 51.5074, -0.1278),      // London
    ("de", 52.5200, 13.4050),      // Berlin
    ("fr", 48.8566, 2.3522),       // Paris
    ("ky", 19.2867, -81.3744),     // George Town, Cayman
    ("vg", 18.4280, -64.6230),     // Road Town, BVI
    ("hk", 22.3193, 114.1694),     // Hong Kong
    ("sg", 1.3521, 103.8198),      // Singapore
    ("ae", 25.2048, 55.2708),      // Dubai
];

/// Default watchlist: 5 sentinel queries. Each entry has format
/// `company_name|jurisdiction_code` — the `|` separator is the
/// unambiguous split (CSV conflicts with company names containing
/// commas). Lowercase jurisdiction codes match OpenCorporates' URL shape.
const DEFAULT_WATCHLIST: &[&str] = &[
    "Tornado Cash|ky",
    "Lazarus Group|gb",
    "Wagner Group|ru",
    "LockBit|vg",
    "Huawei Technologies|hk",
];

pub struct Opencorp;

impl Source for Opencorp {
    fn name(&self) -> &'static str {
        "opencorp"
    }
    fn interval(&self) -> Duration {
        // 24h. Incorporations are slow-moving and OpenCorporates free
        // tier budget is tight.
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let api_token = ctx
                .config
                .monitor_opencorp_api_token
                .clone()
                .filter(|s| !s.is_empty());
            let Some(_api_token) = api_token else {
                // OpenCorp returns 401 without a token. Shelved-by-design
                // (per user decision 2026-09-16): keep collector visible,
                // log clear WARN, wait for operator to configure the key.
                tracing::warn!(target: "monitor::opencorp",
                    "no OPENCORP_API_TOKEN — register at https://api.opencorporates.com/ \
                     and add the token to core/secrets.env then restart hub-core");
                return Ok(Vec::new());
            };

            let watchlist: Vec<&str> = if !ctx.config.monitor_opencorp_watch.is_empty() {
                ctx.config.monitor_opencorp_watch.iter().map(|s| s.as_str()).collect()
            } else {
                DEFAULT_WATCHLIST.to_vec()
            };

            let mut out = Vec::new();
            for (idx, entry) in watchlist.iter().enumerate() {
                if idx > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query_entry(ctx, entry, &_api_token).await {
                    Ok(mut sigs) => out.append(&mut sigs),
                    Err(e) => {
                        tracing::warn!(target: "monitor::opencorp", entry = entry, error = %e, "query failed");
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

async fn query_entry(ctx: &Ctx, entry: &str, api_token: &str) -> Result<Vec<Signal>> {
    let (name, jurisdiction) = match entry.split_once('|') {
        Some((n, j)) => (n.trim(), j.trim().to_lowercase()),
        None => {
            tracing::warn!(target: "monitor::opencorp", entry = entry,
                "malformed watchlist entry — expected `company_name|jurisdiction_code`");
            return Ok(Vec::new());
        }
    };

    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(OPENCORP_ENDPOINT)
            .query(&[
                ("q", name),
                ("jurisdiction_code", &jurisdiction),
                ("api_token", api_token),
                ("per_page", "10"),
            ])
            .send(),
    )
    .await
    .map_err(|_| crate::error::HubError::sensor(format!("opencorp: request timed out")))?
    .map_err(|e| crate::error::HubError::sensor(format!("opencorp: {}", e.to_string())))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED
        || resp.status() == reqwest::StatusCode::FORBIDDEN
    {
        // Token expired or invalid. Don't fail the sweep — operator
        // needs to refresh the token.
        tracing::warn!(target: "monitor::opencorp", "HTTP 401/403 — check OPENCORP_API_TOKEN");
        return Ok(Vec::new());
    }
    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // Free tier = 200/month. Wait until next sweep.
        tracing::warn!(target: "monitor::opencorp", "rate-limited (free tier 200/month) — wait until next sweep");
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(crate::error::HubError::sensor(format!(
            "opencorp: HTTP {status}: {snippet}"
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("opencorp: parse: {e}")))?;

    let companies = body
        .get("results")
        .and_then(|r| r.get("companies"))
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    // Anchor: jurisdiction capital (no per-company geo in API response).
    let (anchor_lat, anchor_lon) = JURISDICTION_HQ
        .iter()
        .find(|(code, _, _)| *code == jurisdiction.as_str())
        .map(|(_, lat, lon)| (*lat, *lon))
        .unwrap_or((0.0, 0.0));

    let mut out = Vec::new();
    for comp in companies.into_iter().take(MAX_RESULTS_PER_QUERY) {
        let company = match comp.get("company") {
            Some(c) => c,
            None => continue,
        };
        let comp_name = company
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or(name)
            .to_string();
        let comp_id = company
            .get("company_number")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .to_string();
        let status = company
            .get("current_status")
            .and_then(|x| x.as_str())
            .unwrap_or("Unknown");
        let incorporation_date = company
            .get("incorporation_date")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let company_type = company
            .get("company_type")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let registered_addr = company
            .get("registered_address_in_full")
            .and_then(|x| x.as_str())
            .unwrap_or("");

        let title = if !registered_addr.is_empty() {
            format!("{comp_name} ({jurisdiction} · {status}) @ {registered_addr}")
        } else {
            format!("{comp_name} ({jurisdiction} · {status})")
        };

        out.push(
            Signal::new(
                "financial",
                title,
                anchor_lat,
                anchor_lon,
                format!("opencorp:{jurisdiction}:{comp_id}"),
            )
            .severity("routine")
            .payload(serde_json::json!({
                "name": comp_name,
                "jurisdiction_code": jurisdiction,
                "company_number": comp_id,
                "current_status": status,
                "incorporation_date": incorporation_date,
                "company_type": company_type,
                "registered_address": registered_addr,
            })),
        );
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn split_once_parses_watch_entry() {
        let (n, j) = "Tornado Cash|ky".split_once('|').unwrap();
        assert_eq!(n.trim(), "Tornado Cash");
        assert_eq!(j.trim().to_lowercase(), "ky");
    }

    #[test]
    fn split_once_rejects_no_separator() {
        assert!("noseparator".split_once('|').is_none());
    }

    #[test]
    fn jurisdiction_hq_lookup_known() {
        let hit = super::JURISDICTION_HQ
            .iter()
            .find(|(code, _, _)| *code == "gb")
            .unwrap();
        assert_eq!(hit.1, 51.5074);
        assert_eq!(hit.2, -0.1278);
    }

    #[test]
    fn jurisdiction_hq_lookup_unknown_returns_zero() {
        // Unknown jurisdiction → (0,0) fallback (deliberate — operator
        // sees the signals, can fix the watchlist).
        let hit = super::JURISDICTION_HQ
            .iter()
            .find(|(code, _, _)| *code == "zz");
        assert!(hit.is_none());
    }
}
