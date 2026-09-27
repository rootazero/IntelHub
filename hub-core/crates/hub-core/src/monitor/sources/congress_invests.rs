//! CongressInvests (https://congressinvests.com) — US politician
//! trades disclosure feed. Phase 3.5 of the public-API integration
//! roadmap (`docs/superpowers/roadmaps/2026-09-27-public-api-
//! integration-roadmap.md`).
//!
//! ## Strategy
//!
//! Polls the CongressInvests public disclosure feed for new trades
//! by US members of Congress. Compounds with the existing `sec_edgar`
//! + `treasury` financial-plane collectors by adding the "are US
//! politicians trading on this?" dimension.
//!
//! ## Auth
//!
//! `HUB_CONGRESSINVESTS_API_KEY` (env-gated, paid tier). Without
//! key → source NOT registered; sp6 reports `shelved-by-design`.
//! User signup at https://congressinvests.com — paid tier.
//!
//! ## Severity ladder
//!
//! - Trade by senior committee member (Speaker / committee chairs /
//   ranking members) → **priority** (politically significant
//!   conflicts of interest).
//! - Trade by rank-and-file member on a sector-aligned committee
//!   → routine (warrants analyst review).
//! - All other trades → info (ambient disclosure baseline).
//!
//! ## external_id
//!
//! `congress_invests:{txn_id}` — server-assigned (stable).
//!
//! ## Cadence
//!
//! 24h. Default for OSINT monitors per roadmap §3.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const BASE_URL: &str = "https://api.congressinvests.com/v1/trades";

/// Env-var name for the CongressInvests key (paid tier).
const ENV_KEY: &str = "HUB_CONGRESSINVESTS_API_KEY";

/// 24h cadence — OSINT monitor default per roadmap §3.
const INTERVAL_SECS: u64 = 24 * 3600;

/// Cap on signals emitted per sweep.
const TOP_N: usize = 50;

/// Senior US politicians whose trades warrant priority severity.
/// This is a **static watchlist** by role, not a per-name list — the
/// upstream's `seniority` + `committee_role` fields drive the
/// classifier without needing a hard-coded name match.
const SENIOR_ROLES: &[&str] = &[
    "Speaker of the House",
    "House Majority Leader",
    "House Minority Leader",
    "Senate Majority Leader",
    "Senate Minority Leader",
    "President pro tempore of the Senate",
    "Committee Chair",
    "Committee Ranking Member",
    "Committee Vice Chair",
];

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
            let Some(api_key) = api_key() else {
                tracing::warn!(
                    "congress_invests: no {ENV_KEY} — collector not registered; sp6 will report 'shelved-by-design' until signup at https://congressinvests.com"
                );
                return Ok(Vec::new());
            };
            let resp = ctx
                .http
                .get(BASE_URL)
                .bearer_auth(&api_key)
                .query(&[("days", "1")])
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

/// Read the API key from env at sweep time.
pub fn api_key() -> Option<String> {
    std::env::var(ENV_KEY).ok().filter(|v| !v.trim().is_empty())
}

/// Parse the trades JSON array and build Signals. Pure function.
fn parse_trades(j: &serde_json::Value) -> Vec<Signal> {
    let Some(arr) = j.get("trades").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for t in arr {
        let txn_id = t.get("txn_id").and_then(|v| v.as_str()).unwrap_or("");
        if txn_id.is_empty() {
            continue;
        }
        let member_name = t.get("member_name").and_then(|v| v.as_str()).unwrap_or("");
        let chamber = t.get("chamber").and_then(|v| v.as_str()).unwrap_or("");
        let state = t.get("state").and_then(|v| v.as_str()).unwrap_or("");
        let committee_role = t.get("committee_role").and_then(|v| v.as_str()).unwrap_or("");
        let ticker = t.get("ticker").and_then(|v| v.as_str()).unwrap_or("");
        let asset_class = t.get("asset_class").and_then(|v| v.as_str()).unwrap_or("");
        let txn_type = t.get("txn_type").and_then(|v| v.as_str()).unwrap_or("");
        let amount_low = t.get("amount_low").and_then(|v| v.as_i64()).unwrap_or(0);
        let amount_high = t.get("amount_high").and_then(|v| v.as_i64()).unwrap_or(0);
        let txn_date = t.get("txn_date").and_then(|v| v.as_str()).unwrap_or("");
        let disclosed_date = t.get("disclosed_date").and_then(|v| v.as_str()).unwrap_or("");
        let (kind, severity) = classify(committee_role, chamber, amount_high);
        let title = build_title(member_name, txn_type, ticker, amount_low, amount_high);
        out.push(
            Signal::new(
                kind,
                title,
                0.0,
                0.0,
                format!("congress_invests:{txn_id}"),
            )
            .severity(severity)
            .payload(serde_json::json!({
                "txn_id": txn_id,
                "member_name": member_name,
                "chamber": chamber,
                "state": state,
                "committee_role": committee_role,
                "ticker": ticker,
                "asset_class": asset_class,
                "txn_type": txn_type,
                "amount_low": amount_low,
                "amount_high": amount_high,
                "txn_date": txn_date,
                "disclosed_date": disclosed_date,
                "extreme_type": kind,
            })),
        );
    }
    out
}

fn classify(
    committee_role: &str,
    chamber: &str,
    amount_high: i64,
) -> (&'static str, &'static str) {
    if SENIOR_ROLES.iter().any(|r| r.eq_ignore_ascii_case(committee_role)) {
        return ("politician_trade_senior", "priority");
    }
    // Committee role hint: any committee assignment plus a notable
    // trade amount (>=$250k) → routine.
    if !committee_role.is_empty() && amount_high >= 250_000 {
        return ("politician_trade_committee_routine", "routine");
    }
    if chamber == "Senate" || chamber == "House" {
        return ("politician_trade_info", "info");
    }
    ("politician_trade_unknown", "info")
}

fn build_title(
    member_name: &str,
    txn_type: &str,
    ticker: &str,
    amount_low: i64,
    amount_high: i64,
) -> String {
    format!(
        "{member_name} {txn_type} {ticker} (${amount_low}..${amount_high})"
    )
    .chars()
    .take(200)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_speaker_trade() -> serde_json::Value {
        serde_json::json!({
            "trades": [{
                "txn_id": "TXN-2026-09-001",
                "member_name": "Nancy Pelosi",
                "chamber": "House",
                "state": "CA",
                "committee_role": "Speaker of the House",
                "ticker": "NVDA",
                "asset_class": "stock",
                "txn_type": "purchase",
                "amount_low": 1_000_000,
                "amount_high": 5_000_000,
                "txn_date": "2026-09-25",
                "disclosed_date": "2026-09-26"
            }]
        })
    }

    fn sample_committee_trade() -> serde_json::Value {
        serde_json::json!({
            "trades": [{
                "txn_id": "TXN-2026-09-002",
                "member_name": "Patrick McHenry",
                "chamber": "House",
                "state": "NC",
                "committee_role": "Committee Chair",
                "ticker": "JPM",
                "asset_class": "stock",
                "txn_type": "sale",
                "amount_low": 100_000,
                "amount_high": 500_000,
                "txn_date": "2026-09-20",
                "disclosed_date": "2026-09-26"
            }]
        })
    }

    fn sample_rank_and_file_trade() -> serde_json::Value {
        serde_json::json!({
            "trades": [{
                "txn_id": "TXN-2026-09-003",
                "member_name": "Adam Kinzinger",
                "chamber": "House",
                "state": "IL",
                "committee_role": "",
                "ticker": "AAPL",
                "asset_class": "stock",
                "txn_type": "purchase",
                "amount_low": 1_000,
                "amount_high": 15_000,
                "txn_date": "2026-09-15",
                "disclosed_date": "2026-09-26"
            }]
        })
    }

    /// Speaker of the House → priority
    #[test]
    fn speaker_trade_is_priority() {
        let out = parse_trades(&sample_speaker_trade());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, "priority");
        assert_eq!(out[0].kind, "politician_trade_senior");
        assert!(out[0].title.contains("Nancy Pelosi"));
        assert!(out[0].title.contains("NVDA"));
    }

    /// Committee Chair → priority (also in SENIOR_ROLES)
    #[test]
    fn committee_chair_is_priority() {
        let out = parse_trades(&sample_committee_trade());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, "priority");
        assert_eq!(out[0].kind, "politician_trade_senior");
    }

    /// Rank-and-file → info
    #[test]
    fn rank_and_file_is_info() {
        let out = parse_trades(&sample_rank_and_file_trade());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, "info");
        assert_eq!(out[0].kind, "politician_trade_info");
    }

    /// external_id = congress_invests:{txn_id}
    #[test]
    fn external_id_shape() {
        let out = parse_trades(&sample_speaker_trade());
        assert_eq!(out[0].external_id, "congress_invests:TXN-2026-09-001");
    }

    /// Records missing txn_id are skipped
    #[test]
    fn skips_no_txn_id() {
        let j = serde_json::json!({
            "trades": [{
                "member_name": "Anonymous",
                "chamber": "House",
                "txn_type": "purchase",
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

    /// Payload retains fields
    #[test]
    fn payload_carries_trade_fields() {
        let out = parse_trades(&sample_speaker_trade());
        let p = &out[0].payload;
        assert_eq!(p["member_name"], "Nancy Pelosi");
        assert_eq!(p["ticker"], "NVDA");
        assert_eq!(p["amount_high"], 5_000_000);
        assert_eq!(p["txn_type"], "purchase");
    }
}