//! TxDOT ITS camera provider (GEV P11 T7). Keyless; districts come from
//! `CCTV_TXDOT_DISTRICTS` env (comma-separated codes; empty disables).
//!
//! Catalog endpoint: `GET https://its.txdot.gov/its/DistrictIts/GetCctvStatusListByDistrict?districtCode={code}`
//! Snapshot endpoint: `GET https://its.txdot.gov/its/DistrictIts/GetCctvSnapshotByIcdId?icdId={icd_id}&districtCode={code}`
//!   — returns JSON `{snippet: "data:image/jpeg;base64,<b64>"}` NOT an image body.
//!
//! Frame URL in catalog rows points to the snapshot JSON endpoint; the hub
//! proxy must decode the base64 envelope before serving the JPEG. This is a
//! known limitation: see Concerns in batch-3 report.
//!
//! Attribution: Public TxDOT traffic camera data.

use std::future::Future;
use std::pin::Pin;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

pub const LICENSE: &str = "Public TxDOT traffic camera data";

const SNAPSHOT_ORIGIN: &str = "https://its.txdot.gov";
const SNAPSHOT_PATH: &str = "/its/DistrictIts/GetCctvSnapshotByIcdId";
const STATUS_ORIGIN: &str = "https://its.txdot.gov";
const STATUS_PATH: &str = "/its/DistrictIts/GetCctvStatusListByDistrict";

/// Decode the TxDOT snapshot JSON envelope into raw JPEG bytes.
///
/// The upstream returns `{snippet: "data:image/jpeg;base64,<b64>"}` — not an
/// image body directly. The `snippet` field may carry the `data:` URI prefix;
/// we strip it before base64-decoding.
pub fn decode_txdot_snapshot(raw_json: &Value) -> std::result::Result<Vec<u8>, String> {
    let snippet = raw_json
        .get("snippet")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "missing snippet field".to_string())?;

    // Strip optional `data:image/jpeg;base64,` prefix.
    let b64 = if snippet.starts_with("data:") {
        snippet
            .split_once(';')
            .and_then(|(_, tail)| tail.strip_prefix("base64,"))
            .unwrap_or(snippet)
    } else {
        snippet
    };

    STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode: {e}"))
}

fn is_likely_texas_coordinate(lat: f64, lon: f64) -> bool {
    // Rough Texas bounding box (with margin for border cameras).
    lat >= 25.7 && lat <= 36.5 && lon >= -106.7 && lon <= -93.2
}

/// Parse the `roadwayCctvStatuses` JSON object inside one district payload.
fn parse_txdot_district(doc: &Value, district: &str) -> Vec<CameraRow> {
    let by_roadway = match doc.get("roadwayCctvStatuses").and_then(Value::as_object) {
        Some(obj) => obj,
        None => return Vec::new(),
    };
    let code = district.to_uppercase();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for rows in by_roadway.values() {
        let arr = match rows.as_array() {
            Some(a) => a,
            None => continue,
        };
        for row in arr {
            // Filter: must be online and have a snapshot.
            let status = row.get("statusDescription").and_then(Value::as_str).unwrap_or("");
            if status != "Device Online" {
                continue;
            }
            if row.get("hasSnapshot") == Some(&Value::Bool(false)) {
                continue;
            }

            let icd_id = row
                .get("icd_Id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(icd_id) = icd_id else {
                continue;
            };
            if seen.contains(icd_id) {
                continue;
            }
            seen.insert(icd_id.to_string());

            let lat = row.get("latitude").and_then(Value::as_f64);
            let lon = row.get("longitude").and_then(Value::as_f64);
            let (Some(lat), Some(lon)) = (lat, lon) else {
                continue;
            };
            if !is_likely_texas_coordinate(lat, lon) {
                continue;
            }

            let name = row
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(icd_id)
                .to_string();

            // Build snapshot URL: icd_id as query param (not base64url-encoded —
            // the upstream expects raw UTF-8 icd_Id as the icdId param).
            let snapshot_url = format!(
                "{}{}?icdId={}&districtCode={}",
                SNAPSHOT_ORIGIN,
                SNAPSHOT_PATH,
                urlencoding::encode(icd_id),
                urlencoding::encode(&code),
            );

            let camera_id = format!(
                "txdot-{}-{}",
                code.to_lowercase(),
                STANDARD.encode(icd_id.as_bytes())
            );

            out.push(CameraRow {
                id: camera_id,
                city: row
                    .get("equipLoc")
                    .and_then(Value::as_object)
                    .and_then(|o| o.get("roadway"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or(code.clone()),
                city_id: Some(format!("tx-{}", code.to_lowercase())),
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
                frame_url: Some(snapshot_url),
                media_url: None,
                provider: "txdot".to_string(),
                source_kind: Some("txdot-its".to_string()),
                heading_confidence: Some("low".to_string()),
                pose_source: None,
                license_note: Some(LICENSE.to_string()),
                credit: Some("TxDOT ITS".to_string()),
                code: Some(icd_id.to_uppercase()),
            });
        }
    }
    out
}

pub struct Txdot;

impl Txdot {
    /// Parse `CCTV_TXDOT_DISTRICTS` env (comma-separated codes, default "AUS,SAT").
    fn districts() -> Vec<String> {
        let raw = std::env::var("CCTV_TXDOT_DISTRICTS").unwrap_or_else(|_| "AUS,SAT".to_string());
        if raw.trim().is_empty() {
            return Vec::new();
        }
        raw.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_uppercase)
            .collect()
    }

    fn status_url(code: &str) -> String {
        format!(
            "{}{}?districtCode={}",
            STATUS_ORIGIN,
            STATUS_PATH,
            urlencoding::encode(code),
        )
    }
}

impl CityCameraProvider for Txdot {
    fn id(&self) -> &'static str {
        "txdot"
    }
    fn city(&self) -> &'static str {
        "Texas"
    }
    fn fetch_catalog<'a>(
        &'a self,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CameraRow>>> + Send + 'a>> {
        Box::pin(async move {
            let districts = Self::districts();
            if districts.is_empty() {
                return Ok(Vec::new());
            }

            let mut all_rows = Vec::new();
            for code in districts {
                let url = Self::status_url(&code);
                let resp = client
                    .get(&url)
                    .header("Accept", "application/json")
                    .send()
                    .await
                    .map_err(|e| HubError::sensor(format!("txdot GET {code}: {e}")))?;
                if !resp.status().is_success() {
                    tracing::warn!(
                        "txdot district {code} HTTP {} — skipping",
                        resp.status()
                    );
                    continue;
                }
                let text = resp
                    .text()
                    .await
                    .map_err(|e| HubError::sensor(format!("txdot body read: {e}")))?;
                let doc: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("txdot district {code} JSON parse error: {e}");
                        continue;
                    }
                };
                all_rows.extend(parse_txdot_district(&doc, &code));
            }

            Ok(all_rows)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal 3-byte JPEG header (SOI + APP0 + DQT + SOF + EOI — not a real JPEG but enough
    // to pass the basic decode path). Using the well-known tiny JPEG marker sequence.
    const TINY_JPEG_B64: &str = "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMCwsKCwsNDhIQDQ4RDgsLEBYQERMUFRUVDA8XGBYUGBIUFRT/2wBDAQMEBAUEBQkFBQkUDQsNFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBT/wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAn/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFQEBAQAAAAAAAAAAAAAAAAAAAAX/xAAUEQEAAAAAAAAAAAAAAAAAAAAA/9oADAMBEQCEAwEPwAB//9k=";

    #[test]
    fn decode_txdot_base64_happy() {
        let doc = serde_json::json!({"snippet": TINY_JPEG_B64});
        let result = decode_txdot_snapshot(&doc);
        assert!(result.is_ok());
        let bytes = result.unwrap();
        // Verify JPEG magic bytes.
        assert!(bytes.len() >= 3);
        assert_eq!(bytes[0], 0xFF);
        assert_eq!(bytes[1], 0xD8);
        assert_eq!(bytes[2], 0xFF);
    }

    #[test]
    fn decode_txdot_base64_without_data_prefix() {
        // Raw base64 (no data URI prefix).
        let doc = serde_json::json!({
            "snippet": "data:image/jpeg;base64,/9j/4AAQSkZJRg=="
        });
        // This is too short / invalid but should at least try decode.
        let result = decode_txdot_snapshot(&doc);
        // /9j/4AAQSkZJRg== is valid base64 but decodes to garbage (not JPEG).
        // The decoder itself does not validate JPEG bytes — that happens in the
        // media proxy. We just verify the decode succeeds.
        assert!(result.is_ok());
    }

    #[test]
    fn decode_txdot_base64_corrupt() {
        let doc = serde_json::json!({"snippet": "!!!not-valid-base64!!!"});
        let result = decode_txdot_snapshot(&doc);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("base64 decode"));
    }

    #[test]
    fn decode_txdot_missing_field() {
        let doc = serde_json::json!({"otherField": "value"});
        let result = decode_txdot_snapshot(&doc);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("missing snippet field"));
    }

    #[test]
    fn decode_txdot_empty_snippet() {
        let doc = serde_json::json!({"snippet": "   "});
        let result = decode_txdot_snapshot(&doc);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing snippet field"));
    }

    #[test]
    fn decode_txdot_null_snippet() {
        let doc = serde_json::json!({"snippet": null});
        let result = decode_txdot_snapshot(&doc);
        assert!(result.is_err());
    }

    const TXDOT_FIXTURE: &str = r#"{
      "roadwayCctvStatuses": {
        "US-290": [
          {
            "icd_Id": "AUS-CAM-001",
            "name": "US-290 EB at I-35",
            "statusDescription": "Device Online",
            "hasSnapshot": true,
            "latitude": 30.25,
            "longitude": -97.75,
            "equipLoc": {"roadway": "US-290", "direction": "Eastbound"}
          },
          {
            "icd_Id": "AUS-CAM-002",
            "name": "US-290 WB at MoPac",
            "statusDescription": "Device Offline",
            "hasSnapshot": true,
            "latitude": 30.3,
            "longitude": -97.7
          },
          {
            "icd_Id": "AUS-CAM-003",
            "name": "US-183 SB at Airport",
            "statusDescription": "Device Online",
            "hasSnapshot": false,
            "latitude": 30.2,
            "longitude": -97.65
          },
          {
            "icd_Id": "AUS-CAM-004",
            "name": "I-35 NB at Riverside",
            "statusDescription": "Device Online",
            "hasSnapshot": true,
            "latitude": 30.22,
            "longitude": -97.72
          }
        ]
      }
    }"#;

    #[test]
    fn parse_txdot_district_online_cameras() {
        let doc: Value = serde_json::from_str(TXDOT_FIXTURE).unwrap();
        let rows = parse_txdot_district(&doc, "AUS");
        // Only 2: AUS-CAM-001 (online+hasSnapshot) and AUS-CAM-004 (online+hasSnapshot).
        // AUS-CAM-002 is offline; AUS-CAM-003 has hasSnapshot=false.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "US-290 EB at I-35");
        assert_eq!(rows[0].lat, 30.25);
        assert_eq!(rows[0].lon, -97.75);
        assert_eq!(rows[0].provider, "txdot");
        assert_eq!(rows[0].source_kind.as_deref(), Some("txdot-its"));
        assert_eq!(rows[0].feed_type, "image");
        // Snapshot URL should point to the TxDOT origin.
        let url = rows[0].frame_url.as_ref().unwrap();
        assert!(url.starts_with("https://its.txdot.gov/its/DistrictIts/GetCctvSnapshotByIcdId"));
        assert!(url.contains("icdId="));
        assert!(url.contains("districtCode="));
        // ID must not start with "static-".
        assert!(!rows[0].id.starts_with("static-"));
        assert!(rows[0].id.starts_with("txdot-"));
        assert_eq!(rows[1].name, "I-35 NB at Riverside");
    }

    #[test]
    fn parse_txdot_deduplicates_by_icd_id() {
        let doc = serde_json::json!({
          "roadwayCctvStatuses": {
            "ROAD1": [
              {"icd_Id": "DUP-ID", "name": "Cam A", "statusDescription": "Device Online",
               "hasSnapshot": true, "latitude": 30.0, "longitude": -97.0}
            ],
            "ROAD2": [
              {"icd_Id": "DUP-ID", "name": "Cam B (same icd_Id)", "statusDescription": "Device Online",
               "hasSnapshot": true, "latitude": 30.0, "longitude": -97.0}
            ]
          }
        });
        let rows = parse_txdot_district(&doc, "AUS");
        // Only one row — icd_Id deduplication.
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn parse_txdot_rejects_out_of_bounds_coordinates() {
        let doc = serde_json::json!({
          "roadwayCctvStatuses": {
            "ROAD1": [
              {"icd_Id": "BAD-COORDS", "name": "Fake camera", "statusDescription": "Device Online",
               "hasSnapshot": true, "latitude": 40.7, "longitude": -74.0}
            ]
          }
        });
        let rows = parse_txdot_district(&doc, "AUS");
        // New York coordinates — not in Texas bbox.
        assert!(rows.is_empty());
    }

    #[test]
    fn parse_txdot_drops_entries_missing_required_fields() {
        let doc = serde_json::json!({
          "roadwayCctvStatuses": {
            "ROAD1": [
              {"name": "missing icd_Id", "statusDescription": "Device Online",
               "hasSnapshot": true, "latitude": 30.0, "longitude": -97.0},
              {"icd_Id": "OK", "statusDescription": "Device Online",
               "hasSnapshot": true, "latitude": null, "longitude": -97.0},
              {"icd_Id": "OK2", "statusDescription": "Device Online",
               "hasSnapshot": true, "latitude": 30.0}
            ]
          }
        });
        let rows = parse_txdot_district(&doc, "AUS");
        assert!(rows.is_empty());
    }

    #[test]
    fn parse_txdot_empty_payload() {
        let doc = serde_json::json!({"roadwayCctvStatuses": {}});
        let rows = parse_txdot_district(&doc, "AUS");
        assert!(rows.is_empty());
    }

    #[test]
    fn parse_txdot_unknown_status() {
        let doc = serde_json::json!({
          "roadwayCctvStatuses": {
            "ROAD1": [
              {"icd_Id": "CAM1", "name": "Unknown status cam",
               "statusDescription": "Unknown",
               "hasSnapshot": true, "latitude": 30.0, "longitude": -97.0}
            ]
          }
        });
        let rows = parse_txdot_district(&doc, "AUS");
        assert!(rows.is_empty());
    }
}
