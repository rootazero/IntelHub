//! BLS native API (v2) — US labor statistics straight from the source.
//!
//! CURRENTLY UNREACHABLE BY DESIGN (kept visible per user decision 2026-09,
//! same pattern as acled/reliefweb): bls.gov sits behind Akamai, which
//! geoblocks non-US IPs AND bans datacenter ASNs — every openclash US exit
//! tested (Seattle 01/05, San Jose Premium 01/02) gets HTTP 403, and the
//! registration page is equally unreachable, so no key can even be
//! obtained. Meanwhile the SAME headline series flow via FRED mirrors
//! (fred:PAYEMS_K / fred:ICSA / fred:AWHE_YOY_PCT / fred:UNRATE_PCT /
//! fred:CPIAUCSL_YOY). This collector retries daily and SELF-HEALS the day
//! a clean residential-US path + BLS_API_KEY exist.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;
use futures::FutureExt;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const URL: &str = "https://api.bls.gov/publicAPI/v2/timeseries/data/";
const BLS_DC: (f64, f64) = (38.8977, -77.0142); // BLS HQ, Washington DC

/// (bls series id, label)
const SERIES: &[(&str, &str)] = &[
    ("LNS14000000", "U-3 unemployment"),
    ("CES0000000001", "nonfarm payrolls"),
    ("CUSR0000SA0", "CPI-U"),
    ("CES0500000003", "avg hourly earnings"),
];

pub struct Bls;

impl Source for Bls {
    fn name(&self) -> &'static str {
        "bls"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(key) = ctx.config.bls_api_key.clone() else {
                return Err(HubError::internal(
                    "BLS_API_KEY not configured — registration page unreachable (Akamai geoblock \
                     + datacenter-IP ban; needs a residential US connection once). FRED mirrors \
                     carry the series meanwhile: fred:PAYEMS_K / fred:ICSA / fred:AWHE_YOY_PCT",
                ));
            };
            let year = Utc::now().format("%Y").to_string();
            let prev = (Utc::now() - chrono::Duration::days(370)).format("%Y").to_string();
            let body = serde_json::json!({
                "seriesid": SERIES.iter().map(|(id, _)| id).collect::<Vec<_>>(),
                "registrationkey": key,
                "startyear": prev,
                "endyear": year,
            });
            let resp = ctx.http.post(URL).json(&body).send().await?;
            match resp.status().as_u16() {
                403 => {
                    return Err(HubError::internal(
                        "BLS 403 — Akamai egress ban (all tested US proxy exits denied; needs a \
                         residential US path). FRED mirrors carry the series meanwhile — see \
                         fred:* in the Signals deck",
                    ))
                }
                s if !(200..300).contains(&s) => {
                    return Err(HubError::sensor(format!("BLS HTTP {s}")))
                }
                _ => {}
            }
            let j: serde_json::Value = resp.json().await?;
            // A successful reach = the network block is gone. Surface one
            // heartbeat event per sweep with the freshest observation dates;
            // series ingestion lands in a dedicated follow-up (this collector
            // was built while the API is unreachable — keep the blast
            // radius small until we can verify the payload live).
            let mut freshest = String::new();
            let mut series_ok = 0usize;
            for s in j
                .get("Results")
                .and_then(|r| r.get("series"))
                .and_then(|a| a.as_array())
                .cloned()
                .unwrap_or_default()
            {
                let latest = s
                    .get("data")
                    .and_then(|d| d.as_array())
                    .and_then(|a| a.first())
                    .and_then(|o| o.get("periodName").and_then(|p| p.as_str()).map(String::from))
                    .and_then(|pn| {
                        let y = s.get("data")?.as_array()?.first()?.get("year")?.as_str()?;
                        Some(format!("{pn} {y}"))
                    });
                if let Some(l) = latest {
                    series_ok += 1;
                    if l > freshest {
                        freshest = l;
                    }
                }
            }
            Ok(vec![Signal::new(
                "economic",
                format!("BLS native API reachable — {series_ok}/{} series fresh through {freshest}", SERIES.len()),
                BLS_DC.0,
                BLS_DC.1,
                format!("bls:heartbeat:{freshest}"),
            )
            .severity("routine")
            .occurred(Utc::now())
            .payload(serde_json::json!({
                "note": "network path unblocked; series ingestion pending live verification",
                "series": SERIES.iter().map(|(id, l)| format!("{id}={l}")).collect::<Vec<_>>(),
            }))])
        }
        .boxed()
    }
}

/// Parse BLS v2 series payload → (date, value) points, newest first.
/// Kept pure + unit-tested since the live endpoint is unreachable.
pub fn parse_series(j: &serde_json::Value) -> Vec<(String, Vec<(DateTime<Utc>, f64)>)> {
    let mut out = Vec::new();
    for s in j
        .get("Results")
        .and_then(|r| r.get("series"))
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let id = s.get("seriesID").and_then(|i| i.as_str()).unwrap_or("?").to_string();
        let mut pts: Vec<(DateTime<Utc>, f64)> = s
            .get("data")
            .and_then(|d| d.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|o| {
                        let period = o.get("period")?.as_str()?; // M01..M12 (M13 = annual avg, skip)
                        if period == "M13" {
                            return None;
                        }
                        let month: u32 = period.trim_start_matches('M').parse().ok()?;
                        let year: i32 = o.get("year")?.as_str()?.parse().ok()?;
                        let value: f64 = o.get("value")?.as_str()?.parse().ok()?;
                        let d = NaiveDate::from_ymd_opt(year, month, 1)?
                            .and_hms_opt(0, 0, 0)?;
                        Some((DateTime::from_naive_utc_and_offset(d, Utc), value))
                    })
                    .collect()
            })
            .unwrap_or_default();
        pts.sort_by_key(|(d, _)| *d);
        out.push((id, pts));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v2_payload_skipping_annual() {
        let j = serde_json::json!({ "Results": { "series": [ {
            "seriesID": "LNS14000000",
            "data": [
                { "year": "2026", "period": "M13", "periodName": "Annual", "value": "4.1" },
                { "year": "2026", "period": "M08", "periodName": "August", "value": "4.3" },
                { "year": "2026", "period": "M07", "periodName": "July", "value": "4.2" },
            ],
        } ] } });
        let rows = parse_series(&j);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.len(), 2); // M13 skipped
        assert_eq!(rows[0].1[1].0.format("%Y-%m").to_string(), "2026-08");
        assert!((rows[0].1[1].1 - 4.3).abs() < 1e-9);
    }
}
