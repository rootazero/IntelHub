//! EPA RadNet near-real-time gamma monitors — OFFICIAL US radiation
//! stations (complements citizen-science safecast). Hard-won lessons
//! (live-probed 2026-09): the Envirofacts RADNET_ANALYTICAL* tables are
//! RETIRED (404 on data.epa.gov, which itself replaced enviro.epa.gov) —
//! the living API is per-station year CSVs at radnet.epa.gov/cdx-radnet-rest.
//! Alerting model: station-relative anomaly, not absolute thresholds —
//! latest dose rate ≥ 2.5× the trailing-200 median is worth a point on the
//! map. A green board here is the expected steady state.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const CSV: &str = "https://radnet.epa.gov/cdx-radnet-rest/api/rest/csv";
/// (state, city slug, label, lat, lon)
const STATIONS: &[(&str, &str, &str, f64, f64)] = &[
    ("AL", "BIRMINGHAM", "Birmingham AL", 33.52, -86.80),
    ("NY", "NYC%20(EML)", "New York NY", 40.71, -74.01),
    ("CA", "LOS%20ANGELES", "Los Angeles CA", 34.05, -118.24),
    ("IL", "CHICAGO", "Chicago IL", 41.88, -87.63),
    ("WA", "SEATTLE", "Seattle WA", 47.61, -122.33),
    ("CO", "DENVER", "Denver CO", 39.74, -104.99),
    ("HI", "HONOLULU", "Honolulu HI", 21.31, -157.86),
    ("AK", "ANCHORAGE", "Anchorage AK", 61.22, -149.90),
    ("FL", "MIAMI", "Miami FL", 25.76, -80.19),
    ("TX", "EL%20PASO", "El Paso TX", 31.76, -106.49),
];
/// Latest/median dose ratio that flags an anomaly.
const ANOMALY_RATIO: f64 = 2.5;

pub struct Epa;

impl Source for Epa {
    fn name(&self) -> &'static str {
        "epa-radnet"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(6 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let year = chrono::Utc::now().format("%Y");
            let mut out = Vec::new();
            let mut ok = 0usize;
            for (state, slug, label, lat, lon) in STATIONS {
                let url = format!("{CSV}/{year}/fixed/{state}/{slug}");
                let resp = match ctx.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => r,
                    _ => continue, // station offline/renamed — skip, not fatal
                };
                ok += 1;
                let text = resp.text().await.unwrap_or_default();
                if let Some(sig) = analyze(&text, label, *lat, *lon) {
                    out.push(sig);
                }
            }
            if ok == 0 {
                return Err(HubError::sensor(
                    "EPA RadNet: all station CSVs unreachable (cdx-radnet-rest)",
                ));
            }
            Ok(out)
        }
        .boxed()
    }
}

/// Parse a station year-CSV; Some(signal) iff the latest dose rate is an
/// anomaly vs the trailing-200 median.
fn analyze(text: &str, label: &str, lat: f64, lon: f64) -> Option<Signal> {
    let mut doses: Vec<(String, f64)> = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 3 {
            continue;
        }
        if let Ok(d) = cols[2].trim().parse::<f64>() {
            if d > 0.0 {
                doses.push((cols[1].trim().to_string(), d));
            }
        }
    }
    if doses.len() < 50 {
        return None;
    }
    let (ts, latest) = doses.last()?.clone();
    let mut baseline: Vec<f64> = doses.iter().rev().skip(1).take(200).map(|(_, d)| *d).collect();
    baseline.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = baseline[baseline.len() / 2];
    if median <= 0.0 || latest < median * ANOMALY_RATIO {
        return None;
    }
    let ratio = latest / median;
    let occurred_at = chrono::NaiveDateTime::parse_from_str(&ts, "%m/%d/%Y %H:%M:%S")
        .map(|t| chrono::DateTime::from_naive_utc_and_offset(t, chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    let ext = {
        use sha2::Digest;
        let h = format!("{:x}", sha2::Sha256::digest(format!("radnet|{label}|{ts}").as_bytes()));
        format!("epa:{}", &h[..16])
    };
    Some(
        Signal::new(
            "radiation",
            format!("RadNet {label}: dose {latest:.0} nSv/h = {ratio:.1}× station median ({median:.0})"),
            lat,
            lon,
            ext,
        )
        .severity(if ratio >= 4.0 { "flash" } else { "priority" })
        .occurred(occurred_at)
        .payload(serde_json::json!({
            "station": label, "dose_nsv_h": latest, "median_nsv_h": median,
            "ratio": ratio, "sample_time": ts,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv(rows: &[(String, f64)]) -> String {
        let mut s = String::from("LOCATION_NAME,SAMPLE COLLECTION TIME,DOSE EQUIVALENT RATE (nSv/h)\n");
        for (t, d) in rows {
            s.push_str(&format!("AL: BIRMINGHAM,{t},{d}\n"));
        }
        s
    }

    #[test]
    fn quiet_board_when_stable() {
        let rows: Vec<_> = (0..300).map(|i| (format!("09/{:02}/2026 10:00:00", i % 28 + 1), 60.0 + (i % 5) as f64)).collect();
        assert!(analyze(&csv(&rows), "Test", 0.0, 0.0).is_none());
    }

    #[test]
    fn spike_flags() {
        let mut rows: Vec<_> = (0..300).map(|i| (format!("09/{:02}/2026 10:00:00", i % 28 + 1), 60.0)).collect();
        rows.push(("09/10/2026 21:44:00".into(), 400.0));
        let sig = analyze(&csv(&rows), "Birmingham AL", 33.5, -86.8).expect("spike must flag");
        assert_eq!(sig.kind, "radiation");
        assert!(sig.severity == "flash" || sig.severity == "priority");
        assert!(sig.title.contains("6.7×"));
    }
}
