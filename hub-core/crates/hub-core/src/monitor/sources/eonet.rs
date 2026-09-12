//! NASA EONET climate events (SP8-D climate plane).
//!
//! EONET v3 tracks natural events by category. We subscribe ONLY to the
//! three climate-system categories — `drought`, `seaLakeIce`,
//! `tempExtremes` — everything else (storms/fires/quakes) is already
//! covered by NOAA / FIRMS / USGS with better cadence, and re-fetching it
//! here would double-tag the same event under two kinds. Free, keyless,
//! no rate limit documented. Live-probed 2026-09-11: open events are
//! currently iceberg tracks (seaLakeIce); drought/tempExtremes populate
//! only while NASA actively tracks an episode — sparse is honest.

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::FutureExt;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};
use super::reliefweb::truncate;

const CATEGORIES: &str = "drought,seaLakeIce,tempExtremes";

pub struct Eonet;

impl Source for Eonet {
    fn name(&self) -> &'static str {
        "eonet"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let url = format!(
                "https://eonet.gsfc.nasa.gov/api/v3/events?status=open&limit=100&category={CATEGORIES}"
            );
            let resp = ctx.http.get(&url).send().await?;
            if !resp.status().is_success() {
                return Err(crate::error::HubError::sensor(format!(
                    "EONET HTTP {}",
                    resp.status()
                )));
            }
            let body = resp.text().await?;
            let j: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| crate::error::HubError::sensor(format!("EONET decode: {e}")))?;
            Ok(parse_events(&j))
        }
        .boxed()
    }
}

/// EONET payload → climate signals. Pure + unit-tested.
pub fn parse_events(j: &serde_json::Value) -> Vec<Signal> {
    let mut out = Vec::new();
    for e in j.get("events").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        let id = e["id"].as_str().unwrap_or("x");
        let title = e["title"].as_str().unwrap_or("").trim();
        if title.is_empty() {
            continue;
        }
        // Latest geometry point carries the freshest position + date.
        let Some(geo) = e["geometry"].as_array().and_then(|g| g.last()) else { continue };
        if geo["type"].as_str() != Some("Point") {
            continue; // polygons (drought regions) — skip for now, no centroid logic yet
        }
        let coords = &geo["coordinates"];
        let (Some(lon), Some(lat)) = (coords[0].as_f64(), coords[1].as_f64()) else { continue };
        let cat = e["categories"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["id"].as_str())
            .unwrap_or("climate");
        let mut sig = Signal::new("climate", truncate(title, 140), lat, lon, format!("eonet:{id}"))
            .severity("routine");
        if let Some(t) = geo["date"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        {
            sig = sig.occurred(t.with_timezone(&Utc));
        }
        out.push(sig.payload(serde_json::json!({
            "category": cat,
            "magnitude": geo["magnitudeValue"].clone(),
            "magnitude_unit": geo["magnitudeUnit"].as_str(),
            "url": e["link"].as_str(),
        })));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iceberg_points() {
        let j = serde_json::json!({ "events": [
            {
                "id": "EONET_7001",
                "title": "Iceberg D33D",
                "link": "https://eonet.gsfc.nasa.gov/api/v3/events/EONET_7001",
                "categories": [{ "id": "seaLakeIce", "title": "Sea and Lake Ice" }],
                "geometry": [
                    { "date": "2026-07-20T00:00:00Z", "type": "Point", "coordinates": [-55.40, -63.70] },
                    { "date": "2026-07-23T00:00:00Z", "type": "Point",
                      "coordinates": [-55.47, -63.78], "magnitudeValue": 90.0, "magnitudeUnit": "NM^2" }
                ]
            },
            { "id": "EONET_X", "title": "", "geometry": [] }, // junk row dropped
        ]});
        let sigs = parse_events(&j);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].kind, "climate");
        assert_eq!(sigs[0].external_id, "eonet:EONET_7001");
        assert!((sigs[0].lat - (-63.78)).abs() < 0.01, "GeoJSON is [lon, lat]");
        assert!((sigs[0].lon - (-55.47)).abs() < 0.01);
    }
}
