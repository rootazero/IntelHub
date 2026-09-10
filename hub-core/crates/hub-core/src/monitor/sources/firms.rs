//! NASA FIRMS active-fire detections (VIIRS_SNPP_NRT, CSV). Ported from
//! Crucix `sources/firms.mjs`: same 6 hotspot bounding boxes, same 2-day
//! window; only high-intensity detections (FRP > 10 MW, top 15 per region)
//! become radar signals — raw counts would flood geo_events.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

/// (west, south, east, north, label) — Crucix HOTSPOTS verbatim.
const HOTSPOTS: &[(f64, f64, f64, f64, &str)] = &[
    (30.0, 12.0, 65.0, 42.0, "Middle East"),
    (22.0, 44.0, 41.0, 53.0, "Ukraine"),
    (44.0, 25.0, 63.0, 40.0, "Iran"),
    (21.0, 2.0, 52.0, 23.0, "Sudan / Horn of Africa"),
    (92.0, 9.0, 102.0, 29.0, "Myanmar"),
    (60.0, 5.0, 98.0, 37.0, "South Asia"),
];

const FRP_MIN_MW: f64 = 10.0;
const TOP_PER_REGION: usize = 15;

pub struct Firms;

impl Source for Firms {
    fn name(&self) -> &'static str {
        "firms"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(900)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(key) = ctx.config.monitor_firms_key.clone() else {
                return Err(HubError::internal(
                    "FIRMS_MAP_KEY not configured (secrets.env) — source degraded by design",
                ));
            };
            let mut out = Vec::new();
            for (w, s, e, n, label) in HOTSPOTS {
                let url = format!(
                    "https://firms.modaps.eosdis.nasa.gov/api/area/csv/{key}/VIIRS_SNPP_NRT/{w},{s},{e},{n}/2"
                );
                let resp = ctx.http.get(&url).send().await?;
                if !resp.status().is_success() {
                    tracing::warn!(region = label, status = %resp.status(), "FIRMS region failed");
                    continue; // per-region isolation: one bad region ≠ source failure
                }
                let body = resp.text().await?;
                let mut rows = parse_csv(&body);
                rows.sort_by(|a, b| b.frp.partial_cmp(&a.frp).unwrap_or(std::cmp::Ordering::Equal));
                for r in rows.into_iter().take(TOP_PER_REGION) {
                    let t = chrono::NaiveDate::parse_from_str(&r.acq_date, "%Y-%m-%d")
                        .ok()
                        .and_then(|d| {
                            let hh: u32 = r.acq_time.get(..2).and_then(|s| s.parse().ok()).unwrap_or(0);
                            let mm: u32 = r.acq_time.get(2..4).and_then(|s| s.parse().ok()).unwrap_or(0);
                            d.and_hms_opt(hh, mm, 0)
                        })
                        .map(|n| chrono::DateTime::from_naive_utc_and_offset(n, chrono::Utc))
                        .unwrap_or_else(chrono::Utc::now);
                    out.push(
                        Signal::new(
                            "fire",
                            format!("VIIRS fire — {label} (FRP {:.0} MW)", r.frp),
                            r.lat,
                            r.lon,
                            format!("{}-{}-{:.4}-{:.4}", r.acq_date, r.acq_time, r.lat, r.lon),
                        )
                        .severity("routine")
                        .occurred(t)
                        .payload(serde_json::json!({
                            "region": label, "frp": r.frp, "bright_ti4": r.bright,
                            "confidence": r.confidence, "daynight": r.daynight,
                        })),
                    );
                }
            }
            Ok(out)
        }
        .boxed()
    }
}

struct FireRow {
    lat: f64,
    lon: f64,
    bright: f64,
    frp: f64,
    acq_date: String,
    acq_time: String,
    confidence: String,
    daynight: String,
}

/// Header-driven CSV parse (Crucix used positional splits; we map column
/// names so upstream column reordering doesn't silently corrupt data).
fn parse_csv(body: &str) -> Vec<FireRow> {
    let mut lines = body.lines();
    let Some(header) = lines.next() else { return Vec::new() };
    let cols: Vec<&str> = header.trim().split(',').collect();
    let idx = |name: &str| cols.iter().position(|c| c.trim() == name);
    let (Some(i_lat), Some(i_lon), Some(i_frp)) = (idx("latitude"), idx("longitude"), idx("frp"))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in lines {
        let f: Vec<&str> = line.trim().split(',').collect();
        if f.len() < cols.len() {
            continue;
        }
        let (Some(lat), Some(lon), Some(frp)) = (
            f[i_lat].parse::<f64>().ok(),
            f[i_lon].parse::<f64>().ok(),
            f[i_frp].parse::<f64>().ok(),
        ) else {
            continue;
        };
        if frp < FRP_MIN_MW {
            continue;
        }
        out.push(FireRow {
            lat,
            lon,
            bright: idx("bright_ti4").and_then(|i| f[i].parse().ok()).unwrap_or(0.0),
            frp,
            acq_date: idx("acq_date").map(|i| f[i].to_string()).unwrap_or_default(),
            acq_time: idx("acq_time").map(|i| f[i].to_string()).unwrap_or_default(),
            confidence: idx("confidence").map(|i| f[i].to_string()).unwrap_or_default(),
            daynight: idx("daynight").map(|i| f[i].to_string()).unwrap_or_default(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_filters_frp() {
        let csv = "latitude,longitude,bright_ti4,scan,track,acq_date,acq_time,satellite,instrument,confidence,version,bright_ti5,frp,daynight\n\
                   35.1234,44.5678,320.5,1.0,1.0,2026-09-09,0135,N,VIIRS,n,2.0NRT,290.1,25.4,N\n\
                   36.0000,45.0000,300.0,1.0,1.0,2026-09-09,0135,N,VIIRS,l,2.0NRT,280.0,3.2,N\n";
        let rows = parse_csv(csv);
        assert_eq!(rows.len(), 1); // 3.2 MW below threshold
        assert!((rows[0].lat - 35.1234).abs() < 1e-9);
        assert!((rows[0].frp - 25.4).abs() < 1e-9);
        assert_eq!(rows[0].acq_time, "0135");
        assert_eq!(rows[0].confidence, "n");
    }

    #[test]
    fn tolerates_column_reorder() {
        let csv = "frp,longitude,latitude,acq_date,acq_time\n42.0,44.5,35.1,2026-09-09,1200\n";
        let rows = parse_csv(csv);
        assert_eq!(rows.len(), 1);
        assert!((rows[0].lon - 44.5).abs() < 1e-9);
    }

    #[test]
    fn empty_on_missing_columns() {
        assert!(parse_csv("a,b,c\n1,2,3\n").is_empty());
        assert!(parse_csv("").is_empty());
    }
}
