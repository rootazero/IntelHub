//! Radiation layer: EPA RadNet (US official stations) + Safecast readings
//! around 6 watched nuclear sites. Keyless. Ported from Crucix
//! `sources/epa.mjs` + `sources/safecast.mjs` (stations, analytes, thresholds
//! verbatim). Anomalies (avg CPM > 100, or analyte above elevated threshold)
//! are FLASH-tier — they become critical hub alerts.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

/// (label, state, lat, lon) — Crucix MONITORING_STATIONS verbatim.
const EPA_STATIONS: &[(&str, &str, f64, f64)] = &[
    ("Washington, DC", "DC", 38.9, -77.0),
    ("New York, NY", "NY", 40.7, -74.0),
    ("Los Angeles, CA", "CA", 34.1, -118.2),
    ("Chicago, IL", "IL", 41.9, -87.6),
    ("Seattle, WA", "WA", 47.6, -122.3),
    ("Denver, CO", "CO", 39.7, -105.0),
    ("Honolulu, HI", "HI", 21.3, -157.9),
    ("Anchorage, AK", "AK", 61.2, -149.9),
    ("Miami, FL", "FL", 25.8, -80.2),
    ("San Francisco, CA", "CA", 37.8, -122.4),
];

/// (analyte, elevated threshold) — Crucix THRESHOLDS (elevated) verbatim.
const THRESHOLDS: &[(&str, f64)] = &[
    ("GROSS BETA", 5.0),
    ("GROSS ALPHA", 0.15),
    ("IODINE-131", 0.1),
    ("CESIUM-137", 0.1),
    ("CESIUM-134", 0.01),
];

/// (label, lat, lon, radius_km) — Crucix SITES verbatim.
const NUKE_SITES: &[(&str, f64, f64, u32)] = &[
    ("Zaporizhzhia NPP (Ukraine)", 47.51, 34.58, 100),
    ("Chernobyl Exclusion Zone", 51.39, 30.10, 50),
    ("Bushehr NPP (Iran)", 28.83, 50.89, 100),
    ("Yongbyon (North Korea)", 39.80, 125.75, 100),
    ("Fukushima Daiichi", 37.42, 141.03, 50),
    ("Dimona (Israel)", 31.00, 35.15, 100),
];

const SAFECAST_ANOMALY_CPM: f64 = 100.0;

pub struct Radiation;

impl Source for Radiation {
    fn name(&self) -> &'static str {
        "radiation"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(1800)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let mut out = Vec::new();
            // EPA RadNet — per-station isolation (one failing station ≠ source failure)
            for (label, state, lat, lon) in EPA_STATIONS {
                match epa_station(ctx, label, state, *lat, *lon).await {
                    Ok(sig) => out.extend(sig),
                    Err(e) => tracing::warn!(station = label, error = %e, "radnet station failed"),
                }
            }
            // Safecast around nuclear sites
            for (label, lat, lon, radius_km) in NUKE_SITES {
                match safecast_site(ctx, label, *lat, *lon, *radius_km).await {
                    Ok(sig) => out.extend(sig),
                    Err(e) => tracing::warn!(site = label, error = %e, "safecast site failed"),
                }
            }
            Ok(out)
        }
        .boxed()
    }
}

async fn epa_station(
    ctx: &Ctx,
    label: &str,
    state_abbr: &str,
    lat: f64,
    lon: f64,
) -> Result<Vec<Signal>> {
    let url = format!(
        "https://enviro.epa.gov/enviro/efservice/RADNET_ANALYTICAL_RESULTS/STATE/{state_abbr}/ROWS/0:25/JSON"
    );
    let rows: Vec<serde_json::Value> = ctx.http.get(&url).send().await?.json().await?;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut out = Vec::new();
    for row in &rows {
        let analyte = get_str(row, &["ANALYTE_NAME", "analyte_name", "ANALYTE"]);
        let Some((_, elevated)) = THRESHOLDS.iter().find(|(a, _)| analyte.contains(a)) else {
            continue;
        };
        let Some(result) = get_num(row, &["RESULT_NUMERIC", "RESULT", "result"]) else {
            continue;
        };
        let unit = get_str(row, &["UNIT", "unit"]);
        let elevated_hit = result >= *elevated;
        out.push(
            Signal::new(
                "radiation",
                format!(
                    "RadNet {analyte} {result} {unit} @ {label}{}",
                    if elevated_hit { " (ELEVATED)" } else { "" }
                ),
                lat,
                lon,
                format!("radnet:{label}:{analyte}:{today}"),
            )
            .severity(if elevated_hit { "flash" } else { "info" })
            .payload(row.clone()),
        );
    }
    Ok(out)
}

async fn safecast_site(
    ctx: &Ctx,
    label: &str,
    lat: f64,
    lon: f64,
    radius_km: u32,
) -> Result<Vec<Signal>> {
    let url = format!(
        "https://api.safecast.org/measurements.json?latitude={lat}&longitude={lon}&distance={}&limit=10",
        radius_km * 1000
    );
    let rows: Vec<serde_json::Value> = ctx.http.get(&url).send().await?.json().await?;
    if rows.is_empty() {
        return Ok(Vec::new()); // no recent readings is normal for remote sites
    }
    let values: Vec<f64> = rows
        .iter()
        .filter_map(|r| get_num(r, &["value", "Value"]))
        .collect();
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let avg = values.iter().sum::<f64>() / values.len() as f64;
    let max = values.iter().cloned().fold(0.0f64, f64::max);
    let anomaly = avg > SAFECAST_ANOMALY_CPM;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    Ok(vec![
        Signal::new(
            "radiation",
            format!("{label}: avg {avg:.0} CPM (max {max:.0}){}", if anomaly { " — ANOMALY" } else { "" }),
            lat,
            lon,
            format!("safecast:{label}:{today}"),
        )
        .severity(if anomaly { "flash" } else { "info" })
        .payload(serde_json::json!({
            "site": label, "readings": values.len(), "avg_cpm": avg, "max_cpm": max,
        })),
    ])
}

fn get_str(v: &serde_json::Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| v.get(k).and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_string()
}

fn get_num(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(n) = v.get(k).and_then(|x| x.as_f64()) {
            return Some(n);
        }
        if let Some(n) = v.get(k).and_then(|x| x.as_str()).and_then(|s| s.parse().ok()) {
            return Some(n);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_match_crucix() {
        assert_eq!(THRESHOLDS[0], ("GROSS BETA", 5.0));
        assert_eq!(THRESHOLDS.len(), 5);
        assert_eq!(NUKE_SITES.len(), 6);
        assert_eq!(EPA_STATIONS.len(), 10);
    }
}
