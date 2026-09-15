//! Watchlist market quotes (spec §3): FMP historical EOD as primary, Finnhub
//! real-time quote as gap-filler — the atlas fallback-chain pattern: each
//! provider only fills symbols the previous one missed, and every observation
//! carries its `source` for audit. Atlas traps handled: FMP premium-gated
//! symbols return HTTP 200 + a bare STRING (validate shape, never trust the
//! status code); Finnhub answers unknown symbols with c=0 (filter).

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;

use crate::error::Result;
use crate::series::{build_dynamic, Source, Unit};
use crate::state::AppState;

use super::super::signals::Observation;
use super::super::{Ctx, SeriesCollector};

const LOOKBACK_DAYS: i64 = 40;

pub struct Markets;

impl SeriesCollector for Markets {
    fn name(&self) -> &'static str {
        "markets"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(6 * 3600)
    }
    fn collect<'a>(&'a self, state: &'a AppState, ctx: &'a Ctx) -> BoxFuture<'a, Result<(usize, usize)>> {
        Box::pin(async move {
            let symbols: Vec<String> = sqlx::query_scalar(
                "SELECT symbol FROM monitor_watchlist WHERE enabled ORDER BY symbol",
            )
            .fetch_all(&state.pg)
            .await?;
            let symbols: Vec<String> = symbols.into_iter().filter(|s| supported_symbol(s)).collect();
            if symbols.is_empty() {
                return Ok((0, 0));
            }
            let mut all: Vec<Observation> = Vec::new();
            let mut missing: Vec<String> = Vec::new();

            // ── Tier 1: FMP historical EOD (full 40d backfill per symbol) ──
            if let Some(key) = ctx.config.fmp_api_key.clone() {
                let from = (Utc::now() - chrono::Duration::days(LOOKBACK_DAYS)).format("%Y-%m-%d");
                let to = Utc::now().format("%Y-%m-%d");
                for sym in &symbols {
                    let url = format!(
                        "https://financialmodelingprep.com/stable/historical-price-eod/full\
                         ?symbol={sym}&from={from}&to={to}&apikey={key}"
                    );
                    match ctx.http.get(&url).send().await {
                        Ok(r) if r.status().is_success() => {
                            let body = r.text().await.unwrap_or_default();
                            let bars = parse_fmp_bars(&body, sym);
                            if bars.is_empty() {
                                // Bare-string premium response or empty — gap for tier 2.
                                missing.push(sym.clone());
                            } else {
                                all.extend(bars);
                            }
                        }
                        Ok(r) => {
                            tracing::warn!(symbol = %sym, status = %r.status(), "fmp http");
                            missing.push(sym.clone());
                        }
                        Err(e) => {
                            tracing::warn!(symbol = %sym, error = %e, "fmp fetch");
                            missing.push(sym.clone());
                        }
                    }
                }
            } else {
                missing = symbols.clone();
            }

            // ── Tier 2: Finnhub real-time quote (fills gaps only, no history) ──
            if !missing.is_empty() {
                if let Some(token) = ctx.config.finnhub_api_key.clone() {
                    for sym in &missing {
                        let url = format!("https://finnhub.io/api/v1/quote?symbol={sym}&token={token}");
                        match ctx.http.get(&url).send().await {
                            Ok(r) if r.status().is_success() => {
                                let body = r.text().await.unwrap_or_default();
                                if let Some(o) = parse_finnhub_quote(&body, sym) {
                                    all.push(o);
                                }
                            }
                            Ok(r) => tracing::warn!(symbol = %sym, status = %r.status(), "finnhub http"),
                            Err(e) => tracing::warn!(symbol = %sym, error = %e, "finnhub fetch"),
                        }
                    }
                }
            }

            let fetched = all.len();
            let new = super::super::signals::persist_observations(state, "monitor:markets", all).await?;
            Ok((fetched, new))
        })
    }
}

/// Symbols this chain can serve (atlas mapping rules): US-style tickers and
/// FMP-style crypto pairs; A-share numerics and exchange-suffixed symbols are
/// out of tier scope (A-shares deferred per spec §9).
pub fn supported_symbol(s: &str) -> bool {
    if s.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    !s.ends_with(".HK")
        && !s.ends_with(".SS")
        && !s.ends_with(".SZ")
        && !s.ends_with(".T")
        && !s.ends_with(".L")
}

/// FMP bars → observations, oldest→newest re-sorted. CRITICAL: the response
/// must be a JSON array — a bare string means premium-gated, not data.
pub fn parse_fmp_bars(body: &str, symbol: &str) -> Vec<Observation> {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let Some(arr) = v.as_array() else {
        return vec![]; // premium-gated string or error object — NOT data
    };
    let mut out: Vec<Observation> = arr
        .iter()
        .filter_map(|b| {
            let d = NaiveDate::parse_from_str(b.get("date")?.as_str()?, "%Y-%m-%d").ok()?;
            let dt = DateTime::from_naive_utc_and_offset(d.and_hms_opt(0, 0, 0)?, Utc);
            let close = b.get("close")?.as_f64()?;
            Some(
                Observation::new(build_dynamic(Source::Quote, symbol, Unit::Symbol), dt, close).payload(serde_json::json!({
                    "open": b.get("open").and_then(|x| x.as_f64()),
                    "high": b.get("high").and_then(|x| x.as_f64()),
                    "low": b.get("low").and_then(|x| x.as_f64()),
                    "volume": b.get("volume").and_then(|x| x.as_f64()),
                    "source": "fmp",
                })),
            )
        })
        .collect();
    out.sort_by_key(|o| o.observed_at);
    out
}

/// Finnhub /quote → single real-time observation. c=0 means unknown symbol.
pub fn parse_finnhub_quote(body: &str, symbol: &str) -> Option<Observation> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let c = v.get("c")?.as_f64()?;
    if c <= 0.0 {
        return None;
    }
    let ts = v.get("t").and_then(|t| t.as_i64()).unwrap_or_else(|| Utc::now().timestamp());
    let dt = DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now);
    Some(
        Observation::new(build_dynamic(Source::Quote, symbol, Unit::Symbol), dt, c).payload(serde_json::json!({
            "open": v.get("o").and_then(|x| x.as_f64()),
            "high": v.get("h").and_then(|x| x.as_f64()),
            "low": v.get("l").and_then(|x| x.as_f64()),
            "change_pct": v.get("dp").and_then(|x| x.as_f64()),
            "history_depth": 1,       // honestly thin (atlas lesson: never fake depth)
            "source": "finnhub",
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmp_bars_parse_and_sort() {
        let body = r#"[
            {"date":"2026-09-09","open":231.0,"high":233.0,"low":229.5,"close":232.5,"volume":4.2e7},
            {"date":"2026-09-08","open":230.0,"high":231.0,"low":228.0,"close":230.1,"volume":3.9e7}
        ]"#;
        let bars = parse_fmp_bars(body, "AAPL");
        assert_eq!(bars.len(), 2);
        assert!(bars[0].observed_at < bars[1].observed_at); // re-sorted ascending
        assert!((bars[1].value - 232.5).abs() < 1e-9);
        assert_eq!(bars[1].payload["source"], "fmp");
    }

    #[test]
    fn fmp_premium_string_is_not_data() {
        // THE atlas trap: HTTP 200 with a bare string body.
        assert!(parse_fmp_bars("\"Premium Query Parameter: ...\"", "^TNX").is_empty());
        assert!(parse_fmp_bars("{\"Error Message\":\"x\"}", "AAPL").is_empty());
    }

    #[test]
    fn finnhub_zero_close_filtered() {
        assert!(parse_finnhub_quote(r#"{"c":0,"o":0,"h":0,"l":0,"dp":0}"#, "NOPE").is_none());
        let q = parse_finnhub_quote(r#"{"c":151.2,"o":150.0,"h":152.0,"l":149.5,"dp":0.8,"t":1757448000}"#, "AAPL").unwrap();
        assert!((q.value - 151.2).abs() < 1e-9);
        assert_eq!(q.payload["history_depth"], 1);
    }

    #[test]
    fn symbol_support_filter() {
        assert!(supported_symbol("AAPL"));
        assert!(supported_symbol("BTCUSD"));
        assert!(!supported_symbol("600519")); // A-share numeric
        assert!(!supported_symbol("0700.HK"));
        assert!(!supported_symbol("7203.T"));
    }
}
