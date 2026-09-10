//! GDELT geo PointData (conflict/crisis mention clusters). Keyless but HARD
//! rate-limited: ≤1 request / 5s — enforced via the shared per-host limiter
//! (Crucix relied on an inline sleep; ours is structural).

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const URL: &str = "https://api.gdeltproject.org/api/v2/geo/geo?query=conflict%20OR%20military%20OR%20protest%20OR%20crisis&mode=PointData&timespan=24h&format=GeoJSON&maxpoints=250";

pub struct Gdelt;

impl Source for Gdelt {
    fn name(&self) -> &'static str {
        "gdelt"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(900)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            ctx.limiter
                .wait("api.gdeltproject.org", Duration::from_secs(5))
                .await;
            let j: serde_json::Value = ctx.http.get(URL).send().await?.json().await?;
            let mut out = Vec::new();
            let Some(features) = j.get("features").and_then(|v| v.as_array()) else {
                return Ok(out);
            };
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
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
                let (lon, lat) = (coords[0].as_f64(), coords[1].as_f64());
                let (Some(lon), Some(lat)) = (lon, lat) else { continue };
                let p = &f["properties"];
                let name = p
                    .get("name")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown");
                let count = p.get("count").and_then(|c| c.as_i64()).unwrap_or(1);
                let sev = if count >= 50 { "routine" } else { "info" };
                out.push(
                    Signal::new(
                        "news",
                        format!("{name} — {count} conflict/crisis mentions (24h)"),
                        lat,
                        lon,
                        format!("{name}:{today}"),
                    )
                    .severity(sev)
                    .payload(p.clone()),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}
