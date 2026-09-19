//! Caltrans CCTV camera provider (GEV P11 T6). Keyless; per-district
//! feeds on `cwwp2.dot.ca.gov`. Districts from `CCTV_CALTRANS_DISTRICTS`
//! env (default "4,7,11,3"); empty → `vec![]` (disabled). Only
//! `inService == true` cameras with a `cwwp2.dot.ca.gov` image URL are
//! kept (host pin).

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

pub const LICENSE: &str = "Public Caltrans highway camera frame";
const DEFAULT_DISTRICTS: &str = "4,7,11,3";

pub struct Caltrans;

impl Caltrans {
    /// Parse the comma-separated districts env; empty → `vec![]`.
    fn districts() -> Vec<u8> {
        let raw = std::env::var("CCTV_CALTRANS_DISTRICTS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_DISTRICTS.to_string());
        raw.split(',')
            .filter_map(|tok| {
                tok.trim()
                    .parse::<u8>()
                    .ok()
                    .filter(|&n| (1..=12).contains(&n))
            })
            .collect()
    }

    /// Build the per-district URL (zero-padded two-digit suffix).
    fn district_url(district: u8) -> String {
        format!(
            "https://cwwp2.dot.ca.gov/data/d{}/cctv/cctvStatusD{:02}.json",
            district, district
        )
    }
}

impl CityCameraProvider for Caltrans {
    fn id(&self) -> &'static str {
        "caltrans"
    }
    fn city(&self) -> &'static str {
        "Caltrans CA"
    }
    fn fetch_catalog<'a>(
        &'a self,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CameraRow>>> + Send + 'a>> {
        Box::pin(async move {
            let districts = Self::districts();
            if districts.is_empty() {
                return Ok(vec![]);
            }

            let mut out = Vec::new();

            for district in districts {
                let url = Self::district_url(district);
                let resp = match client.get(&url).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("caltrans: D{district} GET failed: {e}");
                        continue;
                    }
                };
                if !resp.status().is_success() {
                    tracing::warn!("caltrans: D{district} HTTP {}", resp.status());
                    continue;
                }
                let text = match resp.text().await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!("caltrans: D{district} body read: {e}");
                        continue;
                    }
                };
                let doc: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("caltrans: D{district} json: {e}");
                        continue;
                    }
                };
                parse_caltrans_district(&doc, district, &mut out);
            }
            Ok(out)
        })
    }
}

/// Extract a finite f64, or None (mirrors gods-eye-view `toFiniteNumber`).
fn to_finite(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}

/// Direction string → heading degrees (bare cardinal words allowed).
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
        if text.contains("NORTH") {
            return Some(0.0);
        }
        if text.contains("SOUTH") {
            return Some(180.0);
        }
        if text.contains("EAST") {
            return Some(90.0);
        }
        if text.contains("WEST") {
            return Some(270.0);
        }
    }
    None
}

/// Parse one Caltrans district payload. Mutates `out` in-place so the caller
/// can fan out with `join_all`.
fn parse_caltrans_district(doc: &Value, district: u8, out: &mut Vec<CameraRow>) {
    let data = match doc.get("data").and_then(Value::as_array) {
        Some(arr) => arr,
        None => return,
    };

    for row in data {
        let cctv = match row.get("cctv") {
            Some(v) => v,
            None => continue,
        };

        // Only inService cameras
        let in_service = cctv
            .get("inService")
            .map(|v| {
                String::from(v.as_str().unwrap_or(""))
                    .to_lowercase()
                    == "true"
            })
            .unwrap_or(false);
        if !in_service {
            continue;
        }

        let loc = cctv.get("location");

        let lat = loc.and_then(|l| l.get("latitude").and_then(to_finite));
        let lon = loc.and_then(|l| l.get("longitude").and_then(to_finite));
        let (Some(lat), Some(lon)) = (lat, lon) else {
            continue;
        };
        if !lat.is_finite() || !lon.is_finite() {
            continue;
        }

        let image_url = cctv
            .get("imageData")
            .and_then(|id| id.get("static"))
            .and_then(|s| s.get("currentImageURL"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let Some(image_url) = image_url else {
            continue;
        };
        // Official host pin: reject any image URL not on cwwp2.dot.ca.gov
        if !image_url.starts_with("https://cwwp2.dot.ca.gov/") {
            continue;
        }

        let location_name = loc
            .and_then(|l| l.get("locationName"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();

        // Leading token of locationName is the stable camera code
        let code = location_name
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('-')
            .to_lowercase();

        let camera_id = format!("ca-d{district}-{code}");

        // loc.direction is a dedicated field; bare cardinals allowed
        let direction_raw = loc
            .and_then(|l| l.get("direction"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let heading_deg = direction_to_heading(direction_raw, true);

        let label = location_name
            .strip_prefix(&format!("{code} -- "))
            .unwrap_or(location_name)
            .trim()
            .to_string();
        let name = if label.is_empty() {
            format!("Caltrans D{district} {code}")
        } else {
            label
        };

        // Elevation in FEET → convert to metres
        let elevation_ft = loc
            .and_then(|l| l.get("elevation").and_then(to_finite))
            .unwrap_or(f64::NAN);
        let ground_elevation_m = if elevation_ft.is_finite() {
            Some((elevation_ft * 0.3048).clamp(-100.0, 4000.0) as f32)
        } else {
            None
        };

        out.push(CameraRow {
            id: camera_id,
            city: format!("Caltrans D{district}"),
            city_id: Some(format!("ca-d{district}")),
            name,
            lat,
            lon,
            heading_deg,
            fov_deg: None,
            pitch_deg: None,
            range_m: None,
            mount_height_m: None,
            ground_elevation_m,
            feed_type: "image".to_string(),
            frame_url: Some(image_url.to_string()),
            media_url: None,
            provider: "caltrans".to_string(),
            source_kind: Some("caltrans-open-data".to_string()),
            heading_confidence: heading_deg.map(|_| "high".to_string()),
            pose_source: None,
            license_note: Some(LICENSE.to_string()),
            credit: Some("Caltrans".to_string()),
            code: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALTRANS_FIXTURE: &str = r#"{
      "data": [
        {"cctv": {
          "inService": "true",
          "imageData": {"static": {"currentImageURL": "https://cwwp2.dot.ca.gov/images/abc123.jpg"}},
          "location": {
            "latitude": 37.7749,
            "longitude": -122.4194,
            "locationName": "TV102 -- I-280 : Market St",
            "direction": "North",
            "elevation": 1000
          }
        }},
        {"cctv": {
          "inService": "false",
          "imageData": {"static": {"currentImageURL": "https://cwwp2.dot.ca.gov/images/xyz789.jpg"}},
          "location": {
            "latitude": 37.78,
            "longitude": -122.41,
            "locationName": "TV999 -- Other"
          }
        }},
        {"cctv": {
          "inService": "true",
          "imageData": {"static": {"currentImageURL": "https://evil.com/image.jpg"}},
          "location": {
            "latitude": 37.78,
            "longitude": -122.42,
            "locationName": "TV200 -- Evil"
          }
        }}
      ]
    }"#;

    #[test]
    fn parses_in_service_cameras() {
        let doc: Value = serde_json::from_str(CALTRANS_FIXTURE).unwrap();
        let mut rows = Vec::new();
        parse_caltrans_district(&doc, 4, &mut rows);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.id, "ca-d4-tv102");
        assert_eq!(r.name, "TV102 -- I-280 : Market St");
        assert!((r.lat - 37.7749).abs() < 0.001);
        assert_eq!(r.heading_deg, Some(0.0)); // North
        assert_eq!(
            r.frame_url.as_deref(),
            Some("https://cwwp2.dot.ca.gov/images/abc123.jpg")
        );
        assert_eq!(r.provider, "caltrans");
        // elevation_ft=1000 → metres = 304.8
        assert!((r.ground_elevation_m.unwrap() - 304.8).abs() < 0.1);
    }

    #[test]
    fn drops_offline_cameras() {
        let doc: Value = serde_json::from_str(CALTRANS_FIXTURE).unwrap();
        let mut rows = Vec::new();
        parse_caltrans_district(&doc, 4, &mut rows);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert!(!ids.contains(&"ca-d4-tv999"));
    }

    #[test]
    fn drops_non_official_image_hosts() {
        let doc: Value = serde_json::from_str(CALTRANS_FIXTURE).unwrap();
        let mut rows = Vec::new();
        parse_caltrans_district(&doc, 4, &mut rows);
        let urls: Vec<_> = rows.iter().filter_map(|r| r.frame_url.as_deref()).collect();
        assert!(!urls.iter().any(|u| u.contains("evil.com")));
    }

    #[test]
    fn direction_to_heading_bare_cardinals() {
        assert_eq!(direction_to_heading("North", true), Some(0.0));
        assert_eq!(direction_to_heading("South", true), Some(180.0));
        assert_eq!(direction_to_heading("East", true), Some(90.0));
        assert_eq!(direction_to_heading("West", true), Some(270.0));
        assert_eq!(direction_to_heading("North", false), None); // bare not allowed
        assert_eq!(direction_to_heading("", true), None);
    }
}
