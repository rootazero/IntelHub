//! GEV P3 T11: CCTV REST — camera catalog + health for the engine's
//! cctv layer (contracts.md §3).
//!
//! `GET /api/v1/gev/cctv/sources` → `{sources: [CameraSource]}` — the
//! engine's catalog.js consumes 19 camelCase fields; only id/lat/lon are
//! strictly required, everything else falls back to client-side defaults
//! (heading hash, fov 74, range 700m, …). The `url` field is the
//! feedConfigured flag (catalog.js:156): set iff the camera has any
//! real feed (frame or media). Actual frame/media bytes are proxied via
//! T12's `/api/v1/gev/cctv/frame/{id}` + `/media/{id}` — the engine
//! builds those URLs itself (source.js frameUrlFor/mediaUrlFor), so the
//! upstream URL never has to leave the hub.
//!
//! `GET /api/v1/gev/cctv/health` → `{cameras: [HealthRow]}` — health.js
//! consumes id/status/sourceKind/label/message/updatedAt; rows come from
//! the T9 `cctv-health` probe annotations. Cameras never probed are
//! reported `unknown` (health.js:24-36 tolerates that).
//!
//! T12 media proxy: `/frame/{id}` serves the camera's stored frame_url
//! through the hub (10s Redis cache, 15s upstream timeout, 8MB cap) and
//! `/media/{id}` streams mp4 (concurrency-capped 4 → 429, 30s timeout).
//! SSRF is structurally impossible: the client supplies only the camera
//! id; the fetch URL comes exclusively from the PG catalog (hub-curated
//! providers). Same-origin serving is also what allows the engine's
//! WebGL texture path to consume frames/video without canvas taint
//! (projection.js — the GPU rendering requirement). HLS media answers
//! 501 in P3: a playlist proxy needs segment-URL rewriting, deferred to
//! P4 (511NY cameras carry static frames too, so they degrade to image).

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::state::AppState;

#[derive(sqlx::FromRow)]
pub struct CamRow {
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
    pub health_status: Option<String>,
    pub health_checked_at: Option<DateTime<Utc>>,
}

/// Map a PG row to the client CameraSource shape. `url` = frame_url
/// preferred (still image always renderable), else media_url — presence
/// marks `feedConfigured` (catalog.js:156); the value itself is only
/// informational since frames are proxied by id.
pub fn camera_source_json(r: &CamRow) -> Value {
    let url = r.frame_url.clone().or_else(|| r.media_url.clone());
    let mut v = json!({
        "id": r.id,
        "city": r.city,
        "name": r.name,
        "lat": r.lat,
        "lon": r.lon,
        "feedType": r.feed_type,
        "provider": r.provider,
    });
    // Optional fields: omit when NULL rather than serializing `null` —
    // the client's defaults only engage on absent keys.
    if let Some(x) = &r.city_id { v["cityId"] = json!(x); }
    if let Some(x) = url { v["url"] = json!(x); }
    if let Some(x) = r.heading_deg { v["headingDeg"] = json!(x); }
    if let Some(x) = r.fov_deg { v["fovDeg"] = json!(x); }
    if let Some(x) = r.pitch_deg { v["pitchDeg"] = json!(x); }
    if let Some(x) = r.range_m { v["rangeM"] = json!(x); }
    if let Some(x) = r.mount_height_m { v["mountHeightM"] = json!(x); }
    if let Some(x) = r.ground_elevation_m { v["groundElevationM"] = json!(x); }
    if let Some(x) = &r.source_kind { v["sourceKind"] = json!(x); }
    if let Some(x) = &r.heading_confidence { v["headingConfidence"] = json!(x); }
    if let Some(x) = &r.pose_source { v["poseSource"] = json!(x); }
    if let Some(x) = &r.license_note { v["license"] = json!(x); }
    if let Some(x) = &r.credit { v["credit"] = json!(x); }
    if let Some(x) = &r.code { v["code"] = json!(x); }
    v
}

/// HealthRow (health.js:24-36): id required; status lowercased,
/// default "unknown"; label falls back to provider client-side.
pub fn health_row_json(r: &CamRow) -> Value {
    json!({
        "id": r.id,
        "status": r.health_status.as_deref().unwrap_or("unknown").to_lowercase(),
        "sourceKind": r.source_kind.as_deref().unwrap_or(""),
        "label": r.provider,
        "message": "",
        "updatedAt": r.health_checked_at.map(|t| t.timestamp_millis()).unwrap_or_else(|| Utc::now().timestamp_millis()),
    })
}

const SELECT_COLS: &str = "id, city, city_id, name, lat, lon, heading_deg, fov_deg, \
     pitch_deg, range_m, mount_height_m, ground_elevation_m, feed_type, frame_url, \
     media_url, provider, source_kind, heading_confidence, pose_source, license_note, \
     credit, code, health_status, health_checked_at";

pub async fn gev_cctv_sources(State(state): State<Arc<AppState>>) -> Result<Json<Value>, Response> {
    let rows = sqlx::query_as::<_, CamRow>(
        &format!("SELECT {SELECT_COLS} FROM cctv_cameras WHERE active ORDER BY provider, id"),
    )
    .fetch_all(&state.pg)
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "gev cctv sources query failed");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "cctv catalog unavailable"})),
        )
            .into_response()
    })?;
    Ok(Json(json!({ "sources": rows.iter().map(camera_source_json).collect::<Vec<_>>() })))
}

pub async fn gev_cctv_health(State(state): State<Arc<AppState>>) -> Result<Json<Value>, Response> {
    // Health for every ACTIVE camera that has a frame to probe; never
    // probed → "unknown" (client tolerates, health.js).
    let rows = sqlx::query_as::<_, CamRow>(
        &format!("SELECT {SELECT_COLS} FROM cctv_cameras WHERE active AND frame_url IS NOT NULL ORDER BY provider, id"),
    )
    .fetch_all(&state.pg)
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "gev cctv health query failed");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "cctv health unavailable"})),
        )
            .into_response()
    })?;
    Ok(Json(json!({ "cameras": rows.iter().map(health_row_json).collect::<Vec<_>>() })))
}

// ── T12: frame / media proxy ─────────────────────────────────────────────

/// Dedicated proxy client (OnceLock, gev_traffic.rs pattern). 15s: frame
/// hosts are small city servers; the shared 25s ceiling would let a hung
/// town-hall server pin the request too long for a 10s-refresh UI.
fn cctv_http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("IntelHub/1.0 (+cctv frame proxy)")
            .build()
            .expect("cctv proxy client builds")
    })
}

/// Frame cache TTL: matches the engine's ACTIVE_FRAME_REFRESH_MS=10000
/// (sourcePolicy.js) so every client refresh wave hits one upstream fetch.
pub const FRAME_CACHE_TTL_SECS: u64 = 10;
/// Hard cap on a proxied frame body (town-hall servers occasionally serve
/// multi-MB pngs; 8MB covers every observed case with headroom).
pub const FRAME_MAX_BYTES: usize = 8 * 1024 * 1024;
/// Media (mp4) proxy cap: concurrent streams + per-request timeout.
pub const MEDIA_MAX_CONCURRENT: usize = 4;
pub const MEDIA_TIMEOUT: Duration = Duration::from_secs(30);

fn frame_cache_key(id: &str) -> String {
    format!("hub:gev:cctv:frame:{id}")
}

/// Sanitize an upstream Content-Type to the image family the engine's
/// `new Image()` accepts; anything else (or missing) → jpeg default.
pub fn sanitize_frame_content_type(raw: Option<&str>) -> &'static str {
    match raw.map(|s| s.split(';').next().unwrap_or("").trim().to_lowercase()) {
        Some(ref t) if t == "image/png" => "image/png",
        Some(ref t) if t == "image/gif" => "image/gif",
        Some(ref t) if t == "image/webp" => "image/webp",
        _ => "image/jpeg",
    }
}

/// Look up the fetch URL for a camera id. The client NEVER supplies a
/// URL — SSRF is impossible by construction (catalog-curated hosts only).
async fn lookup_urls(
    state: &AppState,
    id: &str,
) -> Result<(Option<String>, Option<String>, String), Response> {
    sqlx::query_as::<_, (Option<String>, Option<String>, String)>(
        "SELECT frame_url, media_url, feed_type FROM cctv_cameras WHERE id = $1 AND active",
    )
    .bind(id)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "gev cctv url lookup failed");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "cctv catalog unavailable"})),
        )
            .into_response()
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown or inactive camera id"})),
        )
            .into_response()
    })
}

pub async fn gev_cctv_frame(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let (frame_url, _, _) = match lookup_urls(&state, &id).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(url) = frame_url else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "camera has no frame feed"})),
        )
            .into_response();
    };

    // Cache: Redis hash {data, ct} with TTL (binary-safe).
    let key = frame_cache_key(&id);
    let cached: Option<std::collections::HashMap<String, Vec<u8>>> = state
        .redis_timed(redis::cmd("HGETALL").arg(&key).clone(), 2000)
        .await;
    if let Some(map) = cached {
        if let (Some(data), Some(ct)) = (map.get("data"), map.get("ct")) {
            return (
                [(axum::http::header::CONTENT_TYPE, String::from_utf8_lossy(ct).to_string())],
                data.clone(),
            )
                .into_response();
        }
    }

    let resp = match cctv_http().get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e.to_string(), camera = %id, "cctv frame upstream fetch failed");
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "frame upstream failed"})),
            )
                .into_response();
        }
    };
    if !resp.status().is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("frame upstream HTTP {}", resp.status())})),
        )
            .into_response();
    }
    let ct = sanitize_frame_content_type(
        resp.headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
    );
    let body = match resp.bytes().await {
        Ok(b) if b.len() <= FRAME_MAX_BYTES => b,
        Ok(_) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "frame exceeds size cap"})),
            )
                .into_response();
        }
        Err(e) => {
            tracing::warn!(error = %e.to_string(), camera = %id, "cctv frame body read failed");
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "frame body read failed"})),
            )
                .into_response();
        }
    };

    // Best-effort cache write (degrade to plain proxy on Redis trouble,
    // starlink-proxy precedent).
    let _: Option<()> = state
        .redis_timed(
            redis::cmd("HSET")
                .arg(&key)
                .arg("data")
                .arg(body.as_ref())
                .arg("ct")
                .arg(ct)
                .clone(),
            2000,
        )
        .await;
    let _: Option<()> = state
        .redis_timed(
            redis::cmd("EXPIRE").arg(&key).arg(FRAME_CACHE_TTL_SECS).clone(),
            2000,
        )
        .await;

    ([(axum::http::header::CONTENT_TYPE, ct)], body).into_response()
}

/// Media concurrency guard: process-wide semaphore, 429 on saturation
/// (plan: 并发上限 4; frame path is cached and needs no cap).
static MEDIA_SLOTS: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();

fn media_slots() -> &'static tokio::sync::Semaphore {
    MEDIA_SLOTS.get_or_init(|| tokio::sync::Semaphore::new(MEDIA_MAX_CONCURRENT))
}

pub async fn gev_cctv_media(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let (_, media_url, feed_type) = match lookup_urls(&state, &id).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(url) = media_url else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "camera has no media feed"})),
        )
            .into_response();
    };
    // HLS playlist proxying needs segment-URL rewriting — deferred to P4.
    // (511NY hls cameras also carry static frames, so the layer still
    // shows them through the frame path.)
    if feed_type == "hls" {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "hls media proxy not implemented in P3"})),
        )
            .into_response();
    }
    let Ok(_permit) = media_slots().try_acquire() else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "media concurrency cap reached"})),
        )
            .into_response();
    };
    let resp = match tokio::time::timeout(MEDIA_TIMEOUT, cctv_http().get(&url).send()).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::warn!(error = %e.to_string(), camera = %id, "cctv media upstream fetch failed");
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "media upstream failed"})),
            )
                .into_response();
        }
        Err(_) => {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                Json(json!({"error": "media upstream timeout"})),
            )
                .into_response();
        }
    };
    if !resp.status().is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("media upstream HTTP {}", resp.status())})),
        )
            .into_response();
    }
    let ct = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("video/mp4")
        .to_string();
    let body = resp.bytes().await.unwrap_or_default();
    ([(axum::http::header::CONTENT_TYPE, ct)], body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> CamRow {
        CamRow {
            id: "tfl:JamCams_00002.00865".into(),
            city: "London".into(),
            city_id: Some("london".into()),
            name: "A406 Billet Upass E".into(),
            lat: 51.60067,
            lon: -0.01594,
            heading_deg: None,
            fov_deg: None,
            pitch_deg: None,
            range_m: None,
            mount_height_m: None,
            ground_elevation_m: None,
            feed_type: "mp4".into(),
            frame_url: Some("https://s3/x.jpg".into()),
            media_url: Some("https://s3/x.mp4".into()),
            provider: "tfl".into(),
            source_kind: Some("tfl-jamcam".into()),
            heading_confidence: Some("low".into()),
            pose_source: None,
            license_note: Some("Powered by TfL Open Data".into()),
            credit: Some("TfL JamCam".into()),
            code: None,
            health_status: Some("ok".into()),
            health_checked_at: None,
        }
    }

    #[test]
    fn camera_source_carries_contract_fields_and_url_flag() {
        let v = camera_source_json(&row());
        // required
        assert_eq!(v["id"], json!("tfl:JamCams_00002.00865"));
        assert_eq!(v["lat"], json!(51.60067));
        assert_eq!(v["lon"], json!(-0.01594));
        // feedConfigured flag: url = frame_url preferred
        assert_eq!(v["url"], json!("https://s3/x.jpg"));
        assert_eq!(v["feedType"], json!("mp4"));
        assert_eq!(v["cityId"], json!("london"));
        assert_eq!(v["license"], json!("Powered by TfL Open Data"));
        // NULL optionals are omitted, not null (client defaults engage)
        assert!(v.get("headingDeg").is_none());
        assert!(v.get("poseSource").is_none());
        assert!(v.get("code").is_none());
    }

    #[test]
    fn camera_source_omits_url_when_no_feed() {
        let mut r = row();
        r.frame_url = None;
        r.media_url = None;
        let v = camera_source_json(&r);
        assert!(v.get("url").is_none()); // feedConfigured stays false
    }

    #[test]
    fn camera_source_falls_back_to_media_url() {
        let mut r = row();
        r.frame_url = None; // no frame
        let v = camera_source_json(&r);
        assert_eq!(v["url"], json!("https://s3/x.mp4"));
    }

    #[test]
    fn health_row_shape_and_unknown_default() {
        let v = health_row_json(&row());
        assert_eq!(v["id"], json!("tfl:JamCams_00002.00865"));
        assert_eq!(v["status"], json!("ok"));
        assert_eq!(v["sourceKind"], json!("tfl-jamcam"));
        assert_eq!(v["label"], json!("tfl"));
        assert!(v["updatedAt"].is_i64());
        let mut r = row();
        r.health_status = None; // never probed
        assert_eq!(health_row_json(&r)["status"], json!("unknown"));
    }

    // ---- T12: frame/media proxy pure-function surface ----

    #[test]
    fn frame_content_type_sanitized_to_image_family() {
        assert_eq!(sanitize_frame_content_type(Some("image/png")), "image/png");
        assert_eq!(sanitize_frame_content_type(Some("image/jpeg; charset=binary")), "image/jpeg");
        assert_eq!(sanitize_frame_content_type(Some("text/html")), "image/jpeg");
        assert_eq!(sanitize_frame_content_type(Some("application/octet-stream")), "image/jpeg");
        assert_eq!(sanitize_frame_content_type(None), "image/jpeg");
    }

    #[test]
    fn frame_cache_key_namespaced_per_camera() {
        assert_eq!(frame_cache_key("tfl:x"), "hub:gev:cctv:frame:tfl:x");
        assert_eq!(media_slots().available_permits(), MEDIA_MAX_CONCURRENT);
    }

    #[tokio::test]
    async fn media_semaphore_429s_beyond_cap() {
        let sem = media_slots();
        let permits: Vec<_> = (0..MEDIA_MAX_CONCURRENT)
            .map(|_| sem.try_acquire().expect("slot available"))
            .collect();
        assert!(sem.try_acquire().is_err()); // 5th → 429 path
        drop(permits);
        assert!(sem.try_acquire().is_ok());
    }
}
