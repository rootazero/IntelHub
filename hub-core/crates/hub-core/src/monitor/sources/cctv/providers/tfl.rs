//! TfL JamCam provider (GEV P3 T9). Keyless; an optional
//! `TFL_APP_KEY` / `HUB_TFL_APP_KEY` app_key query param raises the
//! anonymous rate cap.
//!
//! `GET https://api.tfl.gov.uk/Place/Type/JamCam` → ~890 cameras, top-
//! level JSON array. Each element: `{$type, id, url, commonName,
//! placeType, lat, lon, additionalProperties:[{key,value},...]}` where
//! additionalProperties carries `imageUrl` (s3 .jpg), `videoUrl` (s3
//! .mp4) and `view` (direction text). No heading/fov upstream → those
//! columns stay None and the engine falls back to `headingFromId`
//! (model.js:107-114).

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

/// TfL Open Data attribution (required by the licence).
pub const LICENSE: &str =
    "Powered by TfL Open Data. Contains OS data © Crown copyright and database rights";
const DEFAULT_URL: &str = "https://api.tfl.gov.uk/Place/Type/JamCam";

pub struct Tfl;

impl Tfl {
    /// `HUB_TFL_APP_KEY` wins, then `TFL_APP_KEY`; keyless otherwise.
    pub fn endpoint() -> String {
        let key = ["HUB_TFL_APP_KEY", "TFL_APP_KEY"].iter().find_map(|v| {
            std::env::var(v)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
        match key {
            Some(k) => format!("{DEFAULT_URL}?app_key={k}"),
            None => DEFAULT_URL.to_string(),
        }
    }
}

impl CityCameraProvider for Tfl {
    fn id(&self) -> &'static str {
        "tfl"
    }
    fn city(&self) -> &'static str {
        "London"
    }
    fn fetch_catalog<'a>(
        &'a self,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CameraRow>>> + Send + 'a>> {
        Box::pin(async move {
            let resp = client
                .get(Self::endpoint())
                .send()
                .await
                .map_err(|e| HubError::sensor(format!("tfl: GET failed: {e}")))?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("tfl: HTTP {}", resp.status())));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("tfl: body read: {e}")))?;
            let doc: Value = serde_json::from_str(&text)
                .map_err(|e| HubError::sensor(format!("tfl: json: {e}")))?;
            Ok(parse_tfl(&doc))
        })
    }
}

/// additionalProperties → key/value map (`imageUrl`/`videoUrl`/`view`/…).
fn props(cam: &Value) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    if let Some(aps) = cam.get("additionalProperties").and_then(Value::as_array) {
        for ap in aps {
            let (Some(k), Some(v)) = (
                ap.get("key").and_then(Value::as_str),
                ap.get("value").and_then(Value::as_str),
            ) else {
                continue;
            };
            m.insert(k.to_string(), v.to_string());
        }
    }
    m
}

/// Parse the TfL JamCam array into normalized rows. One bad entry never
/// costs the catalog its rows (T8 precedent). `videoUrl` present → mp4
/// feed (media_url); image-only → static frame (frame_url). No
/// heading/fov → None (engine hash fallback).
pub fn parse_tfl(doc: &Value) -> Vec<CameraRow> {
    let Some(entries) = doc.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for cam in entries {
        let Some(raw_id) = cam
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let (Some(lat), Some(lon)) = (
            cam.get("lat").and_then(Value::as_f64).filter(|f| f.is_finite()),
            cam.get("lon").and_then(Value::as_f64).filter(|f| f.is_finite()),
        ) else {
            continue;
        };
        let p = props(cam);
        let image_url = p.get("imageUrl").filter(|s| !s.is_empty()).cloned();
        let video_url = p.get("videoUrl").filter(|s| !s.is_empty()).cloned();
        let (feed_type, frame_url, media_url) = match video_url {
            Some(v) => ("mp4".to_string(), image_url, Some(v)),
            None => ("image".to_string(), image_url, None),
        };
        out.push(CameraRow {
            id: format!("tfl:{raw_id}"),
            city: "London".to_string(),
            city_id: Some("london".to_string()),
            name: cam
                .get("commonName")
                .and_then(Value::as_str)
                .unwrap_or(raw_id)
                .to_string(),
            lat,
            lon,
            heading_deg: None,
            fov_deg: None,
            pitch_deg: None,
            range_m: None,
            mount_height_m: None,
            ground_elevation_m: None,
            feed_type,
            frame_url,
            media_url,
            provider: "tfl".to_string(),
            source_kind: Some("tfl-jamcam".to_string()),
            heading_confidence: Some("low".to_string()),
            pose_source: None,
            license_note: Some(LICENSE.to_string()),
            credit: Some("TfL JamCam".to_string()),
            code: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TFL_FIXTURE: &str = r#"[
      {"$type":"Tfl.Api.Presentation.Entities.Place","id":"JamCams_00002.00865",
       "url":"/Place/JamCams_00002.00865","commonName":"A406 Billet Upass E",
       "placeType":"JamCam","lat":51.60067,"lon":-0.01594,
       "additionalProperties":[
         {"key":"available","value":"true"},
         {"key":"imageUrl","value":"https://s3-eu-west-1.amazonaws.com/jamcams.tfl.gov.uk/00002.00865.jpg"},
         {"key":"videoUrl","value":"https://s3-eu-west-1.amazonaws.com/jamcams.tfl.gov.uk/00002.00865.mp4"},
         {"key":"view","value":"West"}
       ]},
      {"$type":"Tfl.Api.Presentation.Entities.Place","id":"JamCams_00001.00001",
       "url":"/Place/JamCams_00001.00001","commonName":"Image-Only Cam",
       "placeType":"JamCam","lat":51.5,"lon":-0.1,
       "additionalProperties":[
         {"key":"imageUrl","value":"https://s3-eu-west-1.amazonaws.com/jamcams.tfl.gov.uk/00001.00001.jpg"}
       ]}
    ]"#;

    #[test]
    fn parses_jamcam_with_video() {
        let doc: Value = serde_json::from_str(TFL_FIXTURE).unwrap();
        let rows = parse_tfl(&doc);
        assert_eq!(rows.len(), 2);
        let r = &rows[0];
        assert_eq!(r.id, "tfl:JamCams_00002.00865");
        assert_eq!(r.name, "A406 Billet Upass E");
        assert_eq!(r.city, "London");
        assert_eq!(r.city_id.as_deref(), Some("london"));
        assert_eq!(r.provider, "tfl");
        assert_eq!(r.feed_type, "mp4");
        assert!(r.media_url.as_ref().unwrap().ends_with(".mp4"));
        assert!(r.frame_url.as_ref().unwrap().ends_with(".jpg"));
        assert_eq!(r.heading_deg, None);
        assert_eq!(r.fov_deg, None);
        assert!(r.license_note.as_ref().unwrap().contains("TfL Open Data"));
    }

    #[test]
    fn parses_image_only_jamcam() {
        let doc: Value = serde_json::from_str(TFL_FIXTURE).unwrap();
        let rows = parse_tfl(&doc);
        let r = &rows[1];
        assert_eq!(r.feed_type, "image");
        assert!(r.media_url.is_none());
        assert!(r.frame_url.as_ref().unwrap().ends_with(".jpg"));
    }

    #[test]
    fn drops_bad_entries() {
        let doc = json!([
            {"id": "", "lat": 1.0, "lon": 2.0},
            {"id": "x", "lat": null, "lon": 2.0},
            {"id": "y", "lat": 1.0, "lon": 2.0, "commonName": "ok",
             "additionalProperties": [{"key":"imageUrl","value":"https://x/y.jpg"}]}
        ]);
        let rows = parse_tfl(&doc);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "tfl:y");
    }

    #[test]
    fn endpoint_app_key_chain() {
        std::env::remove_var("HUB_TFL_APP_KEY");
        std::env::remove_var("TFL_APP_KEY");
        assert_eq!(Tfl::endpoint(), DEFAULT_URL);

        std::env::set_var("TFL_APP_KEY", "secret");
        assert_eq!(Tfl::endpoint(), format!("{DEFAULT_URL}?app_key=secret"));

        // HUB_ prefix wins; blank values fall through
        std::env::set_var("HUB_TFL_APP_KEY", "hub-secret");
        assert_eq!(Tfl::endpoint(), format!("{DEFAULT_URL}?app_key=hub-secret"));
        std::env::set_var("HUB_TFL_APP_KEY", "   ");
        assert_eq!(Tfl::endpoint(), format!("{DEFAULT_URL}?app_key=secret"));

        std::env::remove_var("HUB_TFL_APP_KEY");
        std::env::remove_var("TFL_APP_KEY");
        assert_eq!(Tfl::endpoint(), DEFAULT_URL);
    }
}
