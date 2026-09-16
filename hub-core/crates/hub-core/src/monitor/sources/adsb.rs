//! adsb.lol per-aircraft tracks (Globe P1). Keyless. Dual-channel:
//! full snapshot → Redis `hub:globe:aircraft` (TTL 60s, expiry = death
//! detector); notable events (squawk 7700/7500/7600, military in hotspot)
//! → Signal → geo_events. Positions NEVER touch PG (spec §2.2).

use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use serde_json::json;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};
use super::opensky::HOTSPOTS;

pub const AIRCRAFT_KEY: &str = "hub:globe:aircraft";

const REGIONS_TOTAL: usize = 11; // 10 hotspot points + 1 global /v2/mil

pub struct AdsbPoint {
    pub hex: String,
    pub flight: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f64,
    pub gs: Option<f64>,
    pub track: Option<f64>,
    pub squawk: Option<String>,
    pub mil: bool,
    pub seen: f64,
}

pub fn parse_ac_array(j: &serde_json::Value) -> Vec<AdsbPoint> {
    let Some(arr) = j.get("ac").and_then(|a| a.as_array()) else { return Vec::new() };
    arr.iter()
        .filter_map(|v| {
            let lat = v.get("lat")?.as_f64()?;
            let lon = v.get("lon")?.as_f64()?;
            let alt_ft = v.get("alt_baro").and_then(|a| a.as_f64()).unwrap_or(0.0); // "ground" → 0
            Some(AdsbPoint {
                hex: v.get("hex")?.as_str()?.to_string(),
                flight: v.get("flight").and_then(|f| f.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                lat,
                lon,
                alt_m: alt_ft * 0.3048,
                gs: v.get("gs").and_then(|g| g.as_f64()),
                track: v.get("track").and_then(|t| t.as_f64()),
                squawk: v.get("squawk").and_then(|s| s.as_str()).map(str::to_string),
                mil: v.get("dbFlags").and_then(|f| f.as_i64()).map(|f| f & 1 == 1).unwrap_or(false),
                seen: v.get("seen").and_then(|s| s.as_f64()).unwrap_or(f64::MAX),
            })
        })
        .collect()
}

/// (severity, external_id). Hour-bucketed idempotency, opensky.rs pattern.
pub fn classify_notable(ac: &AdsbPoint, in_hotspot: bool, hour_bucket: &str) -> Option<(&'static str, String)> {
    match ac.squawk.as_deref() {
        Some("7700") => return Some(("flash", format!("adsb:{}:7700:{hour_bucket}", ac.hex))),
        Some(s @ ("7500" | "7600")) => return Some(("priority", format!("adsb:{}:{s}:{hour_bucket}", ac.hex))),
        _ => {}
    }
    if ac.mil && in_hotspot {
        return Some(("routine", format!("adsb:{}:mil:{hour_bucket}", ac.hex)));
    }
    None
}

/// Dedupe by hex, keep lowest `seen` (most recent).
pub fn merge_aircraft(batches: Vec<Vec<AdsbPoint>>) -> Vec<AdsbPoint> {
    let mut by_hex: std::collections::HashMap<String, AdsbPoint> = std::collections::HashMap::new();
    for b in batches {
        for ac in b {
            match by_hex.get(&ac.hex) {
                Some(cur) if cur.seen <= ac.seen => {}
                _ => {
                    by_hex.insert(ac.hex.clone(), ac);
                }
            }
        }
    }
    // Hex-sorted output ⇒ deterministic envelope ordering (HashMap iteration
    // order is random per process; the globe UI diffing and acceptance
    // snapshots both need a stable aircraft list).
    let mut v: Vec<AdsbPoint> = by_hex.into_values().collect();
    v.sort_by(|a, b| a.hex.cmp(&b.hex));
    v
}

pub fn snapshot_envelope(aircraft: &[AdsbPoint], regions_ok: usize) -> serde_json::Value {
    json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "count": aircraft.len(),
        "regions_ok": regions_ok,
        "coverage": "hotspots+mil",
        "aircraft": aircraft.iter().map(|a| json!({
            "hex": a.hex, "flight": a.flight, "lat": a.lat, "lon": a.lon,
            "alt_m": a.alt_m.round() as i64, "gs": a.gs, "track": a.track,
            "squawk": a.squawk, "mil": a.mil,
        })).collect::<Vec<_>>(),
    })
}

pub struct Adsb;

fn in_hotspot(lat: f64, lon: f64) -> bool {
    HOTSPOTS
        .iter()
        .any(|(lamin, lomin, lamax, lomax, _)| lat >= *lamin && lat <= *lamax && lon >= *lomin && lon <= *lomax)
}

impl Source for Adsb {
    fn name(&self) -> &'static str {
        "adsb"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(15)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let hour_bucket = chrono::Utc::now().format("%Y%m%d%H").to_string();
            let urls: Vec<String> = HOTSPOTS
                .iter()
                .map(|(lamin, lomin, lamax, lomax, _)| {
                    format!(
                        "https://api.adsb.lol/v2/point/{:.2}/{:.2}/250",
                        (lamin + lamax) / 2.0,
                        (lomin + lomax) / 2.0
                    )
                })
                .chain(std::iter::once("https://api.adsb.lol/v2/mil".to_string()))
                .collect();
            // Concurrent fan-out (4 in flight), each under Ctx's 25s ceiling.
            let results: Vec<Option<Vec<AdsbPoint>>> = futures::stream::iter(urls)
                .map(|url| {
                    let http = &ctx.http;
                    async move {
                        match http.get(&url).send().await {
                            Ok(r) if r.status().is_success() => {
                                r.json::<serde_json::Value>().await.ok().map(|j| parse_ac_array(&j))
                            }
                            Ok(r) => {
                                tracing::warn!(status = %r.status(), url, "adsb upstream non-2xx");
                                None
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, url, "adsb upstream failed");
                                None
                            }
                        }
                    }
                })
                .buffer_unordered(4)
                .collect()
                .await;
            let regions_ok = results.iter().filter(|r| r.is_some()).count();
            if regions_ok == 0 {
                return Err(HubError::sensor(format!(
                    "adsb: all {REGIONS_TOTAL} endpoints failed this tick"
                )));
            }
            let merged = merge_aircraft(results.into_iter().flatten().collect());
            // Channel 1: full snapshot → Redis (60s TTL = death detector).
            let envelope = snapshot_envelope(&merged, regions_ok);
            let _: Option<String> = ctx
                .state
                .redis_timed(
                    redis::cmd("SETEX").arg(AIRCRAFT_KEY).arg(60).arg(envelope.to_string()).clone(),
                    2000,
                )
                .await;
            // Channel 2: notable events → Signal → geo_events.
            let signals = merged
                .iter()
                .filter_map(|ac| {
                    let (sev, ext_id) = classify_notable(ac, in_hotspot(ac.lat, ac.lon), &hour_bucket)?;
                    let flight = ac.flight.clone().unwrap_or_else(|| ac.hex.clone());
                    let what = match sev {
                        "flash" => "squawking 7700 (emergency)",
                        "priority" => "squawking 7500/7600",
                        _ => "military activity in hotspot",
                    };
                    Some(
                        Signal::new("flight", format!("{flight} {what}"), ac.lat, ac.lon, ext_id)
                            .severity(sev)
                            .payload(json!({
                                "hex": ac.hex, "flight": ac.flight, "squawk": ac.squawk,
                                "alt_m": ac.alt_m, "gs": ac.gs, "track": ac.track, "mil": ac.mil,
                            })),
                    )
                })
                .collect();
            Ok(signals)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ac_array_extracts_positions() {
        let j = json!({"ac": [{"hex":"a1b2c3","flight":"UAL123  ","lat":31.2,"lon":121.4,"alt_baro":37000,"gs":452.0,"track":92.0,"squawk":"2000","dbFlags":0,"seen":1.2}]});
        let ac = parse_ac_array(&j);
        assert_eq!(ac.len(), 1);
        assert_eq!(ac[0].flight.as_deref(), Some("UAL123"));
        assert!((ac[0].alt_m - 11277.6).abs() < 0.5);
        assert!(!ac[0].mil);
    }

    #[test]
    fn parse_ac_array_handles_ground_and_missing() {
        let j = json!({"ac": [{"hex":"deadbeef","lat":1.0,"lon":2.0,"alt_baro":"ground"}]});
        let ac = parse_ac_array(&j);
        assert_eq!(ac.len(), 1);
        assert_eq!(ac[0].alt_m, 0.0);
        assert!(ac[0].flight.is_none());
    }

    #[test]
    fn classify_emergency_squawk_is_flash() {
        let ac = AdsbPoint { hex: "abc".into(), flight: None, lat: 0.0, lon: 0.0, alt_m: 0.0, gs: None, track: None, squawk: Some("7700".into()), mil: false, seen: 0.0 };
        let (sev, id) = classify_notable(&ac, false, "2026091708").unwrap();
        assert_eq!(sev, "flash");
        assert_eq!(id, "adsb:abc:7700:2026091708");
    }

    #[test]
    fn classify_military_requires_hotspot() {
        let ac = AdsbPoint { hex: "abc".into(), flight: None, lat: 0.0, lon: 0.0, alt_m: 0.0, gs: None, track: None, squawk: None, mil: true, seen: 0.0 };
        assert!(classify_notable(&ac, false, "2026091708").is_none());
        let (sev, _) = classify_notable(&ac, true, "2026091708").unwrap();
        assert_eq!(sev, "routine");
    }

    #[test]
    fn merge_keeps_freshest_by_seen() {
        let old = AdsbPoint { hex: "abc".into(), flight: None, lat: 10.0, lon: 0.0, alt_m: 0.0, gs: None, track: None, squawk: None, mil: false, seen: 30.0 };
        let fresh = AdsbPoint { hex: "abc".into(), flight: None, lat: 20.0, lon: 0.0, alt_m: 0.0, gs: None, track: None, squawk: None, mil: false, seen: 1.0 };
        let m = merge_aircraft(vec![vec![old], vec![fresh]]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].lat, 20.0);
    }

    #[test]
    fn classify_priority_squawk_hijack_and_radio() {
        let ac = AdsbPoint { hex: "abc".into(), flight: None, lat: 0.0, lon: 0.0, alt_m: 0.0, gs: None, track: None, squawk: Some("7500".into()), mil: false, seen: 0.0 };
        let (sev, id) = classify_notable(&ac, false, "2026091708").unwrap();
        assert_eq!(sev, "priority");
        assert_eq!(id, "adsb:abc:7500:2026091708");
    }

    #[test]
    fn classify_military_hotspot_external_id_format() {
        let ac = AdsbPoint { hex: "a1b2".into(), flight: None, lat: 0.0, lon: 0.0, alt_m: 0.0, gs: None, track: None, squawk: None, mil: true, seen: 0.0 };
        let (sev, id) = classify_notable(&ac, true, "2026091708").unwrap();
        assert_eq!(sev, "routine");
        assert_eq!(id, "adsb:a1b2:mil:2026091708");
    }

    #[test]
    fn merge_output_is_hex_sorted() {
        let mk = |hex: &str, seen: f64| AdsbPoint { hex: hex.into(), flight: None, lat: 0.0, lon: 0.0, alt_m: 0.0, gs: None, track: None, squawk: None, mil: false, seen };
        let m = merge_aircraft(vec![vec![mk("ff", 1.0), mk("00", 1.0)], vec![mk("80", 1.0)]]);
        assert_eq!(m.iter().map(|a| a.hex.as_str()).collect::<Vec<_>>(), vec!["00", "80", "ff"]);
    }

    #[test]
    fn regions_total_matches_hotspot_list_plus_mil() {
        // REGIONS_TOTAL only appears in the all-endpoints-failed message, but a
        // silent drift would make that message lie about the fan-out width.
        assert_eq!(REGIONS_TOTAL, HOTSPOTS.len() + 1);
    }

    #[test]
    fn snapshot_envelope_shape() {
        let ac = AdsbPoint { hex: "abc".into(), flight: Some("X".into()), lat: 1.0, lon: 2.0, alt_m: 100.0, gs: None, track: None, squawk: None, mil: true, seen: 0.0 };
        let env = snapshot_envelope(&[ac], 11);
        assert_eq!(env["count"], 1);
        assert_eq!(env["coverage"], "hotspots+mil");
        assert_eq!(env["regions_ok"], 11);
        assert_eq!(env["aircraft"][0]["hex"], "abc");
        assert_eq!(env["aircraft"][0]["mil"], true);
    }
}
