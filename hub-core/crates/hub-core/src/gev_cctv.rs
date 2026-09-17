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

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::{DateTime, SecondsFormat, Utc};
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
}
