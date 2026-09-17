//! Ontario 511 camera provider (GEV P3 T9). Keyless.
//!
//! `GET https://511on.ca/api/v2/get/cameras` → ~944 cameras, top-level
//! JSON array. Each element: `{Id, Source, SourceId, Roadway, Direction,
//! Latitude, Longitude, Location, SortOrder, Views:[{Id, Url, Status,
//! Description},...]}`. A View's `Url` is a DIRECT static image (jpeg,
//! despite the /map/Cctv/N path). Only `Status=="Enabled"` views are
//! usable; a camera with no Enabled view is skipped (a disabled
//! placeholder image must not become a live feed). No heading/fov →
//! None (engine hash fallback, model.js:107-114).

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::error::{HubError, Result};

use super::super::CameraRow;
use super::CityCameraProvider;

/// Open Government Licence – Ontario attribution.
pub const LICENSE: &str = "Ontario 511 — Open Government Licence - Ontario";
const DEFAULT_URL: &str = "https://511on.ca/api/v2/get/cameras";

pub struct Ontario511;

impl CityCameraProvider for Ontario511 {
    fn id(&self) -> &'static str {
        "ontario511"
    }
    fn city(&self) -> &'static str {
        "Ontario"
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
                .map_err(|e| HubError::sensor(format!("ontario511: GET failed: {e}")))?;
            if !resp.status().is_success() {
                return Err(HubError::sensor(format!(
                    "ontario511: HTTP {}",
                    resp.status()
                )));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("ontario511: body read: {e}")))?;
            let doc: Value = serde_json::from_str(&text)
                .map_err(|e| HubError::sensor(format!("ontario511: json: {e}")))?;
            Ok(parse_ontario(&doc))
        })
    }
}

/// `Id` may arrive as an integer (observed) or a string (defensive).
fn raw_id(cam: &Value) -> Option<String> {
    if let Some(i) = cam.get("Id").and_then(Value::as_i64) {
        return Some(i.to_string());
    }
    cam.get("Id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// First `Status=="Enabled"` View's `Url`; None when there is no enabled
/// view (the camera is then skipped).
fn enabled_view_url(cam: &Value) -> Option<String> {
    let views = cam.get("Views").and_then(Value::as_array)?;
    views
        .iter()
        .find(|v| v.get("Status").and_then(Value::as_str) == Some("Enabled"))
        .and_then(|v| v.get("Url").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Parse the Ontario 511 array into normalized rows. Entries without a
/// usable Id/position or with no Enabled view are dropped.
pub fn parse_ontario(doc: &Value) -> Vec<CameraRow> {
    let Some(entries) = doc.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for cam in entries {
        let Some(rid) = raw_id(cam) else { continue };
        let (Some(lat), Some(lon)) = (
            cam.get("Latitude")
                .and_then(Value::as_f64)
                .filter(|f| f.is_finite()),
            cam.get("Longitude")
                .and_then(Value::as_f64)
                .filter(|f| f.is_finite()),
        ) else {
            continue;
        };
        let Some(frame_url) = enabled_view_url(cam) else {
            continue; // all Disabled / empty Views → skip camera
        };
        let name = cam
            .get("Location")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Camera {rid}"));
        out.push(CameraRow {
            id: format!("on:{rid}"),
            city: "Ontario".to_string(),
            city_id: Some("ontario".to_string()),
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
            frame_url: Some(frame_url),
            media_url: None,
            provider: "ontario511".to_string(),
            source_kind: Some("ontario-511".to_string()),
            heading_confidence: Some("low".to_string()),
            pose_source: None,
            license_note: Some(LICENSE.to_string()),
            credit: Some("Ontario 511".to_string()),
            code: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ONTARIO_FIXTURE: &str = r#"[
      {"Id":1,"Source":"RWIS (MTO)","SourceId":"CR-1","Roadway":"QEW",
       "Direction":"Unknown","Latitude":42.9142736,"Longitude":-78.9580061,
       "Location":"QEW West of Thompson Road","SortOrder":5,
       "Views":[
         {"Id":1,"Url":"https://511on.ca/map/Cctv/1","Status":"Enabled","Description":"Toronto Bound"},
         {"Id":2,"Url":"https://511on.ca/map/Cctv/1b","Status":"Disabled","Description":"Other"}
       ]},
      {"Id":2,"Source":"RWIS","SourceId":"CR-2","Roadway":"400",
       "Direction":"North","Latitude":43.0,"Longitude":-79.5,
       "Location":"400 at Major Mackenzie","SortOrder":9,
       "Views":[
         {"Id":1,"Url":"https://511on.ca/map/Cctv/2","Status":"Disabled","Description":"X"}
       ]},
      {"Id":3,"Source":"RWIS","SourceId":"CR-3","Roadway":"401",
       "Direction":"East","Latitude":43.7,"Longitude":-79.4,
       "Location":"401 at Yonge","SortOrder":3,
       "Views":[]},
      {"Id":4,"Source":"RWIS","SourceId":"CR-4","Roadway":"403",
       "Direction":"West","Latitude":43.4,"Longitude":-79.7,
       "Location":"403 at Erin Mills","SortOrder":1,
       "Views":[
         {"Id":1,"Url":"https://511on.ca/map/Cctv/4","Status":"Enabled","Description":"A"}
       ]}
    ]"#;

    #[test]
    fn parses_enabled_views_only() {
        let doc: Value = serde_json::from_str(ONTARIO_FIXTURE).unwrap();
        let rows = parse_ontario(&doc);
        // camera 1 (first Enabled view) + camera 4 (Enabled) → 2 rows;
        // camera 2 all-Disabled → skipped; camera 3 empty Views → skipped.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "on:1");
        assert_eq!(rows[0].name, "QEW West of Thompson Road");
        assert_eq!(rows[0].city, "Ontario");
        assert_eq!(rows[0].city_id.as_deref(), Some("ontario"));
        assert_eq!(rows[0].provider, "ontario511");
        assert_eq!(rows[0].feed_type, "image");
        assert!(rows[0].frame_url.as_ref().unwrap().ends_with("/Cctv/1"));
        assert!(rows[0].media_url.is_none());
        assert_eq!(rows[0].heading_deg, None);
        assert!(rows[0]
            .license_note
            .as_ref()
            .unwrap()
            .contains("Open Government Licence"));
        assert_eq!(rows[1].id, "on:4");
    }

    #[test]
    fn first_enabled_view_wins() {
        let doc = json!([{
            "Id": 10, "Latitude": 1.0, "Longitude": 2.0, "Location": "L",
            "Views": [
                {"Id":1,"Url":"https://511on.ca/map/Cctv/a","Status":"Enabled"},
                {"Id":2,"Url":"https://511on.ca/map/Cctv/b","Status":"Enabled"}
            ]
        }]);
        let rows = parse_ontario(&doc);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].frame_url.as_ref().unwrap().ends_with("/Cctv/a"));
    }

    #[test]
    fn empty_views_and_missing_fields_skipped() {
        let doc = json!([
            {"Id": 1, "Latitude": 1.0, "Longitude": 2.0, "Location": "x", "Views": []},
            {"Id": 2, "Latitude": 1.0, "Longitude": 2.0},
            {"Id": 3, "Longitude": 2.0, "Views": [{"Url":"u","Status":"Enabled"}]},
            {"Id": "4", "Latitude": 1.0, "Longitude": 2.0, "Location": "ok",
             "Views": [{"Url":"https://u/4","Status":"Enabled"}]}
        ]);
        let rows = parse_ontario(&doc);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "on:4"); // string Id accepted defensively
    }
}
