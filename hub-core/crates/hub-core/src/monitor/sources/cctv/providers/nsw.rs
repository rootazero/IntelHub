//! Live Traffic NSW camera provider (GEV P11 T11). Keyless; GeoJSON
//! FeatureCollection from `https://data.livetraffic.com/cameras/traffic-cam.json`.
//! Frame URLs must be on `webcams.transport.nsw.gov.au`. Direction
//! field ("N-E" → "NE" → heading via direction_to_heading). Attribution:
//! Live Traffic NSW — Transport for NSW, CC BY 4.0.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

pub const LICENSE: &str = "Live Traffic NSW — Transport for NSW, CC BY 4.0";
const DEFAULT_URL: &str = "https://data.livetraffic.com/cameras/traffic-cam.json";

pub struct Nsw;

impl CityCameraProvider for Nsw {
    fn id(&self) -> &'static str {
        "nsw"
    }
    fn city(&self) -> &'static str {
        "Sydney"
    }
    fn fetch_catalog<'a>(
        &'a self,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CameraRow>>> + Send + 'a>> {
        Box::pin(async move {
            let resp = client
                .get(DEFAULT_URL)
                .header("Accept", "application/json")
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
                )
                .send()
                .await
                .map_err(|e| HubError::sensor(format!("nsw: GET failed: {e}")))?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("nsw: HTTP {}", resp.status())));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("nsw: body read: {e}")))?;
            let doc: Value = serde_json::from_str(&text)
                .map_err(|e| HubError::sensor(format!("nsw: json: {e}")))?;
            Ok(parse_nsw(&doc))
        })
    }
}

/// Extract a finite f64, or None.
fn to_finite(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}

/// NSW bounding box (including ACT and Lord Howe Island).
fn is_likely_nsw_coordinate(lat: f64, lon: f64) -> bool {
    lat >= -38.0 && lat <= -28.0 && lon >= 140.9 && lon <= 159.2
}

/// Deterministic fallback heading from a camera id hash (16 compass points).
fn fallback_heading_from_id(id: &str) -> f32 {
    let hash: u32 = id.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    ((hash % 16) as f32) * 22.5
}

/// Direction string → heading degrees (bare cardinals allowed for dedicated direction field).
fn direction_to_heading(raw: &str, allow_bare: bool) -> Option<f32> {
    let text = raw.trim().to_uppercase();
    if text.is_empty() {
        return None;
    }
    // Explicit travel forms
    if text.contains("NORTHBOUND") || text.contains("NB") {
        return Some(0.0);
    }
    if text.contains("SOUTHBOUND") || text.contains("SB") {
        return Some(180.0);
    }
    if text.contains("EASTBOUND") || text.contains("EB") {
        return Some(90.0);
    }
    if text.contains("WESTBOUND") || text.contains("WB") {
        return Some(270.0);
    }
    if text.contains("NORTHEAST") || text.contains("NE") {
        return Some(45.0);
    }
    if text.contains("NORTHWEST") || text.contains("NW") {
        return Some(315.0);
    }
    if text.contains("SOUTHEAST") || text.contains("SE") {
        return Some(135.0);
    }
    if text.contains("SOUTHWEST") || text.contains("SW") {
        return Some(225.0);
    }
    // Bare cardinals (only when allow_bare)
    if allow_bare {
        // Single-letter cardinals first (must precede the `contains` checks
        // since `"W".contains("WEST")` is false)
        if text == "N" { return Some(0.0); }
        if text == "S" { return Some(180.0); }
        if text == "E" { return Some(90.0); }
        if text == "W" { return Some(270.0); }
        if text.contains("NORTH") { return Some(0.0); }
        if text.contains("SOUTH") { return Some(180.0); }
        if text.contains("EAST") { return Some(90.0); }
        if text.contains("WEST") { return Some(270.0); }
    }
    None
}

/// NSW camera label: `view` if it is a short descriptive sentence, else `title`.
fn nsw_camera_label(view: &str, title: &str) -> String {
    const MAX_VIEW_LABEL: usize = 140;
    if !view.is_empty()
        && view.len() <= MAX_VIEW_LABEL
        && !view.contains('\n')
        && !view.contains('\r')
    {
        view.to_string()
    } else {
        title.to_string()
    }
}

/// Display code: uppercase title, trimmed to 28 chars.
fn camera_display_code(title: &str) -> Option<String> {
    let clean = title.trim().to_uppercase();
    if clean.is_empty() {
        return None;
    }
    if clean.len() <= 28 {
        Some(clean)
    } else {
        Some(format!("{}…", &clean[..27]))
    }
}

/// Parse the NSW GeoJSON FeatureCollection. One feature = one camera.
/// URL host must be `webcams.transport.nsw.gov.au`. Direction field
/// "N-E" → "NE" before heading lookup.
pub fn parse_nsw(doc: &Value) -> Vec<CameraRow> {
    let features = match doc.get("features").and_then(Value::as_array) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    let mut out = Vec::new();

    for feature in features {
        let raw_id = feature
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let Some(raw_id) = raw_id else {
            continue;
        };

        // GeoJSON point: [longitude, latitude]
        let coords = feature
            .get("geometry")
            .and_then(|g| g.get("coordinates"));
        let coords_arr = match coords.and_then(Value::as_array) {
            Some(a) if a.len() >= 2 => a,
            _ => continue,
        };
        let lon = to_finite(&coords_arr[0]);
        let lat = to_finite(&coords_arr[1]);
        let (Some(lat), Some(lon)) = (lat, lon) else {
            continue;
        };

        if !is_likely_nsw_coordinate(lat, lon) {
            continue;
        }

        let props = feature.get("properties");

        let url = props
            .and_then(|p| p.get("href"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let Some(url) = url else {
            continue;
        };

        // Official host pin
        if !url.starts_with("https://webcams.transport.nsw.gov.au/") {
            continue;
        }

        // Direction: "N-E" → "NE" (strip hyphen)
        let direction_raw = props
            .and_then(|p| p.get("direction"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_uppercase()
            .replace('-', "");
        let heading_deg = direction_to_heading(&direction_raw, true);
        let heading = heading_deg.unwrap_or_else(|| fallback_heading_from_id(raw_id));
        let heading_confidence = if heading_deg.is_some() {
            "high"
        } else {
            "low"
        };

        let view = props
            .and_then(|p| p.get("view"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = props
            .and_then(|p| p.get("title"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let name = nsw_camera_label(view, title);

        let region = props
            .and_then(|p| p.get("region"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();

        let city = if region.is_empty() {
            "New South Wales".to_string()
        } else {
            region.replace('_', " ")
        };

        let code = camera_display_code(title);

        out.push(CameraRow {
            id: format!("nsw-{raw_id}"),
            city,
            city_id: Some("nsw".to_string()),
            name,
            lat,
            lon,
            heading_deg: Some(heading),
            fov_deg: None,
            pitch_deg: None,
            range_m: None,
            mount_height_m: None,
            ground_elevation_m: Some(25.0),
            feed_type: "image".to_string(),
            frame_url: Some(url.to_string()),
            media_url: None,
            provider: "nsw".to_string(),
            source_kind: Some("nsw-livetraffic".to_string()),
            heading_confidence: Some(heading_confidence.to_string()),
            pose_source: None,
            license_note: Some(LICENSE.to_string()),
            credit: Some("Live Traffic NSW".to_string()),
            code,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NSW_FIXTURE: &str = r#"{
      "type": "FeatureCollection",
      "features": [
        {"id": "cam001",
         "geometry": {"type": "Point", "coordinates": [151.2093, -33.8688]},
         "properties": {
           "href": "https://webcams.transport.nsw.gov.au/cameras/cam001.jpg",
           "title": "5 Ways (Miranda)",
           "view": "5 Ways at The Boulevarde looking west towards Sutherland",
           "direction": "W",
           "region": "Sydney"
         }},
        {"id": "cam002",
         "geometry": {"type": "Point", "coordinates": [151.0, -34.0]},
         "properties": {
           "href": "https://other.host.com/cam002.jpg",
           "title": "Bad Host",
           "direction": "N",
           "region": "Sydney"
         }},
        {"id": "cam003",
         "geometry": {"type": "Point", "coordinates": [130.0, -25.0]},
         "properties": {
           "href": "https://webcams.transport.nsw.gov.au/cameras/cam003.jpg",
           "title": "Way Off",
           "direction": "NE",
           "region": "Other"
         }}
      ]
    }"#;

    #[test]
    fn parses_nsw_cameras() {
        let doc: Value = serde_json::from_str(NSW_FIXTURE).unwrap();
        let rows = parse_nsw(&doc);
        // Only cam001 passes: valid host, valid bbox
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.id, "nsw-cam001");
        assert_eq!(r.name, "5 Ways at The Boulevarde looking west towards Sutherland");
        assert_eq!(r.heading_deg, Some(270.0)); // W
        assert_eq!(r.heading_confidence.as_deref(), Some("high"));
        assert_eq!(r.city, "Sydney");
        assert_eq!(
            r.frame_url.as_deref(),
            Some("https://webcams.transport.nsw.gov.au/cameras/cam001.jpg")
        );
        assert_eq!(r.provider, "nsw");
    }

    #[test]
    fn drops_non_official_hosts() {
        let doc: Value = serde_json::from_str(NSW_FIXTURE).unwrap();
        let rows = parse_nsw(&doc);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert!(!ids.contains(&"nsw-cam002"));
    }

    #[test]
    fn drops_out_of_bbox() {
        let doc: Value = serde_json::from_str(NSW_FIXTURE).unwrap();
        let rows = parse_nsw(&doc);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert!(!ids.contains(&"nsw-cam003"));
    }

    #[test]
    fn direction_hyphen_stripped() {
        let doc = serde_json::json!({
          "type": "FeatureCollection",
          "features": [{
            "id": "t1",
            "geometry": {"type": "Point", "coordinates": [151.0, -33.5]},
            "properties": {
              "href": "https://webcams.transport.nsw.gov.au/cameras/t1.jpg",
              "title": "Test",
              "direction": "N-E"
            }
          }]
        });
        let rows = parse_nsw(&doc);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].heading_deg, Some(45.0)); // NE
    }

    #[test]
    fn view_label_preferred_over_title() {
        assert_eq!(
            nsw_camera_label("Short view label", "Much longer title that is definitely more than 140 characters long and should not be used as a label because it exceeds the maximum view label length of 140 characters and contains line breaks\nhere"),
            "Short view label"
        );
        assert_eq!(
            nsw_camera_label("A view that is way too long to be a proper label but is still within the limit for a label that would be considered valid and usable as a description of what the camera is looking at and the various directions it might be facing", "Short Title"),
            "Short Title"
        );
    }
}
