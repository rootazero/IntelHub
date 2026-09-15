//! Climate indicator series (SP8-D climate plane).
//!
//! Global warming is a slow trend — its honest medium is the series board,
//! not map dots. Two official keyless feeds, live-probed 2026-09-11:
//!   - NOAA GML Mauna Loa monthly mean CO2 (gml.noaa.gov/webdata txt)
//!   - NASA GISTEMP v4 global land-ocean monthly anomaly (data.giss.nasa.gov csv)
//! NSIDC sea-ice extent was cut: F5 bot-walls both noaadata.apps.nsidc.org
//! and sidads.colorado.edu from our datacenter egress (JS cookie challenge).
//! EONET seaLakeIce events carry the ice story on the map instead.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;

use crate::error::Result;
use crate::series::{normalize, Source};
use crate::state::AppState;

use super::super::signals::Observation;
use super::super::{Ctx, SeriesCollector};

const CO2_URL: &str = "https://gml.noaa.gov/webdata/ccgg/trends/co2/co2_mm_mlo.txt";
const GISTEMP_URL: &str = "https://data.giss.nasa.gov/gistemp/tabledata_v4/GLB.Ts+dSST.csv";
const KEEP: usize = 24; // two years of monthly points

pub struct ClimateSeries;

impl SeriesCollector for ClimateSeries {
    fn name(&self) -> &'static str {
        "climate"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(21600) // monthly data — 6h is generous
    }
    fn collect<'a>(&'a self, state: &'a AppState, ctx: &'a Ctx) -> BoxFuture<'a, Result<(usize, usize)>> {
        Box::pin(async move {
            let mut all: Vec<Observation> = Vec::new();
            let mut fetched = 0usize;
            for (url, parser) in [
                (CO2_URL, parse_co2 as fn(&str) -> Vec<Observation>),
                (GISTEMP_URL, parse_gistemp),
            ] {
                match ctx.http.get(url).send().await {
                    Ok(r) if r.status().is_success() => {
                        fetched += 1;
                        all.extend(parser(&r.text().await.unwrap_or_default()));
                    }
                    Ok(r) => tracing::warn!(status = %r.status(), url, "climate http"),
                    Err(e) => tracing::warn!(error = %e, url, "climate fetch"),
                }
            }
            let new = super::super::signals::persist_observations(state, "monitor:climate", all).await?;
            Ok((fetched, new))
        })
    }
}

/// NOAA monthly-mean txt: `#` comments, then
/// `year month decimal_date average deseasonalized ndays sdev unc`.
/// Missing rows carry average = -99.99.
pub fn parse_co2(body: &str) -> Vec<Observation> {
    // Storage key from catalog (single source of truth).
    let series_key = match normalize(Source::Noaa, "CO2_MLO") {
        Some(n) => n,
        None => return Vec::new(), // catalog mismatch — fail open (parser will return empty)
    };
    let mut out: Vec<Observation> = body
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 4 {
                return None;
            }
            let (y, m): (i32, u32) = (f[0].parse().ok()?, f[1].parse().ok()?);
            let ppm: f64 = f[3].parse().ok()?;
            if ppm < 0.0 {
                return None;
            }
            let d = NaiveDate::from_ymd_opt(y, m, 1)?.and_hms_opt(0, 0, 0)?;
            Some(Observation::new(
                series_key,
                DateTime::from_naive_utc_and_offset(d, Utc),
                ppm,
            ))
        })
        .collect();
    out.split_off(out.len().saturating_sub(KEEP))
}

/// GISTEMP CSV: title line, header `Year,Jan,...,Dec,...`, monthly cols
/// 1..=12 as °C anomaly vs 1951-1980; missing months are `***`.
pub fn parse_gistemp(body: &str) -> Vec<Observation> {
    let series_key = match normalize(Source::Nasa, "GISTEMP") {
        Some(n) => n,
        None => return Vec::new(),
    };
    let mut out: Vec<Observation> = body
        .lines()
        .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .flat_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            let year: i32 = f.first().and_then(|y| y.trim().parse().ok()).unwrap_or(0);
            (1..=12usize).filter_map(move |m| {
                let v: f64 = f.get(m)?.trim().parse().ok()?; // "***" → skip
                let d = NaiveDate::from_ymd_opt(year, m as u32, 1)?.and_hms_opt(0, 0, 0)?;
                Some(Observation::new(
                    series_key,
                    DateTime::from_naive_utc_and_offset(d, Utc),
                    v,
                ))
            })
        })
        .collect();
    out.split_off(out.len().saturating_sub(KEEP))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn co2_skips_comments_and_missing() {
        let body = "# header\n 2025    6   2025.4583      430.50      429.00     20    0.5    0.2\n 2025    7   2025.5417      -99.99      428.78     21    0.61    0.26\n";
        let obs = parse_co2(body);
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].series, "noaa:CO2_MLO_PPM");
        assert!((obs[0].value - 430.50).abs() < 1e-9);
    }

    #[test]
    fn gistemp_monthly_only_skips_stars() {
        let body = "Land-Ocean: Global Means\nYear,Jan,Feb,Mar,Apr,May,Jun,Jul,Aug,Sep,Oct,Nov,Dec,J-D,D-N,DJF,MAM,JJA,SON\n2026,1.09,1.25,1.32,1.17,1.13,1.18,1.25,1.40,***,***,***,***,***,***,1.13,1.21,1.28,***\n";
        let obs = parse_gistemp(body);
        assert_eq!(obs.len(), 8, "Aug is the last real month");
        assert_eq!(obs[0].series, "nasa:GISTEMP_ANOM_C");
        assert!((obs[7].value - 1.40).abs() < 1e-9);
    }
}
