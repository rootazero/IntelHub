//! NOAA/NWS active alerts (Extreme/Severe). Keyless. Ported from Crucix
//! `sources/noaa.mjs`, including the GeoJSON centroid math (Point direct;
//! Polygon/MultiPolygon average of the first ring — an approximation, good
//! enough for a radar dot).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const URL: &str =
    "https://api.weather.gov/alerts/active?status=actual&severity=Extreme,Severe&limit=50";

pub struct Noaa;

impl Source for Noaa {
    fn name(&self) -> &'static str {
        "noaa"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(300)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            // NWS requires a descriptive UA with contact info.
            let j: serde_json::Value = ctx
                .http
                .get(URL)
                .header("Accept", "application/geo+json")
                .header(
                    "User-Agent",
                    "intelhub-monitor/1.0 (OSINT hub; contact: ops@intelhub.local)",
                )
                .send()
                .await?
                .json()
                .await?;
            let mut out = Vec::new();
            let Some(features) = j.get("features").and_then(|v| v.as_array()) else {
                return Ok(out);
            };
            for f in features {
                let Some((lat, lon)) = centroid(&f["geometry"]) else {
                    continue;
                };
                let p = &f["properties"];
                let event = p.get("event").and_then(|s| s.as_str()).unwrap_or("alert");
                let areas = p
                    .get("areaDesc")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .replace(';', ",");
                let id = p
                    .get("id")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string();
                if id.is_empty() {
                    continue;
                }
                let nws_sev = p.get("severity").and_then(|s| s.as_str()).unwrap_or("");
                let sev = if nws_sev == "Extreme" {
                    "priority"
                } else {
                    "routine"
                };
                let t = p
                    .get("onset")
                    .and_then(|s| s.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);
                out.push(
                    Signal::new("disaster", format!("{event} — {areas}"), lat, lon, id)
                        .severity(sev)
                        .occurred(t)
                        .payload(p.clone()),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}

/// (lat, lon) from a GeoJSON geometry: Point direct; Polygon/MultiPolygon →
/// mean of the outer ring vertices (Crucix parity).
fn centroid(geom: &serde_json::Value) -> Option<(f64, f64)> {
    let ty = geom.get("type")?.as_str()?;
    let coords = geom.get("coordinates")?;
    match ty {
        "Point" => {
            let c = coords.as_array()?;
            Some((c.get(1)?.as_f64()?, c.get(0)?.as_f64()?))
        }
        "Polygon" => ring_centroid(coords.get(0)?),
        "MultiPolygon" => ring_centroid(coords.get(0)?.get(0)?),
        _ => None,
    }
}

fn ring_centroid(ring: &serde_json::Value) -> Option<(f64, f64)> {
    let pts = ring.as_array()?;
    let (mut sx, mut sy, mut n) = (0.0f64, 0.0f64, 0usize);
    for p in pts {
        let Some(a) = p.as_array() else { continue };
        if a.len() < 2 {
            continue;
        }
        let (Some(x), Some(y)) = (a[0].as_f64(), a[1].as_f64()) else {
            continue;
        };
        sx += x;
        sy += y;
        n += 1;
    }
    if n == 0 {
        None
    } else {
        Some((sy / n as f64, sx / n as f64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn point_passthrough() {
        let g = json!({"type":"Point","coordinates":[-97.5, 35.4]});
        assert_eq!(centroid(&g), Some((35.4, -97.5)));
    }

    #[test]
    fn polygon_centroid_is_ring_mean() {
        let g = json!({"type":"Polygon","coordinates":[[[0.0,0.0],[2.0,0.0],[2.0,2.0],[0.0,2.0],[0.0,0.0]]]});
        let (lat, lon) = centroid(&g).unwrap();
        assert!((lat - 0.8).abs() < 1e-9); // mean of [0,0,2,2,0]
        assert!((lon - 0.8).abs() < 1e-9);
    }

    #[test]
    fn multipolygon_uses_first_ring() {
        let g = json!({"type":"MultiPolygon","coordinates":[[[[10.0,10.0],[12.0,12.0]]],[[[50.0,50.0],[52.0,52.0]]]]});
        let (lat, lon) = centroid(&g).unwrap();
        assert!((lat - 11.0).abs() < 1e-9);
        assert!((lon - 11.0).abs() < 1e-9);
    }
}
