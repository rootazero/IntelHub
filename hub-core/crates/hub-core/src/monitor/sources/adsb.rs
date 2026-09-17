//! adsb.lol per-aircraft tracks (Globe P1). Keyless. Dual-channel:
//! full snapshot → Redis `hub:globe:aircraft` (TTL 300s, expiry = death
//! detector); notable events (squawk 7700/7500/7600, military in hotspot)
//! → Signal → geo_events. Positions NEVER touch PG (spec §2.2).
//!
//! Rate-limit-safe rotation (controller ruling, 2026-09-17): upstream quota
//! is ~2–5 req/min (measured: burst capacity ~4–8, refill ~1 token/20–30s),
//! so the old 11-endpoint × 15s fan-out (44 req/min) hammered it into 429s
//! while `regions_ok > 0` kept the fetch Ok and the scheduler backoff never
//! fired. Now: ONE request per tick, rotating through a 14-item queue
//! (mil → squawk7700/7500/7600 → 10 hotspot points) = ~4 req/min. A failed
//! tick returns Err (backoff = 429 cooldown) and retries the same item.

use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};
use super::opensky::HOTSPOTS;

pub const AIRCRAFT_KEY: &str = "hub:globe:aircraft";

/// Work-queue length: 1 mil + 3 squawk + 10 hotspot points. Full cycle at the
/// 15s tick = 210s ≈ 4 req/min, inside the measured upstream quota.
const QUEUE_LEN: usize = 14;
/// Snapshot TTL: the globe only loses aircraft if the collector itself is
/// dead for >5min, not because a single endpoint 429'd one tick.
const SNAPSHOT_TTL_SECS: usize = 300;
/// Drop aircraft whose freshest sighting is older than this.
const STALE_SECS: i64 = 600;
/// Full-queue cycle in seconds (QUEUE_LEN × 15s tick); surfaced in the
/// envelope so consumers know the worst-case data age.
const CYCLE_SECS: i64 = QUEUE_LEN as i64 * 15;

#[derive(Clone)]
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

/// Task 4 contract retained: the production path uses apply_tick; this
/// function serves batch-level merge semantics and tests.
/// Dedupe by hex, keep lowest `seen` (most recent). Batch-level helper kept
/// from Task 4; the live collector's cross-tick dedupe lives in apply_tick.
pub fn merge_aircraft(batches: Vec<Vec<AdsbPoint>>) -> Vec<AdsbPoint> {
    let mut by_hex: HashMap<String, AdsbPoint> = HashMap::new();
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

/// Work-queue item for rotation cursor `pos`. Pure — it never mutates state,
/// so a failed tick (which returns before advancing the cursor) naturally
/// re-selects the SAME item on the next pass.
fn queue_item(pos: usize) -> (String, String) {
    match pos % QUEUE_LEN {
        0 => ("mil".into(), "https://api.adsb.lol/v2/mil".into()),
        1 => ("squawk7700".into(), "https://api.adsb.lol/v2/squawk/7700".into()),
        2 => ("squawk7500".into(), "https://api.adsb.lol/v2/squawk/7500".into()),
        3 => ("squawk7600".into(), "https://api.adsb.lol/v2/squawk/7600".into()),
        i => {
            let h = i - 4; // hotspot index
            let (lamin, lomin, lamax, lomax, _) = HOTSPOTS[h];
            (
                format!("point:hotspot_{h}"),
                format!(
                    "https://api.adsb.lol/v2/point/{:.2}/{:.2}/250",
                    (lamin + lamax) / 2.0,
                    (lomin + lomax) / 2.0
                ),
            )
        }
    }
}

/// Success-path state transition (pure; fetch wires it to the Mutex fields):
/// advance the rotation cursor, merge this tick's batch into the cumulative
/// snapshot (same hex keeps the highest seen_abs = freshest sighting,
/// matching merge_aircraft's lowest-relative-seen semantics), drop entries unseen for > STALE_SECS, and
/// build the full-rewrite envelope (hex-sorted).
fn apply_tick(
    pos: &mut usize,
    snap: &mut HashMap<String, (AdsbPoint, i64)>,
    batch: Vec<AdsbPoint>,
    now: i64,
    last_tick: &str,
) -> serde_json::Value {
    *pos = (*pos + 1) % QUEUE_LEN;
    for ac in batch {
        let seen_abs = now - ac.seen.round() as i64;
        match snap.get(&ac.hex) {
            Some((_, cur_abs)) if *cur_abs >= seen_abs => {}
            _ => {
                snap.insert(ac.hex.clone(), (ac, seen_abs));
            }
        }
    }
    snap.retain(|_, (_, seen_abs)| now - *seen_abs <= STALE_SECS);
    let mut all: Vec<(AdsbPoint, i64)> = snap.values().cloned().collect();
    all.sort_by(|a, b| a.0.hex.cmp(&b.0.hex));
    snapshot_envelope(&all, now, last_tick, "hotspots+mil+squawk", CYCLE_SECS)
}

/// Full-rewrite envelope (hex-sorted). `coverage`/`cycle_secs` are params so
/// the OpenSky OAuth collector (GEV P2 T13) can reuse the exact same shape
/// with its own values (`opensky` / 300).
pub fn snapshot_envelope(
    aircraft: &[(AdsbPoint, i64)],
    now: i64,
    last_tick: &str,
    coverage: &str,
    cycle_secs: i64,
) -> serde_json::Value {
    json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "count": aircraft.len(),
        "coverage": coverage,
        "cycle_secs": cycle_secs,
        "last_tick": last_tick,
        "aircraft": aircraft.iter().map(|(a, seen_abs)| json!({
            "hex": a.hex, "flight": a.flight, "lat": a.lat, "lon": a.lon,
            "alt_m": a.alt_m.round() as i64, "gs": a.gs, "track": a.track,
            "squawk": a.squawk, "mil": a.mil,
            "age_s": now - seen_abs,
        })).collect::<Vec<_>>(),
    })
}

/// Ruling 3 (GEV P2 T13): REST read-time merge of the adsb.lol rotating
/// snapshot (`AIRCRAFT_KEY`, TTL 300s) with the OpenSky OAuth full snapshot
/// (`opensky::OPENSKY_AIRCRAFT_KEY`, TTL 120s). Dedupe by hex, fresher wins
/// (smaller `age_s`); when either side lacks age info the adsb row wins
/// (adsb priority, ruling). The adsb envelope is preserved verbatim except
/// `count`/`coverage`/`aircraft`, so with no OpenSky snapshot the response
/// is byte-identical to the pre-merge shape (`hotspots+mil+squawk`,
/// `last_tick`, `cycle_secs` — T15's sp6 globe check relies on this).
pub(crate) fn merge_globe_snapshots(adsb: Option<&serde_json::Value>, opensky: Option<&serde_json::Value>) -> serde_json::Value {
    fn rows(v: Option<&serde_json::Value>) -> Vec<serde_json::Value> {
        v.and_then(|e| e.get("aircraft"))
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default()
    }
    let adsb_rows = rows(adsb);
    let os_rows = rows(opensky);
    if os_rows.is_empty() {
        return match adsb {
            Some(env) => env.clone(),
            None => json!({ "stale": true, "aircraft": [] }),
        };
    }
    let age_of = |r: &serde_json::Value| r.get("age_s").and_then(|a| a.as_f64());
    fn hex_of(r: &serde_json::Value) -> &str {
        r.get("hex").and_then(|h| h.as_str()).unwrap_or("")
    }
    let mut by_hex: HashMap<String, serde_json::Value> = HashMap::new();
    for r in adsb_rows {
        let h = hex_of(&r);
        if !h.is_empty() {
            by_hex.insert(h.to_string(), r);
        }
    }
    for r in os_rows {
        let h = hex_of(&r);
        if h.is_empty() {
            continue;
        }
        let replace = match by_hex.get(h) {
            None => true,
            Some(cur) => match (age_of(&r), age_of(cur)) {
                // Both sides aged → fresher (smaller age_s) wins.
                (Some(os_age), Some(ad_age)) => os_age < ad_age,
                // Any missing age → adsb priority (ruling).
                _ => false,
            },
        };
        if replace {
            by_hex.insert(h.to_string(), r);
        }
    }
    let mut merged: Vec<serde_json::Value> = by_hex.into_values().collect();
    merged.sort_by(|a, b| hex_of(a).cmp(hex_of(b)));
    let mut out = match adsb {
        Some(env) => env.clone(),
        None => json!({}),
    };
    // Backward-compatible coverage concat: `hotspots+mil+squawk` +
    // `+opensky` (idempotent — never double-append).
    let base_cov = adsb
        .and_then(|e| e.get("coverage"))
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty());
    out["coverage"] = match base_cov {
        Some(c) if c.contains("opensky") => json!(c),
        Some(c) => json!(format!("{c}+opensky")),
        None => json!("opensky"),
    };
    out["count"] = json!(merged.len());
    out["aircraft"] = json!(merged);
    out
}

pub struct Adsb {
    /// Rotation cursor into the work queue. The Source trait object lives in
    /// the registry's `Box<dyn Source>` for the whole process lifetime, so
    /// this instance state persists across ticks — that persistence is the
    /// foundation of the 1-request-per-tick rotation and the cumulative
    /// snapshot.
    pos: Mutex<usize>,
    /// Cumulative snapshot: hex → (freshest point, seen_abs unix secs).
    snap: Mutex<HashMap<String, (AdsbPoint, i64)>>,
}

impl Default for Adsb {
    fn default() -> Self {
        Self { pos: Mutex::new(0), snap: Mutex::new(HashMap::new()) }
    }
}

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
            let now = chrono::Utc::now().timestamp();
            let hour_bucket = chrono::Utc::now().format("%Y%m%d%H").to_string();
            // Select this tick's ONE request. The guard is dropped before the
            // HTTP await — a Mutex guard must never be held across .await.
            let (label, url) = queue_item(*self.pos.lock().unwrap_or_else(|p| p.into_inner()));
            // Single request. ANY failure (HTTP error / non-2xx / bad JSON)
            // returns Err so the scheduler backoff fires (30s→600s cap =
            // the 429 cooldown); the cursor is untouched, so the next tick
            // retries this same item.
            let batch: Vec<AdsbPoint> = match ctx.http.get(&url).send().await {
                Ok(r) if r.status().is_success() => {
                    let j = r.json::<serde_json::Value>().await.map_err(|e| {
                        tracing::warn!(error = %e, url = %url, "adsb upstream bad json");
                        HubError::sensor(format!("adsb: {label}: bad json body: {e}"))
                    })?;
                    parse_ac_array(&j)
                }
                Ok(r) => {
                    tracing::warn!(status = %r.status(), url = %url, "adsb upstream non-2xx");
                    return Err(HubError::sensor(format!("adsb: {label}: upstream status {}", r.status())));
                }
                Err(e) => {
                    tracing::warn!(error = %e, url = %url, "adsb upstream failed");
                    return Err(HubError::sensor(format!("adsb: {label}: {e}")));
                }
            };
            // Channel 2: notable events from THIS tick's batch. Squawk-endpoint
            // aircraft carry their squawk code, so emergency classification is
            // now global (not hotspot-bounded); the mil rule stays
            // "mil AND in hotspot → routine".
            let signals: Vec<Signal> = batch
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
            // Channel 1: advance cursor, merge batch into the cumulative
            // snapshot, prune stale entries, full-rewrite the Redis envelope.
            let envelope = {
                let mut pos = self.pos.lock().unwrap_or_else(|p| p.into_inner());
                let mut snap = self.snap.lock().unwrap_or_else(|p| p.into_inner());
                apply_tick(&mut pos, &mut snap, batch, now, &label)
            };
            // Redis write failure must be visible but must NOT fail the
            // tick: channel-2 emergency events (7700/7500/7600) still flow
            // via signals. If redis stays down, the 300s TTL expires and the
            // frontend shows the STALE badge; backoff only fires on upstream
            // failures.
            let wrote: Option<String> = ctx
                .state
                .redis_timed(
                    redis::cmd("SETEX").arg(AIRCRAFT_KEY).arg(SNAPSHOT_TTL_SECS).arg(envelope.to_string()).clone(),
                    2000,
                )
                .await;
            if wrote.is_none() {
                tracing::warn!("adsb snapshot write failed (redis down?)");
            }
            Ok(signals)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(hex: &str, seen: f64) -> AdsbPoint {
        AdsbPoint { hex: hex.into(), flight: None, lat: 0.0, lon: 0.0, alt_m: 0.0, gs: None, track: None, squawk: None, mil: false, seen }
    }

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
        let old = mk("abc", 30.0);
        let fresh = AdsbPoint { lat: 20.0, ..mk("abc", 1.0) };
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
        let m = merge_aircraft(vec![vec![mk("ff", 1.0), mk("00", 1.0)], vec![mk("80", 1.0)]]);
        assert_eq!(m.iter().map(|a| a.hex.as_str()).collect::<Vec<_>>(), vec!["00", "80", "ff"]);
    }

    #[test]
    fn queue_len_matches_work_queue() {
        // Drift guard: the queue is mil + 3 squawk + one point per hotspot.
        // QUEUE_LEN is baked into the rotation modulo and the cycle-time math.
        assert_eq!(QUEUE_LEN, HOTSPOTS.len() + 4);
    }

    #[test]
    fn queue_item_order_and_url_mapping() {
        let (l0, u0) = queue_item(0);
        assert_eq!((l0.as_str(), u0.as_str()), ("mil", "https://api.adsb.lol/v2/mil"));
        let (l1, u1) = queue_item(1);
        assert_eq!((l1.as_str(), u1.as_str()), ("squawk7700", "https://api.adsb.lol/v2/squawk/7700"));
        let (l2, u2) = queue_item(2);
        assert_eq!((l2.as_str(), u2.as_str()), ("squawk7500", "https://api.adsb.lol/v2/squawk/7500"));
        let (l3, u3) = queue_item(3);
        assert_eq!((l3.as_str(), u3.as_str()), ("squawk7600", "https://api.adsb.lol/v2/squawk/7600"));
        // Point items follow hotspot order and match the opensky HOTSPOTS table.
        for h in 0..HOTSPOTS.len() {
            let (label, url) = queue_item(4 + h);
            assert_eq!(label, format!("point:hotspot_{h}"));
            let (lamin, lomin, lamax, lomax, _) = HOTSPOTS[h];
            assert_eq!(
                url,
                format!("https://api.adsb.lol/v2/point/{:.2}/{:.2}/250", (lamin + lamax) / 2.0, (lomin + lomax) / 2.0)
            );
        }
        // Cursor wraps modulo QUEUE_LEN.
        assert_eq!(queue_item(QUEUE_LEN).1, queue_item(0).1);
    }

    #[test]
    fn rotation_advances_on_success_holds_on_failure() {
        let mut pos = 0usize;
        let mut snap = HashMap::new();
        // Failure path: fetch returns Err BEFORE apply_tick, cursor untouched.
        let failed_url = queue_item(pos).1;
        assert_eq!(failed_url, queue_item(pos).1, "failed tick retries same item");
        // Success path: apply_tick advances the cursor.
        let env = apply_tick(&mut pos, &mut snap, vec![mk("aa", 0.0)], 1000, "mil");
        assert_eq!(pos, 1);
        assert_eq!(env["last_tick"], "mil");
        assert_ne!(queue_item(pos).1, failed_url);
        // Cursor wraps at QUEUE_LEN.
        for _ in 0..QUEUE_LEN - 1 {
            apply_tick(&mut pos, &mut snap, vec![], 1000, "x");
        }
        assert_eq!(pos, 0);
    }

    #[test]
    fn apply_tick_merges_batch_keeping_fresher_seen_abs() {
        let mut pos = 0usize;
        let mut snap = HashMap::new();
        // Tick at t=1000: hex aa seen 100s ago (abs 900), hex bb seen 5s ago.
        apply_tick(&mut pos, &mut snap, vec![mk("aa", 100.0), mk("bb", 5.0)], 1000, "point:hotspot_0");
        // Tick at t=1100: aa seen 10s ago (abs 1090 > 900 → fresher, must
        // replace); bb is absent from this batch and survives untouched.
        let newer = AdsbPoint { lat: 55.0, ..mk("aa", 10.0) };
        let env = apply_tick(&mut pos, &mut snap, vec![newer], 1100, "squawk7700");
        assert_eq!(env["count"], 2);
        let hexes: Vec<&str> = env["aircraft"].as_array().unwrap().iter().map(|a| a["hex"].as_str().unwrap()).collect();
        assert_eq!(hexes, vec!["aa", "bb"]); // hex-sorted
        let aa = env["aircraft"].as_array().unwrap().iter().find(|a| a["hex"] == "aa").unwrap();
        assert_eq!(aa["lat"], 55.0);
        assert_eq!(aa["age_s"], 10); // now(1100) - seen_abs(1090)
        let bb = env["aircraft"].as_array().unwrap().iter().find(|a| a["hex"] == "bb").unwrap();
        assert_eq!(bb["age_s"], 105); // now(1100) - seen_abs(995): retained from tick 1
    }

    #[test]
    fn apply_tick_prunes_entries_older_than_600s() {
        let mut pos = 0usize;
        let mut snap = HashMap::new();
        // Tick at t=0: aa seen just now → seen_abs 0.
        apply_tick(&mut pos, &mut snap, vec![mk("aa", 0.0)], 0, "mil");
        assert_eq!(snap.len(), 1);
        // Tick at t=700 with an empty batch: aa's age is 700s > 600s → pruned.
        let env = apply_tick(&mut pos, &mut snap, vec![], 700, "mil");
        assert_eq!(snap.len(), 0);
        assert_eq!(env["count"], 0);
        // Boundary: age exactly 600s survives.
        apply_tick(&mut pos, &mut snap, vec![mk("bb", 0.0)], 1000, "mil");
        let env = apply_tick(&mut pos, &mut snap, vec![], 1600, "mil");
        assert_eq!(env["count"], 1);
        assert_eq!(env["aircraft"][0]["age_s"], 600);
    }

    #[test]
    fn snapshot_envelope_shape() {
        let ac = AdsbPoint { flight: Some("X".into()), lat: 1.0, lon: 2.0, alt_m: 100.0, mil: true, ..mk("abc", 0.0) };
        let env = snapshot_envelope(&[(ac, 970)], 1000, "squawk7700", "hotspots+mil+squawk", CYCLE_SECS);
        assert_eq!(env["count"], 1);
        assert_eq!(env["coverage"], "hotspots+mil+squawk");
        assert_eq!(env["cycle_secs"], 210);
        assert_eq!(env["last_tick"], "squawk7700");
        assert!(env.get("regions_ok").is_none());
        assert_eq!(env["aircraft"][0]["hex"], "abc");
        assert_eq!(env["aircraft"][0]["mil"], true);
        assert_eq!(env["aircraft"][0]["age_s"], 30);
    }

    // ---------- GEV P2 T13: REST read-time merge (Ruling 3) ----------

    fn adsb_env(rows: serde_json::Value) -> serde_json::Value {
        let count = rows.as_array().map(|a| a.len()).unwrap_or(0);
        json!({
            "ts": "2026-09-17T00:00:00Z",
            "count": count,
            "coverage": "hotspots+mil+squawk",
            "cycle_secs": 210,
            "last_tick": "mil",
            "aircraft": rows,
        })
    }

    fn ac_row(hex: &str, age_s: Option<i64>, mil: bool) -> serde_json::Value {
        let mut r = json!({
            "hex": hex, "flight": hex.to_uppercase(), "lat": 1.0, "lon": 2.0,
            "alt_m": 1000, "gs": 250.0, "track": 90.0, "squawk": null, "mil": mil,
        });
        if let Some(a) = age_s {
            r["age_s"] = json!(a);
        }
        r
    }

    #[test]
    fn merge_fresher_wins_by_age_s() {
        let adsb = adsb_env(json!([
            ac_row("aa", Some(100), true),
            ac_row("bb", Some(50), false),
        ]));
        let opensky = json!({
            "count": 2, "coverage": "opensky",
            "aircraft": [ac_row("aa", Some(5), false), ac_row("cc", Some(9), false)],
        });
        let m = merge_globe_snapshots(Some(&adsb), Some(&opensky));
        // aa: opensky age 5 < adsb age 100 → opensky row wins.
        // bb: only in adsb → kept. cc: only in opensky → added.
        assert_eq!(m["count"], 3);
        let rows = m["aircraft"].as_array().unwrap();
        assert_eq!(rows.iter().map(|r| r["hex"].as_str().unwrap()).collect::<Vec<_>>(), vec!["aa", "bb", "cc"]);
        let aa = rows.iter().find(|r| r["hex"] == "aa").unwrap();
        assert_eq!(aa["age_s"], 5);
        assert_eq!(aa["mil"], false, "opensky row replaced the adsb mil row");
    }

    #[test]
    fn merge_adsb_wins_when_age_missing() {
        let adsb = adsb_env(json!([ac_row("aa", Some(100), true)]));
        // Opensky row without age_s → adsb priority (ruling).
        let opensky = json!({"aircraft": [ac_row("aa", None, false)]});
        let m = merge_globe_snapshots(Some(&adsb), Some(&opensky));
        assert_eq!(m["count"], 1);
        assert_eq!(m["aircraft"][0]["mil"], true, "adsb row kept");
        // And when the ADSB row lacks age: adsb still wins.
        let adsb2 = adsb_env(json!([ac_row("aa", None, true)]));
        let m2 = merge_globe_snapshots(Some(&adsb2), Some(&opensky));
        assert_eq!(m2["aircraft"][0]["mil"], true);
    }

    #[test]
    fn merge_coverage_concat_is_backward_compatible() {
        let adsb = adsb_env(json!([ac_row("aa", Some(10), false)]));
        // No opensky snapshot → byte-level fields preserved, coverage unchanged.
        let m = merge_globe_snapshots(Some(&adsb), None);
        assert_eq!(m["coverage"], "hotspots+mil+squawk");
        assert_eq!(m["last_tick"], "mil");
        assert_eq!(m["cycle_secs"], 210);
        assert_eq!(m["count"], 1);
        // Empty opensky aircraft → same.
        let empty_os = json!({"aircraft": [], "coverage": "opensky"});
        let m = merge_globe_snapshots(Some(&adsb), Some(&empty_os));
        assert_eq!(m["coverage"], "hotspots+mil+squawk");
        // With opensky rows → +opensky suffix, exactly once.
        let os = json!({"aircraft": [ac_row("cc", Some(3), false)], "coverage": "opensky"});
        let m = merge_globe_snapshots(Some(&adsb), Some(&os));
        assert_eq!(m["coverage"], "hotspots+mil+squawk+opensky");
        let m_again = merge_globe_snapshots(Some(&m), Some(&os));
        assert_eq!(m_again["coverage"], "hotspots+mil+squawk+opensky", "idempotent — never double-append");
    }

    #[test]
    fn merge_opensky_only_and_both_missing() {
        let os = json!({"count": 1, "coverage": "opensky", "aircraft": [ac_row("cc", Some(3), false)]});
        // adsb snapshot dead, opensky alive → opensky rows still served.
        let m = merge_globe_snapshots(None, Some(&os));
        assert_eq!(m["count"], 1);
        assert_eq!(m["coverage"], "opensky");
        assert_eq!(m["aircraft"][0]["hex"], "cc");
        // Both missing → the historical stale envelope shape.
        let m = merge_globe_snapshots(None, None);
        assert_eq!(m["stale"], true);
        assert_eq!(m["aircraft"].as_array().unwrap().len(), 0);
    }
}
