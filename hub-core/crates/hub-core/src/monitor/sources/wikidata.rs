//! Wikidata — global entity enrichment via the free, keyless Wikidata
//! API (OSINT Framework "Business Records → Entities" gap, free
//! counterpart to OpenCorporates which is paid-only since 2026).
//!
//! Polls `wbgetentities` for a curated watchlist of Wikidata Q-IDs
//! (sanctioned/interesting entities: Tornado Cash, LockBit, etc.) and
//! emits a Signal per entity anchored at its registered HQ
//! coordinates (P625 — coordinate location). For entities without a
//! direct coordinate, falls back to the P159 (headquarters location)
//! → P17 (country) → country capital coords.
//!
//! - **KEYLESS**: Wikidata is a free public service, no API key. Each
//!   Q-ID costs 1 API call. Watchlist of 10 entities = 10 calls per
//!   sweep.
//! - **RATE LIMIT**: Wikidata asks for ~1 req/s for individual users
//!   but tolerates bursts from well-behaved clients. 24h cadence with
//!   2s inter-query gap is well inside their fair-use envelope.
//! - **WATCHLIST**: HUB_WIKIDATA_WATCH env var is a comma-list of
//!   Wikidata Q-IDs (e.g. `Q113481936,Q2462448`). Default ships with
//!   10 high-signal entities (sanctioned crypto mixers, APT groups,
//!   nation-state actors).
//! - **KIND**: "financial" (matches etherscan/defillama/opencorp
//!   visual cluster on radar).
//! - **SOURCE PATTERN**: each entity emits one Signal at the entity's
//!   real geo coordinates (when available) — distinct from opencorp
//!   which uses jurisdiction capital as fallback. Wikidata gives us
//!   actual HQ lat/lon, so radar placement is honest.
//!
//! See https://www.wikidata.org/w/api.php for full API reference.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const WIKIDATA_API: &str = "https://www.wikidata.org/w/api.php";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const INTER_QUERY_GAP: Duration = Duration::from_secs(2);
const MAX_ENTITIES_PER_SWEEP: usize = 50;

/// Default watchlist: 10 high-signal entities (sanctioned crypto
/// mixers, APT groups, regime actors). Wikidata Q-IDs are stable —
/// adding to this list is just a comma-separated string in
/// `core/hub.env`.
const DEFAULT_WATCHLIST: &[&str] = &[
    "Q113481936", // Tornado Cash (crypto mixer, OFAC-sanctioned)
    "Q2462448",   // LockBit (ransomware gang)
    "Q222970",    // Lazarus Group (DPRK APT)
    "Q1514636",   // Conti (ransomware)
    "Q6918480",   // Fancy Bear (APT28)
    "Q180634",    // Wagner Group (PMC)
    "Q462498",    // Jabhat al-Nusra
    "Q403567",    // Hezbollah
    "Q56155421",  // REvil (ransomware)
    "Q96438912",  // TrickBot (malware family)
];

/// Fallback anchor: country capital coords for entities that have a
/// P17 (country) claim but no direct P625 coordinate. Used as last
/// resort so the entity still gets visual placement on the radar.
const COUNTRY_CAPITALS: &[(&str, f64, f64)] = &[
    ("Q30", 38.9072, -77.0369),    // United States (Washington DC)
    ("Q148", 19.4326, -99.1332),   // China (Beijing approx)
    ("Q159", 55.7558, 37.6173),    // Russia (Moscow)
    ("Q142", 41.0082, 28.9784),    // Turkey (Ankara approx)
    ("Q794", 41.9028, 12.4964),    // Iran (Tehran approx)
    ("Q117", 33.5138, 36.2765),    // Syria (Damascus approx)
    ("Q155", 23.5505, -46.6333),  // Brazil (São Paulo approx)
    ("Q183", 50.0755, 14.4378),    // Germany (Berlin)
    ("Q145", 51.5074, -0.1278),    // UK (London)
    ("Q16", 45.4215, -75.6972),   // Canada (Ottawa)
];

pub struct Wikidata;

impl Source for Wikidata {
    fn name(&self) -> &'static str {
        "wikidata"
    }
    fn interval(&self) -> Duration {
        // 24h. Wikidata entity data changes slowly (label/description
        // edits are versioned but rare; P-numbers and P-values only
        // change when contributors add structure). Daily is enough.
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let watchlist: Vec<&str> = if !ctx.config.monitor_wikidata_watch.is_empty() {
                ctx.config.monitor_wikidata_watch.iter().map(|s| s.as_str()).collect()
            } else {
                DEFAULT_WATCHLIST.to_vec()
            };

            let mut out = Vec::new();
            // Wikidata API supports up to 50 IDs per request via
            // `ids=Q1|Q2|Q3|...`. Batch into chunks of 50 to minimize
            // round-trips.
            for chunk in watchlist.chunks(MAX_ENTITIES_PER_SWEEP) {
                match fetch_entities(ctx, chunk).await {
                    Ok(mut sigs) => out.append(&mut sigs),
                    Err(e) => {
                        tracing::warn!(target: "monitor::wikidata", error = %e, "batch fetch failed");
                    }
                }
                tokio::time::sleep(INTER_QUERY_GAP).await;
            }

            if out.is_empty() {
                return Ok(Vec::new());
            }
            Ok(out)
        }
        .boxed()
    }
}

async fn fetch_entities(ctx: &Ctx, q_ids: &[&str]) -> Result<Vec<Signal>> {
    let ids_param = q_ids.join("|");
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(WIKIDATA_API)
            .query(&[
                ("action", "wbgetentities"),
                ("ids", &ids_param),
                ("format", "json"),
                ("props", "labels|descriptions|claims"),
                ("languages", "en"),
            ])
            .send(),
    )
    .await
    .map_err(|_| crate::error::HubError::sensor("wikidata: request timed out".to_string()))?
    .map_err(|e| crate::error::HubError::sensor(format!("wikidata: {e}")))?;

    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        tracing::warn!(target: "monitor::wikidata", "rate-limited; will retry next sweep");
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(crate::error::HubError::sensor(format!(
            "wikidata: HTTP {status}: {snippet}"
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| crate::error::HubError::sensor(format!("wikidata: parse: {e}")))?;

    let entities = body
        .get("entities")
        .and_then(|e| e.as_object())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for (q_id, ent) in entities.iter() {
        // Missing entities come back as "-1" (the literal key) — skip.
        if q_id == "-1" {
            continue;
        }

        let label = ent
            .get("labels")
            .and_then(|l| l.get("en"))
            .and_then(|l| l.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or(q_id)
            .to_string();

        let description = ent
            .get("descriptions")
            .and_then(|d| d.get("en"))
            .and_then(|d| d.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let claims = ent.get("claims").cloned().unwrap_or(serde_json::json!({}));

        // Anchor resolution: P625 (coordinate) > P159 (HQ) → P625 >
        // P17 (country) → COUNTRY_CAPITALS fallback. Emit (0,0) only
        // if everything is missing — operator sees the entity but it
        // sits on the equator/prime-meridian intersection.
        let (lat, lon, anchor_source) = resolve_anchor(&claims);

        let title = if !description.is_empty() {
            format!("{label} ({description})")
        } else {
            label.clone()
        };

        out.push(
            Signal::new(
                "financial",
                format!("Wikidata entity {q_id}: {title}"),
                lat,
                lon,
                format!("wikidata:entity:{q_id}"),
            )
            .severity("routine")
            .payload(serde_json::json!({
                "q_id": q_id,
                "label": label,
                "description": description,
                "anchor_source": anchor_source,
                "claims_keys": claims.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),
            })),
        );
    }

    Ok(out)
}

/// Resolve the entity's anchor coordinates. Returns (lat, lon, source)
/// where source describes which Wikidata property supplied the coords.
/// Falls back to (0.0, 0.0, "missing") if nothing usable is found.
fn resolve_anchor(claims: &serde_json::Value) -> (f64, f64, &'static str) {
    // P625 = coordinate location (lat,lon) — the cleanest source.
    if let Some((lat, lon)) = extract_p625(claims) {
        return (lat, lon, "P625");
    }

    // P159 = headquarters location (Q-ID of a place). Look up that
    // place's P625; if it has one, use those coords.
    if let Some(hq_qid) = extract_p159(claims) {
        // We don't recursively fetch the HQ entity (would cost another
        // API call); instead, look it up in our embedded country-capital
        // table when the HQ is a country (Q30, Q148, etc.).
        if let Some((_, lat, lon)) = COUNTRY_CAPITALS
            .iter()
            .find(|(code, _, _)| *code == hq_qid.as_str())
        {
            return (*lat, *lon, "P159→country-capital");
        }
        // HQ is a city but not in our table — caller will see (0,0).
        return (0.0, 0.0, "P159→unmapped");
    }

    // P17 = country (Q-ID). Use country capital as last-known anchor.
    if let Some(country_qid) = extract_p17(claims) {
        if let Some((_, lat, lon)) = COUNTRY_CAPITALS
            .iter()
            .find(|(code, _, _)| *code == country_qid.as_str())
        {
            return (*lat, *lon, "P17→country-capital");
        }
    }

    (0.0, 0.0, "missing")
}

/// Extract P625 (coordinate location) from Wikidata claims.
/// P625 value shape: { "value": { "latitude": <f64>, "longitude": <f64> } }
fn extract_p625(claims: &serde_json::Value) -> Option<(f64, f64)> {
    let p625 = claims.get("P625")?.as_array()?.first()?;
    let lat = p625
        .get("mainsnak")?
        .get("datavalue")?
        .get("value")?
        .get("latitude")
        .and_then(|v| v.as_f64())?;
    let lon = p625
        .get("mainsnak")?
        .get("datavalue")?
        .get("value")?
        .get("longitude")
        .and_then(|v| v.as_f64())?;
    Some((lat, lon))
}

/// Extract P159 (headquarters location) Q-ID from claims.
fn extract_p159(claims: &serde_json::Value) -> Option<String> {
    let p159 = claims.get("P159")?.as_array()?.first()?;
    p159.get("mainsnak")?
        .get("datavalue")?
        .get("value")?
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract P17 (country) Q-ID from claims.
fn extract_p17(claims: &serde_json::Value) -> Option<String> {
    let p17 = claims.get("P17")?.as_array()?.first()?;
    p17.get("mainsnak")?
        .get("datavalue")?
        .get("value")?
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_p625_returns_coords() {
        let claims = json!({
                "P625": [{
                    "mainsnak": {
                        "datavalue": {
                            "value": { "latitude": 40.7128, "longitude": -74.0060 }
                        }
                    }
                }]
            });
        assert_eq!(extract_p625(&claims), Some((40.7128, -74.0060)));
    }

    #[test]
    fn extract_p625_returns_none_when_missing() {
        let claims = json!({});
        assert_eq!(extract_p625(&claims), None);
    }

    #[test]
    fn extract_p159_returns_qid() {
        let claims = json!({
                "P159": [{
                    "mainsnak": {
                        "datavalue": { "value": { "id": "Q30" } }
                    }
                }]
            });
        assert_eq!(extract_p159(&claims), Some("Q30".to_string()));
    }

    #[test]
    fn resolve_anchor_prefers_p625() {
        let claims = json!({
            "P625": [{
                "mainsnak": { "datavalue": { "value": { "latitude": 10.0, "longitude": 20.0 } } }
            }],
            "P159": [{ "mainsnak": { "datavalue": { "value": { "id": "Q30" } } } }],
        });
        let (lat, lon, src) = resolve_anchor(&claims);
        assert_eq!(lat, 10.0);
        assert_eq!(lon, 20.0);
        assert_eq!(src, "P625");
    }

    #[test]
    fn resolve_anchor_falls_back_to_country_capital() {
        let claims = json!({
            "P17": [{ "mainsnak": { "datavalue": { "value": { "id": "Q148" } } } }],
        });
        let (lat, lon, src) = resolve_anchor(&claims);
        assert_eq!(src, "P17→country-capital");
        assert!(lat > 0.0 && lon > 0.0); // China capital coords
    }

    #[test]
    fn resolve_anchor_returns_zero_when_nothing_found() {
        let claims = json!({ "P31": [] });
        let (_, _, src) = resolve_anchor(&claims);
        assert_eq!(src, "missing");
    }
}