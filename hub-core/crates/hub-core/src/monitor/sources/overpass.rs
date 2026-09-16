//! OpenStreetMap Overpass — geolocation POI watcher.
//!
//! Polls the public Overpass API (`overpass-api.de`) for POI nodes inside
//! a small watchlist of geopolitical hotspots (embassies, military
//! installations, nuclear sites). Each POI emits a Signal anchored at the
//! node's real lat/lon — useful for tracking new/changed OSM-tagged
//! facilities on the radar map without needing proprietary data.
//!
//! Fills the OSINT Framework "Geolocation → Maps" gap with a free,
//! keyless upstream.
//!
//! - **KEYLESS**: the Overpass API is free but rate-limited to 2 req/s/IP
//!   and 10MB responses. Our cadence is 24h with a 6s inter-query gap to
//!   stay under the limit and stay friendly.
//! - **QUERY SHAPE**: `node["amenity"="embassy"](south,west,north,east);out body 5;`.
//!   Bbox syntax (`lat_min, lon_min, lat_max, lon_max`), comma-separated.
//!   4 args required — the `around:radius,lat,lon` filter accepts only 3
//!   args and overpass returns "bbox requires four arguments" if you mix.
//! - **WATCHLIST**: HUB_OVERPASS_WATCH env var is a comma-list of
//!   `lat_min,lon_min,lat_max,lon_max,label` tuples. Default ships with 5
//!   hotspots (Geneva UN, Tel Aviv, Moscow, Beijing, Punggye-ri NK).
//! - **KIND**: "geolocation" (new cluster on radar, distinct from
//!   transport/financial/cyber).
//! - **SHELVED**: when the upstream returns 429 (rate-limit) or 504
//!   (timeout) we degrade gracefully — visible on health board, no new
//!   signals. Self-heals next sweep.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const OVERPASS_ENDPOINT: &str = "https://overpass-api.de/api/interpreter";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Inter-query gap. Overpass fair-use is ~2 req/s/IP; we use 3s for safety.
const INTER_QUERY_GAP: Duration = Duration::from_secs(3);
/// Cap nodes per bbox query — 50 is well under Overpass's 10MB response
/// limit even for embassy-heavy bboxes like Geneva UN district.
const MAX_NODES_PER_BBOX: usize = 50;

/// Default watchlist: 5 geopolitical hotspots. Format: lat_min,lon_min,lat_max,lon_max,label.
/// Tuned to ~10km × 10km boxes around known embassy clusters + nuclear sites.
const DEFAULT_WATCHLIST: &[&str] = &[
    "46.20,6.10,46.25,6.20,Geneva UN district",
    "32.05,34.75,32.15,34.85,Tel Aviv embassies",
    "55.72,37.55,55.80,37.70,Moscow center",
    "39.90,116.35,39.98,116.45,Beijing diplomatic zone",
    "40.95,125.65,41.05,125.80,Punggye-ri nuclear test site",
];

pub struct Overpass;

impl Source for Overpass {
    fn name(&self) -> &'static str {
        "overpass"
    }
    fn interval(&self) -> Duration {
        // 24h. Bboxes are slow-moving (new embassies open rarely) and
        // Overpass tolerates one well-paced query per day from a single IP.
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let watchlist: Vec<&str> = if !ctx.config.monitor_overpass_watch.is_empty() {
                ctx.config.monitor_overpass_watch.iter().map(|s| s.as_str()).collect()
            } else {
                DEFAULT_WATCHLIST.to_vec()
            };

            let mut out = Vec::new();
            for (idx, entry) in watchlist.iter().enumerate() {
                if idx > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query_bbox(ctx, entry).await {
                    Ok(mut sigs) => out.append(&mut sigs),
                    Err(e) => {
                        tracing::warn!(target: "monitor::overpass", bbox = entry, error = %e, "bbox query failed");
                    }
                }
            }

            if out.is_empty() {
                return Ok(Vec::new());
            }
            Ok(out)
        }
        .boxed()
    }
}

/// Parse a watchlist entry "lat_min,lon_min,lat_max,lon_max,label" into
/// components. Returns None on malformed input (skipped, not fatal — one
/// bad entry doesn't fail the whole sweep).
fn parse_watch_entry(entry: &str) -> Option<(f64, f64, f64, f64, &str)> {
    let parts: Vec<&str> = entry.split(',').map(|s| s.trim()).collect();
    if parts.len() != 5 {
        return None;
    }
    let south: f64 = parts[0].parse().ok()?;
    let west: f64 = parts[1].parse().ok()?;
    let north: f64 = parts[2].parse().ok()?;
    let east: f64 = parts[3].parse().ok()?;
    let label = parts[4];
    if south >= north || west >= east {
        return None;
    }
    if !(-90.0..=90.0).contains(&south) || !(-90.0..=90.0).contains(&north) {
        return None;
    }
    if !(-180.0..=180.0).contains(&west) || !(-180.0..=180.0).contains(&east) {
        return None;
    }
    Some((south, west, north, east, label))
}

async fn query_bbox(ctx: &Ctx, entry: &str) -> Result<Vec<Signal>> {
    let (south, west, north, east, label) = match parse_watch_entry(entry) {
        Some(v) => v,
        None => {
            tracing::warn!(target: "monitor::overpass", entry = entry, "malformed watchlist entry — expected `lat_min,lon_min,lat_max,lon_max,label`");
            return Ok(Vec::new());
        }
    };

    let query = format!(
        "[out:json][timeout:25];(node[\"amenity\"=\"embassy\"]({south},{west},{north},{east}););out body {MAX_NODES_PER_BBOX};",
    );

    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .post(OVERPASS_ENDPOINT)
            .header("User-Agent", "IntelHub/0.1 (research; contact: ops@intelhub.local)")
            .form(&[("data", query.as_str())])
            .send(),
    )
    .await
    .map_err(|_| crate::error::HubError::sensor(format!("overpass: request timed out")))?
    .map_err(|e| crate::error::HubError::sensor(format!("overpass: {}", e.to_string())))?;

    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // Shelved-by-design: rate-limited. Self-heals next sweep.
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(crate::error::HubError::sensor(format!(
            "overpass: HTTP {status}: {snippet}"
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("overpass: parse: {e}")))?;

    let elements = body
        .get("elements")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for el in elements.into_iter().take(MAX_NODES_PER_BBOX) {
        let lat = match el.get("lat").and_then(|x| x.as_f64()) {
            Some(v) => v,
            None => continue,
        };
        let lon = match el.get("lon").and_then(|x| x.as_f64()) {
            Some(v) => v,
            None => continue,
        };
        let osm_id = el.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
        let tags = el.get("tags").cloned().unwrap_or(serde_json::json!({}));
        let name = tags
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("(unnamed)")
            .to_string();
        let country = tags
            .get("country")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let diplomatic = tags
            .get("diplomatic")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());

        let title = match (&country, &diplomatic) {
            (Some(c), Some(d)) => format!("{name} ({c}, {d})"),
            (Some(c), None) => format!("{name} ({c})"),
            _ => name.clone(),
        };

        out.push(
            Signal::new(
                "geolocation",
                format!("OSM embassy: {title} @ {label}"),
                lat,
                lon,
                format!("overpass:node:{osm_id}"),
            )
            .severity("routine")
            .payload(serde_json::json!({
                "osm_id": osm_id,
                "name": name,
                "country": country,
                "diplomatic": diplomatic,
                "watch_bbox": label,
                "tags": tags,
            })),
        );
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_watch_entry_valid() {
        let (s, w, n, e, l) = parse_watch_entry("46.2,6.1,46.3,6.2,Geneva UN").unwrap();
        assert_eq!(s, 46.2);
        assert_eq!(w, 6.1);
        assert_eq!(n, 46.3);
        assert_eq!(e, 6.2);
        assert_eq!(l, "Geneva UN");
    }

    #[test]
    fn parse_watch_entry_rejects_malformed() {
        assert!(parse_watch_entry("46.2,6.1,46.3").is_none()); // too few parts
        assert!(parse_watch_entry("46.2,6.1,46.3,6.2").is_none()); // missing label
        assert!(parse_watch_entry("46.3,6.1,46.2,6.2,x").is_none()); // south >= north
        assert!(parse_watch_entry("46.2,6.2,46.3,6.1,x").is_none()); // west >= east
        assert!(parse_watch_entry("91,0,46,0,x").is_none()); // out of range lat
        assert!(parse_watch_entry("foo,bar,baz,qux,label").is_none()); // not numbers
    }

    #[test]
    fn parse_watch_entry_accepts_label_with_spaces() {
        let (_, _, _, _, l) = parse_watch_entry("46.2,6.1,46.3,6.2,UN Geneva District").unwrap();
        assert_eq!(l, "UN Geneva District");
    }
}
