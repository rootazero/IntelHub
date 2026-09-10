//! KiwiSDR public receiver map (SIGINT-awareness layer). Keyless. Ported
//! from Crucix `sources/kiwisdr.mjs`: the receiver list is embedded in the
//! receiverbook.de page as a JS variable — parse with structure validation
//! (page redesign → loud error, never silent zero). Watch regions reuse the
//! conflict-zone boxes shared with FIRMS/OpenSky.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const URL: &str = "https://www.receiverbook.de/map?type=kiwisdr";
const MAX_SIGNALS: usize = 200;

/// (lamin, lomin, lamax, lomax, label) — conflict-zone watch boxes.
const WATCH: &[(f64, f64, f64, f64, &str)] = &[
    (12.0, 30.0, 42.0, 65.0, "Middle East"),
    (44.0, 22.0, 53.0, 41.0, "Ukraine"),
    (20.0, 115.0, 28.0, 125.0, "Taiwan Strait"),
    (5.0, 105.0, 23.0, 122.0, "South China Sea"),
    (33.0, 124.0, 43.0, 132.0, "Korean Peninsula"),
    (5.0, 40.0, 15.0, 55.0, "Horn of Africa"),
];

pub struct KiwiSdr;

impl Source for KiwiSdr {
    fn name(&self) -> &'static str {
        "kiwisdr"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let html = ctx.http.get(URL).send().await?.text().await?;
            let receivers = extract_receivers(&html)?;
            let mut out = Vec::new();
            for rx in &receivers {
                let lat = num(rx, &["lat", "latitude"]);
                let lon = num(rx, &["lon", "lng", "longitude"]);
                let (Some(lat), Some(lon)) = (lat, lon) else { continue };
                let Some(region) = WATCH.iter().find(|(s, w, n, e, _)| {
                    lat >= *s && lat <= *n && lon >= *w && lon <= *e
                }) else {
                    continue;
                };
                let name = rx
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("receiver")
                    .to_string();
                out.push(
                    Signal::new(
                        "other",
                        format!("KiwiSDR online — {} ({})", name, region.4),
                        lat,
                        lon,
                        format!("kiwisdr:{name}"),
                    )
                    .payload(serde_json::json!({ "region": region.4, "receiver": rx })),
                );
                if out.len() >= MAX_SIGNALS {
                    break;
                }
            }
            Ok(out)
        }
        .boxed()
    }
}

/// Extract the embedded `var receivers = [...]` array with structure checks.
fn extract_receivers(html: &str) -> Result<Vec<serde_json::Value>> {
    let start = html
        .find("var receivers")
        .ok_or_else(|| HubError::internal("receiverbook page changed: no 'var receivers'"))?;
    let arr_start = html[start..]
        .find('[')
        .map(|i| start + i)
        .ok_or_else(|| HubError::internal("receiverbook page changed: no array start"))?;
    // Parse the array directly with serde's deserializer: naive bracket
    // counting breaks on '[' / ']' inside string values (real page data has
    // them — e.g. receiver notes), which stranded the source behind a bogus
    // "unbalanced array" error while the data itself parsed fine.
    let mut de = serde_json::Deserializer::from_str(&html[arr_start..]);
    let arr: Vec<serde_json::Value> =
        <Vec<serde_json::Value> as serde::Deserialize>::deserialize(&mut de)
            .map_err(|e| HubError::internal(format!("receiverbook array not JSON: {e}")))?;
    if arr.is_empty() {
        return Err(HubError::internal("receiverbook array empty — page structure changed?"));
    }
    Ok(arr)
}

fn num(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
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
    fn extracts_embedded_array() {
        let html = r#"<html><script>var receivers = [{"name":"rx1","lat":36.0,"lon":44.0}]; var other = 1;</script></html>"#;
        let rx = extract_receivers(html).unwrap();
        assert_eq!(rx.len(), 1);
        assert_eq!(rx[0]["name"], "rx1");
    }

    #[test]
    fn page_redesign_is_a_loud_error() {
        assert!(extract_receivers("<html>no receivers here</html>").is_err());
    }
}
