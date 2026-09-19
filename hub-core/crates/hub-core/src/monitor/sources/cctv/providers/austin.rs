//! Austin Transportation & Public Works camera provider (GEV P11 T5).
//!
//! `GET https://data.austintexas.gov/resource/dx9v-zd7x.json?$limit=500`
//! → Socrata 2.0 rows.json payload: `{meta: {view: {columns:
//! [...]}}, data: [[...], ...]}`. Column index → field name via
//! `meta.view.columns[].fieldName`. Only `camera_status == "TURNED_ON"`
//! cameras are listed — the dataset also carries DESIRED/REMOVED/VOID rows
//! whose frame URLs never resolve. Coordinates within Austin bbox
//! 30.02–30.58 / -98.12 to -97.4.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

pub const LICENSE: &str =
    "City of Austin Transportation & Public Works — Public city traffic camera frame";
const DEFAULT_URL: &str =
    "https://data.austintexas.gov/resource/dx9v-zd7x.json?$limit=500";

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

/// Parse the Austin Socrata rows.json payload. `TURNED_ON` status only;
/// everything else (DESIRED/REMOVED/VOID) is dropped. Bbox-filtered.
/// Frame URL: `https://cctv.austinmobility.io/image/{cameraId}.jpg`.
pub fn parse_austin(doc: &Value) -> Vec<CameraRow> {
    let columns = doc
        .pointer("/meta/view/columns")
        .and_then(Value::as_array)
        .map(|a| a.clone());
    let rows = doc.pointer("/data").and_then(Value::as_array);

    let (Some(columns), Some(rows)) = (columns, rows) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for row in rows {
        let row_arr = match row.as_array() {
            Some(a) => a,
            None => continue,
        };
        let record = row_array_to_object(row_arr, &columns);

        let Some(camera_id) = extract_camera_id(&record) else {
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

        let Some((lat, lon)) = extract_coords(&record) else {
            continue;
        };
        if !is_likely_austin_coordinate(lat, lon) {
            continue;
        }

        let name = extract_name(&record, &camera_id);
        let frame_url =
            Some(format!("https://cctv.austinmobility.io/image/{camera_id}.jpg"));

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

    const AUSTIN_FIXTURE: &str = r#"{
      "meta": {
        "view": {
          "columns": [
            {"fieldName": "camera_id", "name": "Camera ID"},
            {"fieldName": "camera_status", "name": "Status"},
            {"fieldName": "latitude", "name": "Latitude"},
            {"fieldName": "longitude", "name": "Longitude"},
            {"fieldName": "location_name", "name": "Location"},
            {"fieldName": "camera_name", "name": "Camera Name"}
          ]
        }
      },
      "data": [
        [1001, "TURNED_ON", 30.2672, -97.7431, "Congress & 6th", "Congress NB"],
        [1002, "DESIRED", 30.27, -97.74, "Planned Camera", "Future"],
        [1003, "TURNED_ON", 30.3, -97.8, "Riverside", "Riverside SB"],
        [1004, "REMOVED", 30.28, -97.75, "Old Camera", "Old"]
      ]
    }"#;

    #[test]
    fn parses_turned_on_cameras() {
        let doc: Value = serde_json::from_str(AUSTIN_FIXTURE).unwrap();
        let rows = parse_austin(&doc);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "austin:1001");
        assert_eq!(rows[0].name, "Congress NB");
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
        // Row with coordinates outside Austin bbox
        let doc = serde_json::json!({
          "meta": { "view": { "columns": [
            {"fieldName": "camera_id", "name": "ID"},
            {"fieldName": "camera_status", "name": "Status"},
            {"fieldName": "latitude", "name": "Lat"},
            {"fieldName": "longitude", "name": "Lon"},
            {"fieldName": "location_name", "name": "Loc"}
          ]}},
          "data": [[5, "TURNED_ON", 25.0, -80.0, "Miami Cam"]]
        });
        let rows = parse_austin(&doc);
        assert!(rows.is_empty());
    }

    #[test]
    fn drops_missing_camera_id() {
        let doc = serde_json::json!({
          "meta": { "view": { "columns": [
            {"fieldName": "camera_id", "name": "ID"},
            {"fieldName": "camera_status", "name": "Status"},
            {"fieldName": "latitude", "name": "Lat"},
            {"fieldName": "longitude", "name": "Lon"}
          ]}},
          "data": [[null, "TURNED_ON", 30.27, -97.74]]
        });
        let rows = parse_austin(&doc);
        assert!(rows.is_empty());
    }
}
