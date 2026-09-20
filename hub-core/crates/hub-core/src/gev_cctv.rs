//! GEV P3 T11+P11: CCTV REST — camera catalog + health for the engine's
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
//!
//! P11 T4 frame fallback chain (gev_cctv_frame):
//!   1. Redis cache (existing, no change)
//!   2. Upstream fetch (existing, no change)
//!   3. Street View Static API (new, env-gated, never 502s when key absent)
//!   4. Synthetic SVG placeholder (new, always succeeds)
//! Fallback tiers 3-4 do NOT write to Redis cache — only a successful
//! upstream response is cached (avoids polluting the upstream namespace).

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::state::AppState;

use super::gev_cctv_frame_fallback::{build_synthetic_svg, street_view_fallback};

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

/// TxDOT ITS upstream returns a JSON envelope, NOT image bytes directly
/// (per the upstream `fetchTxdotSnapshot` reference in
/// `gods-eye-view/server/providers/cctv/media.js`). The envelope shape
/// is `{icd_Id: "...", snippet: "<base64 JPEG>"}` and `snippet` may
/// carry an optional `data:image/jpeg;base64,` URI prefix.
///
/// This helper:
/// 1. Extracts the base64 string (strips URI prefix if present)
/// 2. Validates canonical base64 (rejects junk characters that the
///    decoder would silently skip)
/// 3. Decodes and enforces JPEG magic bytes (`FF D8 FF`) so the proxy
///    never serves a non-image body with the `image/jpeg` content-type
///    label
///
/// Errors are returned to the caller (which falls through to the
/// Street View / SVG fallback chain) — never silently passed through.
pub fn parse_txdot_envelope(raw_json: &Value) -> std::result::Result<Vec<u8>, String> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine as _;

    let snippet = raw_json
        .get("snippet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "txdot envelope: missing or empty snippet field".to_string())?;

    // Strip optional `data:image/jpeg;base64,` prefix.
    let b64 = if let Some(rest) = snippet.strip_prefix("data:") {
        rest.split_once(';')
            .and_then(|(_, tail)| tail.strip_prefix("base64,"))
            .unwrap_or(rest)
    } else {
        snippet
    };

    // Canonical base64 (groups of 4, padding only at end). Buffer.from()
    // silently skips junk — a permissive check would let non-image
    // bodies decode into "something" with `image/jpeg` on the wire.
    if !b64.chars().all(|c| matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' | '='))
        || b64.contains('=') && !b64.ends_with('=')
    {
        return Err("txdot envelope: snippet is not canonical base64".to_string());
    }
    let decoded = B64
        .decode(b64)
        .map_err(|e| format!("txdot envelope: base64 decode: {e}"))?;

    if decoded.len() < 4 {
        return Err(format!(
            "txdot envelope: decoded body too small ({} bytes)",
            decoded.len()
        ));
    }
    if decoded[0] != 0xFF || decoded[1] != 0xD8 || decoded[2] != 0xFF {
        return Err("txdot envelope: decoded body lacks JPEG magic bytes".to_string());
    }
    Ok(decoded)
}

/// Content-type + magic-byte guard for non-TxDOT upstreams.
///
/// Browsers refuse to render an `<img>` whose bytes do not match the
/// declared content-type. Upstreams like NSW livetraffic have been
/// observed to return HTTP 200 + `text/html` "Page not found" when a
/// camera URL goes stale — passing that through with a coerced
/// `image/jpeg` content-type is the original black-screen bug.
///
/// Required:
/// - `content_type` starts with `image/`
/// - body starts with the magic bytes of the declared image family
///   (JPEG `FF D8 FF`, PNG `89 50 4E 47`, GIF `47 49 46 38`, WEBP `RIFF...WEBP`)
pub fn accept_upstream_body_for_browser(content_type: Option<&str>, body: &[u8]) -> bool {
    let Some(ct) = content_type else {
        return false;
    };
    let family = ct.split(';').next().unwrap_or("").trim().to_lowercase();
    match family.as_str() {
        "image/jpeg" => body.len() >= 3 && body[0] == 0xFF && body[1] == 0xD8 && body[2] == 0xFF,
        "image/png" => body.len() >= 4 && body[..4] == [0x89, b'P', b'N', b'G'],
        "image/gif" => body.len() >= 3 && body[..3] == [b'G', b'I', b'F'],
        "image/webp" => body.len() >= 12 && body[..4] == [b'R', b'I', b'F', b'F']
            && body[8..12] == [b'W', b'E', b'B', b'P'],
        _ => false,
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

/// Source-kind-aware frame lookup. Returns `(frame_url, source_kind)` so
/// the proxy can route TxDOT ITS (JSON envelope) through `parse_txdot_envelope`
/// before serving to the client — the upstream gods-eye-view cctv/media.js
/// reference gates `fetchTxdotSnapshot` on `source.sourceKind === 'txdot-its'`.
async fn lookup_frame_target(
    state: &AppState,
    id: &str,
) -> Result<(Option<String>, Option<String>), Response> {
    sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT frame_url, source_kind FROM cctv_cameras WHERE id = $1 AND active",
    )
    .bind(id)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "gev cctv frame lookup failed");
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

/// Dedicated HTTP client for Street View fallback (5s timeout, modest header).
fn fallback_http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .user_agent("IntelHub/gev-cctv-fallback/1.0")
            .build()
            .expect("fallback http client builds")
    })
}

pub async fn gev_cctv_frame(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    // ── Tier 1: upstream fetch ──────────────────────────────────
    let (frame_url, source_kind) = match lookup_frame_target(&state, &id).await {
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

    // TxDOT ITS returns JSON envelope; everything else returns image bytes
    // (verified upstream Content-Type matches the body magic bytes).
    let txdot_kind = source_kind.as_deref() == Some("txdot-its");

    let (body, content_type, source_header): (Vec<u8>, &'static str, &'static str) =
        if txdot_kind {
            match fetch_txdot_envelope_via_proxy(&url).await {
                Ok(bytes) => (bytes, "image/jpeg", "txdot"),
                Err(e) => {
                    tracing::warn!(error = %e, camera = %id, "cctv frame txdot decode failed");
                    return frame_fallback_on_upstream_failure(&state, &id).await;
                }
            }
        } else {
            match fetch_image_via_proxy(&url).await {
                Ok((bytes, ct, src)) => (bytes, ct, src),
                Err(e) => {
                    tracing::warn!(error = %e, camera = %id, "cctv frame upstream invalid");
                    return frame_fallback_on_upstream_failure(&state, &id).await;
                }
            }
        };

    // Best-effort cache write (degrade to plain proxy on Redis trouble).
    let _: Option<()> = state
        .redis_timed(
            redis::cmd("HSET")
                .arg(&key)
                .arg("data")
                .arg(body.as_slice())
                .arg("ct")
                .arg(content_type)
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

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        content_type.parse().unwrap(),
    );
    headers.insert("x-cctv-source", source_header.parse().unwrap());
    (headers, body).into_response()
}

/// Fetch + decode a TxDOT JSON envelope via the shared `cctv_http` client.
/// The upstream's `application/json` body holds `snippet: data:image/jpeg;base64,`
/// (or bare base64); `parse_txdot_envelope` validates magic bytes so a
/// non-image body never reaches the browser with the `image/jpeg` label.
pub async fn fetch_txdot_envelope_via_proxy(url: &str) -> std::result::Result<Vec<u8>, String> {
    let resp = cctv_http()
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("upstream fetch: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("upstream HTTP {}", resp.status()));
    }
    let body = resp
        .bytes()
        .await
        .map_err(|e| format!("upstream body read: {e}"))?;
    if body.len() > FRAME_MAX_BYTES {
        return Err(format!("envelope exceeds {} bytes", FRAME_MAX_BYTES));
    }
    let json: Value =
        serde_json::from_slice(&body).map_err(|e| format!("envelope parse: {e}"))?;
    parse_txdot_envelope(&json)
}

/// Fetch a non-TxDOT upstream image with the content-type + magic-byte guard.
/// Returns `(bytes, content_type, source_header)` on success or an error
/// message describing why the upstream failed the guard.
pub async fn fetch_image_via_proxy(url: &str) -> std::result::Result<(Vec<u8>, &'static str, &'static str), String> {
    let resp = cctv_http()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("upstream fetch: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("upstream HTTP {}", resp.status()));
    }
    let upstream_ct = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = resp
        .bytes()
        .await
        .map_err(|e| format!("upstream body read: {e}"))?;
    if body.len() > FRAME_MAX_BYTES {
        return Err(format!("body exceeds {} bytes", FRAME_MAX_BYTES));
    }
    if !accept_upstream_body_for_browser(upstream_ct.as_deref(), &body) {
        return Err(format!(
            "body rejected: ct={:?} magic={:?}",
            upstream_ct,
            body.get(..4)
        ));
    }
    let ct = sanitize_frame_content_type(upstream_ct.as_deref());
    Ok((body.to_vec(), ct, "upstream"))
}

/// P11 T4: 3-tier fallback chain — Street View (env-gated) then SVG.
///
/// Called when the upstream fetch fails (network error, HTTP non-2xx,
/// or body read error).  Does NOT write to Redis — only a successful
/// upstream response is cached (avoids polluting the upstream namespace).
async fn frame_fallback_on_upstream_failure(
    state: &Arc<AppState>,
    id: &str,
) -> Response {
    // Look up camera metadata for Street View coordinates.
    // Reuse the existing query (same table + filter as lookup_urls).
    let cam: Option<(f64, f64, String, String)> = sqlx::query_as::<_, (f64, f64, String, String)>(
        "SELECT lat, lon, name, city FROM cctv_cameras WHERE id = $1 AND active",
    )
    .bind(id)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten();

    let Some((lat, lon, name, city)) = cam else {
        // Camera not in catalog → synthetic SVG with minimal info.
        let svg = build_synthetic_svg(id, id, "UNKNOWN", "DOWN");
        return svg_response(svg);
    };

    // ── Tier 2: Street View (env-gated; None when no key) ─────────────
    if let Some(sv_bytes) = street_view_fallback(fallback_http(), lat, lon, 5).await {
        return (
            [(axum::http::header::CONTENT_TYPE, "image/jpeg")],
            sv_bytes,
        )
            .into_response();
    }

    // ── Tier 3: synthetic SVG (always succeeds) ───────────────────────
    let svg = build_synthetic_svg(id, &name, &city, "DOWN");
    svg_response(svg)
}

fn svg_response(svg: Vec<u8>) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::CONTENT_TYPE, "image/svg+xml".parse().unwrap());
    (headers, svg).into_response()
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

    // ---- TxDOT JSON envelope: pure helpers (TDD red phase) ----

    /// Real TxDOT response shape — `{"icd_Id": "...", "snippet": "<base64 JPEG>"}`.
    /// The snippet may carry a `data:image/jpeg;base64,` URI prefix that
    /// must be stripped before base64-decoding.
    #[test]
    fn txdot_decode_envelope_returns_jpeg_bytes_with_magic() {
        // Tiny valid JPEG (FFD8FFE0 ... FFD9) base64-encoded. ~134 bytes.
        // Constructed from "real" JPEG header so the magic-byte check passes.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpeg.extend_from_slice(&[0u8; 64]);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg);

        let envelope = json!({
            "icd_Id": "LP-1 @ Gault Rd",
            "snippet": format!("data:image/jpeg;base64,{}", b64),
        });

        let out = parse_txdot_envelope(&envelope).expect("envelope decodes");
        assert_eq!(out, jpeg, "decoded bytes must match the original JPEG");
    }

    /// TxDOT sometimes omits the `data:` URI prefix — the helper must still
    /// decode raw base64.
    #[test]
    fn txdot_decode_envelope_handles_bare_base64() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 0xFF, 0xD9];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg);
        let envelope = json!({ "snippet": b64 });
        let out = parse_txdot_envelope(&envelope).expect("bare base64 decodes");
        assert_eq!(out, jpeg);
    }

    /// Decoded bytes that lack JPEG magic must be rejected — prevents a
    /// permissive decoder from serving a non-image body with the
    /// `image/jpeg` content-type label (the pre-fix bug).
    #[test]
    fn txdot_decode_envelope_rejects_non_jpeg_decoded_bytes() {
        // Non-JPEG bytes base64-encoded: starts with 0x89 (PNG) and
        // the rest is random. A valid base64 decode must NOT pass the
        // JPEG magic-byte check.
        let png_like = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_like);
        let envelope = json!({ "snippet": b64 });

        let err = parse_txdot_envelope(&envelope).expect_err("non-JPEG must reject");
        assert!(err.contains("magic bytes"), "error must mention magic bytes: {err}");
    }

    /// Strict base64: junk characters (length not multiple of 4) must
    /// reject rather than silently decoding.
    #[test]
    fn txdot_decode_envelope_rejects_non_canonical_base64() {
        let envelope = json!({ "snippet": "!!!not-base64!!!" });
        let err = parse_txdot_envelope(&envelope).expect_err("junk must reject");
        assert!(err.contains("base64"), "error must mention base64: {err}");
    }

    /// Missing snippet field is an upstream contract violation — must error
    /// so the caller falls through to the Street View / SVG chain.
    #[test]
    fn txdot_decode_envelope_rejects_missing_snippet() {
        let envelope = json!({ "icd_Id": "no snippet here" });
        assert!(parse_txdot_envelope(&envelope).is_err());
    }

    /// Empty snippet is also an error (whitespace-only after trim).
    #[test]
    fn txdot_decode_envelope_rejects_empty_snippet() {
        let envelope = json!({ "snippet": "   " });
        assert!(parse_txdot_envelope(&envelope).is_err());
    }

    // ---- Content-type + magic-byte guard for non-TxDOT upstreams ----

    /// Real upstream JPEG with `Content-Type: image/jpeg` must pass
    /// `accept_upstream_body_for_browser`.
    #[test]
    fn accept_upstream_body_passes_real_jpeg() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpeg.extend_from_slice(&[0u8; 8]);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        assert!(accept_upstream_body_for_browser(
            Some("image/jpeg"),
            &jpeg
        ));
    }

    /// NSW livetraffic returns 200 + `text/html` "Page not found" with
    /// no image body. The guard must reject so the proxy falls through
    /// to the SVG fallback chain (the pre-fix bug masked this).
    #[test]
    fn accept_upstream_body_rejects_html_404_page() {
        let html = b"<html><body>Page not found</body></html>";
        assert!(!accept_upstream_body_for_browser(Some("text/html"), html));
    }

    /// `application/json` must also reject — upstream errors or TxDOT
    /// leaks past the source_kind gate must not serve JSON to the browser.
    #[test]
    fn accept_upstream_body_rejects_application_json() {
        let json = br#"{"snippet":"foo"}"#;
        assert!(!accept_upstream_body_for_browser(
            Some("application/json"),
            json
        ));
    }

    /// A body with a missing/falsy Content-Type must reject so the
    /// browser never receives an unlabeled payload as `image/jpeg`.
    #[test]
    fn accept_upstream_body_rejects_missing_content_type() {
        let png = b"\x89PNG\r\n\x1a\n";
        assert!(!accept_upstream_body_for_browser(None, png));
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
