//! FRED macro series (spec §3). One call per series per sweep, rolling window.
//! CPI is stored as computed YoY% under a self-describing name — atlas lesson:
//! never store an index level under a name readers will read as a rate.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;

use crate::error::Result;
use crate::state::AppState;

use super::super::signals::Observation;
use super::super::{Ctx, SeriesCollector};

/// (fred series id, hub series name, keep last N points per sweep)
const SERIES: &[(&str, &str, usize)] = &[
    ("VIXCLS", "fred:VIXCLS", 5),
    ("DGS10", "fred:DGS10_PCT", 5),
    ("DGS2", "fred:DGS2_PCT", 5),
    ("T10Y2Y", "fred:T10Y2Y", 5),
    ("FEDFUNDS", "fred:FEDFUNDS_PCT", 3),
    ("UNRATE", "fred:UNRATE_PCT", 3),
    ("BAMLH0A0HYM2", "fred:HY_OAS_PCT", 5),
];

/// Slow series (weekly/monthly) need the 500d lookback, not the 45d daily
/// window — BLS-via-FRED mirrors (bls.gov Akamai-bans every datacenter
/// exit we have, so the native API is unreachable; same data, same source).
const SLOW_SERIES: &[(&str, &str, usize)] = &[
    ("PAYEMS", "fred:PAYEMS_K", 3),     // nonfarm payrolls, thousands
    ("ICSA", "fred:ICSA", 12),          // initial jobless claims, weekly
    ("CES0500000003", "__AWHE__", 14),  // avg hourly earnings → YoY below
];

pub struct Fred;

impl SeriesCollector for Fred {
    fn name(&self) -> &'static str {
        "fred"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(3600)
    }
    fn collect<'a>(&'a self, state: &'a AppState, ctx: &'a Ctx) -> BoxFuture<'a, Result<(usize, usize)>> {
        Box::pin(async move {
            let Some(key) = ctx.config.fred_api_key.clone() else {
                tracing::warn!("fred: no FRED_API_KEY — collector degraded by design");
                return Ok((0, 0));
            };
            let mut all: Vec<Observation> = Vec::new();
            // Daily series: short window. Monthly CPI needs 13-month lookback.
            let daily_start = (Utc::now() - chrono::Duration::days(45)).format("%Y-%m-%d");
            for (fid, hub_name, keep) in SERIES {
                let url = format!(
                    "https://api.stlouisfed.org/fred/series/observations?series_id={fid}\
                     &api_key={key}&file_type=json&observation_start={daily_start}"
                );
                match ctx.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let body = r.text().await.unwrap_or_default();
                        all.extend(parse_observations(&body, hub_name, *keep));
                    }
                    Ok(r) => tracing::warn!(series = fid, status = %r.status(), "fred http"),
                    Err(e) => tracing::warn!(series = fid, error = %e, "fred fetch"),
                }
            }
            // CPI → YoY% (index levels are meaningless to readers).
            let cpi_start = (Utc::now() - chrono::Duration::days(500)).format("%Y-%m-%d");
            let url = format!(
                "https://api.stlouisfed.org/fred/series/observations?series_id=CPIAUCSL\
                 &api_key={key}&file_type=json&observation_start={cpi_start}"
            );
            if let Ok(r) = ctx.http.get(&url).send().await {
                if r.status().is_success() {
                    let body = r.text().await.unwrap_or_default();
                    all.extend(yoy(&body, "fred:CPIAUCSL_YOY", 3));
                }
            }
            // Slow BLS-mirror series (500d window): payrolls/claims as levels,
            // hourly earnings as YoY% (same lesson as CPI — levels mislead).
            for (fid, hub_name, keep) in SLOW_SERIES {
                let url = format!(
                    "https://api.stlouisfed.org/fred/series/observations?series_id={fid}\
                     &api_key={key}&file_type=json&observation_start={cpi_start}"
                );
                match ctx.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let body = r.text().await.unwrap_or_default();
                        if *hub_name == "__AWHE__" {
                            all.extend(yoy(&body, "fred:AWHE_YOY_PCT", 3));
                        } else {
                            all.extend(parse_observations(&body, hub_name, *keep));
                        }
                    }
                    Ok(r) => tracing::warn!(series = fid, status = %r.status(), "fred http"),
                    Err(e) => tracing::warn!(series = fid, error = %e, "fred fetch"),
                }
            }
            let fetched = all.len();
            let new = super::super::signals::persist_observations(state, "monitor:fred", all).await?;
            Ok((fetched, new))
        })
    }
}

fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc))
}

/// FRED observations JSON → newest `keep` points. "." means missing — skip.
pub fn parse_observations(body: &str, hub_series: &str, keep: usize) -> Vec<Observation> {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut pts: Vec<(DateTime<Utc>, f64)> = v
        .get("observations")
        .and_then(|o| o.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|o| {
                    let d = parse_date(o.get("date")?.as_str()?)?;
                    let val: f64 = o.get("value")?.as_str()?.parse().ok()?;
                    Some((d, val))
                })
                .collect()
        })
        .unwrap_or_default();
    pts.sort_by_key(|(d, _)| *d);
    pts.into_iter()
        .rev()
        .take(keep)
        .map(|(d, val)| Observation::new(hub_series, d, val))
        .collect()
}

/// CPI index levels → YoY% (needs i-13 for each point; monthly series).
/// Index-level monthly series → YoY% (`keep` newest points). Levels under
/// a rate-sounding name are how dashboards lie — never store them raw.
pub fn yoy(body: &str, hub_series: &str, keep: usize) -> Vec<Observation> {
    let raw = parse_observations(body, "tmp", usize::MAX);
    let mut pts: Vec<(DateTime<Utc>, f64)> = raw.into_iter().map(|o| (o.observed_at, o.value)).collect();
    pts.sort_by_key(|(d, _)| *d);
    let mut out = Vec::new();
    for i in 13..pts.len() {
        let (d, v) = pts[i];
        let base = pts[i - 13].1;
        if base > 0.0 {
            let yy = (v / base - 1.0) * 100.0;
            out.push(
                Observation::new(hub_series, d, (yy * 100.0).round() / 100.0)
                    .payload(serde_json::json!({"index_level": v, "base_level": base})),
            );
        }
    }
    out.into_iter().rev().take(keep).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{"observations":[
        {"date":"2026-09-04","value":"15.2"},
        {"date":"2026-09-05","value":"."},
        {"date":"2026-09-08","value":"16.81"}]}"#;

    #[test]
    fn skips_missing_and_keeps_newest() {
        let pts = parse_observations(BODY, "fred:VIXCLS", 5);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].value, 16.81); // newest first
        assert_eq!(pts[1].value, 15.2);
    }

    #[test]
    fn cpi_yoy_computes_percent() {
        // 14 monthly points, +2% over the year on the last point.
        let mut obs = String::from(r#"{"observations":["#);
        for i in 0..14 {
            let v = if i == 13 { 102.0 } else { 100.0 };
            obs.push_str(&format!(
                r#"{{"date":"2025-{m:02}-01","value":"{v}"}},"#,
                m = (i % 12) + 1
            ));
        }
        obs.push_str(r#"{"date":"2026-02-01","value":"102.0"}]}"#);
        let body = obs.replace("2025-13-01", "2026-01-01");
        let out = cpi_yoy(&body, 5);
        assert!(!out.is_empty());
        let last = out.iter().find(|o| o.value > 1.9 && o.value < 2.1);
        assert!(last.is_some(), "expected a ~2.0% YoY point, got {out:?}");
        assert!(out.iter().all(|o| o.series == "fred:CPIAUCSL_YOY"));
    }
}
