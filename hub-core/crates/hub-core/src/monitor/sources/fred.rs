//! FRED macro series (spec §3). One call per series per sweep, rolling window.
//! CPI is stored as computed YoY% under a self-describing name — atlas lesson:
//! never store an index level under a name readers will read as a rate.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;

use crate::error::Result;
use crate::series::{normalize, Source};
use crate::state::AppState;

use super::super::signals::Observation;
use super::super::{Ctx, SeriesCollector};

/// Daily series. Storage key is derived from the catalog via
/// `series::normalize(Source::Fred, upstream_id)` — the table below is the
/// list of upstream FRED series IDs to fetch, NOT the storage names.
const SERIES: &[(&str, usize)] = &[
    ("VIXCLS", 5),
    ("DGS10", 5),
    ("DGS2", 5),
    ("T10Y2Y", 5),
    ("FEDFUNDS", 3),
    ("UNRATE", 3),
    ("BAMLH0A0HYM2", 5),
];

/// Slow series (weekly/monthly) need the 500d lookback, not the 45d daily
/// window — BLS-via-FRED mirrors (bls.gov Akamai-bans every datacenter
/// exit we have, so the native API is unreachable; same data, same source).
///
/// CPIAUCSL and AWHE are sent through `yoy()` to compute year-over-year
/// percent change before storage. The catalog still maps them to the
/// canonical storage names `fred:CPIAUCSL_YOY` and `fred:AWHE_YOY_PCT`.
const SLOW_SERIES: &[(&str, SeriesKind, usize)] = &[
    ("PAYEMS", SeriesKind::Level, 3),     // nonfarm payrolls, thousands
    ("ICSA", SeriesKind::Level, 12),       // initial jobless claims, weekly
    ("CPIAUCSL", SeriesKind::Yoy, 3),      // CPI index → YoY%
    ("AWHE", SeriesKind::Yoy, 3),          // avg hourly earnings → YoY%
];

enum SeriesKind {
    /// Store raw observation values (parse_observations).
    Level,
    /// Compute YoY% before storage (yoy). Catalog maps to `*_YOY[_PCT]` storage key.
    Yoy,
}

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
            for (fid, keep) in SERIES {
                let url = format!(
                    "https://api.stlouisfed.org/fred/series/observations?series_id={fid}\
                     &api_key={key}&file_type=json&observation_start={daily_start}"
                );
                match ctx.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let body = r.text().await.unwrap_or_default();
                        // Storage key from catalog — single source of truth.
                        let hub_name = match normalize(Source::Fred, fid) {
                            Some(n) => n,
                            None => {
                                tracing::error!(series = fid, "fred: upstream_id not in catalog; add to series.rs");
                                continue;
                            }
                        };
                        all.extend(parse_observations(&body, hub_name, *keep));
                    }
                    Ok(r) => tracing::warn!(series = fid, status = %r.status(), "fred http"),
                    Err(e) => tracing::warn!(series = fid, error = %e, "fred fetch"),
                }
            }
            // Slow BLS-mirror series (500d window): payrolls/claims as levels,
            // CPI/wages as YoY% (same lesson — index levels mislead).
            let cpi_start = (Utc::now() - chrono::Duration::days(500)).format("%Y-%m-%d");
            for (fid, kind, keep) in SLOW_SERIES {
                let url = format!(
                    "https://api.stlouisfed.org/fred/series/observations?series_id={fid}\
                     &api_key={key}&file_type=json&observation_start={cpi_start}"
                );
                match ctx.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        let body = r.text().await.unwrap_or_default();
                        let hub_name = match normalize(Source::Fred, fid) {
                            Some(n) => n,
                            None => {
                                tracing::error!(series = fid, "fred: upstream_id not in catalog; add to series.rs");
                                continue;
                            }
                        };
                        match kind {
                            SeriesKind::Level => all.extend(parse_observations(&body, hub_name, *keep)),
                            SeriesKind::Yoy => all.extend(yoy(&body, hub_name, *keep)),
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
