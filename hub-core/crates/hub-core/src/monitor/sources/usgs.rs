//! USGS earthquakes (NEW layer vs Crucix — it had no quake source; spec §3).
//! Keyless GeoJSON feed, M2.5+ past day.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const URL: &str = "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_day.geojson";

pub struct Usgs;

impl Source for Usgs {
    fn name(&self) -> &'static str {
        "usgs"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(300)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let j: serde_json::Value = ctx.http.get(URL).send().await?.json().await?;
            Ok(parse_feed(&j))
        }
        .boxed()
    }
}

fn parse_feed(j: &serde_json::Value) -> Vec<Signal> {
    let mut out = Vec::new();
    let Some(features) = j.get("features").and_then(|v| v.as_array()) else {
        return out;
    };
    for f in features {
        let Some(coords) = f
            .pointer("/geometry/coordinates")
            .and_then(|c| c.as_array())
        else {
            continue;
        };
        if coords.len() < 2 {
            continue;
        }
        let (lon, lat) = (
            coords[0].as_f64().unwrap_or(f64::NAN),
            coords[1].as_f64().unwrap_or(f64::NAN),
        );
        if !lat.is_finite() || !lon.is_finite() {
            continue;
        }
        let p = &f["properties"];
        let mag = p.get("mag").and_then(|m| m.as_f64()).unwrap_or(0.0);
        let place = p.get("place").and_then(|s| s.as_str()).unwrap_or("unknown");
        let id = f
            .get("id")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            continue;
        }
        let t = p
            .get("time")
            .and_then(|t| t.as_i64())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .unwrap_or_else(chrono::Utc::now);
        let sev = if mag >= 6.0 {
            "priority"
        } else if mag >= 4.5 {
            "routine"
        } else {
            "info"
        };
        out.push(
            Signal::new("quake", format!("M{mag:.1} — {place}"), lat, lon, id)
                .severity(sev)
                .occurred(t)
                .payload(p.clone()),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_geojson_features() {
        let j = serde_json::json!({
            "features": [{
                "id": "us7000abcd",
                "geometry": {"type": "Point", "coordinates": [142.3, 38.3, 40.0]},
                "properties": {"mag": 6.2, "place": "off the east coast of Honshu", "time": 1757500000000i64}
            }, {
                "id": "ci1234",
                "geometry": {"type": "Point", "coordinates": [-117.0, 34.1, 8.0]},
                "properties": {"mag": 3.1, "place": "5km NW of Ridgecrest, CA", "time": 1757500000000i64}
            }]
        });
        let sigs = parse_feed(&j);
        assert_eq!(sigs.len(), 2);
        assert_eq!(sigs[0].kind, "quake");
        assert_eq!(sigs[0].severity, "priority"); // M6.2
        assert_eq!(sigs[1].severity, "info"); // M3.1
        assert!((sigs[0].lat - 38.3).abs() < 1e-9);
        assert!((sigs[0].lon - 142.3).abs() < 1e-9);
        assert_eq!(sigs[0].external_id, "us7000abcd");
    }

    #[test]
    fn skips_malformed() {
        let j = serde_json::json!({"features": [{"geometry": null, "properties": {}}]});
        assert!(parse_feed(&j).is_empty());
    }
}
