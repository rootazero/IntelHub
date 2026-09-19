//! Austin Transportation & Public Works camera provider (GEV P11 T5).
//!
//! `GET https://data.austintexas.gov/resource/b4k4-adkb.json?$limit=500`
//! → Socrata object-array payload (one JSON object per camera).
//! Fields: `camera_id`, `camera_status`, `location.coordinates` (GeoJSON
//! `[lon, lat]`), `location_name`, `screenshot_address` (full frame URL).
//! Only `camera_status == "TURNED_ON"` cameras are listed — the dataset
//! also carries DESIRED/REMOVED/VOID rows whose frame URLs never resolve.
//! Coordinates within Austin bbox 30.02–30.58 / -98.12 to -97.4.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

pub const LICENSE: &str =
    "City of Austin Transportation & Public Works — Public city traffic camera frame";
const DEFAULT_URL: &str =
    "https://data.austintexas.gov/resource/b4k4-adkb.json?$limit=500";

pub struct Austin;

impl CityCameraProvider for Austin {
    fn id(&self) -> &'static str {
        "austin"
    }
    fn city(&self) -> &'static str {
        "Austin"
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
                .map_err(|e| HubError::sensor(format!("austin: GET failed: {e}")))?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!("austin: HTTP {}", resp.status())));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("austin: body read: {e}")))?;
            let doc: Value = serde_json::from_str(&text)
                .map_err(|e| HubError::sensor(format!("austin: json: {e}")))?;
            Ok(parse_austin(&doc))
        })
    }
}

/// Convert a Socrata rows.json row array into a keyed object using column
/// metadata (normalize.js `rowArrayToObject`).
fn row_array_to_object(row: &[Value], columns: &[Value]) -> std::collections::HashMap<String, Value> {
    let mut map = std::collections::HashMap::new();
    for (idx, cell) in row.iter().enumerate() {
        let col = match columns.get(idx) {
            Some(v) => v,
            None => continue,
        };
        let key = col
            .get("fieldName")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                col.get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        if let Some(k) = key {
            if !k.is_empty() {
                map.insert(k, cell.clone());
            }
        }
    }
    map
}

/// Extract Austin camera id from a Socrata record.
fn extract_camera_id(record: &std::collections::HashMap<String, Value>) -> Option<String> {
    for key in ["camera_id", "cameraid", "cam_id", "device_id", "intersection_id", "id"] {
        if let Some(v) = record.get(key) {
            if let Some(s) = v.as_str() {
                let t = s.trim();
                if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
                    return Some(t.to_string());
                }
            }
            if let Some(n) = v.as_i64() {
                return Some(n.to_string());
            }
        }
    }
    None
}

/// Extract lat/lon from a Socrata record (try multiple known field names).
fn extract_coords(
    record: &std::collections::HashMap<String, Value>,
) -> Option<(f64, f64)> {
    // Try nested point/location objects first
    for key in ["point", "location", "coordinates", "the_geom", "geocoded_column"] {
        if let Some(obj) = record.get(key) {
            // GeoJSON Point: {type:"Point", coordinates:[lon, lat]}
            if let Some(arr) = obj.get("coordinates").and_then(Value::as_array) {
                if arr.len() == 2 {
                    let lon = arr[0].as_f64();
                    let lat = arr[1].as_f64();
                    if let (Some(lat), Some(lon)) = (lat, lon) {
                        if lat.is_finite() && lon.is_finite() {
                            return Some((lat, lon));
                        }
                    }
                }
            }
            let lat = obj.get("latitude").or_else(|| obj.get("lat")).and_then(Value::as_f64);
            let lon = obj.get("longitude").or_else(|| obj.get("lon")).and_then(Value::as_f64);
            if let (Some(lat), Some(lon)) = (lat, lon) {
                if lat.is_finite() && lon.is_finite() {
                    return Some((lat, lon));
                }
            }
        }
    }
    // Fall back to flat fields
    let lat = ["latitude", "lat", "camera_latitude", "location_latitude"]
        .iter()
        .find_map(|k| record.get(*k).and_then(Value::as_f64));
    let lon = ["longitude", "lon", "lng", "camera_longitude", "location_longitude"]
        .iter()
        .find_map(|k| record.get(*k).and_then(Value::as_f64));
    match (lat, lon) {
        (Some(lat), Some(lon)) if lat.is_finite() && lon.is_finite() => Some((lat, lon)),
        _ => None,
    }
}

/// Extract camera name from a Socrata record.
fn extract_name(record: &std::collections::HashMap<String, Value>, camera_id: &str) -> String {
    for key in [
        "camera_name",
        "location_name",
        "intersection_name",
        "location",
        "cross_street",
        "description",
        "name",
    ] {
        if let Some(v) = record.get(key) {
            if let Some(s) = v.as_str() {
                let t = s.trim();
                if !t.is_empty() {
                    return t.to_string();
                }
            }
        }
    }
    format!("Austin Camera {camera_id}")
}

/// Austin bounding box: 30.02–30.58 N, -98.12 to -97.4 W.
fn is_likely_austin_coordinate(lat: f64, lon: f64) -> bool {
    lat >= 30.02 && lat <= 30.58 && lon >= -98.12 && lon <= -97.4
}

/// Parse the Austin Socrata object-array payload. `TURNED_ON` status only;
/// everything else (DESIRED/REMOVED/VOID) is dropped. Bbox-filtered.
/// Frame URL: `screenshot_address` (upstream-provided) or fallback
/// `https://cctv.austinmobility.io/image/{cameraId}.jpg` if missing.
pub fn parse_austin(doc: &Value) -> Vec<CameraRow> {
    let rows = doc.as_array();

    let Some(rows) = rows else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for row in rows {
        let record = match row.as_object() {
            Some(m) => m,
            None => continue,
        };
        // Coerce serde_json::Map into HashMap for helper functions
        let record_hash: std::collections::HashMap<String, Value> = record
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let Some(camera_id) = extract_camera_id(&record_hash) else {
            continue;
        };

        // Only TURNED_ON cameras
        let status = record
            .get("camera_status")
            .and_then(Value::as_str)
            .map(str::trim)
            .map(str::to_uppercase)
            .unwrap_or_default();
        if !status.is_empty() && status != "TURNED_ON" {
            continue;
        }

        let Some((lat, lon)) = extract_coords(&record_hash) else {
            continue;
        };
        if !is_likely_austin_coordinate(lat, lon) {
            continue;
        }

        let name = extract_name(&record_hash, &camera_id);
        // Prefer upstream-provided screenshot_address; fallback to canonical URL
        let frame_url = record_hash
            .get("screenshot_address")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                Some(format!(
                    "https://cctv.austinmobility.io/image/{camera_id}.jpg"
                ))
            });

        out.push(CameraRow {
            id: format!("austin:{camera_id}"),
            city: "Austin".to_string(),
            city_id: Some("austin".to_string()),
            name,
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
            provider: "austin".to_string(),
            source_kind: Some("austin-open-data".to_string()),
            heading_confidence: Some("low".to_string()),
            pose_source: None,
            license_note: Some(LICENSE.to_string()),
            credit: Some("Austin Transportation & Public Works".to_string()),
            code: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUSTIN_FIXTURE: &str = r#"[
      {
        "camera_id": "1001",
        "camera_status": "TURNED_ON",
        "location_name": "Congress & 6th",
        "location": {"type": "Point", "coordinates": [-97.7431, 30.2672]},
        "screenshot_address": "https://cctv.austinmobility.io/image/1001.jpg"
      },
      {
        "camera_id": "1002",
        "camera_status": "DESIRED",
        "location_name": "Planned Camera",
        "location": {"type": "Point", "coordinates": [-97.74, 30.27]}
      },
      {
        "camera_id": "1003",
        "camera_status": "TURNED_ON",
        "location_name": "Riverside",
        "location": {"type": "Point", "coordinates": [-97.8, 30.3]}
      },
      {
        "camera_id": "1004",
        "camera_status": "REMOVED",
        "location_name": "Old Camera",
        "location": {"type": "Point", "coordinates": [-97.75, 30.28]}
      }
    ]"#;

    #[test]
    fn parses_turned_on_cameras() {
        let doc: Value = serde_json::from_str(AUSTIN_FIXTURE).unwrap();
        let rows = parse_austin(&doc);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "austin:1001");
        assert_eq!(rows[0].name, "Congress & 6th");
        assert!((rows[0].lat - 30.2672).abs() < 0.001);
        assert!((rows[0].lon - (-97.7431)).abs() < 0.001);
        assert_eq!(rows[0].city, "Austin");
        assert_eq!(rows[0].provider, "austin");
        assert_eq!(
            rows[0].frame_url.as_deref(),
            Some("https://cctv.austinmobility.io/image/1001.jpg")
        );
        assert_eq!(rows[1].id, "austin:1003");
    }

    #[test]
    fn drops_non_turned_on_statuses() {
        let doc: Value = serde_json::from_str(AUSTIN_FIXTURE).unwrap();
        let rows = parse_austin(&doc);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert!(!ids.contains(&"austin:1002"));
        assert!(!ids.contains(&"austin:1004"));
    }

    #[test]
    fn drops_out_of_bbox_coords() {
        // Object with coordinates outside Austin bbox (Miami)
        let doc = serde_json::json!([
          {
            "camera_id": "5",
            "camera_status": "TURNED_ON",
            "location_name": "Miami Cam",
            "location": {"type": "Point", "coordinates": [-80.0, 25.0]}
          }
        ]);
        let rows = parse_austin(&doc);
        assert!(rows.is_empty());
    }

    #[test]
    fn drops_missing_camera_id() {
        let doc = serde_json::json!([
          {
            "camera_id": null,
            "camera_status": "TURNED_ON",
            "location": {"type": "Point", "coordinates": [-97.74, 30.27]}
          }
        ]);
        let rows = parse_austin(&doc);
        assert!(rows.is_empty());
    }

    #[test]
    fn falls_back_when_screenshot_address_missing() {
        // TURNED_ON row with no screenshot_address \u2192 canonical URL fallback
        let doc = serde_json::json!([
          {
            "camera_id": "42",
            "camera_status": "TURNED_ON",
            "location_name": "No Screenshot Cam",
            "location": {"type": "Point", "coordinates": [-97.74, 30.27]}
          }
        ]);
        let rows = parse_austin(&doc);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].frame_url.as_deref(),
            Some("https://cctv.austinmobility.io/image/42.jpg")
        );
    }
}
