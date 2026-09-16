//! Nominatim (OpenStreetMap) geocoding.
//!
//! Free keyless OSM Nominatim search API. Each entry in the
//! watchlist is a free-form query string (e.g. "Tor Project",
//! "NSO Group", "Sandvine Waterloo"); Nominatim returns the
//! best-matching place with lat/lon + display_name, which we
//! surface as a `cyber`-kind Signal anchored at that lat/lon.
//!
//! Use case: anchor threat-actor locations (APT HQ, known
//! surveillance-vendor offices, ransomware-group infrastructure)
//! on the radar when the OSINT Framework or intel feeds name
//! them but lack coordinates.
//!
//! Rate limit: Nominatim TOS = 1 req/sec. We add a 1.1s sleep
//! between queries to stay strictly under the limit.

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const INTER_QUERY_GAP: Duration = Duration::from_millis(1100);

const DEFAULT_QUERIES: &[&str] = &[
    "Herzliya Pituach Israel",      // NSO Group / surveillance vendor HQ area
    "Waterloo Ontario",             // Sandvine HQ area
    "Beersheba Israel",             // IDF Unit 8200 cyber HQ area
    "Krasnoarmeysk Moscow Oblast",  // GRU Unit 26165 HQ area (APT28)
    "Shenyang Liaoning",            // PLA Unit 61398 HQ area (APT1)
];

#[derive(Clone)]
pub struct Nominatim {
    pub watch: Vec<String>,
}

impl Default for Nominatim {
    fn default() -> Self {
        Self {
            watch: DEFAULT_QUERIES.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Deserialize)]
struct NominatimHit {
    lat: String,
    lon: String,
    display_name: String,
    #[serde(rename = "class")]
    class: Option<String>,
    #[serde(rename = "type")]
    ty: Option<String>,
    importance: Option<f64>,
}

impl Source for Nominatim {
    fn name(&self) -> &'static str {
        "nominatim"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move {
            let mut sigs = Vec::new();
            for (i, q) in self.watch.iter().enumerate() {
                if i > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query(ctx, q).await {
                    Ok(Some(sig)) => sigs.push(sig),
                    Ok(None) => {
                        tracing::debug!(query = %q, "nominatim: no result");
                    }
                    Err(e) => {
                        tracing::warn!(
                            query = %q,
                            error = %e,
                            "nominatim: query failed",
                        );
                    }
                }
            }
            Ok(sigs)
        })
    }
}

async fn query(ctx: &Ctx, q: &str) -> Result<Option<Signal>, HubError> {
    let url = format!(
        "https://nominatim.openstreetmap.org/search?q={}&format=json&limit=1&addressdetails=0",
        urlencoded(q),
    );
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(&url)
            .header("Accept", "application/json")
            // OSM Nominatim TOS requires a real UA with contact info.
            // Override Ctx.http's browser UA — Nominatim specifically
            // bans default reqwest/bot UAs.
            .header(
                "User-Agent",
                "IntelHub-research/1.0 (admin@intelhub.local)",
            )
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("nominatim: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("nominatim: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "nominatim: HTTP {s}: {snip}"
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("nominatim: parse: {e}")))?;

    let arr = body.as_array().ok_or_else(|| {
        HubError::sensor("nominatim: response not array".to_string())
    })?;

    let Some(first) = arr.first() else {
        return Ok(None);
    };
    let hit: NominatimHit = serde_json::from_value(first.clone())
        .map_err(|e| HubError::sensor(format!("nominatim: hit parse: {e}")))?;

    let lat: f64 = hit
        .lat
        .parse()
        .map_err(|e| HubError::sensor(format!("nominatim: lat: {e}")))?;
    let lon: f64 = hit
        .lon
        .parse()
        .map_err(|e| HubError::sensor(format!("nominatim: lon: {e}")))?;

    let payload = json!({
        "query": q,
        "display_name": hit.display_name,
        "osm_class": hit.class,
        "osm_subtype": hit.ty,
        "importance": hit.importance,
        "geocoded_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("Nominatim geocode: {q}"),
            lat,
            lon,
            format!("nominatim:{}", content_hash(q)),
        )
        .payload(payload),
    ))
}

/// Percent-encode a string for a URL query component.
/// Minimal stdlib implementation (avoid `urlencoding` crate just
/// for this one helper).
fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Stable content hash for external_id (FNV-1a 64-bit, hex).
fn content_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}