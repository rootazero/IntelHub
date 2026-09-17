//! Singapore LTA DataMall provider (GEV P3 T10). **Env-gated** on
//! `LTA_API_KEY` / `HUB_LTA_API_KEY` (free registration, AccountKey
//! header auth).
//!
//! ⚠ Verification gap (2026-09-17): every probed path variant
//! (`/ltaodataservice/CCTV`, `/Traffic-Imagesv2`, …) answers
//! `404 The requested API was not found` from the 315 egress without a
//! key — LTA appears to have reorganised the DataMall catalog. The
//! default endpoint below follows the last documented Traffic Images v2
//! shape; `LTA_API_BASE` allows repointing without a rebuild. Without a
//! key this provider is never registered (shelved-by-design), so the
//! gap costs nothing; with a key, watch the `cctv-lta` health cell.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

const DEFAULT_BASE: &str = "https://datamall2.mytransport.sg/ltaodataservice/Traffic-Imagesv2";
pub const LICENSE: &str = "Singapore Land Transport Authority — DataMall open data";

/// `HUB_LTA_API_KEY` wins, then `LTA_API_KEY`; blank treated as unset
/// (opensky.rs / 410-empty-keys precedent).
pub fn api_key() -> Option<String> {
    ["HUB_LTA_API_KEY", "LTA_API_KEY"].iter().find_map(|v| {
        std::env::var(v)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

fn endpoint() -> String {
    std::env::var("LTA_API_BASE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

pub struct Lta;

impl CityCameraProvider for Lta {
    fn id(&self) -> &'static str {
        "lta"
    }
    fn city(&self) -> &'static str {
        "Singapore"
    }
    fn fetch_catalog<'a>(
        &'a self,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CameraRow>>> + Send + 'a>> {
        Box::pin(async move {
            let Some(key) = api_key() else {
                return Err(HubError::sensor("lta: no LTA_API_KEY"));
            };
            let resp = client
                .get(endpoint())
                .header("AccountKey", key)
                .header("accept", "application/json")
                .send()
                .await
                .map_err(|e| HubError::sensor(format!("lta: GET failed: {e}")))?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("lta: HTTP {}", resp.status())));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("lta: body read: {e}")))?;
            let doc: Value = serde_json::from_str(&text)
                .map_err(|e| HubError::sensor(format!("lta: json: {e}")))?;
            Ok(parse_lta(&doc))
        })
    }
}

/// DataMall envelope: `{ "odata.metadata": ..., "value": [ {CameraID,
/// Latitude, Longitude, ImageLink}, ... ] }`. ImageLink is a
/// token-signed static jpeg (refreshes upstream ~2-5min).
pub fn parse_lta(doc: &Value) -> Vec<CameraRow> {
    let Some(entries) = doc.get("value").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for cam in entries {
        let id_num = cam
            .get("CameraID")
            .and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_i64().map(|n| n.to_string())))
            .filter(|s| !s.is_empty());
        let Some(raw_id) = id_num else { continue };
        let (Some(lat), Some(lon)) = (
            cam.get("Latitude").and_then(Value::as_f64).filter(|f| f.is_finite()),
            cam.get("Longitude").and_then(Value::as_f64).filter(|f| f.is_finite()),
        ) else {
            continue;
        };
        let frame_url = cam
            .get("ImageLink")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if frame_url.is_none() {
            continue;
        }
        out.push(CameraRow {
            id: format!("lta:{raw_id}"),
            city: "Singapore".to_string(),
            city_id: Some("singapore".to_string()),
            name: format!("LTA camera {raw_id}"),
            lat,
            lon,
            heading_deg: None,
            fov_deg: None,
            pitch_deg: None,
            range_m: None,
            mount_height_m: None,
            ground_elevation_m: None,
            feed_type: "image".to_string(),
            frame_url,
            media_url: None,
            provider: "lta".to_string(),
            source_kind: Some("lta-datamall".to_string()),
            heading_confidence: Some("low".to_string()),
            pose_source: None,
            license_note: Some(LICENSE.to_string()),
            credit: Some("LTA DataMall".to_string()),
            code: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_datamall_envelope() {
        let doc = json!({"odata.metadata": "x", "value": [
            {"CameraID": "1701", "Latitude": 1.323, "Longitude": 103.8,
             "ImageLink": "https://datamall2.mytransport.sg/images/1701.jpg?token=abc"},
            {"CameraID": 1702, "Latitude": 1.325, "Longitude": 103.9,
             "ImageLink": "https://datamall2.mytransport.sg/images/1702.jpg?token=def"},
            {"CameraID": "1703", "Latitude": null, "Longitude": 103.9, "ImageLink": "http://x/3.jpg"},
            {"CameraID": "1704", "Latitude": 1.3, "Longitude": 103.9, "ImageLink": ""}
        ]});
        let rows = parse_lta(&doc);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "lta:1701");
        assert_eq!(rows[1].id, "lta:1702"); // numeric CameraID tolerated
        assert_eq!(rows[0].feed_type, "image");
        assert!(rows[0].frame_url.as_ref().unwrap().contains("token=abc"));
        assert!(rows[0].media_url.is_none());
        assert_eq!(rows[0].provider, "lta");
    }

    #[test]
    fn api_key_chain_hub_prefix_wins_blank_ignored() {
        std::env::remove_var("HUB_LTA_API_KEY");
        std::env::remove_var("LTA_API_KEY");
        assert_eq!(api_key(), None);
        std::env::set_var("LTA_API_KEY", "k1");
        assert_eq!(api_key().as_deref(), Some("k1"));
        std::env::set_var("HUB_LTA_API_KEY", "  ");
        assert_eq!(api_key().as_deref(), Some("k1"));
        std::env::set_var("HUB_LTA_API_KEY", "k0");
        assert_eq!(api_key().as_deref(), Some("k0"));
        std::env::remove_var("HUB_LTA_API_KEY");
        std::env::remove_var("LTA_API_KEY");
    }
}
