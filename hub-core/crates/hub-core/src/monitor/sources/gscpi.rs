//! GSCPI — NY Fed Global Supply Chain Pressure Index (monthly, stddevs
//! from historical average; >1 = elevated pressure, <-1 = unusually loose).
//! Free CSV, no key. Ported from Crucix `sources/gscpi.mjs`: the CSV is
//! wide-format (one column per revision vintage) — take the LAST non-empty
//! column per row (latest vintage estimate). Dates: "31-Jan-2026".

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;

use crate::error::{HubError, Result};
use crate::state::AppState;

use super::super::signals::Observation;
use super::super::{Ctx, SeriesCollector};

const CSV_URL: &str =
    "https://www.newyorkfed.org/medialibrary/research/interactives/data/gscpi/gscpi_interactive_data.csv";
const SERIES_KEY: &str = "gscpi:index"; // legacy alias; see series::normalize(Source::Gscpi, "index")
/// How many trailing months to (idempotently) upsert per sweep.
const KEEP: usize = 24;

pub struct Gscpi;

impl SeriesCollector for Gscpi {
    fn name(&self) -> &'static str {
        "gscpi"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn collect<'a>(&'a self, state: &'a AppState, ctx: &'a Ctx) -> BoxFuture<'a, Result<(usize, usize)>> {
        Box::pin(async move {
            let resp = ctx.http.get(CSV_URL).send().await?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("GSCPI CSV HTTP {}", resp.status())));
            }
            let text = resp.text().await?;
            let obs = parse_csv(&text, KEEP);
            if obs.is_empty() {
                return Err(HubError::sensor("GSCPI CSV parsed to zero rows"));
            }
            let n = super::super::signals::persist_observations(state, "monitor:gscpi", obs).await?;
            Ok((1, n))
        })
    }
}

fn parse_csv(text: &str, keep: usize) -> Vec<Observation> {
    let mut rows: Vec<(NaiveDate, f64)> = Vec::new();
    for line in text.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() || line.starts_with(',') {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        let Some(date) = NaiveDate::parse_from_str(cols[0].trim(), "%d-%b-%Y").ok() else {
            continue;
        };
        // Last non-empty, non-#N/A column = latest vintage estimate.
        let value = cols
            .iter()
            .skip(1)
            .rev()
            .map(|c| c.trim())
            .find(|c| !c.is_empty() && *c != "#N/A")
            .and_then(|c| c.parse::<f64>().ok());
        if let Some(v) = value {
            rows.push((date, v));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows.into_iter()
        .rev()
        .take(keep)
        .map(|(d, v)| {
            Observation::new(
                SERIES_KEY,
                DateTime::from_naive_utc_and_offset(d.and_hms_opt(0, 0, 0).unwrap(), Utc),
                v,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_latest_vintage_column() {
        let csv = "Date,2025-01,2025-02,2025-03\n\
                   31-Dec-2024,0.10,0.11,\n\
                   31-Jan-2025,#N/A,-0.42,-0.40\n\
                   28-Feb-2025,0.05,0.07,0.09\n";
        let obs = parse_csv(csv, 12);
        assert_eq!(obs.len(), 3);
        assert_eq!(obs[0].series, SERIES_KEY);
        // Newest first after sort+rev+take.
        let feb = obs.iter().find(|o| o.observed_at.format("%Y-%m").to_string() == "2025-02").unwrap();
        assert!((feb.value - 0.09).abs() < 1e-9);
        // Jan: trailing empty col skipped, -0.40 is the latest vintage.
        let jan = obs.iter().find(|o| o.observed_at.format("%Y-%m").to_string() == "2025-01").unwrap();
        assert!((jan.value - (-0.40)).abs() < 1e-9);
        // Dec: only trailing-empty → latest vintage 0.11.
        let dec = obs.iter().find(|o| o.observed_at.format("%Y-%m").to_string() == "2024-12").unwrap();
        assert!((dec.value - 0.11).abs() < 1e-9);
    }
}
