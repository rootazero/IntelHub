//! DriveBC highway camera provider (GEV P11 T8). Keyless; GeoJSON camera
//! list from `https://www.drivebc.ca/api/webcams/`. Only `is_on == true &&
//! should_appear == true` cameras with a positive integer id are kept.
//! Orientation codes give a high-confidence heading. Frame URL is built
//! from the official host: `https://www.drivebc.ca/images/{id}.jpg`.
//! Attribution: Open Government Licence – British Columbia.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

pub const LICENSE: &str =
    "DriveBC, Open Government Licence – British Columbia";
const DEFAULT_URL: &str = "https://www.drivebc.ca/api/webcams/";

/// DriveBC orientation codes → compass headings (8 compass points).
const ORIENTATION_HEADINGS: &[(&str, f32)] = &[
    ("N", 0.0),
    ("NE", 45.0),
    ("E", 90.0),
    ("SE", 135.0),
    ("S", 180.0),
    ("SW", 225.0),
    ("W", 270.0),
    ("NW", 315.0),
];

pub struct DriveBc;

impl CityCameraProvider for DriveBc {
    fn id(&self) -> &'static str {
        "drivebc"
    }
    fn city(&self) -> &'static str {
        "British Columbia"
    }
    fn fetch_catalog<'a>(
        &'a self,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CameraRow>>> + Send + 'a>> {
        Box::pin(async move {
            let resp = client
                .get(DEFAULT_URL)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| HubError::sensor(format!("drivebc: GET failed: {e}")))?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!(
                    "drivebc: HTTP {}",
                    resp.status()
                )));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("drivebc: body read: {e}")))?;
            let doc: Value = serde_json::from_str(&text)
                .map_err(|e| HubError::sensor(format!("drivebc: json: {e}")))?;
            Ok(parse_drivebc(&doc))
        })
    }
}

/// Extract a finite f64, or None.
fn to_finite(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}

/// British Columbia bounding box (with the neighbouring border crossings).
fn is_likely_bc_coordinate(lat: f64, lon: f64) -> bool {
    lat >= 48.0
        && lat <= 60.5
        && lon >= -139.5
        && lon <= -114.0
}

/// Orientation code → heading degrees.
fn orientation_heading(raw: &str) -> Option<f32> {
    let upper = raw.trim().to_uppercase();
    ORIENTATION_HEADINGS
        .iter()
        .find(|(k, _)| *k == upper)
        .map(|(_, h)| h).copied()
}

/// Strip HTML tags from a credit string.
fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// DriveBC `credit` field: only attribution phrases ("courtesy of ...")
/// are carried forward; everything else is dropped.
fn image_credit(raw: &str) -> Option<String> {
    let text = strip_html(raw);
    let lower = text.to_lowercase();
    let is_attribution = lower.contains("courtesy")
        || lower.contains("provided by")
        || lower.contains("presented in cooperation")
        || lower.starts_with("city of")
        || lower.contains("parks canada");
    if is_attribution && !text.is_empty() {
        Some(text)
    } else {
        None
    }
}

/// Deterministic fallback heading from a camera id hash (mirrors
/// gods-eye-view `fallbackHeadingFromId`: 16 evenly-spaced directions).
fn fallback_heading_from_id(id: &str) -> f32 {
    let hash: u32 = id.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    ((hash % 16) as f32) * 22.5
}

/// Parse the DriveBC camera list. Each camera must be on/off+published
/// with a positive integer id and BC coords.
pub fn parse_drivebc(doc: &Value) -> Vec<CameraRow> {
    let rows = match doc.as_array() {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    let mut out = Vec::new();
    for row in rows {
        // is_on AND should_appear must both be true
        if row.get("is_on").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        if row.get("should_appear").and_then(Value::as_bool) != Some(true) {
            continue;
        }

        // Positive integer id only
        let id = match row.get("id").and_then(Value::as_i64) {
            Some(n) if n > 0 => n,
            _ => continue,
        };

        // GeoJSON point: [longitude, latitude]
        let coords = row.get("location").and_then(|l| l.get("coordinates"));
        let coords_arr = match coords.and_then(Value::as_array) {
            Some(a) if a.len() >= 2 => a,
            _ => continue,
        };
        let lon = to_finite(&coords_arr[0]);
        let lat = to_finite(&coords_arr[1]);
        let (Some(lat), Some(lon)) = (lat, lon) else {
            continue;
        };

        if !is_likely_bc_coordinate(lat, lon) {
            continue;
        }

        let camera_id = format!("drivebc-{id}");
        let orientation_raw = row
            .get("orientation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let heading_deg = orientation_heading(orientation_raw);
        let heading = heading_deg.unwrap_or_else(|| fallback_heading_from_id(&camera_id));
        let heading_confidence = if heading_deg.is_some() {
            "high"
        } else {
            "low"
        };

        let region = row
            .get("region_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let city = if region == "Border Cams" {
            "BC Border"
        } else if region.is_empty() {
            "British Columbia"
        } else {
            region
        };

        let name = row
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("DriveBC camera {id}"));

        let frame_url = Some(format!("https://www.drivebc.ca/images/{id}.jpg"));

        let elevation_raw = row.get("elevation").and_then(to_finite);
        let ground_elevation_m: Option<f32> = elevation_raw
            .map(|e| (e.clamp(-100.0, 4000.0)) as f32);

        let credit = row
            .get("credit")
            .and_then(Value::as_str)
            .and_then(image_credit);

        out.push(CameraRow {
            id: camera_id,
            city: city.to_string(),
            city_id: Some("british-columbia".to_string()),
            name,
            lat,
            lon,
            heading_deg: Some(heading),
            fov_deg: None,
            pitch_deg: None,
            range_m: None,
            mount_height_m: None,
            ground_elevation_m,
            feed_type: "image".to_string(),
            frame_url,
            media_url: None,
            provider: "drivebc".to_string(),
            source_kind: Some("drivebc-open-data".to_string()),
            heading_confidence: Some(heading_confidence.to_string()),
            pose_source: None,
            license_note: Some(LICENSE.to_string()),
            credit,
            code: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRIVEBC_FIXTURE: &str = r#"[
      {"id": 101, "is_on": true, "should_appear": true, "name": "Main & Broadway",
       "region_name": "Lower Mainland", "orientation": "NE",
       "location": {"coordinates": [-123.1207, 49.2827]},
       "elevation": 85, "credit": "Images courtesy of City of Vancouver"},
      {"id": 202, "is_on": false, "should_appear": true, "name": "Offline Cam",
       "region_name": "Vancouver Island", "orientation": "S",
       "location": {"coordinates": [-124.0, 49.5]}},
      {"id": 303, "is_on": true, "should_appear": false, "name": "Hidden Cam",
       "region_name": "Southern Interior", "orientation": "W",
       "location": {"coordinates": [-120.0, 50.0]}},
      {"id": 404, "is_on": true, "should_appear": true, "name": "BC Border",
       "region_name": "Border Cams", "orientation": "E",
       "location": {"coordinates": [-122.5, 49.0]}},
      {"id": -1, "is_on": true, "should_appear": true, "name": "Negative Id",
       "orientation": "N",
       "location": {"coordinates": [-123.0, 49.3]}},
      {"id": 606, "is_on": true, "should_appear": true, "name": "Way Off",
       "orientation": "N",
       "location": {"coordinates": [-80.0, 40.0]}}
    ]"#;

    #[test]
    fn parses_on_and_appearing_cameras() {
        let doc: Value = serde_json::from_str(DRIVEBC_FIXTURE).unwrap();
        let rows = parse_drivebc(&doc);
        // Only id 101 (on+appear) and 404 (on+appear, border) pass
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"drivebc-101"));
        assert!(ids.contains(&"drivebc-404"));
        // id 202 is offline, 303 is hidden, -1 is negative, 606 is out of BC bbox
        assert!(!ids.contains(&"drivebc-202"));
        assert!(!ids.contains(&"drivebc-303"));
        assert!(!ids.contains(&"drivebc-606"));
    }

    #[test]
    fn test_orientation_heading() {
        assert_eq!(orientation_heading("N"), Some(0.0));
        assert_eq!(orientation_heading("NE"), Some(45.0));
        assert_eq!(orientation_heading("E"), Some(90.0));
        assert_eq!(orientation_heading("SE"), Some(135.0));
        assert_eq!(orientation_heading("S"), Some(180.0));
        assert_eq!(orientation_heading("SW"), Some(225.0));
        assert_eq!(orientation_heading("W"), Some(270.0));
        assert_eq!(orientation_heading("NW"), Some(315.0));
        assert_eq!(orientation_heading(""), None);
        assert_eq!(orientation_heading("XXX"), None);
    }

    #[test]
    fn bbox_rejects_out_of_region() {
        let doc = serde_json::json!([{
            "id": 1, "is_on": true, "should_appear": true, "name": "Way off",
            "orientation": "N",
            "location": {"coordinates": [-80.0, 40.0]}
        }]);
        let rows = parse_drivebc(&doc);
        assert!(rows.is_empty());
    }

    #[test]
    fn strips_html_from_credit() {
        assert_eq!(strip_html("<b>Hello</b> World"), "Hello World");
        assert_eq!(strip_html("no tags"), "no tags");
    }

    #[test]
    fn credit_extraction() {
        assert_eq!(
            image_credit("Images courtesy of City of Vancouver"),
            Some("Images courtesy of City of Vancouver".to_string())
        );
        assert_eq!(image_credit("Camera is solar powered"), None);
        assert_eq!(image_credit(""), None);
    }
}
