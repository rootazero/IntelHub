//! UN Comtrade — strategic commodity trade flows (annual, world aggregate).
//! Keyed full API (free registration; preview endpoint is key-less but
//! row-capped). Ported from Crucix `sources/comtrade.mjs` strategic list,
//! trimmed to six high-signal series. Annual data lags ~1y by nature —
//! we request the last five years and take whatever has been reported
//! (e.g. Russia simply stops answering after 2022; absence is honest).

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;

use crate::error::{HubError, Result};
use crate::state::AppState;

use super::super::signals::Observation;
use super::super::{Ctx, SeriesCollector};

const URL: &str = "https://comtradeapi.un.org/data/v1/get/C/A/HS";

/// (reporter, commodity, flow, hub series name)
const SERIES: &[(&str, &str, &str, &str)] = &[
    ("156", "8542", "X", "comtrade:CN.exp.semiconductors_usd"),
    // Taiwan reports as "Other Asia, nes" (490), not 158 (live-probed).
    ("490", "8542", "X", "comtrade:TW.exp.semiconductors_usd"),
    ("410", "8542", "X", "comtrade:KR.exp.semiconductors_usd"),
    ("842", "2709", "M", "comtrade:US.imp.crude_usd"),
    ("156", "7108", "M", "comtrade:CN.imp.gold_usd"),
    ("276", "93", "X", "comtrade:DE.exp.arms_usd"),
];

pub struct Comtrade;

impl SeriesCollector for Comtrade {
    fn name(&self) -> &'static str {
        "comtrade"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn collect<'a>(&'a self, state: &'a AppState, ctx: &'a Ctx) -> BoxFuture<'a, Result<(usize, usize)>> {
        Box::pin(async move {
            let Some(key) = ctx.config.comtrade_api_key.clone() else {
                tracing::warn!("comtrade: no COMTRADE_API_KEY — collector degraded by design");
                return Ok((0, 0));
            };
            let year = Utc::now().format("%Y").to_string().parse::<i64>().unwrap_or(2026);
            let period = (year - 5..year).map(|y| y.to_string()).collect::<Vec<_>>().join(",");
            let mut all: Vec<Observation> = Vec::new();
            let mut series_hit = 0usize;
            for (i, (reporter, cmd, flow, name)) in SERIES.iter().enumerate() {
                // Free tier ≈ 1 req/s — rapid fire gets 429'd (observed live).
                if i > 0 {
                    tokio::time::sleep(Duration::from_millis(1600)).await;
                }
                let url = format!(
                    "{URL}?reporterCode={reporter}&period={period}&cmdCode={cmd}\
                     &flowCode={flow}&partnerCode=0&subscription-key={key}"
                );                match ctx.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let j: serde_json::Value = r.json().await.unwrap_or(serde_json::json!({}));
                        let n_before = all.len();
                        all.extend(parse_rows(&j, name));
                        if all.len() > n_before {
                            series_hit += 1;
                        }
                    }
                    Ok(r) => tracing::warn!(series = name, status = %r.status(), "comtrade http"),
                    Err(e) => tracing::warn!(series = name, error = %e, "comtrade fetch"),
                }
            }
            if all.is_empty() {
                return Err(HubError::sensor(
                    "comtrade: all six series returned nothing (key invalid or upstream down)",
                ));
            }
            let new = super::super::signals::persist_observations(state, "monitor:comtrade", all).await?;
            Ok((series_hit, new))
        })
    }
}

fn parse_rows(j: &serde_json::Value, series: &str) -> Vec<Observation> {
    let mut out = Vec::new();
    for r in j.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default() {
        let Some(year) = r.get("refYear").and_then(|y| y.as_i64()) else { continue };
        let Some(value) = r.get("primaryValue").and_then(|v| v.as_f64()) else { continue };
        let Some(date) = NaiveDate::from_ymd_opt(year as i32, 1, 1) else { continue };
        let Some(t) = date.and_hms_opt(0, 0, 0) else { continue };
        out.push(Observation::new(
            series,
            DateTime::from_naive_utc_and_offset(t, Utc),
            value,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_parse() {
        let j = serde_json::json!({ "data": [
            { "refYear": 2023, "primaryValue": 1.72e11 },
            { "refYear": 2024, "primaryValue": 1.74e11 },
            { "refYear": 2025 },
        ]});
        let obs = parse_rows(&j, "comtrade:X");
        assert_eq!(obs.len(), 2); // missing value row skipped
        assert_eq!(obs[0].observed_at.format("%Y").to_string(), "2023");
        assert!((obs[1].value - 1.74e11).abs() < 1.0);
    }
}
