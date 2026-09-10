//! Financial event intel (spec §3): Finnhub news / insider transactions /
//! market-wide earnings calendar + StockTwits retail sentiment. Events land
//! as EVIDENCE (searchable, embeddable, graph-feeding — the hub's native
//! substrate); sentiment ratios also land as a `sentiment:*` time series.
//! Everything fail-soft: one dead upstream never sinks the sweep (atlas rule:
//! record honestly, never fabricate).

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;

use crate::error::Result;
use crate::state::AppState;

use super::super::signals::Observation;
use super::super::{Ctx, SeriesCollector};

pub struct FinIntel;

struct Watched {
    symbol: String,
    asset_class: String,
}

impl SeriesCollector for FinIntel {
    fn name(&self) -> &'static str {
        "finintel"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(3600)
    }
    fn collect<'a>(&'a self, state: &'a AppState, ctx: &'a Ctx) -> BoxFuture<'a, Result<(usize, usize)>> {
        Box::pin(async move {
            let watched: Vec<Watched> = sqlx::query_as::<_, (String, String)>(
                "SELECT symbol, asset_class FROM monitor_watchlist WHERE enabled ORDER BY symbol",
            )
            .fetch_all(&state.pg)
            .await?
            .into_iter()
            .map(|(symbol, asset_class)| Watched { symbol, asset_class })
            .collect();
            let mut fetched = 0usize;
            let mut new = 0usize;

            // ── Finnhub (keyed): news + insiders + earnings calendar ──
            if let Some(token) = ctx.config.finnhub_api_key.clone() {
                let (f, n) = earnings_calendar(state, ctx, &token, &watched).await?;
                fetched += f;
                new += n;
                for w in &watched {
                    if w.asset_class != "us_stock" {
                        continue; // free tier: company news/insiders are US-equity only
                    }
                    let (f1, n1) = company_news(state, ctx, &token, &w.symbol).await?;
                    let (f2, n2) = insider_transactions(state, ctx, &token, &w.symbol).await?;
                    fetched += f1 + f2;
                    new += n1 + n2;
                }
            } else {
                tracing::warn!("finintel: no FINNHUB_API_KEY — news/insider/earnings degraded");
            }

            // ── StockTwits (keyless): sentiment series + regime evidence ──
            for w in &watched {
                let (f, n) = stocktwits_sentiment(state, ctx, &w.symbol).await?;
                fetched += f;
                new += n;
            }

            Ok((fetched, new))
        })
    }
}

/// Market-wide earnings calendar, ONE call, filtered locally to the watchlist
/// (atlas pattern — never per-symbol calendar calls).
async fn earnings_calendar(
    state: &AppState,
    ctx: &Ctx,
    token: &str,
    watched: &[Watched],
) -> Result<(usize, usize)> {
    let from = Utc::now().format("%Y-%m-%d");
    let to = (Utc::now() + chrono::Duration::days(30)).format("%Y-%m-%d");
    let url = format!("https://finnhub.io/api/v1/calendar/earnings?from={from}&to={to}&token={token}");
    let body = match ctx.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
        Ok(r) => {
            tracing::warn!(status = %r.status(), "finnhub earnings http");
            return Ok((0, 0));
        }
        Err(e) => {
            tracing::warn!(error = %e, "finnhub earnings fetch");
            return Ok((0, 0));
        }
    };
    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let list = v
        .get("earningsCalendar")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let mut fetched = 0;
    let mut new = 0;
    for e in list {
        let sym = e.get("symbol").and_then(|s| s.as_str()).unwrap_or("");
        if !watched.iter().any(|w| w.symbol == sym) {
            continue;
        }
        fetched += 1;
        let date = e.get("date").and_then(|s| s.as_str()).unwrap_or("");
        let eps = e.get("epsEstimate").and_then(|x| x.as_f64());
        let content = format!(
            "Earnings scheduled: {sym} reports on {date} (hour: {}). EPS estimate: {}. Revenue estimate: {}.",
            e.get("hour").and_then(|s| s.as_str()).unwrap_or("n/a"),
            eps.map(|x| format!("{x:.2}")).unwrap_or_else(|| "n/a".into()),
            e.get("revenueEstimate").and_then(|x| x.as_f64()).map(|x| format!("{x:.0}")).unwrap_or_else(|| "n/a".into()),
        );
        // URL unique per symbol+date → content_hash stable → idempotent re-sweeps.
        let item_url = format!("https://finnhub.io/earnings/{sym}/{date}");
        let out = crate::ingest::ingest_content(
            state, "finnhub", &item_url, Some(&format!("{sym} earnings {date}")),
            &content, None,
            serde_json::json!({"kind": "earnings_calendar", "symbol": sym, "date": date}),
            serde_json::json!({"collector": "monitor:finintel"}),
            None, "auto",
        )
        .await?;
        if !out.duplicate {
            new += 1;
        }
    }
    Ok((fetched, new))
}

async fn company_news(
    state: &AppState,
    ctx: &Ctx,
    token: &str,
    symbol: &str,
) -> Result<(usize, usize)> {
    let from = (Utc::now() - chrono::Duration::days(7)).format("%Y-%m-%d");
    let to = Utc::now().format("%Y-%m-%d");
    let url = format!(
        "https://finnhub.io/api/v1/company-news?symbol={symbol}&from={from}&to={to}&token={token}"
    );
    let body = match ctx.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
        _ => return Ok((0, 0)),
    };
    let items = parse_news(&body, symbol, 5);
    let mut new = 0;
    let fetched = items.len();
    for it in items {
        let out = crate::ingest::ingest_content(
            state, "finnhub", &it.url, Some(&it.headline), &it.content,
            it.published,
            serde_json::json!({"kind": "company_news", "symbol": symbol, "source_name": it.source_name}),
            serde_json::json!({"collector": "monitor:finintel"}),
            None, "auto",
        )
        .await?;
        if !out.duplicate {
            new += 1;
        }
    }
    Ok((fetched, new))
}

pub struct NewsItem {
    pub url: String,
    pub headline: String,
    pub content: String,
    pub published: Option<DateTime<Utc>>,
    pub source_name: String,
}

pub fn parse_news(body: &str, symbol: &str, take: usize) -> Vec<NewsItem> {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut out: Vec<NewsItem> = Vec::new();
    let Some(arr) = v.as_array() else {
        return out;
    };
    for n in arr.iter().take(take) {
        let headline = match n.get("headline").and_then(|h| h.as_str()) {
            Some(h) if !h.is_empty() => h,
            _ => continue,
        };
        let Some(url) = n.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        let summary = n.get("summary").and_then(|s| s.as_str()).unwrap_or("");
        out.push(NewsItem {
            url: url.to_string(),
            headline: headline.chars().take(200).collect(),
            content: format!("{headline}\n\n{}", summary.chars().take(400).collect::<String>()),
            published: n
                .get("datetime")
                .and_then(|d| d.as_i64())
                .and_then(|ts| DateTime::from_timestamp(ts, 0)),
            source_name: n.get("source").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        });
    }
    let _ = symbol;
    out
}

async fn insider_transactions(
    state: &AppState,
    ctx: &Ctx,
    token: &str,
    symbol: &str,
) -> Result<(usize, usize)> {
    let url = format!("https://finnhub.io/api/v1/stock/insider-transactions?symbol={symbol}&token={token}");
    let body = match ctx.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
        _ => return Ok((0, 0)),
    };
    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let rows = v.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default();
    let mut fetched = 0;
    let mut new = 0;
    for t in rows.iter().take(10) {
        let change = t.get("change").and_then(|c| c.as_f64()).unwrap_or(0.0);
        if change == 0.0 {
            continue;
        }
        fetched += 1;
        let name = t.get("name").and_then(|s| s.as_str()).unwrap_or("unknown");
        let date = t.get("transactionDate").and_then(|s| s.as_str()).unwrap_or("");
        let dir = if change > 0.0 { "BUY" } else { "SELL" };
        let content = format!(
            "Insider transaction: {name} {dir} {} shares of {symbol} on {date} (change: {change:+.0}, held after: {}).",
            change.abs(),
            t.get("share").and_then(|s| s.as_f64()).map(|x| format!("{x:.0}")).unwrap_or_else(|| "n/a".into()),
        );
        let item_url = format!("https://finnhub.io/insider/{symbol}/{date}/{}", name.replace(' ', "_"));
        let out = crate::ingest::ingest_content(
            state, "finnhub", &item_url, Some(&format!("{symbol} insider {dir} {date}")),
            &content, None,
            serde_json::json!({"kind": "insider_transaction", "symbol": symbol, "direction": dir, "insider": name}),
            serde_json::json!({"collector": "monitor:finintel"}),
            None, "auto",
        )
        .await?;
        if !out.duplicate {
            new += 1;
        }
    }
    Ok((fetched, new))
}

/// StockTwits symbol stream → net sentiment ratio as `sentiment:SYM` series;
/// a dominant regime (|ratio| > 0.30) also emits one evidence note per day.
/// Keyless, ~200 req/hr unauthenticated (atlas-surveyed) — our cadence is
/// watchlist-size per hour, far below the ceiling.
async fn stocktwits_sentiment(state: &AppState, ctx: &Ctx, symbol: &str) -> Result<(usize, usize)> {
    let st_symbol = match symbol {
        "BTCUSD" => "BTC.X".to_string(),
        "ETHUSD" => "ETH.X".to_string(),
        s => s.to_string(),
    };
    let url = format!("https://api.stocktwits.com/api/2/streams/symbol/{st_symbol}.json");
    let body = match ctx.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
        _ => return Ok((0, 0)),
    };
    let Some((bull, bear)) = parse_sentiment(&body) else {
        return Ok((0, 0));
    };
    let ratio = (bull - bear) as f64 / (bull + bear).max(1) as f64;
    let regime = if ratio > 0.30 {
        "bullish_dominant"
    } else if ratio < -0.30 {
        "bearish_dominant"
    } else {
        "neutral"
    };
    let obs = Observation::new(format!("sentiment:{symbol}"), Utc::now(), (ratio * 1000.0).round() / 1000.0)
        .payload(serde_json::json!({
            "bull": bull, "bear": bear, "regime": regime, "source": "stocktwits",
        }));
    let new = super::super::signals::persist_observations(state, "monitor:finintel", vec![obs]).await?;
    let mut ev_new = 0;
    if regime != "neutral" {
        let day = Utc::now().format("%Y-%m-%d");
        let content = format!(
            "Retail sentiment {regime} on {symbol}: {bull} bullish vs {bear} bearish messages (net {ratio:+.2}) in the latest StockTwits stream window."
        );
        let item_url = format!("https://stocktwits.com/symbol/{st_symbol}/sentiment/{day}");
        let out = crate::ingest::ingest_content(
            state, "stocktwits", &item_url, Some(&format!("{symbol} retail sentiment {regime}")),
            &content, None,
            serde_json::json!({"kind": "retail_sentiment", "symbol": symbol, "regime": regime, "net_ratio": ratio}),
            serde_json::json!({"collector": "monitor:finintel"}),
            None, "auto",
        )
        .await?;
        if !out.duplicate {
            ev_new = 1;
        }
    }
    Ok((1, new + ev_new))
}

/// (bullish, bearish) counts over the latest message window (hard cap 30).
pub fn parse_sentiment(body: &str) -> Option<(i64, i64)> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let msgs = v.get("messages")?.as_array()?;
    let mut bull = 0;
    let mut bear = 0;
    for m in msgs.iter().take(30) {
        match m.pointer("/entities/sentiment/basic").and_then(|s| s.as_str()) {
            Some("Bullish") => bull += 1,
            Some("Bearish") => bear += 1,
            _ => {}
        }
    }
    Some((bull, bear))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn news_parse_caps_and_shapes() {
        let body = r#"[
            {"datetime":1757448000,"headline":"A beats","source":"Reuters","summary":"s","url":"https://x/1"},
            {"datetime":1757440000,"headline":"B","source":"BB","summary":"","url":"https://x/2"},
            {"headline":"no url"}
        ]"#;
        let items = parse_news(body, "AAPL", 5);
        assert_eq!(items.len(), 2); // missing url dropped
        assert_eq!(items[0].source_name, "Reuters");
        assert!(items[0].published.is_some());
    }

    #[test]
    fn sentiment_counts_and_ratio_inputs() {
        let body = r#"{"messages":[
            {"entities":{"sentiment":{"basic":"Bullish"}}},
            {"entities":{"sentiment":{"basic":"Bullish"}}},
            {"entities":{"sentiment":{"basic":"Bearish"}}},
            {"entities":{"sentiment":null}}
        ]}"#;
        let (bull, bear) = parse_sentiment(body).unwrap();
        assert_eq!((bull, bear), (2, 1));
    }
}
