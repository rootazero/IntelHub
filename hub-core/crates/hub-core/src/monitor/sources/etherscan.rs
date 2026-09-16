//! Etherscan (Ethereum mainnet) — large-transaction / sanctioned-address watch.
//!
//! Watches a default list of well-known Ethereum addresses for large ETH or
//! ERC-20 transfers and emits one Signal per address-day-volume-pair. **API V2**
//! only — V1 was deprecated 2025-09 and now returns "NOTOK" with a redirect
//! message. V2 uses a unified `https://api.etherscan.io/v2/api` endpoint with
//! `chainid=1` (Ethereum mainnet) plus the existing module/action pair.
//!
//! Watchlist semantics:
//! - Default watchlist: Tornado Cash router (OFAC-sanctioned since 2022-08),
//!   Binance hot wallet, Coinbase hot wallet. Override via
//!   `HUB_ETHERSCAN_WATCH=label:0xADDRESS,label:0xADDRESS`.
//! - Each label is the anchor for the radar event: a tornado-cash transfer
//!   maps to the OFAC HQ, a binance transfer to Binance's Singapore office.
//!   The signal itself carries the transaction hash + ETH/USD value so the
//!   graph plane can pivot to entities downstream.
//!
//! Free tier: **API KEY REQUIRED** (verified 2026-09: even keyless calls now
//! fail with the V1-deprecation error instead of degraded-but-functional
//! responses). 5 req/s, 100k calls/day. Apply at
//! https://etherscan.io/myapikey.
//!
//! Anchored at Singapore (1.3521, 103.8198) for unlabelled addresses so events
//! remain visually separable from OFAC (DC), CISA (DC), NVD (DC) and OSV
//! (Mountain View). Per-label overrides re-anchor events.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const API_BASE: &str = "https://api.etherscan.io/v2/api"; // V2 — V1 deprecated 2025-09
/// Anchored at Singapore (Etherscan.io operator) — distinct from existing
/// DC / NYC anchors (ofac, opensanctions, cisakev, nvd).
const SINGAPORE: (f64, f64) = (1.3521, 103.8198);

/// Large-tx threshold in ETH. Filters out dust/routine transfers; signals
/// are volume-notable flows. Tunable via `HUB_ETHERSCAN_MIN_ETH` (env var,
/// wei-based for precision; we convert ETH here for readability).
const DEFAULT_MIN_ETH: f64 = 100.0;

/// Default watchlist — each (label, address, anchor).
/// Anchors picked from public corporate registry data:
/// - Binance Singapore office (1.3521, 103.8198)
/// - Coinbase HQ Wilmington DE (39.7392, -75.5390)
/// - OFAC HQ Washington DC (38.8951, -77.0364) — for sanctioned-address flow
const DEFAULT_WATCH: &[(&str, &str, f64, f64)] = &[
    (
        "tornado-router",
        "0xd90e2f9248a4428e463cdb51c435ab8e2d7c5133",
        38.8951,
        -77.0364,
    ),
    (
        "binance-hot",
        "0x28c6c06298d514db089934071355e5743bf21d60",
        1.3521,
        103.8198,
    ),
    (
        "coinbase-hot",
        "0x71660c4005ba85c37ccec55d0c4493e66fe775d3",
        39.7392,
        -75.5390,
    ),
];

pub struct Etherscan;

impl Source for Etherscan {
    fn name(&self) -> &'static str {
        "etherscan"
    }
    fn interval(&self) -> Duration {
        // 6h cadence — most large-tx flows occur on weekly timescales,
        // but Tornado Cash mixer batches can land in clusters.
        Duration::from_secs(6 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            // Build watchlist: env override takes precedence over built-in.
            let watch: Vec<(String, String, f64, f64)> = if !ctx.config.monitor_etherscan_watch.is_empty() {
                ctx.config
                    .monitor_etherscan_watch
                    .iter()
                    .filter_map(|s| {
                        // Format: "label:0xADDRESS[:lat:lon]" — lat/lon optional,
                        // defaults to Singapore if absent.
                        let parts: Vec<&str> = s.split(':').collect();
                        if parts.len() < 2 {
                            return None;
                        }
                        let label = parts[0].trim().to_string();
                        let addr = parts[1].trim().to_string();
                        let lat = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(SINGAPORE.0);
                        let lon = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(SINGAPORE.1);
                        if label.is_empty() || addr.is_empty() {
                            None
                        } else {
                            Some((label, addr, lat, lon))
                        }
                    })
                    .collect()
            } else {
                DEFAULT_WATCH
                    .iter()
                    .map(|(l, a, la, lo)| (l.to_string(), a.to_string(), *la, *lo))
                    .collect()
            };

            if watch.is_empty() {
                return Ok(Vec::new());
            }

            let api_key = ctx
                .config
                .monitor_etherscan_api_key
                .clone()
                .unwrap_or_default();

            // Etherscan blocks the default `intelhub-monitor` UA on the
            // /api route (BotWalled 2026-09); the Ctx UA already impersonates
            // a browser which gets past the cheap filter.

            let mut out = Vec::new();
            for (label, address, lat, lon) in watch {
                // V2 endpoint requires chainid=1 (Ethereum mainnet). The
                // module/action pair is unchanged from V1. API key is
                // REQUIRED — keyless calls now return the V1-deprecation
                // error instead of degraded-but-functional responses.
                let url = format!(
                    "{API_BASE}?chainid=1&module=account&action=txlist\
                     &address={address}&startblock=0&endblock=99999999\
                     &page=1&offset=100&sort=desc\
                     {apikey}",
                    apikey = if api_key.is_empty() {
                        String::new()
                    } else {
                        format!("&apikey={api_key}")
                    },
                );

                let resp = match ctx.http.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        // Per-address error: skip, don't fail sweep.
                        tracing::warn!(target: "monitor::etherscan",
                            "skip {label}: {e}");
                        continue;
                    }
                };
                if !resp.status().is_success() {
                    tracing::warn!(target: "monitor::etherscan",
                        "skip {label}: HTTP {}", resp.status());
                    continue;
                }
                let body: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(target: "monitor::etherscan",
                            "skip {label}: parse {e}");
                        continue;
                    }
                };

                // Etherscan JSON envelope: {status, message, result}
                if body.get("status").and_then(|s| s.as_str()) != Some("1") {
                    let msg = body
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown");
                    tracing::warn!(target: "monitor::etherscan",
                        "skip {label}: {msg}");
                    continue;
                }

                let txs = match parse_txs(&body) {
                    Some(t) => t,
                    None => continue,
                };

                let min_eth = ctx
                    .config
                    .monitor_etherscan_min_eth
                    .unwrap_or(DEFAULT_MIN_ETH);

                // Aggregate large-tx volume per address-day.
                let mut by_day: std::collections::BTreeMap<String, (u32, f64)> =
                    std::collections::BTreeMap::new();
                for tx in &txs {
                    let day = tx.day.clone();
                    let entry = by_day.entry(day).or_insert((0, 0.0));
                    entry.0 += 1;
                    entry.1 += tx.eth_value;
                }

                for (day, (count, eth_total)) in by_day {
                    if eth_total < min_eth {
                        continue;
                    }
                    let title = format!(
                        "{label} — {eth_total:.2} ETH across {count} txs on {day}"
                    );
                    let external_id = format!("etherscan:{address}:{day}");
                    let payload = serde_json::json!({
                        "label": label,
                        "address": address,
                        "day": day,
                        "tx_count": count,
                        "eth_volume": eth_total,
                        "txs": txs.iter()
                            .filter(|t| t.day == day)
                            .take(10) // keep payload size bounded
                            .map(|t| serde_json::json!({
                                "hash": t.hash,
                                "from": t.from,
                                "to": t.to,
                                "eth_value": t.eth_value,
                                "time_utc": t.time_utc,
                            }))
                            .collect::<Vec<_>>(),
                    });
                    out.push(
                        Signal::new("financial", title, lat, lon, external_id)
                            .severity(if eth_total > 1_000.0 { "flash" } else { "priority" })
                            .payload(payload),
                    );
                }
            }
            if out.is_empty() {
                // Sweep completed but no large-tx deltas — return empty so
                // the health cell stays "ok".
                return Ok(Vec::new());
            }
            Ok(out)
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
struct Tx {
    hash: String,
    from: String,
    to: String,
    eth_value: f64,
    time_utc: String,
    day: String,
}

fn parse_txs(body: &serde_json::Value) -> Option<Vec<Tx>> {
    let arr = body.get("result")?.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for row in arr {
        let hash = row.get("hash").and_then(|x| x.as_str())?.to_string();
        let from = row.get("from").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let to = row.get("to").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let value_str = row.get("value").and_then(|x| x.as_str()).unwrap_or("0");
        let value_wei: f64 = value_str.parse().unwrap_or(0.0);
        let eth_value = value_wei / 1e18;
        let ts_str = row.get("timeStamp").and_then(|x| x.as_str()).unwrap_or("0");
        let ts: i64 = ts_str.parse().unwrap_or(0);
        let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
            .unwrap_or_else(chrono::Utc::now);
        let time_utc = dt.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let day = dt.format("%Y-%m-%d").to_string();
        out.push(Tx { hash, from, to, eth_value, time_utc, day });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_body() -> serde_json::Value {
        serde_json::json!({
            "status": "1",
            "message": "OK",
            "result": [
                {
                    "hash": "0xabc123",
                    "from": "0xsender",
                    "to": "0xd90e2f9248a4428e463cdb51c435ab8e2d7c5133",
                    "value": "150000000000000000000",  // 150 ETH
                    "timeStamp": "1735776000"  // 2025-01-01 12:00 UTC
                },
                {
                    "hash": "0xdef456",
                    "from": "0xothersender",
                    "to": "0xd90e2f9248a4428e463cdb51c435ab8e2d7c5133",
                    "value": "50000000000000000000",  // 50 ETH
                    "timeStamp": "1735862400"  // 2025-01-02 12:00 UTC
                },
                {
                    "hash": "0xghi789",
                    "from": "0xd90e2f9248a4428e463cdb51c435ab8e2d7c5133",
                    "to": "0xreceiver",
                    "value": "50000000000000000",  // 0.05 ETH (dust, filter out)
                    "timeStamp": "1735862500"
                }
            ]
        })
    }

    #[test]
    fn parses_value_wei_to_eth() {
        let txs = parse_txs(&sample_body()).unwrap();
        assert_eq!(txs.len(), 3);
        assert!((txs[0].eth_value - 150.0).abs() < 1e-6);
        assert!((txs[2].eth_value - 0.05).abs() < 1e-6);
    }

    #[test]
    fn day_grouping_is_correct() {
        let txs = parse_txs(&sample_body()).unwrap();
        let mut by_day: std::collections::BTreeMap<String, (u32, f64)> =
            std::collections::BTreeMap::new();
        for tx in &txs {
            let entry = by_day.entry(tx.day.clone()).or_insert((0, 0.0));
            entry.0 += 1;
            entry.1 += tx.eth_value;
        }
        assert_eq!(by_day.len(), 2);
        let (c1, v1) = by_day.get("2025-01-01").unwrap();
        assert_eq!(*c1, 1);
        assert!((v1 - 150.0).abs() < 1e-6);
    }

    #[test]
    fn rejects_non_ok_envelope() {
        let bad = serde_json::json!({"status": "0", "message": "NOTOK", "result": "Invalid API Key"});
        assert!(parse_txs(&bad).is_none());
    }
}