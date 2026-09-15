//! US Treasury Fiscal Data (spec §3) — no API key required.
//! Total public debt + average interest rate on marketable debt.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;

use crate::error::Result;
use crate::series::{normalize, Source};
use crate::state::AppState;

use super::super::signals::Observation;
use super::super::{Ctx, SeriesCollector};

const BASE: &str = "https://api.fiscaldata.treasury.gov/services/api/fiscal_service/v2/accounting/od";

pub struct Treasury;

impl SeriesCollector for Treasury {
    fn name(&self) -> &'static str {
        "treasury"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(12 * 3600)
    }
    fn collect<'a>(&'a self, state: &'a AppState, ctx: &'a Ctx) -> BoxFuture<'a, Result<(usize, usize)>> {
        Box::pin(async move {
            let mut all = Vec::new();
            // Storage keys come from the catalog.
            let total_debt = match normalize(Source::Treasury, "TOTAL_DEBT") {
                Some(n) => n,
                None => {
                    tracing::error!("treasury: TOTAL_DEBT not in catalog; add to series.rs");
                    return Ok((0, 0));
                }
            };
            let avg_rate = match normalize(Source::Treasury, "AVG_RATE_MARKETABLE") {
                Some(n) => n,
                None => {
                    tracing::error!("treasury: AVG_RATE_MARKETABLE not in catalog; add to series.rs");
                    return Ok((0, 0));
                }
            };
            let debt_url = format!("{BASE}/debt_to_penny?sort=-record_date&page[size]=3");
            if let Ok(r) = ctx.http.get(&debt_url).send().await {
                if r.status().is_success() {
                    let body = r.text().await.unwrap_or_default();
                    all.extend(parse_rows(&body, "tot_pub_debt_out_amt", total_debt, None));
                }
            }
            let rate_url = format!("{BASE}/avg_interest_rates?sort=-record_date&page[size]=10");
            if let Ok(r) = ctx.http.get(&rate_url).send().await {
                if r.status().is_success() {
                    let body = r.text().await.unwrap_or_default();
                    all.extend(parse_rows(
                        &body,
                        "avg_interest_rate_amt",
                        avg_rate,
                        Some("Total Marketable"),
                    ));
                }
            }
            let fetched = all.len();
            let new =
                super::super::signals::persist_observations(state, "monitor:treasury", all).await?;
            Ok((fetched, new))
        })
    }
}

pub fn parse_rows(
    body: &str,
    field: &str,
    hub_series: &str,
    security_desc: Option<&str>,
) -> Vec<Observation> {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    v.get("data")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter(|row| match security_desc {
                    Some(want) => row.get("security_desc").and_then(|s| s.as_str()) == Some(want),
                    None => true,
                })
                .filter_map(|row| {
                    let d = NaiveDate::parse_from_str(row.get("record_date")?.as_str()?, "%Y-%m-%d").ok()?;
                    let dt = DateTime::from_naive_utc_and_offset(d.and_hms_opt(0, 0, 0)?, Utc);
                    // fiscaldata renders numbers as strings with commas.
                    let raw = row.get(field)?.as_str()?.replace(',', "");
                    let value: f64 = raw.parse().ok()?;
                    Some(Observation::new(hub_series, dt, value))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_debt_and_filters_security() {
        let debt = r#"{"data":[{"record_date":"2026-09-08","tot_pub_debt_out_amt":"37,400,000,000,000.00"}]}"#;
        let pts = parse_rows(debt, "tot_pub_debt_out_amt", "treasury:TOTAL_DEBT_USD", None);
        assert_eq!(pts.len(), 1);
        assert!((pts[0].value - 3.74e13).abs() < 1.0);

        let rates = r#"{"data":[
            {"record_date":"2026-08-31","avg_interest_rate_amt":"3.4","security_desc":"Bills"},
            {"record_date":"2026-08-31","avg_interest_rate_amt":"3.6","security_desc":"Total Marketable"}]}"#;
        let pts = parse_rows(rates, "avg_interest_rate_amt", "treasury:AVG_RATE_MARKETABLE_PCT", Some("Total Marketable"));
        assert_eq!(pts.len(), 1);
        assert!((pts[0].value - 3.6).abs() < 1e-9);
    }
}
