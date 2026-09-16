//! DefiLlama — DeFi protocol TVL anomaly detector.
//!
//! Watches a default list of major DeFi protocols and emits a Signal when a
//! protocol's 24h TVL change crosses a volatility threshold (default 15%
//! absolute change). The signal kind is "financial" and each watch entry
//! carries an anchor at the protocol's known operating city so radar events
//! are visually separable from other financial sources (Etherscan → SG,
//! BLS → DC, etc.).
//!
//! - Free, no key, no rate limit (DefiLlama is community-funded open data).
//! - Each protocol's TVL history endpoint is `https://api.llama.fi/protocol/{slug}`
//!   which returns a JSON blob with `tvl.history: [[ts, value], ...]` and
//!   `currentChainTvls: {chain: value}` for the latest snapshot.
//! - Built-in watchlist: Aave, Uniswap, MakerDAO, Curve, Lido (top-5 by TVL).
//! - Override via `HUB_DEFILLAMA_WATCH=slug:lat:lon,slug:lat:lon` — anchor
//!   falls back to London (DefiLlama HQ) if lat/lon omitted.
//!
//! Anomaly rule: compare latest TVL value with the TVL value from 24h
//! (or the most recent prior sample if 24h boundary has no data). Signal
//! severity scales with absolute % delta: 15-30% = priority, > 30% = flash.
//! A protocol that loses >50% TVL in 24h is almost always an exploit, hack,
//! or rug pull; the signal will surface that to downstream entity resolution
//! in time for triage.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const API_BASE: &str = "https://api.llama.fi";
/// DefiLlama is operated by a team based in London, UK. Distinct from
/// existing financial anchors (Etherscan→Singapore, BLS→DC).
const LONDON: (f64, f64) = (51.5074, -0.1278);

/// Absolute 24h TVL change threshold (percent). Below this, no signal.
/// 15% catches rug pulls, hacks, and major liquidations; routine
/// market-volatility moves (1-5%) stay below the noise floor.
const DEFAULT_THRESHOLD_PCT: f64 = 15.0;

const DEFAULT_WATCH: &[(&str, &str, f64, f64)] = &[
    // (slug, label, lat, lon)
    ("aave", "aave", 51.5074, -0.1278),         // Aave Companies UK (London)
    ("uniswap", "uniswap", 40.7128, -74.0060),  // Uniswap Labs (NYC)
    ("makerdao", "makerdao", 47.3769, 8.5417),  // MakerDAO (Zürich, CH)
    ("curve-dex", "curve", 32.0853, 34.7818),  // Curve (Tel Aviv, IL)
    ("lido", "lido", 47.3769, 8.5417),          // Lido (Zürich, CH)
];

pub struct DefiLlama;

impl Source for DefiLlama {
    fn name(&self) -> &'static str {
        "defillama"
    }
    fn interval(&self) -> Duration {
        // 4h cadence. TVL is updated continuously by DefiLlama, but 4h is
        // frequent enough to catch exploits before they propagate.
        Duration::from_secs(4 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let watch: Vec<(String, String, f64, f64)> = if !ctx.config.monitor_defillama_watch.is_empty() {
                ctx.config
                    .monitor_defillama_watch
                    .iter()
                    .filter_map(|s| {
                        let parts: Vec<&str> = s.split(':').collect();
                        if parts.is_empty() {
                            return None;
                        }
                        let slug = parts[0].trim().to_string();
                        let label = parts.get(1).cloned().unwrap_or(&slug).trim().to_string();
                        let lat = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(LONDON.0);
                        let lon = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(LONDON.1);
                        if slug.is_empty() {
                            None
                        } else {
                            Some((slug, label, lat, lon))
                        }
                    })
                    .collect()
            } else {
                DEFAULT_WATCH
                    .iter()
                    .map(|(s, l, la, lo)| (s.to_string(), l.to_string(), *la, *lo))
                    .collect()
            };

            if watch.is_empty() {
                return Ok(Vec::new());
            }

            let threshold = ctx
                .config
                .monitor_defillama_threshold_pct
                .unwrap_or(DEFAULT_THRESHOLD_PCT);

            let mut out = Vec::new();
            for (slug, label, lat, lon) in watch {
                let url = format!("{API_BASE}/protocol/{slug}");
                let resp = match ctx.http.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(target: "monitor::defillama",
                            "skip {slug}: {e}");
                        continue;
                    }
                };
                if !resp.status().is_success() {
                    tracing::warn!(target: "monitor::defillama",
                        "skip {slug}: HTTP {}", resp.status());
                    continue;
                }
                let body: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(target: "monitor::defillama",
                            "skip {slug}: parse {e}");
                        continue;
                    }
                };

                let samples = match parse_tvl_history(&body) {
                    Some(s) if !s.is_empty() => s,
                    _ => continue,
                };

                // Compute 24h delta. If the most recent sample is older than
                // 25h we skip (stale data, don't false-alarm).
                let now = chrono::Utc::now().timestamp();
                let (latest_ts, latest_tvl) = *samples.last().unwrap();
                if now - latest_ts > 25 * 3600 {
                    tracing::debug!(target: "monitor::defillama",
                        "skip {slug}: stale data (latest = {}h ago)",
                        (now - latest_ts) / 3600);
                    continue;
                }

                // Find sample closest to 24h ago.
                let target_ts = now - 24 * 3600;
                let prior = samples
                    .iter()
                    .min_by_key(|(ts, _)| (ts - target_ts).abs())
                    .copied();
                let Some((prior_ts, prior_tvl)) = prior else { continue };
                if prior_tvl <= 0.0 || latest_tvl <= 0.0 {
                    continue;
                }
                if (latest_ts - prior_ts).abs() < 6 * 3600 {
                    // Same-day samples within 6h — too close to call a 24h delta.
                    continue;
                }

                let delta_pct = ((latest_tvl - prior_tvl) / prior_tvl) * 100.0;
                let abs_delta = delta_pct.abs();
                if abs_delta < threshold {
                    continue;
                }

                let direction = if delta_pct > 0.0 { "gained" } else { "lost" };
                let severity = if abs_delta >= 30.0 {
                    "flash"
                } else if abs_delta >= 20.0 {
                    "priority"
                } else {
                    "routine"
                };

                let title = format!(
                    "{label} {direction} {:.1}% TVL in 24h (${:.0}M → ${:.0}M)",
                    delta_pct,
                    prior_tvl / 1e6,
                    latest_tvl / 1e6,
                );
                let external_id = format!(
                    "defillama:{slug}:{}",
                    chrono::DateTime::<chrono::Utc>::from_timestamp(latest_ts, 0)
                        .map(|d| d.format("%Y-%m-%dT%H:%M").to_string())
                        .unwrap_or_else(|| latest_ts.to_string())
                );

                out.push(
                    Signal::new("financial", title, lat, lon, external_id)
                        .severity(severity)
                        .payload(serde_json::json!({
                            "slug": slug,
                            "label": label,
                            "delta_pct": delta_pct,
                            "tvl_usd_latest": latest_tvl,
                            "tvl_usd_24h_ago": prior_tvl,
                            "ts_latest": latest_ts,
                            "ts_24h_ago": prior_ts,
                            "anomaly_class": if abs_delta >= 30.0 {
                                "extreme"
                            } else if abs_delta >= 20.0 {
                                "major"
                            } else {
                                "notable"
                            },
                        })),
                );
            }
            if out.is_empty() {
                return Ok(Vec::new());
            }
            Ok(out)
        }
        .boxed()
    }
}

/// Extract `(ts_seconds, tvl_usd)` pairs from a DefiLlama protocol response.
/// Path: `body.tvl.history[]` where each element is `[ts, value]`.
/// Falls back to skipping if the schema doesn't match (e.g. chain TVL endpoints).
fn parse_tvl_history(body: &serde_json::Value) -> Option<Vec<(i64, f64)>> {
    let arr = body.get("tvl")?.get("history")?.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for row in arr {
        let pair = row.as_array()?;
        if pair.len() < 2 {
            continue;
        }
        let ts = pair[0].as_i64()?;
        let tvl = match &pair[1] {
            serde_json::Value::Number(n) => n.as_f64()?,
            serde_json::Value::String(s) => s.parse().ok()?,
            _ => return None,
        };
        out.push((ts, tvl));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tvl_history() {
        let body = serde_json::json!({
            "id": "aave",
            "name": "Aave",
            "tvl": {
                "history": [
                    [1735689600, 12_500_000_000.5],
                    [1735776000, 12_650_000_000.0],
                ]
            }
        });
        let samples = parse_tvl_history(&body).unwrap();
        assert_eq!(samples.len(), 2);
        assert!((samples[1].1 - 12_650_000_000.0).abs() < 1e-3);
    }

    #[test]
    fn rejects_missing_tvl() {
        let body = serde_json::json!({"id": "x", "name": "X"});
        assert!(parse_tvl_history(&body).is_none());
    }

    #[test]
    fn accepts_string_values() {
        // DefiLlama occasionally returns stringified floats.
        let body = serde_json::json!({
            "tvl": {"history": [[1735689600, "12500000000"]]}
        });
        let samples = parse_tvl_history(&body).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].1, 12_500_000_000.0);
    }

    #[test]
    fn rejects_short_pairs() {
        let body = serde_json::json!({"tvl": {"history": [[1735689600]]}});
        let samples = parse_tvl_history(&body).unwrap();
        assert!(samples.is_empty());
    }
}