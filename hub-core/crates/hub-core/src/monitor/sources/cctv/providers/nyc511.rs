//! 511NY camera provider (GEV P3 T10). Keyless.
//!
//! `GET https://511ny.org/api/getcameras?format=json` → ~2933 cameras
//! (NYC + New York State), top-level JSON array. T0 probed from the 315
//! egress (contracts.md §5): elements carry `{ID, Name, Latitude,
//! Longitude, RoadwayName, DirectionOfTravel, Url, VideoUrl, Disabled,
//! Blocked}`; `Url` is a DIRECT static png (136KB real frame for enabled
//! cameras; a 15KB placeholder for disabled ones — hence the
//! Disabled/Blocked filter below), `VideoUrl` is an HLS m3u8 playlist
//! (~1765 cameras). No heading/fov upstream → None (engine hash
//! fallback, model.js:107-114).

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

const DEFAULT_URL: &str = "https://511ny.org/api/getcameras?format=json";
/// 511NY / NYSDOT attribution.
pub const LICENSE: &str = "511NY / New York State Department of Transportation";

pub struct Ny511;

impl CityCameraProvider for Ny511 {
    fn id(&self) -> &'static str {
        "ny511"
    }
    fn city(&self) -> &'static str {
        "New York"
    }
    fn fetch_catalog<'a>(
        &'a self,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CameraRow>>> + Send + 'a>> {
        Box::pin(async move {
            let resp = client
                .get(DEFAULT_URL)
                .send()
                .await
                .map_err(|e| HubError::sensor(format!("ny511: GET failed: {e}")))?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("ny511: HTTP {}", resp.status())));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("ny511: body read: {e}")))?;
            let doc: Value = serde_json::from_str(&text)
                .map_err(|e| HubError::sensor(format!("ny511: json: {e}")))?;
            Ok(parse_ny511(&doc))
        })
    }
}

/// Parse the 511NY array. Disabled/Blocked cameras are dropped — their
/// `Url` serves a placeholder image, not a frame. `VideoUrl` (m3u8) →
/// feed_type `hls` with media_url; otherwise static image via `Url`.
pub fn parse_ny511(doc: &Value) -> Vec<CameraRow> {
    let Some(entries) = doc.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for cam in entries {
        // Disabled/Blocked → placeholder image upstream; never list them.
        if cam.get("Disabled").and_then(Value::as_bool).unwrap_or(false)
            || cam.get("Blocked").and_then(Value::as_bool).unwrap_or(false)
        {
            continue;
        }
        let Some(raw_id) = cam
            .get("ID")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let (Some(lat), Some(lon)) = (
            cam.get("Latitude").and_then(Value::as_f64).filter(|f| f.is_finite()),
            cam.get("Longitude").and_then(Value::as_f64).filter(|f| f.is_finite()),
        ) else {
            continue;
        };
        let frame_url = cam
            .get("Url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let media_url = cam
            .get("VideoUrl")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // A camera with neither frame nor media is useless to the layer.
        if frame_url.is_none() && media_url.is_none() {
            continue;
        }
        let feed_type = if media_url.is_some() { "hls" } else { "image" };
        let roadway = cam.get("RoadwayName").and_then(Value::as_str).unwrap_or("");
        let name = cam
            .get("Name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{roadway} cam {raw_id}"));
        out.push(CameraRow {
            id: format!("ny511:{raw_id}"),
            city: "New York".to_string(),
            city_id: Some("new-york".to_string()),
            name,
            lat,
            lon,
            heading_deg: None,
            fov_deg: None,
            pitch_deg: None,
            range_m: None,
            mount_height_m: None,
            ground_elevation_m: None,
            feed_type: feed_type.to_string(),
            frame_url,
            media_url,
            provider: "ny511".to_string(),
            source_kind: Some("511ny".to_string()),
            heading_confidence: Some("low".to_string()),
            pose_source: None,
            license_note: Some(LICENSE.to_string()),
            credit: Some("511NY".to_string()),
            code: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NY_FIXTURE: &str = r#"[
      {"Latitude":42.9223627,"Longitude":-78.8435134,"ID":"Skyline-10213",
       "Name":"NY 33 at NY 198 Interchange (1)","DirectionOfTravel":"Unknown",
       "RoadwayName":"NY 33","Url":"https://511ny.org/map/Cctv/4436",
       "VideoUrl":"https://s52.nysdot.skyvdn.com/rtplive/R5_013/playlist.m3u8",
       "Disabled":false,"Blocked":false},
      {"Latitude":42.9,"Longitude":-78.8,"ID":"Skyline-99999",
       "Name":"Still Cam","RoadwayName":"I-90","Url":"https://511ny.org/map/Cctv/9999",
       "VideoUrl":"","Disabled":false,"Blocked":false},
      {"Latitude":42.9,"Longitude":-78.8,"ID":"Skyline-00000",
       "Name":"Disabled Cam","RoadwayName":"I-90","Url":"https://511ny.org/map/Cctv/1",
       "VideoUrl":"","Disabled":true,"Blocked":false}
    ]"#;

    #[test]
    fn parses_hls_and_image_cams_drops_disabled() {
        let doc: Value = serde_json::from_str(NY_FIXTURE).unwrap();
        let rows = parse_ny511(&doc);
        assert_eq!(rows.len(), 2); // disabled dropped
        let hls = &rows[0];
        assert_eq!(hls.id, "ny511:Skyline-10213");
        assert_eq!(hls.feed_type, "hls");
        assert!(hls.media_url.as_ref().unwrap().ends_with(".m3u8"));
        assert_eq!(hls.frame_url.as_deref(), Some("https://511ny.org/map/Cctv/4436"));
        assert_eq!(hls.provider, "ny511");
        let img = &rows[1];
        assert_eq!(img.feed_type, "image");
        assert!(img.media_url.is_none());
        assert!(img.frame_url.is_some());
    }

    #[test]
    fn drops_cams_without_any_feed() {
        let doc = json!([{"Latitude":1.0,"Longitude":2.0,"ID":"x","Name":"n",
                          "Url":"","VideoUrl":"","Disabled":false,"Blocked":false}]);
        assert!(parse_ny511(&doc).is_empty());
    }

    #[test]
    fn drops_bad_coords_and_empty_ids() {
        let doc = json!([
            {"Latitude":null,"Longitude":2.0,"ID":"a","Url":"http://x/1.png","Disabled":false,"Blocked":false},
            {"Latitude":1.0,"Longitude":2.0,"ID":"","Url":"http://x/2.png","Disabled":false,"Blocked":false}
        ]);
        assert!(parse_ny511(&doc).is_empty());
    }
}
