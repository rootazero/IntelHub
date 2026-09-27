//! CongressInvests (https://congressinvests.com) — US politician
//! trades disclosure feed. Phase 3.5 of the public-API integration
//! roadmap (`docs/superpowers/roadmaps/2026-09-27-public-api-
//! integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Polls the CongressInvests public disclosure feed for recent US
//! politician stock trades and emits a Signal per trade. Compounds
//! with the existing `sec_edgar` + `treasury` financial-plane
//! collectors by adding the "are US politicians trading on this?"
//! dimension.
//!
//! ## Auth
//!
//! **Keyless** — the CongressInvests public `/trades` endpoint
//! requires no signup or API key. User confirmed in 2026-09-27:
//! "congress_invests 免费无需注册 https://congressinvests.com/docs".
//! Sent UA = default browser UA (shared `Ctx::http`).
//!
//! ## Severity ladder
//!
//! - Senate trade (smaller body, higher per-trade news value) →
//!   **routine** (politically significant disclosure).
//! - House trade ≥ $250K (large position) → routine.
//! - All other House / "Both" chamber trades → info (ambient
//!   disclosure baseline).
//!
//! Note: the public endpoint doesn't expose `committee_role` or
//! leadership titles (the upstream keeps that for premium). The
//! simpler "Senate / amount-bucket" ladder is what we can ship on
//! the public endpoint; a future 3.5.x with a paid tier could
//! refine the priority signal.
//!
//! ## external_id
//!
//! `congress_invests:{member_slug}:{tx_date}:{ticker}:{trade_type}` —
//! composite key, stable across re-polls. 24h cadence means re-polls
//! of the same trade dedup. The upstream does NOT expose a stable
//! `id` per trade.
//!
//! ## Cadence
//!
//! 24h. Default for OSINT monitors per roadmap §3.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://congressinvests.com/trades";

/// 24h cadence — OSINT monitor default per roadmap §3.
const INTERVAL_SECS: u64 = 24 * 3600;

/// Cap on signals emitted per sweep. Upstream returns a sliding
/// window of recent trades; we cap at 50/day to keep geo_events
/// flowing without flooding the bus.
const TOP_N: usize = 50;

/// Trade-amount regex helpers. The upstream returns amounts as
/// strings like "$1,001 - $15,000" or "$1,000,001 - $5,000,000".
/// We bucket by upper bound for severity.
fn parse_amount_high(amount: &str) -> i64 {
    // The amount format is always "low - high" with $ and commas.
    // Split on '-' and parse the second half.
    if let Some(high_part) = amount.split('-').nth(1) {
        // Strip $, commas, whitespace
        let cleaned: String = high_part
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        cleaned.parse::<i64>().unwrap_or(0)
    } else {
        0
    }
}

/// Slugify a member name for use in external_id (lower-case, alnum + dash).
fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub struct CongressInvests;

impl Source for CongressInvests {
    fn name(&self) -> &'static str {
        "congress_invests"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            // Public keyless endpoint. POST is not required; the
            // default GET returns top-N recent trades.
            let resp = ctx
                .http
                .get(BASE_URL)
                .query(&[("limit", "200")])
                .send()
                .await
                .map_err(|e| crate::error::HubError::sensor(format!("congress http: {e}")))?;
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "congress_invests non-2xx");
                return Ok(Vec::new());
            }
            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| crate::error::HubError::sensor(format!("congress json: {e}")))?;
            let mut out = parse_trades(&body);
            out.truncate(TOP_N);
            Ok(out)
        }
        .boxed()
    }
}

/// Parse the trades response. Pure function for tests.
fn parse_trades(j: &serde_json::Value) -> Vec<Signal> {
    let Some(arr) = j.get("trades").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for t in arr {
        let member = t.get("member").and_then(|v| v.as_str()).unwrap_or("");
        if member.is_empty() {
            continue; // skip records missing member name
        }
        let chamber = t.get("chamber").and_then(|v| v.as_str()).unwrap_or("");
        let trade_type = t.get("trade_type").and_then(|v| v.as_str()).unwrap_or("");
        let amount = t.get("amount").and_then(|v| v.as_str()).unwrap_or("");
        let tx_date = t.get("tx_date").and_then(|v| v.as_str()).unwrap_or("");
        let disclosed = t.get("disclosed").and_then(|v| v.as_str()).unwrap_or("");
        let asset = t.get("asset").and_then(|v| v.as_str()).unwrap_or("");
        let ticker = t.get("ticker").and_then(|v| v.as_str()).unwrap_or("");
        let link = t.get("link").and_then(|v| v.as_str()).unwrap_or("");
        let amount_high = parse_amount_high(amount);
        let (kind, severity) = classify(chamber, amount_high);
        let title = build_title(member, trade_type, ticker, amount);
        let external_id = format!(
            "congress_invests:{}:{}:{}:{}",
            slugify(member),
            tx_date,
            ticker.replace(' ', "-"),
            trade_type,
        );
        out.push(
            Signal::new(kind, title, 0.0, 0.0, external_id)
                .severity(severity)
                .payload(serde_json::json!({
                    "member": member,
                    "chamber": chamber,
                    "trade_type": trade_type,
                    "amount": amount,
                    "amount_high": amount_high,
                    "tx_date": tx_date,
                    "disclosed": disclosed,
                    "asset": asset,
                    "ticker": ticker,
                    "link": link,
                    "extreme_type": kind,
                })),
        );
    }
    out
}

fn classify(chamber: &str, amount_high: i64) -> (&'static str, &'static str) {
    if chamber == "Senate" {
        return ("politician_trade_senate", "routine");
    }
    if amount_high >= 250_000 {
        return ("politician_trade_house_large", "routine");
    }
    ("politician_trade_info", "info")
}

fn build_title(
    member: &str,
    trade_type: &str,
    ticker: &str,
    amount: &str,
) -> String {
    format!("{member} {trade_type} {ticker} ({amount})")
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_senate() -> serde_json::Value {
        serde_json::json!({
            "total": 4680,
            "offset": 0,
            "limit": 50,
            "has_more": true,
            "trades": [{
                "member": "John Boozman",
                "chamber": "Senate",
                "trade_type": "sell",
                "amount": "$1,001 - $15,000",
                "tx_date": "2026-08-27",
                "disclosed": "2026-09-11",
                "asset": "iShares Core S&P 500 ETF",
                "ticker": "IVV",
                "link": "https://efdsearch.senate.gov/search/view/ptr/6298991b-e48f-4b11-9bbb-dfc94a7e1b32/"
            }]
        })
    }

    fn sample_house_large() -> serde_json::Value {
        serde_json::json!({
            "trades": [{
                "member": "Steve Cohen",
                "chamber": "House",
                "trade_type": "buy",
                "amount": "$1,000,001 - $5,000,000",
                "tx_date": "2026-12-26",
                "disclosed": "2026-02-09",
                "asset": "Sony Group Corp",
                "ticker": "SONY",
                "link": "https://disclosures-clerk.house.gov/public_disc/ptr-pdfs/2026/20033889.pdf"
            }]
        })
    }

    fn sample_house_small() -> serde_json::Value {
        serde_json::json!({
            "trades": [{
                "member": "Adam Kinzinger",
                "chamber": "House",
                "trade_type": "buy",
                "amount": "$1,001 - $15,000",
                "tx_date": "2026-09-15",
                "disclosed": "2026-09-26",
                "asset": "Apple Inc",
                "ticker": "AAPL",
                "link": ""
            }]
        })
    }

    /// Senate trade → routine
    #[test]
    fn senate_trade_is_routine() {
        let out = parse_trades(&sample_senate());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, "routine");
        assert_eq!(out[0].kind, "politician_trade_senate");
        assert!(out[0].title.contains("John Boozman"));
        assert!(out[0].title.contains("IVV"));
    }

    /// House + $5M trade → routine (large bucket)
    #[test]
    fn house_large_trade_is_routine() {
        let out = parse_trades(&sample_house_large());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, "routine");
        assert_eq!(out[0].kind, "politician_trade_house_large");
        assert!(out[0].title.contains("Steve Cohen"));
    }

    /// House + small trade → info
    #[test]
    fn house_small_trade_is_info() {
        let out = parse_trades(&sample_house_small());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, "info");
        assert_eq!(out[0].kind, "politician_trade_info");
    }

    /// external_id = slug(member):tx_date:ticker:trade_type
    #[test]
    fn external_id_shape() {
        let out = parse_trades(&sample_senate());
        assert_eq!(
            out[0].external_id,
            "congress_invests:john-boozman:2026-08-27:IVV:sell"
        );
        let out2 = parse_trades(&sample_house_large());
        assert_eq!(
            out2[0].external_id,
            "congress_invests:steve-cohen:2026-12-26:SONY:buy"
        );
    }

    /// Records missing member name are skipped
    #[test]
    fn skips_no_member() {
        let j = serde_json::json!({
            "trades": [{
                "chamber": "House",
                "trade_type": "buy",
                "ticker": "X"
            }]
        });
        let out = parse_trades(&j);
        assert!(out.is_empty());
    }

    /// Body without trades array → empty
    #[test]
    fn handles_missing_trades_array() {
        let j = serde_json::json!({"error": "upstream boom"});
        let out = parse_trades(&j);
        assert!(out.is_empty());
    }

    /// parse_amount_high extracts the upper bound
    #[test]
    fn amount_high_parsing() {
        assert_eq!(parse_amount_high("$1,001 - $15,000"), 15_000);
        assert_eq!(parse_amount_high("$1,000,001 - $5,000,000"), 5_000_000);
        assert_eq!(parse_amount_high("$15,001 - $50,000"), 50_000);
        assert_eq!(parse_amount_high(""), 0);
    }

    /// slugify lowercases + replaces non-alnum with dash
    #[test]
    fn slugify_test() {
        assert_eq!(slugify("John Boozman"), "john-boozman");
        assert_eq!(slugify("María O'Connor-Jones"), "mar-a-o-connor-jones");
        assert_eq!(slugify("   trim me   "), "trim-me");
    }

    /// Payload retains key fields
    #[test]
    fn payload_carries_trade_fields() {
        let out = parse_trades(&sample_senate());
        let p = &out[0].payload;
        assert_eq!(p["member"], "John Boozman");
        assert_eq!(p["ticker"], "IVV");
        assert_eq!(p["amount_high"], 15_000);
        assert_eq!(p["trade_type"], "sell");
    }
}