//! Transpordiamet / Tarktee (Estonia) camera provider (GEV P11 T10).
//! Keyless; fetches two DATEX2 XML feeds in parallel:
//!   locations: `GET https://tarktee.transpordiamet.ee/api/v1/predefinedLocations.xml`
//!   images:    `GET https://tarktee.transpordiamet.ee/api/v1/trafficViews.xml`
//!
//! The XML is parsed with `quick-xml` (DATEX2 schema). Image URLs are joined
//! to location coordinates by matching the location reference id.
//!
//! Attribution: Public Transpordiamet / Tarktee road weather camera data.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use crate::error::Result;
use quick_xml::events::Event;
use quick_xml::Reader;

use super::super::CameraRow;
use super::CityCameraProvider;

pub const LICENSE: &str = "Public Transpordiamet / Tarktee road weather camera data";

const LOCATIONS_URL: &str =
    "https://tarktee.transpordiamet.ee/api/v1/predefinedLocations.xml";
const IMAGES_URL: &str =
    "https://tarktee.transpordiamet.ee/api/v1/trafficViews.xml";
const IMAGE_ORIGIN: &str = "https://tarktee.transpordiamet.ee/images/";

// Estonia bounding box (mainland + nearby islands).
fn in_estonia_bbox(lat: f64, lon: f64) -> bool {
    lat >= 57.4 && lat <= 59.9 && lon >= 21.5 && lon <= 28.4
}

/// Parse predefined-location id → {name, lat, lon} from DATEX2 locations XML.
/// Uses a state machine over quick-xml events; never panics on malformed input.
pub fn parse_tarktee_locations(xml: &str) -> BTreeMap<String, (String, f64, f64)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut locations: BTreeMap<String, (String, f64, f64)> = BTreeMap::new();

    let mut buf = Vec::new();
    let mut in_location = false;
    let mut current_id: Option<String> = None;
    let mut current_lat: Option<f64> = None;
    let mut current_lon: Option<f64> = None;
    let mut current_name: Option<String> = None;
    let mut in_lat = false;
    let mut in_lon = false;
    let mut in_value = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = e.name();
                let tag = name.as_ref();
                if tag == b"predefinedLocation" {
                    in_location = true;
                    current_id = None;
                    current_lat = None;
                    current_lon = None;
                    current_name = None;
                    // Extract id attribute.
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"id" {
                            current_id =
                                Some(String::from_utf8_lossy(&attr.value).to_string());
                            break;
                        }
                    }
                } else if in_location {
                    if tag == b"latitude" {
                        in_lat = true;
                    } else if tag == b"longitude" {
                        in_lon = true;
                    } else if tag == b"value" {
                        in_value = true;
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                let _ = e;
            }
            Ok(Event::Text(e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                if in_lat {
                    current_lat = text.parse().ok();
                    in_lat = false;
                } else if in_lon {
                    current_lon = text.parse().ok();
                    in_lon = false;
                } else if in_value && current_name.is_none() {
                    current_name = Some(text.trim().to_string());
                }
            }
            Ok(Event::End(ref e)) => {
                let end_name = e.name();
                let tag = end_name.as_ref();
                if tag == b"predefinedLocation" {
                    if let (Some(id), Some(lat), Some(lon)) =
                        (current_id.clone(), current_lat, current_lon)
                    {
                        if lat.is_finite() && lon.is_finite() {
                            let name =
                                current_name.clone().unwrap_or_else(|| id.clone());
                            locations.insert(id, (name, lat, lon));
                        }
                    }
                    in_location = false;
                    current_id = None;
                    current_lat = None;
                    current_lon = None;
                    current_name = None;
                }
                in_lat = false;
                in_lon = false;
                in_value = false;
            }
            Ok(Event::Eof) => break,
            Err(_) => {
                // Lenient: skip malformed events, continue parsing.
                break;
            }
            _ => {}
        }
        buf.clear();
    }
    locations
}

/// Parse traffic-view location-ref id → image URL from DATEX2 images XML.
/// Image URLs are validated against the official tarktee image origin.
pub fn parse_tarktee_images(xml: &str) -> BTreeMap<String, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut images: BTreeMap<String, String> = BTreeMap::new();

    let mut buf = Vec::new();
    let mut in_traffic_view = false;
    let mut in_location_ref = false;
    let mut in_url = false;
    let mut current_ref_id: Option<String> = None;
    let mut current_url: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let start_name = e.name();
                let tag = start_name.as_ref();
                if tag == b"trafficView" {
                    in_traffic_view = true;
                    current_ref_id = None;
                    current_url = None;
                } else if tag == b"linearPredefinedLocationReference" {
                    in_location_ref = true;
                    // Extract id attribute.
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"id" {
                            current_ref_id =
                                Some(String::from_utf8_lossy(&attr.value).to_string());
                            break;
                        }
                    }
                } else if tag == b"urlLinkAddress" {
                    in_url = true;
                }
            }
            Ok(Event::Empty(ref e)) => {
                let empty_name = e.name();
                let tag = empty_name.as_ref();
                // Self-closing <linearPredefinedLocationReference id="..."/> — DATEX2
                // payload emits these as Empty events, not Start.
                if tag == b"linearPredefinedLocationReference" && in_traffic_view {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"id" {
                            current_ref_id =
                                Some(String::from_utf8_lossy(&attr.value).to_string());
                            break;
                        }
                    }
                }
            }
            Ok(Event::Text(e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                if in_url {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        current_url = Some(trimmed);
                    }
                    in_url = false;
                }
            }
            Ok(Event::End(ref e)) => {
                let end_name = e.name();
                let tag = end_name.as_ref();
                if tag == b"trafficView" {
                    if let (Some(ref_id), Some(url)) =
                        (current_ref_id.clone(), current_url.clone())
                    {
                        if url.starts_with(IMAGE_ORIGIN) {
                            images.insert(ref_id, url);
                        }
                    }
                    in_traffic_view = false;
                    current_ref_id = None;
                    current_url = None;
                }
                if tag == b"linearPredefinedLocationReference" {
                    in_location_ref = false;
                }
                if tag == b"urlLinkAddress" {
                    in_url = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => {
                // Lenient: skip malformed events.
                break;
            }
            _ => {}
        }
        buf.clear();
    }
    images
}

/// Join locations + images into CameraRow vec.
fn join_tarktee(
    locations: &BTreeMap<String, (String, f64, f64)>,
    images: &BTreeMap<String, String>,
) -> Vec<CameraRow> {
    let mut out = Vec::new();
    for (id, (name, lat, lon)) in locations {
        let Some(image_url) = images.get(id) else {
            continue;
        };
        if !in_estonia_bbox(*lat, *lon) {
            continue;
        }
        // Extract numeric camera id from URL path: /images/<num>/...
        let num_match = image_url.rsplit('/').nth(1);
        let camera_id = match num_match {
            Some(n) if n.parse::<u64>().is_ok() => format!("ee-tarktee-{n}"),
            _ => format!("ee-tarktee-{id}"),
        };
        out.push(CameraRow {
            id: camera_id,
            city: name.clone(),
            city_id: Some("estonia".to_string()),
            name: name.clone(),
            lat: *lat,
            lon: *lon,
            heading_deg: None,
            fov_deg: Some(44.0),
            pitch_deg: Some(-18.0),
            range_m: Some(145.0),
            mount_height_m: Some(8.0),
            ground_elevation_m: Some(40.0),
            feed_type: "image".to_string(),
            frame_url: Some(image_url.clone()),
            media_url: None,
            provider: "tarktee".to_string(),
            source_kind: Some("tarktee-datex".to_string()),
            heading_confidence: Some("low".to_string()),
            pose_source: None,
            license_note: Some(LICENSE.to_string()),
            credit: Some("Transpordiamet Tarktee".to_string()),
            code: None,
        });
    }
    out
}

pub struct Tarktee;

impl CityCameraProvider for Tarktee {
    fn id(&self) -> &'static str {
        "tarktee"
    }
    fn city(&self) -> &'static str {
        "Estonia"
    }
    fn fetch_catalog<'a>(
        &'a self,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CameraRow>>> + Send + 'a>> {
        Box::pin(async move {
            let (loc_resp, img_resp) = tokio::join! {
                client.get(LOCATIONS_URL)
                    .header("Accept", "application/xml,text/xml,*/*")
                    .send(),
                client.get(IMAGES_URL)
                    .header("Accept", "application/xml,text/xml,*/*")
                    .send(),
            };

            let loc_resp = match loc_resp {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("tarktee locations fetch failed: {e}");
                    return Ok(Vec::new());
                }
            };
            let img_resp = match img_resp {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("tarktee images fetch failed: {e}");
                    return Ok(Vec::new());
                }
            };

            if !loc_resp.status().is_success() {
                tracing::warn!(
                    "tarktee locations HTTP {} — skipping",
                    loc_resp.status()
                );
                return Ok(Vec::new());
            }
            if !img_resp.status().is_success() {
                tracing::warn!("tarktee images HTTP {} — skipping", img_resp.status());
                return Ok(Vec::new());
            }

            let (loc_xml, img_xml) = tokio::join! {
                loc_resp.text(),
                img_resp.text(),
            };

            let loc_xml = match loc_xml {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("tarktee locations body read: {e}");
                    return Ok(Vec::new());
                }
            };
            let img_xml = match img_xml {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("tarktee images body read: {e}");
                    return Ok(Vec::new());
                }
            };

            let locations = parse_tarktee_locations(&loc_xml);
            let images = parse_tarktee_images(&img_xml);

            if locations.is_empty() || images.is_empty() {
                tracing::warn!(
                    "tarktee parse empty: locations={}, images={}",
                    locations.len(),
                    images.len()
                );
                return Ok(Vec::new());
            }

            Ok(join_tarktee(&locations, &images))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCATIONS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<d2LogicalModel xmlns="http://datex2.eu/schema/3/common" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <predefinedLocation id="LOC-001">
    <predefinedLocationTableReference id="TarkteeCameras"/>
    <locationForDisplay>
      <latitude>58.5953</latitude>
      <longitude>25.0136</longitude>
    </locationForDisplay>
    <point by="ReferencePoint">
      <pointCoordinates>
        <latitude>58.5953</latitude>
        <longitude>25.0136</longitude>
        <elevation srsName="WGS84[EPSG:4326]">45.0</elevation>
      </pointCoordinates>
    </point>
    <namedLocation>
      <name>
        <value>Tartu Highway km 45</value>
      </name>
    </namedLocation>
  </predefinedLocation>
  <predefinedLocation id="LOC-002">
    <predefinedLocationTableReference id="TarkteeCameras"/>
    <locationForDisplay>
      <latitude>59.4270</latitude>
      <longitude>24.7428</longitude>
    </locationForDisplay>
    <point by="ReferencePoint">
      <pointCoordinates>
        <latitude>59.4270</latitude>
        <longitude>24.7428</longitude>
      </pointCoordinates>
    </point>
    <namedLocation>
      <name>
        <value>Tallinn Entrance North</value>
      </name>
    </namedLocation>
  </predefinedLocation>
</d2LogicalModel>"#;

    const IMAGES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<d2LogicalModel xmlns="http://datex2.eu/schema/3/common" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <payloadPublication extension="TrafficViewPublicationExtension">
    <trafficView>
      <linearPredefinedLocationReference id="LOC-001"/>
      <urlLink>
        <urlLinkAddress>https://tarktee.transpordiamet.ee/images/1001/cam1.jpg</urlLinkAddress>
        <urlLinkNature>stillImage</urlLinkNature>
      </urlLink>
    </trafficView>
    <trafficView>
      <linearPredefinedLocationReference id="LOC-002"/>
      <urlLink>
        <urlLinkAddress>https://tarktee.transpordiamet.ee/images/1002/cam2.jpg</urlLinkAddress>
        <urlLinkNature>stillImage</urlLinkNature>
      </urlLink>
    </trafficView>
    <trafficView>
      <linearPredefinedLocationReference id="LOC-003"/>
      <urlLink>
        <urlLinkAddress>https://evil.example.com/images/malicious.jpg</urlLinkAddress>
        <urlLinkNature>stillImage</urlLinkNature>
      </urlLink>
    </trafficView>
  </payloadPublication>
</d2LogicalModel>"#;

    #[test]
    fn parse_tarktee_locations_xml() {
        let locations = parse_tarktee_locations(LOCATIONS_XML);
        assert_eq!(locations.len(), 2);
        let (name, lat, lon) = locations.get("LOC-001").unwrap();
        assert_eq!(name, "Tartu Highway km 45");
        assert!((lat - 58.5953).abs() < 0.001);
        assert!((lon - 25.0136).abs() < 0.001);
        let (_, lat2, lon2) = locations.get("LOC-002").unwrap();
        assert!((lat2 - 59.4270).abs() < 0.001);
        assert!((lon2 - 24.7428).abs() < 0.001);
    }

    #[test]
    fn parse_tarktee_images_xml() {
        let images = parse_tarktee_images(IMAGES_XML);
        assert_eq!(images.len(), 2); // LOC-001 and LOC-002; LOC-003 rejected by origin check.
        assert_eq!(
            images.get("LOC-001").unwrap(),
            "https://tarktee.transpordiamet.ee/images/1001/cam1.jpg"
        );
        assert_eq!(
            images.get("LOC-002").unwrap(),
            "https://tarktee.transpordiamet.ee/images/1002/cam2.jpg"
        );
        // Evil origin should be filtered out.
        assert!(images.get("LOC-003").is_none());
    }

    #[test]
    fn parse_tarktee_join() {
        let locations = parse_tarktee_locations(LOCATIONS_XML);
        let images = parse_tarktee_images(IMAGES_XML);
        let rows = join_tarktee(&locations, &images);
        assert_eq!(rows.len(), 2);
        // Camera IDs come from the image URL path segment.
        assert_eq!(rows[0].id, "ee-tarktee-1001");
        assert_eq!(rows[1].id, "ee-tarktee-1002");
        // Estonia bbox check: both are in range.
        for r in &rows {
            assert!(r.lat >= 57.4 && r.lat <= 59.9);
            assert!(r.lon >= 21.5 && r.lon <= 28.4);
            assert!(!r.id.starts_with("static-"));
            assert_eq!(r.provider, "tarktee");
            assert_eq!(r.feed_type, "image");
            assert!(r.frame_url.is_some());
            assert!(r
                .license_note
                .as_ref()
                .unwrap()
                .contains("Transpordiamet"));
        }
    }

    #[test]
    fn parse_tarktee_rejects_out_of_bbox() {
        let xml_locations = r#"<?xml version="1.0" encoding="UTF-8"?>
<d2LogicalModel xmlns="http://datex2.eu/schema/3/common">
  <predefinedLocation id="BAD">
    <locationForDisplay><latitude>55.0</latitude><longitude>10.0</longitude></locationForDisplay>
    <namedLocation><name><value>Out of bounds</value></name></namedLocation>
  </predefinedLocation>
</d2LogicalModel>"#;
        let xml_images = r#"<?xml version="1.0" encoding="UTF-8"?>
<d2LogicalModel xmlns="http://datex2.eu/schema/3/common">
  <payloadPublication>
    <trafficView>
      <linearPredefinedLocationReference id="BAD"/>
      <urlLink><urlLinkAddress>https://tarktee.transpordiamet.ee/images/1/cam.jpg</urlLinkAddress></urlLink>
    </trafficView>
  </payloadPublication>
</d2LogicalModel>"#;
        let locations = parse_tarktee_locations(xml_locations);
        let images = parse_tarktee_images(xml_images);
        let rows = join_tarktee(&locations, &images);
        // Bbox rejects it (55.0 is below Estonia south boundary).
        assert!(rows.is_empty());
    }

    #[test]
    fn parse_malformed_xml_returns_empty_maps() {
        // Completely broken XML — parser should not panic.
        let locations = parse_tarktee_locations("<<<<<not xml at all");
        let images = parse_tarktee_images("also not xml >>>");
        assert!(locations.is_empty());
        assert!(images.is_empty());
    }

    #[test]
    fn parse_tarktee_locations_partial_malformed() {
        // XML with a good entry and some garbage entries.
        let xml = r#"<?xml version="1.0"?>
<d2LogicalModel xmlns="http://datex2.eu/schema/3/common">
  <predefinedLocation id="GOOD">
    <locationForDisplay><latitude>59.0</latitude><longitude>25.0</longitude></locationForDisplay>
    <namedLocation><name><value>Valid Camera</value></name></namedLocation>
  </predefinedLocation>
  <predefinedLocation id="BAD">
    <locationForDisplay><latitude>garbage</latitude><longitude/></locationForDisplay>
  </predefinedLocation>
</d2LogicalModel>"#;
        let locations = parse_tarktee_locations(xml);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations.get("GOOD").unwrap().0, "Valid Camera");
    }

    #[test]
    fn parse_tarktee_locations_missing_coordinates() {
        let xml = r#"<?xml version="1.0"?>
<d2LogicalModel xmlns="http://datex2.eu/schema/3/common">
  <predefinedLocation id="NO-COORDS">
    <namedLocation><name><value>No coords</value></name></namedLocation>
  </predefinedLocation>
</d2LogicalModel>"#;
        let locations = parse_tarktee_locations(xml);
        // No coordinates — should not be included.
        assert!(locations.get("NO-COORDS").is_none());
    }

    #[test]
    fn parse_tarktee_images_missing_url() {
        let xml = r#"<?xml version="1.0"?>
<d2LogicalModel xmlns="http://datex2.eu/schema/3/common">
  <payloadPublication>
    <trafficView>
      <linearPredefinedLocationReference id="LOC-001"/>
      <!-- no urlLinkAddress -->
    </trafficView>
  </payloadPublication>
</d2LogicalModel>"#;
        let images = parse_tarktee_images(xml);
        assert!(images.is_empty());
    }

    #[test]
    fn tarktee_provider_id_and_city() {
        let p = Tarktee;
        assert_eq!(p.id(), "tarktee");
        assert_eq!(p.city(), "Estonia");
    }
}
