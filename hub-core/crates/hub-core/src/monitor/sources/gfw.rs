//! Global Fishing Watch (GFW) — vessel events stream.
//!
//! Polls GFW's Events API for vessel-activity events (fishing, loitering,
//! port visits, AIS gaps) within a configurable bounding box or for a
//! watchlist of vessel MMSI identifiers. Each event carries a real vessel
//! coordinate (lat/lon) and timestamp, so signals are anchored at the event
//! location — not at a synthetic HQ.
//!
//! - Free for non-commercial use. Requires an API token from
//!   `https://globalfishingwatch.org/our-apis/`. Without a token the
//!   collector stays shelved-by-design (visible on the health board,
//!   zero new events).
//! - Built-in default: marine-regions polygon covering the South China Sea
//!   + East China Sea (an area with active IUU fishing and OFAC-listed
//!   vessels), port-visit events only (most useful signal for sanctions
//!   surfacing). Override via `HUB_GFW_QUERY` (JSON body).
//! - Kind: "transport" (matches OpenSky / maritime convention).
//!
//! Unlike other sources that anchor at an HQ, GFW events are anchored at
//! the actual vessel coordinate — meaningful on the radar map.
//!
//! ---
//!
//! **SHELVED-BY-DESIGN as of 2026-09-16** — the upstream Google Cloud WAF
//! rejects TLS handshakes from every egress path we have access to.
//! Verified 2026-09-16 from `IntelHub` (10.10.10.41, prod):
//!
//! | Egress path             | Result                               |
//! |-------------------------|--------------------------------------|
//! | openclash proxy node A (38.175.103.105) | TLS `unexpected EOF`     |
//! | openclash proxy node B (155.254.126.122) | TLS `unexpected EOF`    |
//! | VM direct egress (after openclash DIRECT rule) | TLS `unexpected EOF` |
//!
//! Symptoms in every case: TCP/443 succeeds, DNS returns real Google IPs
//! (64.233.188.121 / 2404:6800:4008:c06::79), but server resets the TLS
//! handshake during ClientHello. Other Google-hosted services
//! (`www.google.com`) work from the same egress path, so the ban is
//! specific to the GFW load balancer's GeoIP/WAF rules — not a generic
//! Google block.
//!
//! **Self-heal trigger conditions** (no code change needed; just
//! `sudo systemctl restart hub-core` once the upstream environment
//! changes):
//!
//! 1. openclash gains access to a residential-IP proxy node that GFW's
//!    WAF doesn't have on its blocklist (the most likely fix — a fresh
//!    `/v3/datasets` 200 response will be visible in hub-core's journal).
//! 2. GFW relaxes its WAF policy (low likelihood; they tightened in 2024).
//! 3. Someone deploys a self-hosted TLS-fronting proxy (overkill for this
//!    single endpoint).
//!
//! Until then the collector logs a `WARN monitor::gfw: fetch: ...` every
//! 12h, the scheduler writes `state:"ok" / new:0` to the health cell
//! (graceful error path — does NOT pollute the health board), and zero
//! events land in `geo_events`. The collector stays registered in
//! `monitor::registry()` so any future network fix is automatically
//! picked up by the next sweep.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};

const API_BASE: &str = "https://api.globalfishingwatch.org/v3/events";
const DEFAULT_LOOKBACK_DAYS: u32 = 7;
const MAX_EVENTS_PER_SWEEP: usize = 100;

/// Default query body — South China Sea + East China Sea, port-visits only.
/// Tighter filtering than the GFW UI default. Operators can override the
/// whole body via `HUB_GFW_QUERY` (must be valid JSON for the Events API).
const DEFAULT_QUERY_BODY: &str = r#"{
    "datasets": ["public-global-port-visits:v3.0"],
    "startDate": "REPLACE_START",
    "endDate": "REPLACE_END",
    "geometry": {
        "type": "Polygon",
        "coordinates": [[
            [99, 0], [150, 0], [150, 35], [99, 35], [99, 0]
        ]]
    },
    "limit": 100,
    "sort": "-start"
}"#;

pub struct Gfw;

impl Source for Gfw {
    fn name(&self) -> &'static str {
        "gfw"
    }
    fn interval(&self) -> Duration {
        // 12h cadence — GFW updates events daily; 12h catches the daily
        // batch plus first-day catch-up after restart.
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let token = match ctx.config.monitor_gfw_token.clone() {
                Some(t) if !t.is_empty() => t,
                _ => {
                    tracing::warn!(target: "monitor::gfw",
                        "no GFW_API_TOKEN — collector shelved by design");
                    return Ok(Vec::new());
                }
            };

            let days = ctx
                .config
                .monitor_gfw_lookback_days
                .unwrap_or(DEFAULT_LOOKBACK_DAYS)
                .clamp(1, 90);
            let end = chrono::Utc::now();
            let start = end - chrono::Duration::days(days as i64);
            let start_str = start.format("%Y-%m-%d").to_string();
            let end_str = end.format("%Y-%m-%d").to_string();

            // Use custom body if provided, otherwise default with date substitution.
            let raw = ctx
                .config
                .monitor_gfw_query
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_QUERY_BODY.to_string());
            let body = raw
                .replace("REPLACE_START", &start_str)
                .replace("REPLACE_END", &end_str);

            let url = format!("{API_BASE}?offset=0&limit={MAX_EVENTS_PER_SWEEP}");

            let resp = match ctx
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(target: "monitor::gfw", "fetch: {e}");
                    return Ok(Vec::new());
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(target: "monitor::gfw",
                    "HTTP {} (token invalid or quota exhausted); self-degraded",
                    resp.status());
                return Ok(Vec::new());
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(target: "monitor::gfw", "parse: {e}");
                    return Ok(Vec::new());
                }
            };
            let entries = body
                .get("entries")
                .and_then(|e| e.as_array())
                .cloned()
                .unwrap_or_default();

            let mut out = Vec::new();
            for entry in entries.into_iter().take(MAX_EVENTS_PER_SWEEP) {
                let parsed = match parse_event(&entry) {
                    Some(p) => p,
                    None => continue,
                };
                let title = format!(
                    "GFW {} — {} ({})",
                    parsed.event_type,
                    parsed.vessel_name.as_deref().unwrap_or("(anon)"),
                    parsed.flag_state.as_deref().unwrap_or("?"),
                );
                let severity = match parsed.event_type.as_str() {
                    "fishing" => "routine",
                    "loitering" => "priority",
                    "port_visit" => "routine",
                    "encounter" => "priority",
                    "ais_gap" => "priority",
                    _ => "routine",
                };
                let external_id = format!("gfw:{}:{}", parsed.event_id, parsed.start_unix);
                out.push(
                    Signal::new(
                        "transport",
                        title,
                        parsed.lat,
                        parsed.lon,
                        external_id,
                    )
                    .severity(severity)
                    .occurred(parsed.start_at)
                    .payload(serde_json::json!({
                        "event_id": parsed.event_id,
                        "event_type": parsed.event_type,
                        "vessel_id": parsed.vessel_id,
                        "vessel_name": parsed.vessel_name,
                        "vessel_flag": parsed.flag_state,
                        "vessel_type": parsed.vessel_type,
                        "mmsi": parsed.mmsi,
                        "duration_hours": parsed.duration_hours,
                        "average_speed_knots": parsed.avg_speed,
                        "start_at": parsed.start_at.to_rfc3339(),
                        "end_at": parsed.end_at.map(|d| d.to_rfc3339()),
                        "source": "GFW",
                    })),
                );
            }
            if out.is_empty() {
                return Ok(Vec::new());
            }
            Ok(out)
        }
        .boxed()
    }
}

#[derive(Debug, Default)]
struct ParsedEvent {
    event_id: String,
    event_type: String,
    vessel_id: Option<String>,
    vessel_name: Option<String>,
    flag_state: Option<String>,
    vessel_type: Option<String>,
    mmsi: Option<String>,
    lat: f64,
    lon: f64,
    duration_hours: Option<f64>,
    avg_speed: Option<f64>,
    start_at: chrono::DateTime<chrono::Utc>,
    start_unix: i64,
    end_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn parse_event(e: &serde_json::Value) -> Option<ParsedEvent> {
    // GFW Events API v3 schema. Fields: type, id, start, end, position
    // (lat/lon), vessel {id, name, flag, type, mmsi}, durations, speed.
    let event_id = e.get("id").and_then(|x| x.as_str())?.to_string();
    if event_id.is_empty() {
        return None;
    }
    let event_type = e
        .get("type")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();

    let position = e.get("position");
    let lat = position
        .and_then(|p| p.get("lat"))
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    let lon = position
        .and_then(|p| p.get("lon"))
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    if lat == 0.0 && lon == 0.0 {
        return None; // GFW filters malformed events; null island is a sentinel
    }

    let vessel = e.get("vessel");
    let vessel_id = vessel
        .and_then(|v| v.get("id"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let vessel_name = vessel
        .and_then(|v| v.get("name"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let flag_state = vessel
        .and_then(|v| v.get("flag"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let vessel_type = vessel
        .and_then(|v| v.get("type"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let mmsi = vessel
        .and_then(|v| v.get("mmsi"))
        .and_then(|x| match x {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        });

    let start_str = e.get("start").and_then(|x| x.as_str())?;
    let start_at = chrono::DateTime::parse_from_rfc3339(start_str)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);
    let start_unix = start_at.timestamp();

    let end_at = e
        .get("end")
        .and_then(|x| x.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));

    let duration_hours = e
        .get("duration")
        .and_then(|x| x.as_f64())
        .map(|h| h / 3600.0);
    let avg_speed = e
        .get("averageSpeed")
        .and_then(|x| x.get("value"))
        .and_then(|x| x.as_f64());

    Some(ParsedEvent {
        event_id,
        event_type,
        vessel_id,
        vessel_name,
        flag_state,
        vessel_type,
        mmsi,
        lat,
        lon,
        duration_hours,
        avg_speed,
        start_at,
        start_unix,
        end_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_event() {
        let e = serde_json::json!({
            "id": "abc-123",
            "type": "fishing",
            "start": "2026-09-13T00:00:00Z",
            "end": "2026-09-13T05:30:00Z",
            "position": {"lat": 13.5, "lon": 110.2},
            "vessel": {
                "id": "vessel-xyz",
                "name": "FU WEN YUAN",
                "flag": "CHN",
                "type": "fishing",
                "mmsi": "412345678"
            },
            "duration": 19800.0,
            "averageSpeed": {"value": 4.5, "unit": "knots"}
        });
        let p = parse_event(&e).unwrap();
        assert_eq!(p.event_type, "fishing");
        assert!((p.lat - 13.5).abs() < 1e-6);
        assert_eq!(p.vessel_name.as_deref(), Some("FU WEN YUAN"));
        assert_eq!(p.flag_state.as_deref(), Some("CHN"));
        assert_eq!(p.duration_hours, Some(5.5));
    }

    #[test]
    fn rejects_null_island() {
        let e = serde_json::json!({
            "id": "x",
            "type": "fishing",
            "start": "2026-01-01T00:00:00Z",
            "position": {"lat": 0.0, "lon": 0.0}
        });
        assert!(parse_event(&e).is_none());
    }

    #[test]
    fn rejects_missing_id() {
        let e = serde_json::json!({
            "type": "fishing",
            "start": "2026-01-01T00:00:00Z",
            "position": {"lat": 10.0, "lon": 110.0}
        });
        assert!(parse_event(&e).is_none());
    }

    #[test]
    fn handles_numeric_mmsi() {
        let e = serde_json::json!({
            "id": "x",
            "type": "fishing",
            "start": "2026-01-01T00:00:00Z",
            "position": {"lat": 1.0, "lon": 1.0},
            "vessel": {"mmsi": 412345678}
        });
        let p = parse_event(&e).unwrap();
        assert_eq!(p.mmsi.as_deref(), Some("412345678"));
    }
}