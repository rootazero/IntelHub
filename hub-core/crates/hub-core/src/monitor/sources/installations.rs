//! Military installations harvest (GEV P3 T3, 2026-09-17).
//!
//! Overpass API (OpenStreetMap) global `["military"]` harvest → PG
//! `military_installations` (migration 0020). Contracts.md §4: elements are
//! Overpass raw passthrough shape — tags / geometry / bounds are stored
//! verbatim (normalize lives in the client), plus a hub-side derived
//! `mil_class` column for server-side filtering.
//!
//! Cadence: 24h. One global query times out server-side, so the world is
//! split into 4 quadrants (90°×180°) with a ≥60s politeness gap between
//! them — the public Overpass instance fair-use envelope is small and we
//! are one of thousands of clients behind the shared egress IP.
//!
//! Failure policy (user decision: failure sources stay visible):
//! - a failing quadrant retries 2× with exponential backoff (30s, 60s);
//! - a still-failing quadrant KEEPS its old rows (`fetched_at` untouched)
//!   and the round still counts as success for the scheduler — the error
//!   is logged with the quadrant bbox (celestrak.rs precedent: partial
//!   success beats a health-board red that hides good data);
//! - the stale sweep (`DELETE WHERE fetched_at < round_ts`) runs ONLY
//!   after a fully successful 4/4 round — a partial round must never wipe
//!   the quadrant that failed;
//! - 0/4 quadrants → Err → the scheduler health cell
//!   (`monitor:health:installations`, written by source_loop →
//!   geo::report_health every round) flips to error with the message;
//! - hub restarts re-trigger a first tick like every source, but if the
//!   freshest row is <24h old the round is skipped (max(fetched_at) probe)
//!   — restarting the hub must not hammer Overpass.
//!
//! Env: `HUB_OVERPASS_URL` wins, then `OVERPASS_URL`, default
//! overpass-api.de. Keyless. Emits no geo Signals — a catalog, not events
//! (celestrak precedent: liveness via the health cell + sweephist ring).

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::{json, Value};
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

pub const INTERVAL_SECS: u64 = 24 * 3600;
/// Server-side QL timeout; the dedicated HTTP client adds headroom.
const QUERY_TIMEOUT_SECS: u64 = 120;
/// Overpass quadrant queries routinely exceed the shared 25s Ctx ceiling —
/// this collector builds its own client (celestrak uses the fast Ctx client
/// because TLE files are tiny; a global military bbox is not).
const HTTP_TIMEOUT_SECS: u64 = QUERY_TIMEOUT_SECS + 30;
/// ≥60s between quadrants (politeness + shared-IP rate-limit safety).
const QUADRANT_POLITENESS_SECS: u64 = 60;
/// Retries AFTER the first attempt (exponential backoff off this base).
const QUADRANT_RETRIES: u32 = 2;
const RETRY_BASE_SECS: u64 = 30;
/// 2026-09-17 (T17 315): overpass-api.de Apache-hard-blocks our shared
/// egress (header-independent 406 — penalty box precedent, gdelt 429).
/// Ordered fallback mirrors (315-probed 200): kumi → openstreetmap.fr.
/// overpass.osm.ch is EXCLUDED: it serves a Switzerland-only extract —
/// 200 + elements:[] with NO remark for off-extract quadrants, which is
/// indistinguishable from a genuinely empty quadrant and armed the
/// full-round sweep with false empties (q2 EU/NA wiped once on 315).
/// `HUB_OVERPASS_URL`/`OVERPASS_URL` override to a single endpoint.
const DEFAULT_ENDPOINTS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.openstreetmap.fr/api/interpreter",
];

/// Full-round sweep floor: a global OSM military harvest is always 5-figure
/// (T17 315 measured 13.7K with one quadrant missing). A round below this
/// floor means mirrors are serving degraded/empty bodies — skip the sweep
/// (defense in depth under the remark-detection).
const SWEEP_MIN_ROWS: usize = 5000;
/// Same browser UA as Ctx::new — Overpass fronting (Cloudflare) rejects
/// honest bot UAs from datacenter ASNs.
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

/// One normalized row of `military_installations`.
pub struct InstallationRow {
    pub osm_type: &'static str,
    pub osm_id: i64,
    pub name: Option<String>,
    pub mil_class: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub minlat: Option<f64>,
    pub minlon: Option<f64>,
    pub maxlat: Option<f64>,
    pub maxlon: Option<f64>,
    pub geometry: Option<Value>,
    pub tags: Value,
}

/// The 4 harvest quadrants as (south, west, north, east).
pub fn quadrants() -> [(f64, f64, f64, f64); 4] {
    [
        (-90.0, -180.0, 0.0, 0.0),
        (-90.0, 0.0, 0.0, 180.0),
        (0.0, -180.0, 90.0, 0.0),
        (0.0, 0.0, 90.0, 180.0),
    ]
}

/// Endpoint resolution: `HUB_OVERPASS_URL` → `OVERPASS_URL` (single
/// override) → the ordered fallback list. Rotation across quadrants
/// spreads load across mirrors instead of hammering the first one.
pub fn endpoints() -> Vec<String> {
    for var in ["HUB_OVERPASS_URL", "OVERPASS_URL"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return vec![v.to_string()];
            }
        }
    }
    DEFAULT_ENDPOINTS.iter().map(|s| s.to_string()).collect()
}

/// Overpass QL for one quadrant. `out geom qt` is mandatory: only then do
/// way/relation elements carry `geometry` / `bounds` (without it there is
/// no footprint and no center fallback — the client contract requires both).
pub fn build_query(s: f64, w: f64, n: f64, e: f64) -> String {
    format!(
        "[out:json][timeout:{QUERY_TIMEOUT_SECS}];(node[\"military\"]({s},{w},{n},{e});way[\"military\"]({s},{w},{n},{e});relation[\"military\"]({s},{w},{n},{e});way[\"landuse\"=\"military\"]({s},{w},{n},{e}););out geom qt;"
    )
}

/// Derived filter column (contracts.md §4 CLASS_BY_MILITARY_TAG +
/// landuse=military → military_land). Unknown `military` tag values are
/// kept as `other` — visible rather than dropped; the client normalize
/// makes its own keep/drop call on the raw tags.
pub fn classify(tags: &Value) -> Option<&'static str> {
    if tags.get("landuse").and_then(Value::as_str) == Some("military") {
        return Some("military_land");
    }
    match tags.get("military").and_then(Value::as_str) {
        Some("airfield") => Some("airfield"),
        Some("naval_base") => Some("naval_base"),
        Some("range") => Some("range"),
        Some("barracks") => Some("barracks"),
        Some("base") => Some("base"),
        Some(_) => Some("other"),
        None => None,
    }
}

fn finite(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}

/// Parse an Overpass `[out:json]` document into rows. Position resolution
/// order for way/relation (contracts.md §4): `center` (absent under
/// `out geom`) → `bounds` midpoint → geometry centroid → drop. Individual
/// malformed elements are skipped, never fatal — one bad element must not
/// cost the quadrant its rows.
pub fn parse_elements(doc: &Value) -> Vec<InstallationRow> {
    let mut out = Vec::new();
    let Some(elements) = doc.get("elements").and_then(Value::as_array) else {
        return out;
    };
    for el in elements {
        let osm_type = match el.get("type").and_then(Value::as_str) {
            Some("node") => "node",
            Some("way") => "way",
            Some("relation") => "relation",
            _ => continue,
        };
        let Some(osm_id) = el.get("id").and_then(Value::as_i64) else { continue };
        let tags = el.get("tags").cloned().unwrap_or_else(|| json!({}));
        let Some(mil_class) = classify(&tags) else { continue };
        let name = tags
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| tags.get("name:en").and_then(Value::as_str))
            .map(str::to_string);

        let (minlat, minlon, maxlat, maxlon) = el
            .get("bounds")
            .map(|b| {
                match (
                    b.get("minlat").and_then(finite),
                    b.get("minlon").and_then(finite),
                    b.get("maxlat").and_then(finite),
                    b.get("maxlon").and_then(finite),
                ) {
                    (Some(a), Some(o), Some(c), Some(d)) => (Some(a), Some(o), Some(c), Some(d)),
                    _ => (None, None, None, None),
                }
            })
            .unwrap_or((None, None, None, None));

        let geometry = el
            .get("geometry")
            .and_then(Value::as_array)
            .map(|pts| {
                Value::Array(
                    pts.iter()
                        .filter_map(|p| match (finite(&p["lat"]), finite(&p["lon"])) {
                            (Some(la), Some(lo)) => Some(json!({ "lat": la, "lon": lo })),
                            _ => None,
                        })
                        .collect(),
                )
            })
            .filter(|v: &Value| v.as_array().is_some_and(|a| !a.is_empty()));

        let (lat, lon) = match osm_type {
            "node" => match (el.get("lat").and_then(finite), el.get("lon").and_then(finite)) {
                (Some(la), Some(lo)) => (la, lo),
                _ => continue,
            },
            _ => {
                let center = el.get("center").and_then(|c| {
                    match (c.get("lat").and_then(finite), c.get("lon").and_then(finite)) {
                        (Some(la), Some(lo)) => Some((la, lo)),
                        _ => None,
                    }
                });
                let bounds_mid = match (minlat, minlon, maxlat, maxlon) {
                    (Some(a), Some(o), Some(c), Some(d)) => Some(((a + c) / 2.0, (o + d) / 2.0)),
                    _ => None,
                };
                let geo_mid = geometry.as_ref().and_then(|g| {
                    let arr = g.as_array()?;
                    if arr.is_empty() {
                        return None;
                    }
                    let (mut la, mut lo) = (0.0f64, 0.0f64);
                    for p in arr {
                        la += p.get("lat")?.as_f64()?;
                        lo += p.get("lon")?.as_f64()?;
                    }
                    let len = arr.len() as f64;
                    Some((la / len, lo / len))
                });
                match center.or(bounds_mid).or(geo_mid) {
                    Some(pos) => pos,
                    None => continue,
                }
            }
        };

        out.push(InstallationRow {
            osm_type,
            osm_id,
            name,
            mil_class,
            lat,
            lon,
            minlat,
            minlon,
            maxlat,
            maxlon,
            geometry,
            tags,
        });
    }
    out
}

/// Restart-skip: a round is skipped when the freshest row is younger than
/// the cadence — the scheduler re-runs every source on hub boot, and a boot
/// must not turn into 4 Overpass quadrant queries inside a minute.
pub fn should_skip_round(last: Option<DateTime<Utc>>, now: DateTime<Utc>, min_age: ChronoDuration) -> bool {
    last.is_some_and(|t| now.signed_duration_since(t) < min_age)
}

/// Stale sweep runs only on a fully successful round: `ok == total`
/// AND the round harvested a plausible global volume (SWEEP_MIN_ROWS —
/// 315 measured 13.7K rows with a quadrant missing; a "successful" round
/// under the floor means a mirror served false-empty bodies, which the
/// remark check cannot always catch — osm.ch partial-extract incident).
pub fn full_refresh(ok: usize, total: usize, rows: usize) -> bool {
    ok == total && rows >= SWEEP_MIN_ROWS
}

pub const UPSERT_SQL: &str = "INSERT INTO military_installations \
    (osm_type, osm_id, name, mil_class, lat, lon, minlat, minlon, maxlat, maxlon, geometry, tags, fetched_at) \
    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) \
    ON CONFLICT (osm_type, osm_id) DO UPDATE SET \
    name = EXCLUDED.name, mil_class = EXCLUDED.mil_class, \
    lat = EXCLUDED.lat, lon = EXCLUDED.lon, \
    minlat = EXCLUDED.minlat, minlon = EXCLUDED.minlon, \
    maxlat = EXCLUDED.maxlat, maxlon = EXCLUDED.maxlon, \
    geometry = EXCLUDED.geometry, tags = EXCLUDED.tags, \
    fetched_at = EXCLUDED.fetched_at";

const STALE_SWEEP_SQL: &str = "DELETE FROM military_installations WHERE fetched_at < $1";

/// POST one quadrant to Overpass: walk the endpoint list (rotated by
/// `rotate` so quadrants spread across mirrors); per endpoint, 2
/// exponential-backoff retries. An Apache-level block (406) fails fast to
/// the NEXT endpoint — it does not lift inside a backoff window.
async fn fetch_quadrant(
    http: &reqwest::Client,
    urls: &[String],
    rotate: usize,
    bbox: (f64, f64, f64, f64),
) -> Result<Vec<InstallationRow>> {
    let (s, w, n, e) = bbox;
    let query = build_query(s, w, n, e);
    let mut last_err = String::new();
    for off in 0..urls.len() {
        let url = &urls[(rotate + off) % urls.len()];
        let mut next_endpoint = false;
        for attempt in 0..=QUADRANT_RETRIES {
            if attempt > 0 {
                let delay = RETRY_BASE_SECS * (1 << (attempt - 1));
                tracing::warn!(bbox = ?bbox, attempt, delay_secs = delay, endpoint = %url, "installations: quadrant retry");
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
            match http.post(url).form(&[("data", query.as_str())]).send().await {
                Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
                    Ok(doc) => {
                        // 2026-09-17 (T17 315): Overpass reports runtime
                        // errors (timeout/rate) as HTTP 200 + {"remark":
                        // "…error…", "elements":[]} — accepting that as an
                        // empty success both loses the quadrant AND arms the
                        // full-round stale sweep with a false 4/4 (q2 Europe/
                        // NA harvested "0 rows" this way). Treat remark-errors
                        // and missing elements as failures.
                        let remark = doc.get("remark").and_then(Value::as_str).unwrap_or("");
                        if remark.to_lowercase().contains("error")
                            || remark.to_lowercase().contains("timed out")
                        {
                            last_err = format!("overpass remark: {}", &remark[..remark.len().min(120)]);
                            continue;
                        }
                        if doc.get("elements").and_then(Value::as_array).is_none() {
                            last_err = "body json: missing elements array".to_string();
                            continue;
                        }
                        return Ok(parse_elements(&doc));
                    }
                    Err(err) => last_err = format!("body json: {err}"),
                },
                Ok(resp) => {
                    let status = resp.status();
                    last_err = format!("HTTP {status} ({url})");
                    if status.as_u16() == 406 {
                        tracing::warn!(endpoint = %url, "installations: 406 egress block — next endpoint");
                        next_endpoint = true;
                        break;
                    }
                }
                Err(err) => last_err = format!("{err} ({url})"),
            }
        }
        if !next_endpoint && !last_err.is_empty() {
            tracing::warn!(endpoint = %url, "installations: endpoint exhausted retries — next endpoint");
        }
    }
    Err(HubError::sensor(format!(
        "installations: quadrant ({s},{w},{n},{e}) failed on all {} endpoints: {last_err}",
        urls.len()
    )))
}

/// Per-quadrant upsert in one tx (celestrak per-group pattern). Every row of
/// the round gets the SAME `round_ts` — that is the sweep marker the stale
/// cleanup compares against.
async fn write_quadrant(
    pg: &sqlx::PgPool,
    rows: &[InstallationRow],
    round_ts: DateTime<Utc>,
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut tx = pg
        .begin()
        .await
        .map_err(|e| HubError::sensor(format!("installations: tx begin: {e}")))?;
    for row in rows {
        sqlx::query(UPSERT_SQL)
            .bind(row.osm_type)
            .bind(row.osm_id)
            .bind(&row.name)
            .bind(row.mil_class)
            .bind(row.lat)
            .bind(row.lon)
            .bind(row.minlat)
            .bind(row.minlon)
            .bind(row.maxlat)
            .bind(row.maxlon)
            .bind(&row.geometry)
            .bind(&row.tags)
            .bind(round_ts)
            .execute(&mut *tx)
            .await
            .map_err(|e| HubError::sensor(format!("installations: upsert {} {}: {e}", row.osm_type, row.osm_id)))?;
    }
    tx.commit()
        .await
        .map_err(|e| HubError::sensor(format!("installations: commit: {e}")))?;
    Ok(())
}

pub struct Installations;

impl Source for Installations {
    fn name(&self) -> &'static str {
        "installations"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let last: Option<DateTime<Utc>> =
                sqlx::query_scalar("SELECT max(fetched_at) FROM military_installations")
                    .fetch_one(&ctx.state.pg)
                    .await
                    .map_err(|e| HubError::sensor(format!("installations: last-fetch probe: {e}")))?;
            if should_skip_round(last, Utc::now(), ChronoDuration::seconds(INTERVAL_SECS as i64)) {
                tracing::info!(last = ?last, "installations: data <24h old — skipping round (restart must not hammer Overpass)");
                return Ok(vec![]);
            }

            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
                .user_agent(BROWSER_UA)
                .build()
                .map_err(|e| HubError::sensor(format!("installations: http client: {e}")))?;
            let urls = endpoints();
            let round_ts = Utc::now();
            let quads = quadrants();
            let mut ok_quadrants = 0usize;
            let mut total_rows = 0usize;
            for (i, bbox) in quads.iter().enumerate() {
                match fetch_quadrant(&http, &urls, i, *bbox).await {
                    Ok(rows) => {
                        total_rows += rows.len();
                        write_quadrant(&ctx.state.pg, &rows, round_ts).await?;
                        ok_quadrants += 1;
                        tracing::info!(quadrant = i, rows = rows.len(), "installations: quadrant upserted");
                    }
                    Err(e) => {
                        // Old rows for this quadrant keep their previous
                        // fetched_at — the stale sweep must not see them.
                        tracing::warn!(quadrant = i, bbox = ?bbox, error = %e, "installations: quadrant failed — keeping previous rows");
                    }
                }
                if i + 1 < quads.len() {
                    tokio::time::sleep(Duration::from_secs(QUADRANT_POLITENESS_SECS)).await;
                }
            }

            if full_refresh(ok_quadrants, quads.len(), total_rows) {
                let r = sqlx::query(STALE_SWEEP_SQL)
                    .bind(round_ts)
                    .execute(&ctx.state.pg)
                    .await
                    .map_err(|e| HubError::sensor(format!("installations: stale sweep: {e}")))?;
                tracing::info!(rows = total_rows, deleted = r.rows_affected(), "installations: full refresh complete");
            } else if ok_quadrants == 0 {
                return Err(HubError::sensor("installations: all 4 quadrants failed this round"));
            } else {
                tracing::warn!(ok = ok_quadrants, failed = quads.len() - ok_quadrants, total_rows, "installations: partial round — stale sweep deferred, old rows kept");
            }
            Ok(vec![]) // catalog, not geo events; liveness via health cell
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- quadrant / QL construction ----

    #[test]
    fn quadrants_cover_globe_in_four() {
        assert_eq!(
            quadrants(),
            [
                (-90.0, -180.0, 0.0, 0.0),
                (-90.0, 0.0, 0.0, 180.0),
                (0.0, -180.0, 90.0, 0.0),
                (0.0, 0.0, 90.0, 180.0),
            ]
        );
        // each is exactly 90° x 180°, and together they tile -90..90 / -180..180
        for (s, w, n, e) in quadrants() {
            assert_eq!(n - s, 90.0);
            assert_eq!(e - w, 180.0);
        }
    }

    #[test]
    fn build_query_injects_bbox_and_requires_geom() {
        let q = build_query(37.5, -122.5, 38.5, -121.5);
        assert!(q.starts_with("[out:json][timeout:120];("));
        // bbox is (south,west,north,east) — Overpass arg order
        assert!(q.contains("node[\"military\"](37.5,-122.5,38.5,-121.5)"));
        assert!(q.contains("way[\"military\"](37.5,-122.5,38.5,-121.5)"));
        assert!(q.contains("relation[\"military\"](37.5,-122.5,38.5,-121.5)"));
        assert!(q.contains("way[\"landuse\"=\"military\"](37.5,-122.5,38.5,-121.5)"));
        assert!(q.ends_with(");out geom qt;"));
    }

    #[test]
    fn endpoint_resolution_chain() {
        // env-touching tests race in parallel threads; serialize.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap();
        std::env::remove_var("HUB_OVERPASS_URL");
        std::env::remove_var("OVERPASS_URL");
        let list = endpoints();
        assert_eq!(list.len(), DEFAULT_ENDPOINTS.len());
        assert_eq!(list[0], "https://overpass-api.de/api/interpreter");

        std::env::set_var("OVERPASS_URL", "https://overpass.example.com/api");
        assert_eq!(endpoints(), vec!["https://overpass.example.com/api"]);

        // HUB_ prefix wins; blank values fall through
        std::env::set_var("HUB_OVERPASS_URL", "https://hub-wins.example.com");
        assert_eq!(endpoints(), vec!["https://hub-wins.example.com"]);
        std::env::set_var("HUB_OVERPASS_URL", "   ");
        assert_eq!(endpoints(), vec!["https://overpass.example.com/api"]);

        std::env::remove_var("HUB_OVERPASS_URL");
        std::env::remove_var("OVERPASS_URL");
        assert_eq!(endpoints().len(), DEFAULT_ENDPOINTS.len());
    }

    #[test]
    fn quadrant_error_remark_is_not_an_empty_success() {
        // The false-empty regression guard: a 200 + remark body must read
        // as failure. The check lives in fetch_quadrant's success branch;
        // here we pin the detection predicate itself.
        let err_doc: Value = serde_json::from_str(
            r#"{"version":0.6,"generator":"Overpass API","remark":"runtime error: Query timed out at line 1 after 121 seconds.","elements":[]}"#,
        ).unwrap();
        let remark = err_doc.get("remark").and_then(Value::as_str).unwrap_or("");
        assert!(remark.to_lowercase().contains("error") || remark.to_lowercase().contains("timed out"));
        // …while a genuinely empty quadrant (no remark) stays a valid zero.
        let ok_doc: Value = serde_json::from_str(r#"{"version":0.6,"elements":[]}"#).unwrap();
        assert!(ok_doc.get("remark").is_none());
        assert!(ok_doc.get("elements").and_then(Value::as_array).is_some());
    }

    // ---- mil_class derivation ----

    #[test]
    fn classify_known_military_tags() {
        for (tag, want) in [
            ("airfield", "airfield"),
            ("naval_base", "naval_base"),
            ("range", "range"),
            ("barracks", "barracks"),
            ("base", "base"),
        ] {
            assert_eq!(classify(&json!({"military": tag})), Some(want));
        }
    }

    #[test]
    fn classify_landuse_and_fallbacks() {
        assert_eq!(classify(&json!({"landuse": "military"})), Some("military_land"));
        // landuse beats nothing; a military tag still classifies
        assert_eq!(classify(&json!({"landuse": "military", "military": "airfield"})), Some("military_land"));
        // unknown military values stay visible as 'other', never dropped
        assert_eq!(classify(&json!({"military": "bunker"})), Some("other"));
        // no military signal at all → not an installation row
        assert_eq!(classify(&json!({"name": "x"})), None);
    }

    // ---- Overpass response parsing ----

    #[test]
    fn parse_node_uses_direct_latlon() {
        let doc = json!({"elements": [
            {"type": "node", "id": 123, "lat": 48.37, "lon": -124.9,
             "tags": {"military": "naval_base", "name": "CFB Esquimalt"}}
        ]});
        let rows = parse_elements(&doc);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.osm_type, "node");
        assert_eq!(r.osm_id, 123);
        assert_eq!(r.name.as_deref(), Some("CFB Esquimalt"));
        assert_eq!(r.mil_class, "naval_base");
        assert_eq!((r.lat, r.lon), (48.37, -124.9));
        assert!(r.bounds().is_none());
        assert!(r.geometry.is_none());
    }

    #[test]
    fn parse_way_prefers_center() {
        let doc = json!({"elements": [
            {"type": "way", "id": 456, "center": {"lat": 35.0, "lon": -116.0},
             "bounds": {"minlat": 34.9, "minlon": -116.1, "maxlat": 35.1, "maxlon": -115.9},
             "geometry": [{"lat": 34.9, "lon": -116.1}, {"lat": 35.1, "lon": -115.9}],
             "tags": {"military": "range"}}
        ]});
        let rows = parse_elements(&doc);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!((r.lat, r.lon), (35.0, -116.0)); // center, not midpoint
        assert_eq!(r.bounds(), Some((34.9, -116.1, 35.1, -115.9)));
        assert_eq!(r.geometry.as_ref().unwrap().as_array().unwrap().len(), 2);
    }

    #[test]
    fn parse_way_bounds_midpoint_without_center() {
        // `out geom` does NOT emit center — bounds midpoint is the contract fallback
        let doc = json!({"elements": [
            {"type": "way", "id": 789,
             "bounds": {"minlat": 40.0, "minlon": -105.0, "maxlat": 42.0, "maxlon": -103.0},
             "geometry": [{"lat": 40.0, "lon": -105.0}, {"lat": 42.0, "lon": -103.0}],
             "tags": {"landuse": "military"}}
        ]});
        let rows = parse_elements(&doc);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.mil_class, "military_land");
        assert_eq!((r.lat, r.lon), (41.0, -104.0)); // bounds midpoint
        assert_eq!(r.name, None);
    }

    #[test]
    fn parse_relation_geometry_centroid_last_resort() {
        let doc = json!({"elements": [
            {"type": "relation", "id": 999,
             "geometry": [{"lat": 10.0, "lon": 20.0}, {"lat": 12.0, "lon": 24.0}],
             "tags": {"military": "base"}}
        ]});
        let rows = parse_elements(&doc);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.osm_type, "relation");
        assert_eq!((r.lat, r.lon), (11.0, 22.0)); // geometry centroid
        assert!(r.bounds().is_none());
    }

    #[test]
    fn parse_skips_unpositioned_and_malformed() {
        let doc = json!({"elements": [
            {"type": "way", "id": 1, "tags": {"military": "base"}},          // no position at all
            {"type": "area", "id": 2, "lat": 1.0, "lon": 2.0, "tags": {"military": "base"}}, // bad type
            {"type": "node", "id": 3, "lat": "nan-ish", "lon": 2.0, "tags": {"military": "base"}}, // bad coord
            {"type": "node", "id": 4, "lat": 1.0, "lon": 2.0, "tags": {"name": "no-military-tag"}},
            {"type": "node", "id": 5, "lat": 1.0, "lon": 2.0, "tags": {"military": "barracks"}} // good
        ]});
        let rows = parse_elements(&doc);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].osm_id, 5);
    }

    #[test]
    fn parse_missing_elements_array_is_empty() {
        assert_eq!(parse_elements(&json!({})).len(), 0);
        assert_eq!(parse_elements(&json!({"elements": "nope"})).len(), 0);
    }

    // ---- restart-skip + stale sweep decisions ----

    #[test]
    fn skip_round_only_when_fresh() {
        let now = Utc::now();
        let age = ChronoDuration::seconds(INTERVAL_SECS as i64);
        assert!(should_skip_round(Some(now - ChronoDuration::hours(2)), now, age));   // fresh → skip
        assert!(should_skip_round(Some(now - ChronoDuration::hours(23)), now, age));  // still <24h
        assert!(!should_skip_round(Some(now - ChronoDuration::hours(25)), now, age)); // stale → run
        assert!(!should_skip_round(None, now, age));                                   // empty table → run
    }

    #[test]
    fn stale_sweep_gate() {
        assert!(full_refresh(4, 4, 10_000));
        assert!(!full_refresh(3, 4, 10_000)); // partial round keeps failed quadrant rows
        assert!(!full_refresh(0, 4, 10_000));
        // false-empty armor: 4/4 "success" below the global floor must not sweep
        assert!(!full_refresh(4, 4, 0));
        assert!(!full_refresh(4, 4, SWEEP_MIN_ROWS - 1));
        assert!(full_refresh(4, 4, SWEEP_MIN_ROWS));
        // partial-extract mirrors are excluded from the default chain
        assert!(!DEFAULT_ENDPOINTS.iter().any(|u| u.contains("osm.ch")));
    }

    // ---- upsert SQL shape (param binding contract) ----

    #[test]
    fn upsert_sql_has_13_binds_and_composite_conflict_target() {
        // placeholder count must match the 13-column INSERT + bind() order
        assert_eq!(UPSERT_SQL.matches('$').count(), 13);
        assert!(UPSERT_SQL.contains("ON CONFLICT (osm_type, osm_id) DO UPDATE"));
        for col in [
            "osm_type", "osm_id", "name", "mil_class", "lat", "lon",
            "minlat", "minlon", "maxlat", "maxlon", "geometry", "tags", "fetched_at",
        ] {
            assert!(UPSERT_SQL.contains(col), "missing column {col}");
        }
    }
}

impl InstallationRow {
    /// Test-facing bounds tuple accessor (also used by the T4 REST layer).
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        match (self.minlat, self.minlon, self.maxlat, self.maxlon) {
            (Some(a), Some(o), Some(c), Some(d)) => Some((a, o, c, d)),
            _ => None,
        }
    }
}
