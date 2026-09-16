//! OpenSky air-activity summaries over 10 hotspot regions. Keyless
//! (anonymous ~4000 credits/day; 10 regions × 4/hour ≈ 960 — comfortable).
//! Crucix emitted nothing mappable here (region counts, no coordinates);
//! we emit one hourly-bucketed summary dot per region at its centroid —
//! a small, deliberate improvement over parity.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

/// (lamin, lomin, lamax, lomax, label) — Crucix OPENSKY HOTSPOTS verbatim.
pub(crate) const HOTSPOTS: &[(f64, f64, f64, f64, &str)] = &[
    (12.0, 30.0, 42.0, 65.0, "Middle East"),
    (20.0, 115.0, 28.0, 125.0, "Taiwan Strait"),
    (44.0, 22.0, 53.0, 41.0, "Ukraine Region"),
    (53.0, 19.0, 60.0, 29.0, "Baltic Region"),
    (5.0, 105.0, 23.0, 122.0, "South China Sea"),
    (33.0, 124.0, 43.0, 132.0, "Korean Peninsula"),
    (18.0, -90.0, 30.0, -72.0, "Caribbean"),
    (-2.0, -5.0, 8.0, 10.0, "Gulf of Guinea"),
    (-38.0, 12.0, -28.0, 24.0, "Cape Route"),
    (5.0, 40.0, 15.0, 55.0, "Horn of Africa"),
];

const HIGH_ALT_M: f64 = 12000.0;

pub struct OpenSky;

impl Source for OpenSky {
    fn name(&self) -> &'static str {
        "opensky"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(900)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let hour_bucket = chrono::Utc::now().format("%Y%m%d%H").to_string();
            let mut out = Vec::new();
            for (lamin, lomin, lamax, lomax, label) in HOTSPOTS {
                let url = format!(
                    "https://opensky-network.org/api/states/all?lamin={lamin}&lomin={lomin}&lamax={lamax}&lomax={lomax}"
                );
                let resp = match ctx.http.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(region = label, error = %e, "opensky region failed");
                        continue;
                    }
                };
                if resp.status().as_u16() == 429 || resp.status().as_u16() == 403 {
                    // anonymous quota exhausted — degrade the whole source till next tick
                    tracing::warn!(status = %resp.status(), "opensky rate-limited; backing off");
                    break;
                }
                if !resp.status().is_success() {
                    continue;
                }
                let Ok(j) = resp.json::<serde_json::Value>().await else {
                    continue;
                };
                let Some(states) = j.get("states").and_then(|s| s.as_array()) else {
                    continue;
                };
                let total = states.len();
                let high_alt = states
                    .iter()
                    .filter(|s| {
                        s.get(7).and_then(|v| v.as_f64()).unwrap_or(0.0) > HIGH_ALT_M
                    })
                    .count();
                let no_callsign = states
                    .iter()
                    .filter(|s| {
                        s.get(1)
                            .and_then(|v| v.as_str())
                            .map(|c| c.trim().is_empty())
                            .unwrap_or(true)
                    })
                    .count();
                if total == 0 {
                    continue;
                }
                let (clat, clon) = ((lamin + lamax) / 2.0, (lomin + lomax) / 2.0);
                out.push(
                    Signal::new(
                        "flight",
                        format!("{label}: {total} aircraft ({high_alt} high-altitude, {no_callsign} dark)"),
                        clat,
                        clon,
                        format!("{label}:{hour_bucket}"),
                    )
                    .severity(if high_alt >= 5 { "routine" } else { "info" })
                    .payload(serde_json::json!({
                        "region": label, "total": total, "high_altitude": high_alt,
                        "no_callsign": no_callsign,
                    })),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}
