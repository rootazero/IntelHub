//! CCTV static catalog loader (GEV P3 T8, 2026-09-17).
//!
//! One-shot base load: vendor static camera directories
//! `console/gev-engine/config/cctv_sources.<city>.json` → PG
//! `cctv_cameras` (migration 0021). Contracts.md §3 defines the
//! CameraSource field set the client projection pipeline consumes;
//! this loader persists that catalog server-side so the T9 REST layer
//! (`/api/cctv/sources`) and the later periodic upstream refresh
//! (511on / TfL / 511NY) share one table.
//!
//! Cadence note: registered as a Source with a 24h interval only
//! because the scheduler drives sources on that cadence — the upsert
//! is fully idempotent, so re-runs are harmless. This is deliberately
//! NOT a periodic upstream collector; T9 adds the periodic refresh.
//!
//! Failure policy:
//! - config dir missing / a file unreadable / a file malformed →
//!   `tracing::warn!` + skip, NEVER fatal (the catalog is a
//!   vendor-pure convenience; the hub must boot without it);
//! - the stale sweep (`DELETE WHERE fetched_at < round_ts`) runs only
//!   after a fully parsed 4/4 directory — a partial read must never
//!   wipe the table (installations.rs precedent);
//! - per-city row counts + elapsed ms land in a supplemental health
//!   cell field `cctv-loader:catalog` next to the scheduler-written
//!   `hub:monitor:health` field `cctv-loader` (distinct field, so the
//!   scheduler's own report_health pass never clobbers it).
//!
//! Env: `GEV_ENGINE_CONFIG_DIR` wins; default is the deployment-root
//! relative path (config.rs env_or pattern, same as HUB_CONSOLE_DIR).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::{json, Value};

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

/// Placeholder cadence (see module docstring): the load is idempotent, so
/// a re-run every 24h is a no-op reconcile, not a re-seed storm.
pub const INTERVAL_SECS: u64 = 24 * 3600;
/// Deployment-root-relative default (env_or pattern, config.rs precedent).
const DEFAULT_CONFIG_DIR: &str = "/home/zou/IntelHub/console/gev-engine/config";
/// The four vendor static directories (contracts.md §3 upstream pack).
const CITIES: [&str; 4] = ["austin", "shinjuku", "tallinn", "warendorf"];
/// Supplemental health-cell field — distinct from the scheduler-written
/// `cctv-loader` field so report_health never overwrites the detail.
const HEALTH_FIELD: &str = "cctv-loader:catalog";

pub const UPSERT_SQL: &str = "INSERT INTO cctv_cameras \
    (id, city, city_id, name, lat, lon, heading_deg, fov_deg, pitch_deg, range_m, \
     mount_height_m, ground_elevation_m, feed_type, frame_url, media_url, provider, \
     source_kind, heading_confidence, pose_source, license_note, credit, code, fetched_at) \
    VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23) \
    ON CONFLICT (id) DO UPDATE SET \
    city = EXCLUDED.city, city_id = EXCLUDED.city_id, name = EXCLUDED.name, \
    lat = EXCLUDED.lat, lon = EXCLUDED.lon, \
    heading_deg = EXCLUDED.heading_deg, fov_deg = EXCLUDED.fov_deg, \
    pitch_deg = EXCLUDED.pitch_deg, range_m = EXCLUDED.range_m, \
    mount_height_m = EXCLUDED.mount_height_m, \
    ground_elevation_m = EXCLUDED.ground_elevation_m, \
    feed_type = EXCLUDED.feed_type, frame_url = EXCLUDED.frame_url, \
    media_url = EXCLUDED.media_url, provider = EXCLUDED.provider, \
    source_kind = EXCLUDED.source_kind, \
    heading_confidence = EXCLUDED.heading_confidence, \
    pose_source = EXCLUDED.pose_source, license_note = EXCLUDED.license_note, \
    credit = EXCLUDED.credit, code = EXCLUDED.code, \
    fetched_at = EXCLUDED.fetched_at";

const STALE_SWEEP_SQL: &str = "DELETE FROM cctv_cameras WHERE fetched_at < $1";

/// One normalized row of `cctv_cameras`.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraRow {
    pub id: String,
    pub city: String,
    pub city_id: Option<String>,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub heading_deg: Option<f32>,
    pub fov_deg: Option<f32>,
    pub pitch_deg: Option<f32>,
    pub range_m: Option<f32>,
    pub mount_height_m: Option<f32>,
    pub ground_elevation_m: Option<f32>,
    pub feed_type: String,
    pub frame_url: Option<String>,
    pub media_url: Option<String>,
    pub provider: String,
    pub source_kind: Option<String>,
    pub heading_confidence: Option<String>,
    pub pose_source: Option<String>,
    pub license_note: Option<String>,
    pub credit: Option<String>,
    pub code: Option<String>,
}

/// Config dir resolution: `GEV_ENGINE_CONFIG_DIR` → deployment default.
pub fn config_dir() -> PathBuf {
    let v = std::env::var("GEV_ENGINE_CONFIG_DIR").unwrap_or_default();
    let v = v.trim();
    if v.is_empty() {
        PathBuf::from(DEFAULT_CONFIG_DIR)
    } else {
        PathBuf::from(v)
    }
}

/// feedType normalization (contracts.md §3, catalog.js:153-155 +
/// model.js:73-91): alias map into the image/mjpeg/mp4/hls/webm
/// vocabulary — mjpg→mjpeg, jpeg/jpg/png/gif→image, video→mp4,
/// stream→hls — case-insensitive, default (and unknown-value fallback)
/// 'image'.
pub fn normalize_feed_type(raw: Option<&str>) -> &'static str {
    match raw.unwrap_or("").trim().to_lowercase().as_str() {
        "" | "jpeg" | "jpg" | "png" | "gif" => "image",
        "mjpg" | "mjpeg" => "mjpeg",
        "video" | "mp4" => "mp4",
        "stream" | "hls" => "hls",
        "webm" => "webm",
        // unknown values fall back to image rather than inventing a type
        _ => "image",
    }
}

/// Feed types served by a `<video>` element on the client
/// (model.js isVideoFeedType): those belong in `media_url`; everything
/// else is a static frame → `frame_url`.
pub fn is_video_feed_type(feed_type: &str) -> bool {
    matches!(feed_type, "mp4" | "hls" | "webm" | "mjpeg")
}

fn finite_f64(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|f| f.is_finite())
}

fn finite_f32(v: Option<&Value>) -> Option<f32> {
    finite_f64(v).map(|f| f as f32)
}

/// First non-empty string among alias keys (`sourceKind`/`kind`,
/// `feedType`/`type`, `license`/`licenseNote` per contracts.md §3).
fn str_field<'v>(obj: &'v Value, keys: &[&str]) -> Option<&'v str> {
    keys.iter()
        .filter_map(|k| obj.get(k).and_then(Value::as_str))
        .find(|s| !s.trim().is_empty())
}

/// Lenient document parse: top-level array OR `{"cameras": [...]}`
/// (contract-tolerant; austin ships `[]`, others ship bare arrays, but
/// the envelope shape costs nothing to accept). `city_stem` is the
/// file-name stem (e.g. "tallinn") used as fallback when an entry
/// carries no city/cityId of its own (warendorf.json does exactly
/// that). Entries without a usable id or finite lat/lon are dropped —
/// one bad row must not cost the file its rows.
pub fn parse_cameras(doc: &Value, city_stem: &str) -> Vec<CameraRow> {
    let entries: Option<&Vec<Value>> = doc
        .as_array()
        .or_else(|| doc.get("cameras").and_then(Value::as_array));
    let Some(entries) = entries else { return Vec::new() };
    let mut out = Vec::new();
    for cam in entries {
        let Some(id) = str_field(cam, &["id"]).map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let (Some(lat), Some(lon)) = (
            finite_f64(cam.get("lat").or_else(|| cam.get("latitude"))),
            finite_f64(cam.get("lon").or_else(|| cam.get("longitude"))),
        ) else {
            continue;
        };
        let city_id = str_field(cam, &["cityId", "city_id"])
            .map(str::to_string)
            .or_else(|| Some(city_stem.to_string()));
        let city = str_field(cam, &["city"])
            .map(str::to_string)
            .unwrap_or_else(|| city_id.clone().unwrap_or_else(|| city_stem.to_string()));
        // Provider marks static-catalog provenance (T8 spec): the
        // upstream's own provider string lives on in license/credit.
        let provider = format!("static-{}", city_id.as_deref().unwrap_or(city_stem));
        let feed_type = normalize_feed_type(str_field(cam, &["feedType", "type"]));
        let url = str_field(cam, &["url", "snapshotUrl"]).map(str::to_string);
        let (frame_url, media_url) = match (url, is_video_feed_type(feed_type)) {
            (Some(u), true) => (None, Some(u)),
            (Some(u), false) => (Some(u), None),
            (None, _) => (None, None),
        };
        out.push(CameraRow {
            id: id.to_string(),
            city,
            city_id,
            name: str_field(cam, &["name"])
                .map(str::to_string)
                .unwrap_or_else(|| id.to_string()),
            lat,
            lon,
            heading_deg: finite_f32(cam.get("headingDeg")),
            fov_deg: finite_f32(cam.get("fovDeg")),
            pitch_deg: finite_f32(cam.get("pitchDeg")),
            range_m: finite_f32(cam.get("rangeM")),
            mount_height_m: finite_f32(cam.get("mountHeightM")),
            ground_elevation_m: finite_f32(cam.get("groundElevationM")),
            feed_type: feed_type.to_string(),
            frame_url,
            media_url,
            provider,
            source_kind: str_field(cam, &["sourceKind", "kind"]).map(str::to_string),
            heading_confidence: str_field(cam, &["headingConfidence"]).map(|s| s.to_lowercase()),
            pose_source: str_field(cam, &["poseSource"]).map(str::to_string),
            license_note: str_field(cam, &["license", "licenseNote"]).map(str::to_string),
            credit: str_field(cam, &["credit"]).map(str::to_string),
            code: str_field(cam, &["code"]).map(str::to_string),
        });
    }
    out
}

/// Read every `cctv_sources.<city>.json` under `dir`.
///
/// Returns `None` when the directory itself is missing (caller degrades,
/// never fails — the hub must boot without the vendor tree). Missing /
/// unreadable / malformed individual files warn and skip; the caller
/// decides from the returned `(city, rows)` list whether the round was
/// complete enough to stale-sweep.
pub fn read_catalog(dir: &Path) -> Option<Vec<(String, Vec<CameraRow>)>> {
    if !dir.is_dir() {
        return None;
    }
    let mut out = Vec::new();
    for city in CITIES {
        let path = dir.join(format!("cctv_sources.{city}.json"));
        if !path.is_file() {
            tracing::warn!(path = %path.display(), "cctv-loader: catalog file missing — skipping");
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "cctv-loader: catalog file unreadable — skipping");
                continue;
            }
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(doc) => out.push((city.to_string(), parse_cameras(&doc, city))),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "cctv-loader: catalog file malformed — skipping");
            }
        }
    }
    Some(out)
}

async fn write_catalog_cell(ctx: &Ctx, cell: Value) {
    let _: Option<()> = ctx
        .state
        .redis_timed(
            redis::cmd("HSET")
                .arg("hub:monitor:health")
                .arg(HEALTH_FIELD)
                .arg(cell.to_string())
                .clone(),
            2000,
        )
        .await;
}

pub struct CctvLoader;

impl Source for CctvLoader {
    fn name(&self) -> &'static str {
        "cctv-loader"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let started = Instant::now();
            let dir = config_dir();
            let Some(files) = read_catalog(&dir) else {
                tracing::warn!(
                    dir = %dir.display(),
                    "cctv-loader: config dir missing — static catalog load skipped (degraded-by-design)"
                );
                write_catalog_cell(
                    ctx,
                    json!({
                        "state": "degraded",
                        "detail": format!("config dir missing: {}", dir.display()),
                        "ts": Utc::now().to_rfc3339(),
                    }),
                )
                .await;
                return Ok(vec![]); // catalog, not geo events
            };

            let round_ts: DateTime<Utc> = Utc::now();
            let mut tx = ctx
                .state
                .pg
                .begin()
                .await
                .map_err(|e| HubError::sensor(format!("cctv-loader: tx begin: {e}")))?;
            let mut per_city = serde_json::Map::new();
            let mut total = 0usize;
            for (city, rows) in &files {
                total += rows.len();
                per_city.insert(city.clone(), json!(rows.len()));
                for row in rows {
                    sqlx::query(UPSERT_SQL)
                        .bind(&row.id)
                        .bind(&row.city)
                        .bind(&row.city_id)
                        .bind(&row.name)
                        .bind(row.lat)
                        .bind(row.lon)
                        .bind(row.heading_deg)
                        .bind(row.fov_deg)
                        .bind(row.pitch_deg)
                        .bind(row.range_m)
                        .bind(row.mount_height_m)
                        .bind(row.ground_elevation_m)
                        .bind(&row.feed_type)
                        .bind(&row.frame_url)
                        .bind(&row.media_url)
                        .bind(&row.provider)
                        .bind(&row.source_kind)
                        .bind(&row.heading_confidence)
                        .bind(&row.pose_source)
                        .bind(&row.license_note)
                        .bind(&row.credit)
                        .bind(&row.code)
                        .bind(round_ts)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| HubError::sensor(format!("cctv-loader: upsert {}: {e}", row.id)))?;
                }
            }
            // Stale sweep only on a fully parsed 4/4 directory: a partial
            // round (missing/malformed file) must never wipe good rows.
            let full_round = files.len() == CITIES.len();
            if full_round {
                let r = sqlx::query(STALE_SWEEP_SQL)
                    .bind(round_ts)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| HubError::sensor(format!("cctv-loader: stale sweep: {e}")))?;
                tracing::info!(deleted = r.rows_affected(), "cctv-loader: stale sweep");
            } else {
                tracing::warn!(
                    parsed = files.len(),
                    expected = CITIES.len(),
                    "cctv-loader: partial catalog read — stale sweep deferred"
                );
            }
            tx.commit()
                .await
                .map_err(|e| HubError::sensor(format!("cctv-loader: commit: {e}")))?;

            let elapsed_ms = started.elapsed().as_millis() as u64;
            tracing::info!(total, ?per_city, elapsed_ms, "cctv-loader: static catalog upserted");
            write_catalog_cell(
                ctx,
                json!({
                    "state": "ok",
                    "loaded_at": round_ts.to_rfc3339(),
                    "elapsed_ms": elapsed_ms,
                    "total": total,
                    "per_city": per_city,
                }),
            )
            .await;
            Ok(vec![]) // catalog, not geo events (celestrak precedent)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- fixtures: one real entry per vendor file (2026-09-17) ----

    const AUSTIN_DOC: &str = "[]";

    const SHINJUKU_ENTRY: &str = r#"{
        "id": "tokyo-shinjuku-east-1",
        "name": "Shinjuku Crossing East Cam",
        "city": "Tokyo",
        "cityId": "tokyo",
        "provider": "Pilot Feed Pack",
        "sourceKind": "pilot",
        "feedType": "mp4",
        "url": "https://storage.googleapis.com/gtv-videos-bucket/sample/ForBiggerEscapes.mp4",
        "lat": 35.689614,
        "lon": 139.700523,
        "headingDeg": 242,
        "pitchDeg": -19,
        "fovDeg": 66,
        "rangeM": 560,
        "mountHeightM": 29,
        "groundElevationM": 40,
        "license": "Demo sample stream for projection pipeline testing"
    }"#;

    const TALLINN_ENTRY: &str = r#"{
        "id": "tln-103",
        "name": "Viru väljak (suund Mere pst ja Narva mnt)",
        "city": "Tallinn",
        "cityId": "tallinn",
        "provider": "City of Tallinn",
        "lat": 59.436563,
        "lon": 24.752702,
        "headingDeg": 60.1,
        "headingConfidence": "LOW",
        "pitchDeg": -18,
        "fovDeg": 44,
        "rangeM": 145,
        "mountHeightM": 8,
        "groundElevationM": 15,
        "feedType": "image",
        "url": "https://ristmikud.tallinn.ee/last/cam103.jpg",
        "snapshotUrl": "https://ristmikud.tallinn.ee/last/cam103.jpg",
        "sourceKind": "tallinn-ristmikud",
        "license": "Public City of Tallinn traffic camera frame (ristmikud.tallinn.ee)",
        "poseSource": "curated"
    }"#;

    // warendorf.json entries carry NO city/cityId — the file stem is the fallback.
    const WARENDORF_ENTRY: &str = r#"{
        "id": "warendorf-marktplatz-rathaus",
        "name": "Marktplatz / Historisches Rathaus",
        "provider": "Stadt Warendorf",
        "sourceKind": "municipal-webcam",
        "feedType": "image",
        "url": "http://webcam.warendorf.de/image/jpeg.cgi",
        "lat": 51.9526613,
        "lon": 7.9908867,
        "headingDeg": 221,
        "headingConfidence": "high",
        "pitchDeg": -23,
        "fovDeg": 84,
        "rangeM": 260,
        "mountHeightM": 14,
        "groundElevationM": 55,
        "poseSource": "curated",
        "license": "Public municipal webcam data — Stadt Warendorf (courtesy)."
    }"#;

    fn parse_one(entry: &str, stem: &str) -> CameraRow {
        let doc: Value = serde_json::from_str(entry).unwrap();
        let rows = parse_cameras(&json!([doc]), stem);
        assert_eq!(rows.len(), 1);
        rows.into_iter().next().unwrap()
    }

    // ---- real-file fixtures ----

    #[test]
    fn parse_austin_empty_array() {
        let doc: Value = serde_json::from_str(AUSTIN_DOC).unwrap();
        assert!(parse_cameras(&doc, "austin").is_empty());
    }

    #[test]
    fn parse_shinjuku_video_feed() {
        let r = parse_one(SHINJUKU_ENTRY, "shinjuku");
        assert_eq!(r.id, "tokyo-shinjuku-east-1");
        assert_eq!(r.city, "Tokyo");
        assert_eq!(r.city_id.as_deref(), Some("tokyo"));
        assert_eq!(r.feed_type, "mp4");
        // video feeds land in media_url, frame_url stays None
        assert!(r.media_url.is_some_and(|u| u.ends_with("ForBiggerEscapes.mp4")));
        assert!(r.frame_url.is_none());
        assert!(is_video_feed_type(&r.feed_type));
        // provider marks static-catalog provenance, not the upstream name
        assert_eq!(r.provider, "static-tokyo");
        // license passes through into license_note
        assert!(r.license_note.is_some_and(|l| l.contains("projection pipeline")));
    }

    #[test]
    fn parse_tallinn_image_feed_lowercases_confidence() {
        let r = parse_one(TALLINN_ENTRY, "tallinn");
        assert_eq!(r.id, "tln-103");
        assert_eq!(r.feed_type, "image");
        assert!(r.frame_url.as_ref().is_some_and(|u| u.contains("cam103.jpg")));
        assert!(r.media_url.is_none());
        assert!(!is_video_feed_type(&r.feed_type));
        assert_eq!(r.provider, "static-tallinn");
        // headingConfidence + poseSource pass through (contract: 'curated' → CAL badge)
        assert_eq!(r.heading_confidence.as_deref(), Some("low")); // "LOW" → lowercase
        assert_eq!(r.pose_source.as_deref(), Some("curated"));
        // url wins over snapshotUrl (both present here, identical)
        assert_eq!(r.frame_url.as_deref(), Some("https://ristmikud.tallinn.ee/last/cam103.jpg"));
    }

    #[test]
    fn parse_warendorf_falls_back_to_file_stem() {
        let r = parse_one(WARENDORF_ENTRY, "warendorf");
        assert_eq!(r.city, "warendorf"); // no city field → stem
        assert_eq!(r.city_id.as_deref(), Some("warendorf"));
        assert_eq!(r.provider, "static-warendorf");
        assert_eq!(r.source_kind.as_deref(), Some("municipal-webcam"));
        assert_eq!(r.heading_confidence.as_deref(), Some("high"));
    }

    #[test]
    fn parse_accepts_cameras_envelope() {
        let doc = json!({"cameras": [serde_json::from_str::<Value>(TALLINN_ENTRY).unwrap()]});
        assert_eq!(parse_cameras(&doc, "tallinn").len(), 1);
    }

    // ---- feedType normalization (contracts.md §3) ----

    #[test]
    fn feed_type_alias_map() {
        assert_eq!(normalize_feed_type(None), "image");
        assert_eq!(normalize_feed_type(Some("")), "image");
        assert_eq!(normalize_feed_type(Some("mjpg")), "mjpeg");
        for alias in ["jpeg", "jpg", "png", "gif"] {
            assert_eq!(normalize_feed_type(Some(alias)), "image", "{alias} → image");
        }
        assert_eq!(normalize_feed_type(Some("video")), "mp4");
        assert_eq!(normalize_feed_type(Some("stream")), "hls");
        // pass-through vocabulary, case-insensitive
        for canonical in ["image", "mjpeg", "mp4", "hls", "webm"] {
            assert_eq!(normalize_feed_type(Some(canonical)), canonical);
            assert_eq!(normalize_feed_type(Some(&canonical.to_uppercase())), canonical);
        }
        // unknown values fall back to image rather than inventing a type
        assert_eq!(normalize_feed_type(Some("rtsp")), "image");
    }

    // ---- robustness: bad rows never kill the file ----

    #[test]
    fn parse_drops_entries_without_id_or_position() {
        let doc = json!([
            {"name": "no id", "lat": 1.0, "lon": 2.0},
            {"id": "", "lat": 1.0, "lon": 2.0},
            {"id": "no-lat", "lon": 2.0},
            {"id": "no-lon", "lat": 1.0},
            {"id": "nan-lat", "lat": "NaN", "lon": 2.0},
            {"id": "good", "lat": 59.4, "lon": 24.7}
        ]);
        let rows = parse_cameras(&doc, "tallinn");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "good");
    }

    #[test]
    fn parse_missing_top_level_array_is_empty() {
        assert!(parse_cameras(&json!({}), "tallinn").is_empty());
        assert!(parse_cameras(&json!({"cameras": "nope"}), "tallinn").is_empty());
    }

    // ---- idempotency: same input → identical rows (upsert is conflict-keyed) ----

    #[test]
    fn parse_is_deterministic_for_idempotent_upsert() {
        let doc: Value = serde_json::from_str(&format!("[{SHINJUKU_ENTRY},{TALLINN_ENTRY}]")).unwrap();
        let a = parse_cameras(&doc, "shinjuku");
        let b = parse_cameras(&doc, "shinjuku");
        assert_eq!(a, b);
        assert!(UPSERT_SQL.contains("ON CONFLICT (id) DO UPDATE"));
    }

    // ---- upsert SQL shape (param binding contract) ----

    #[test]
    fn upsert_sql_has_23_binds_and_id_conflict_target() {
        assert_eq!(UPSERT_SQL.matches('$').count(), 23);
        assert!(UPSERT_SQL.contains("ON CONFLICT (id) DO UPDATE"));
        for col in [
            "id", "city", "city_id", "name", "lat", "lon", "heading_deg", "fov_deg",
            "pitch_deg", "range_m", "mount_height_m", "ground_elevation_m", "feed_type",
            "frame_url", "media_url", "provider", "source_kind", "heading_confidence",
            "pose_source", "license_note", "credit", "code", "fetched_at",
        ] {
            assert!(UPSERT_SQL.contains(col), "missing column {col}");
        }
    }

    // ---- read_catalog degradation ----

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ih-cctv-loader-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_catalog_missing_dir_returns_none() {
        let dir = std::env::temp_dir().join(format!("ih-cctv-loader-never-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_catalog(&dir).is_none());
    }

    #[test]
    fn read_catalog_parses_array_and_envelope_shapes() {
        let dir = temp_dir("happy");
        std::fs::write(
            dir.join("cctv_sources.austin.json"),
            AUSTIN_DOC,
        )
        .unwrap();
        std::fs::write(
            dir.join("cctv_sources.tallinn.json"),
            json!({"cameras": [serde_json::from_str::<Value>(TALLINN_ENTRY).unwrap()]}).to_string(),
        )
        .unwrap();
        // shinjuku present but malformed, warendorf missing → partial round
        std::fs::write(dir.join("cctv_sources.shinjuku.json"), "{not json").unwrap();
        let files = read_catalog(&dir).expect("dir exists → Some");
        let by_city: std::collections::HashMap<_, _> = files.into_iter().collect();
        assert_eq!(by_city.len(), 2); // malformed + missing files skipped
        assert!(by_city["austin"].is_empty());
        assert_eq!(by_city["tallinn"].len(), 1);
        assert_eq!(by_city["tallinn"][0].id, "tln-103");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
