//! Fintraffic / Digitraffic road weather camera provider (GEV P11 T9).
//!
//! `GET https://tie.digitraffic.fi/api/weathercam/v1/stations` →
//! GeoJSON FeatureCollection. Each station with `collectionStatus ==
//! "GATHERING"` yields one camera per `inCollection == true` preset.
//! Frame URLs are built from the official origin:
//! `https://weathercam.digitraffic.fi/{presetId}.jpg`. Header
//! `Digitraffic-User: intelhub-cctv-port/1.0` is sent (as required by
//! the service). Attribution: Fintraffic / digitraffic.fi (CC BY 4.0).

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

pub const LICENSE: &str = "Fintraffic / digitraffic.fi (CC BY 4.0)";
const DEFAULT_URL: &str = "https://tie.digitraffic.fi/api/weathercam/v1/stations";

pub struct Fintraffic;

impl CityCameraProvider for Fintraffic {
    fn id(&self) -> &'static str {
        "fintraffic"
    }
    fn city(&self) -> &'static str {
        "Finland"
    }
    fn fetch_catalog<'a>(
        &'a self,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CameraRow>>> + Send + 'a>> {
        Box::pin(async move {
            let resp = client
                .get(DEFAULT_URL)
                .header("Accept", "application/json")
                .header("Digitraffic-User", "intelhub-cctv-port/1.0")
                .send()
                .await
                .map_err(|e| HubError::sensor(format!("fintraffic: GET failed: {e}")))?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!(
                    "fintraffic: HTTP {}",
                    resp.status()
                )));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("fintraffic: body read: {e}")))?;
            let doc: Value = serde_json::from_str(&text)
                .map_err(|e| HubError::sensor(format!("fintraffic: json: {e}")))?;
            Ok(parse_fintraffic(&doc))
        })
    }
}

/// Extract a finite f64, or None.
fn to_finite(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}

/// Finland bounding box.
fn is_likely_finland_coordinate(lat: f64, lon: f64) -> bool {
    lat >= 59.5 && lat <= 70.5 && lon >= 19.0 && lon <= 32.0
}

/// Deterministic fallback heading from a camera id hash (16 compass points).
fn fallback_heading_from_id(id: &str) -> f32 {
    let hash: u32 = id.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    ((hash % 16) as f32) * 22.5
}

/// Station `name` → human label. Raw names are machine-shaped road codes
/// ("vt3_Hyvinkää_Noppo"); underscores become spaces.
fn fintraffic_camera_name(station_name: &str, station_id: &str, preset_id: &str) -> String {
    let base = station_name.replace('_', " ");
    let base = base.trim();
    let base = if base.is_empty() {
        format!("Fintraffic {station_id}")
    } else {
        base.to_string()
    };
    let view_suffix: String = preset_id
        .strip_prefix(station_id)
        .unwrap_or("")
        .to_string();
    if view_suffix.is_empty() {
        base
    } else {
        format!("{base} (view {view_suffix})")
    }
}

/// Parse the Fintraffic GeoJSON FeatureCollection. One preset = one camera.
/// `collectionStatus == "GATHERING"` (station must be actively collecting) AND
/// `inCollection == true` (preset must be in the collection) are both required.
/// Frame URLs are built from the preset id, not read from the payload.
pub fn parse_fintraffic(doc: &Value) -> Vec<CameraRow> {
    let features = match doc.get("features").and_then(Value::as_array) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    let mut out = Vec::new();

    for feature in features {
        let props = match feature.get("properties") {
            Some(v) => v,
            None => continue,
        };

        let station_id = props
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let Some(station_id) = station_id else {
            continue;
        };

        // Only GATHERING stations
        let collection_status = props
            .get("collectionStatus")
            .and_then(Value::as_str)
            .map(str::trim)
            .map(str::to_uppercase)
            .unwrap_or_default();
        if collection_status != "GATHERING" {
            continue;
        }

        // GeoJSON point: [longitude, latitude, elevation_m?]
        let coords = feature.get("geometry").and_then(|g| g.get("coordinates"));
        let coords_arr = match coords.and_then(Value::as_array) {
            Some(a) if a.len() >= 2 => a,
            _ => continue,
        };
        let lon = to_finite(&coords_arr[0]);
        let lat = to_finite(&coords_arr[1]);
        let (Some(lat), Some(lon)) = (lat, lon) else {
            continue;
        };

        if !is_likely_finland_coordinate(lat, lon) {
            continue;
        }

        // elevation: 0 means "not reported" (sea level is the prior for the coastal default anchors)
        let reported_elevation = coords_arr
            .get(2)
            .and_then(to_finite)
            .unwrap_or(0.0);
        let ground_elevation_m: f32 = if reported_elevation > 0.0 {
            (reported_elevation as f32).min(1400.0)
        } else {
            90.0 // FINTRAFFIC_GROUND_ELEVATION_M prior
        };

        let station_name = props
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();

        for preset in props.get("presets").and_then(Value::as_array).into_iter().flatten() {
            // Only inCollection presets
            if preset.get("inCollection").and_then(Value::as_bool) != Some(true) {
                continue;
            }

            let preset_id = preset
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let Some(preset_id) = preset_id else {
                continue;
            };

            // Strict preset id shape: station id + two-digit view (e.g. "C0150301")
            if preset_id.len() < station_id.len() + 2
                || !preset_id.starts_with(station_id)
            {
                continue;
            }

            let camera_id = format!("fi-{}", preset_id.to_lowercase());
            let name = fintraffic_camera_name(station_name, station_id, preset_id);
            let frame_url =
                Some(format!("https://weathercam.digitraffic.fi/{preset_id}.jpg"));
            let heading_deg = fallback_heading_from_id(&camera_id);

            out.push(CameraRow {
                id: camera_id,
                city: "Finland".to_string(),
                city_id: Some("finland".to_string()),
                name,
                lat,
                lon,
                heading_deg: Some(heading_deg),
                fov_deg: None,
                pitch_deg: None,
                range_m: None,
                mount_height_m: None,
                ground_elevation_m: Some(ground_elevation_m),
                feed_type: "image".to_string(),
                frame_url,
                media_url: None,
                provider: "fintraffic".to_string(),
                source_kind: Some("fintraffic-open-data".to_string()),
                heading_confidence: Some("low".to_string()),
                pose_source: None,
                license_note: Some(LICENSE.to_string()),
                credit: Some("Fintraffic".to_string()),
                code: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FINTRAFFIC_FIXTURE: &str = r#"{
      "type": "FeatureCollection",
      "features": [
        {"type": "Feature",
         "geometry": {"type": "Point", "coordinates": [24.9384, 60.1699, 28.0]},
         "properties": {
           "id": "C01503",
           "name": "vt3_Helsinki_Noppo",
           "collectionStatus": "GATHERING",
           "presets": [
             {"id": "C0150301", "inCollection": true},
             {"id": "C0150302", "inCollection": true},
             {"id": "C0150303", "inCollection": false}
           ]
         }},
        {"type": "Feature",
         "geometry": {"type": "Point", "coordinates": [25.0, 60.5, 0.0]},
         "properties": {
           "id": "C02000",
           "name": "vt3_Espoo",
           "collectionStatus": "REMOVED_TEMPORARILY",
           "presets": [{"id": "C0200001", "inCollection": true}]
         }},
        {"type": "Feature",
         "geometry": {"type": "Point", "coordinates": [10.0, 50.0]},
         "properties": {
           "id": "C03000",
           "name": "Wrong Place",
           "collectionStatus": "GATHERING",
           "presets": [{"id": "C0300001", "inCollection": true}]
         }}
      ]
    }"#;

    #[test]
    fn parses_gathering_stations_with_incollection_presets() {
        let doc: Value = serde_json::from_str(FINTRAFFIC_FIXTURE).unwrap();
        let rows = parse_fintraffic(&doc);
        // Only C01503 (GATHERING) with C0150301 and C0150302 (both inCollection) = 2 rows
        // C02000 is REMOVED_TEMPORARILY → dropped
        // C03000 is out of Finland bbox → dropped
        assert_eq!(rows.len(), 2);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"fi-c0150301"));
        assert!(ids.contains(&"fi-c0150302"));
        assert!(!ids.contains(&"fi-c0150303")); // inCollection=false
        assert!(!ids.contains(&"fi-c0200001")); // REMOVED_TEMPORARILY
        assert!(!ids.contains(&"fi-c0300001")); // out of bbox
    }

    #[test]
    fn frame_url_built_from_preset_id() {
        let doc: Value = serde_json::from_str(FINTRAFFIC_FIXTURE).unwrap();
        let rows = parse_fintraffic(&doc);
        assert_eq!(
            rows[0].frame_url.as_deref(),
            Some("https://weathercam.digitraffic.fi/C0150301.jpg")
        );
    }

    #[test]
    fn camera_name_formatted() {
        assert_eq!(
            fintraffic_camera_name("vt3_Helsinki_Noppo", "C01503", "C0150301"),
            "vt3 Helsinki Noppo (view 01)"
        );
        assert_eq!(
            fintraffic_camera_name("", "C01503", "C0150301"),
            "Fintraffic C01503 (view 01)"
        );
    }

    #[test]
    fn ground_elevation_uses_prior_when_zero() {
        // Station C02000 has elevation=0 (not reported) → should get FINTRAFFIC_GROUND_ELEVATION_M prior
        // But it's REMOVED_TEMPORARILY so won't appear
        let doc = serde_json::json!({
          "type": "FeatureCollection",
          "features": [{
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [24.9, 60.1, 0.0]},
            "properties": {
              "id": "C04000", "name": "No Elev", "collectionStatus": "GATHERING",
              "presets": [{"id": "C0400001", "inCollection": true}]
            }
          }]
        });
        let rows = parse_fintraffic(&doc);
        assert_eq!(rows.len(), 1);
        // ground_elevation_m should be 90.0 (the prior)
        assert!((rows[0].ground_elevation_m.unwrap() - 90.0).abs() < 0.1);
    }

    #[test]
    fn drops_preset_with_bad_id_shape() {
        // Preset id must start with station id
        let doc = serde_json::json!({
          "type": "FeatureCollection",
          "features": [{
            "type": "Feature",
            "geometry": {"type": "Point", "coordinates": [24.9, 60.1]},
            "properties": {
              "id": "C01503", "name": "Test", "collectionStatus": "GATHERING",
              "presets": [{"id": "X999999", "inCollection": true}]
            }
          }]
        });
        let rows = parse_fintraffic(&doc);
        assert!(rows.is_empty());
    }
}
