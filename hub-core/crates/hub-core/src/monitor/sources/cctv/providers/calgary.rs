//! City of Calgary traffic camera provider (GEV P11 T12). Keyless;
//! Socrata endpoint `https://data.calgary.ca/resource/k7p9-kppz.json?$limit=500`.
//! Frame URLs are on `trafficcam.calgary.ca` (upstream serves http but
//! redirects to https; we use https directly). Attribution: Open
//! Government Licence – City of Calgary.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

pub const LICENSE: &str =
    "City of Calgary traffic camera data — Open Government Licence – City of Calgary";
const DEFAULT_URL: &str =
    "https://data.calgary.ca/resource/k7p9-kppz.json?$limit=500";

pub struct Calgary;

impl CityCameraProvider for Calgary {
    fn id(&self) -> &'static str {
        "calgary"
    }
    fn city(&self) -> &'static str {
        "Calgary"
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
                .map_err(|e| HubError::sensor(format!("calgary: GET failed: {e}")))?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!(
                    "calgary: HTTP {}",
                    resp.status()
                )));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("calgary: body read: {e}")))?;
            let doc: Value = serde_json::from_str(&text)
                .map_err(|e| HubError::sensor(format!("calgary: json: {e}")))?;
            Ok(parse_calgary(&doc))
        })
    }
}

/// Extract a finite f64, or None.
fn to_finite(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}

/// Calgary bounding box: municipal extent with slack for the ring road.
fn is_likely_calgary_coordinate(lat: f64, lon: f64) -> bool {
    lat >= 50.8 && lat <= 51.25 && lon >= -114.4 && lon <= -113.8
}

/// Deterministic fallback heading from a camera id hash (16 compass points).
fn fallback_heading_from_id(id: &str) -> f32 {
    let hash: u32 = id.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    ((hash % 16) as f32) * 22.5
}

/// Normalize a Calgary frame URL: upgrade http → https, then pin to the
/// official host. Returns None for non-official origins.
fn normalize_calgary_image_url(raw: Option<&str>) -> Option<String> {
    let text = raw?.trim();
    if text.is_empty() {
        return None;
    }
    // Must start with http:// or https:// trafficcam.calgary.ca/
    const OFFICIAL_ORIGIN: &str = "https://trafficcam.calgary.ca/";
    if text.starts_with(OFFICIAL_ORIGIN) {
        return Some(text.to_string());
    }
    // Upgrade http → https and pin
    if text.starts_with("http://trafficcam.calgary.ca/") {
        return Some(text.replace("http://", "https://"));
    }
    None
}

/// Stable camera id from a normalized frame URL. Uses the filename
/// pattern `loc{number}.jpg` when present; otherwise slugs the path.
fn calgary_camera_id(image_url: &str) -> String {
    // Try loc{n}.jpg pattern first (simple string scan, no regex crate needed)
    if let Some(pos) = image_url.find("loc") {
        let rest = &image_url[pos + 3..];
        let num_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let num_str = &rest[..num_end];
        if !num_str.is_empty() && rest[num_end..].starts_with(".jpg") {
            return format!("calgary-{}", num_str);
        }
    }
    // Fall back to slug of the path
    let path = image_url.split('/').last().unwrap_or(image_url);
    let slug: String = path
        .trim_end_matches(|c: char| !c.is_alphanumeric())
        .chars()
        .take(20)
        .collect();
    format!("calgary-{}", slug.to_lowercase())
}

/// Camera label: `camera_location` first, then `camera_url.description`,
/// then a default.
fn calgary_camera_name(record: &serde_json::Map<std::string::String, Value>, camera_id: &str) -> String {
    if let Some(v) = record.get("camera_location").and_then(Value::as_str) {
        let t = v.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    if let Some(url_obj) = record.get("camera_url") {
        if let Some(desc) = url_obj.get("description").and_then(Value::as_str) {
            let t = desc.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    format!(
        "Calgary Camera {}",
        camera_id.strip_prefix("calgary-").unwrap_or(camera_id)
    )
}

/// Display code: uppercase name, trimmed to 28 chars.
fn camera_display_code(name: &str) -> Option<String> {
    let clean = name.trim().to_uppercase();
    if clean.is_empty() {
        return None;
    }
    if clean.len() <= 28 {
        Some(clean)
    } else {
        Some(format!("{}…", &clean[..27]))
    }
}

/// Parse the Calgary Socrata JSON array. Each record must have valid
/// Calgary coords and an official host frame URL.
pub fn parse_calgary(doc: &Value) -> Vec<CameraRow> {
    let rows = match doc.as_array() {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for record in rows {
        let record_map = match record.as_object() {
            Some(m) => m,
            None => continue,
        };

        // Extract coordinates from nested `point.coordinates`
        let coords = record_map
            .get("point")
            .and_then(|p| p.get("coordinates"));
        let coords_arr = match coords.and_then(Value::as_array) {
            Some(a) if a.len() >= 2 => a,
            _ => continue,
        };
        let lon = to_finite(&coords_arr[0]);
        let lat = to_finite(&coords_arr[1]);
        let (Some(lat), Some(lon)) = (lat, lon) else {
            continue;
        };

        if !is_likely_calgary_coordinate(lat, lon) {
            continue;
        }

        // Normalize and pin the image URL
        let raw_url = record_map
            .get("camera_url")
            .and_then(|u| u.get("url"))
            .and_then(Value::as_str);
        let Some(image_url) = normalize_calgary_image_url(raw_url) else {
            continue;
        };
        let camera_id = calgary_camera_id(&image_url);

        // Deduplicate by camera id
        if seen.contains(&camera_id) {
            continue;
        }
        seen.insert(camera_id.clone());

        let name = calgary_camera_name(record_map, &camera_id);
        let heading_deg = fallback_heading_from_id(&camera_id);

        out.push(CameraRow {
            id: camera_id,
            city: "Calgary".to_string(),
            city_id: Some("calgary".to_string()),
            name,
            lat,
            lon,
            heading_deg: Some(heading_deg),
            fov_deg: None,
            pitch_deg: None,
            range_m: None,
            mount_height_m: None,
            ground_elevation_m: Some(1045.0), // Calgary prairie prior
            feed_type: "image".to_string(),
            frame_url: Some(image_url),
            media_url: None,
            provider: "calgary".to_string(),
            source_kind: Some("calgary-open-data".to_string()),
            heading_confidence: Some("low".to_string()),
            pose_source: None,
            license_note: Some(LICENSE.to_string()),
            credit: Some("The City of Calgary".to_string()),
            code: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALGARY_FIXTURE: &str = r#"[
      {"point": {"coordinates": [-114.0626, 51.0461]},
       "camera_url": {"url": "https://trafficcam.calgary.ca/loc86.jpg", "description": "Centre St & 7 Ave SW"}},
      {"point": {"coordinates": [-114.0, 50.95]},
       "camera_url": {"url": "http://trafficcam.calgary.ca/loc200.jpg", "description": "Crowchild Trail"}},
      {"point": {"coordinates": [-114.1, 51.0]},
       "camera_url": {"url": "https://trafficcam.calgary.ca/loc300.jpg"}},
      {"point": {"coordinates": [-80.0, 43.0]},
       "camera_url": {"url": "https://trafficcam.calgary.ca/loc400.jpg", "description": "Toronto Cam"}},
      {"point": {"coordinates": [-114.1, 51.1]},
       "camera_url": {"url": "https://evil.com/loc500.jpg", "description": "Evil Cam"}}
    ]"#;

    #[test]
    fn parses_calgary_cameras() {
        let doc: Value = serde_json::from_str(CALGARY_FIXTURE).unwrap();
        let rows = parse_calgary(&doc);
        // rows with Calgary bbox: loc86 (51.0461, -114.0626) and loc200 (50.95, -114.0)
        // loc300: lat 51.0, lon -114.1 → in bbox (50.8-51.25, -114.4 to -113.8) ✓
        // So loc86 + loc200 + loc300 = 3
        // loc400: out of bbox (Toronto) → dropped
        // loc500: wrong host → dropped
        assert_eq!(rows.len(), 3);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"calgary-86"));
        assert!(ids.contains(&"calgary-200"));
        assert!(ids.contains(&"calgary-300"));
        assert!(!ids.contains(&"calgary-400")); // out of bbox
        assert!(!ids.contains(&"calgary-500")); // wrong host
    }

    #[test]
    fn normalizes_http_to_https() {
        assert_eq!(
            normalize_calgary_image_url(Some("http://trafficcam.calgary.ca/loc1.jpg")),
            Some("https://trafficcam.calgary.ca/loc1.jpg".to_string())
        );
        assert_eq!(
            normalize_calgary_image_url(Some("https://trafficcam.calgary.ca/loc2.jpg")),
            Some("https://trafficcam.calgary.ca/loc2.jpg".to_string())
        );
        assert_eq!(normalize_calgary_image_url(Some("https://evil.com/loc.jpg")), None);
        assert_eq!(normalize_calgary_image_url(None), None);
        assert_eq!(normalize_calgary_image_url(Some("")), None);
    }

    #[test]
    fn dedupes_by_camera_id() {
        let doc = serde_json::json!([
          {"point": {"coordinates": [-114.0, 51.0]},
           "camera_url": {"url": "https://trafficcam.calgary.ca/loc1.jpg", "description": "A"}},
          {"point": {"coordinates": [-114.1, 51.1]},
           "camera_url": {"url": "https://trafficcam.calgary.ca/loc1.jpg", "description": "B"}}
        ]);
        let rows = parse_calgary(&doc);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn camera_id_from_loc_pattern() {
        assert_eq!(calgary_camera_id("https://trafficcam.calgary.ca/loc123.jpg"), "calgary-123");
        assert_eq!(calgary_camera_id("https://trafficcam.calgary.ca/loc00456.jpg"), "calgary-00456");
    }

    #[test]
    fn camera_name_prefers_camera_location() {
        let doc = serde_json::json!([{
          "point": {"coordinates": [-114.0, 51.0]},
          "camera_location": "Bow Trail / 37 Street SW",
          "camera_url": {"url": "https://trafficcam.calgary.ca/loc1.jpg", "description": "Wrong label"}
        }]);
        let rows = parse_calgary(&doc);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Bow Trail / 37 Street SW");
    }
}
