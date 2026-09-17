//! GEV P3 T2: vessels (AIS live) REST endpoints.
//!
//! Two routes under `/api/v1/gev/` (the console adapter's T2 source fetches
//! these directly):
//!
//! - `GET /api/v1/gev/ais-live?maxRows=N` — Redis snapshot passthrough. The
//!   ais.rs collector accumulates the envelope into `hub:globe:vessels`
//!   (SETEX 300s). Response is the envelope verbatim (rows truncated to
//!   maxRows), with the contract's three degraded-but-200 envelopes for the
//!   states the collector cannot represent: `missing-key` (no
//!   AISSTREAM_API_KEY configured — 200, NOT 503: the engine's chip copy
//!   keys off the envelope `status` field, contracts.md §1/§5), `connecting`
//!   (key present, collector has not written a snapshot yet / it expired).
//!   Redis itself unreachable → 502.
//! - `GET /api/v1/gev/ais-live/track?mmsi=<ref>` — per-MMSI last-50-position
//!   ring from the process-wide `ais::tracks()` map. Unknown MMSI → 404,
//!   missing key → 503 (the engine's tracking backfill swallows any failure
//!   and keeps the live-only trail, tracking.js:124-133).
//!
//! Contracts: `.superpowers/sdd/2026-09-17-gev-p3/contracts.md` §1.

use std::collections::VecDeque;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::monitor::sources::ais::{self, TrackPoint};
use crate::state::AppState;

pub const DEFAULT_MAX_ROWS: usize = 12_000;
pub const MAX_ROWS_CAP: usize = 50_000;

/// Redis budget, same 2s degrade-to-None budget the other GEV routes use.
const REDIS_BUDGET_MS: u64 = 2000;

fn ais_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// Parse + clamp the `maxRows` query param (default 12000, hard cap 50000 —
/// contracts.md §1). Unparseable values fall back to the default; 0 clamps
/// to 1 so the passthrough always carries a well-formed slice.
pub fn parse_max_rows(raw: Option<&str>) -> usize {
    raw.and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_ROWS)
        .clamp(1, MAX_ROWS_CAP)
}

/// Contract §1: `newestPositionAt` is ISO 8601 or null, `nextAttemptAt` is
/// epoch MS, the remaining diagnostics are null when there is nothing to
/// report. `refreshing:true` ⇒ the client renders the snapshot stale.
fn empty_envelope(status: &str) -> Value {
    json!({
        "rows": [],
        "newestPositionAt": null,
        "refreshing": true,
        "status": status,
        "lastMessageAt": null,
        "nextAttemptAt": null,
        "silentForMs": null,
        "reconnectAttempt": 0,
    })
}

/// No AISSTREAM_API_KEY anywhere (ais.rs `api_key()` double-name check):
/// 200 + `missing-key`, never 503 — the engine's policy.js chip copy is
/// keyed off this exact status string (contracts.md §5).
pub fn missing_key_envelope() -> Value {
    empty_envelope("missing-key")
}

/// Key configured but no snapshot in Redis yet (collector between ticks, or
/// TTL-expired during a long reconnect): 200 + `connecting`. `connecting`
/// is grace-eligible in the engine's health classification
/// (policy.js:60-95 AIS_HEALTHY_STATUSES), so a fresh boot does not flash a
/// degraded chip.
pub fn connecting_envelope() -> Value {
    empty_envelope("connecting")
}

/// Truncate the stored envelope's `rows` to the client's maxRows. Every
/// other field passes through untouched (the stored envelope is already
/// contract-shaped; ais.rs `vessels_envelope` owns field production).
pub fn truncate_rows(mut envelope: Value, max_rows: usize) -> Value {
    if let Some(rows) = envelope.get_mut("rows").and_then(|r| r.as_array_mut()) {
        rows.truncate(max_rows);
    }
    envelope
}

/// Track body for one MMSI ring — `{samples:[{lat,lon,t}]}` with `t` in
/// Unix SECONDS (contracts.md §1; the client multiplies by 1000).
pub fn track_body(samples: &VecDeque<TrackPoint>) -> Value {
    json!({
        "samples": samples
            .iter()
            .map(|p| json!({ "lat": p.lat, "lon": p.lon, "t": p.t }))
            .collect::<Vec<_>>(),
    })
}

/// Pure lookup over the shared ring map: `None` ⇒ 404 (unknown MMSI — the
/// ring is dropped with the vessel on prune, ais.rs).
pub fn track_lookup(
    tracks: &std::collections::HashMap<String, VecDeque<TrackPoint>>,
    mmsi: &str,
) -> Option<Value> {
    tracks.get(mmsi).filter(|ring| !ring.is_empty()).map(track_body)
}

#[derive(Debug, Deserialize)]
pub struct AisLiveParams {
    #[serde(rename = "maxRows", default)]
    pub max_rows: Option<String>,
}

pub async fn gev_ais_live(
    State(state): State<Arc<AppState>>,
    Query(p): Query<AisLiveParams>,
) -> Result<Json<Value>, Response> {
    // Key check FIRST (before touching Redis): the collector is only
    // registered when a key exists, so a missing key is a config state, not
    // a transport state — answer it without implying anything about Redis.
    if ais::api_key().is_none() {
        return Ok(Json(missing_key_envelope()));
    }
    let max_rows = parse_max_rows(p.max_rows.as_deref());
    // PING before GET: a GET-miss (None) is indistinguishable from a Redis
    // outage without it, and this endpoint must map them differently
    // (connecting 200 vs 502). The layer polls every 60s, so one extra
    // round trip per poll is noise.
    let pong: Option<String> = state
        .redis_timed(redis::cmd("PING").clone(), REDIS_BUDGET_MS)
        .await;
    if pong.is_none() {
        return Err(ais_err(
            StatusCode::BAD_GATEWAY,
            "redis unavailable (vessels snapshot unreachable)",
        ));
    }
    let blob: Option<String> = state
        .redis_timed(redis::cmd("GET").arg(ais::VESSELS_KEY).clone(), REDIS_BUDGET_MS)
        .await;
    match blob {
        Some(blob) => {
            let envelope: Value = serde_json::from_str(&blob).map_err(|e| {
                tracing::warn!(error = %e, "vessels snapshot corrupt");
                ais_err(StatusCode::BAD_GATEWAY, "vessels snapshot corrupt")
            })?;
            Ok(Json(truncate_rows(envelope, max_rows)))
        }
        // Key configured, collector simply hasn't landed a snapshot yet.
        None => Ok(Json(connecting_envelope())),
    }
}

#[derive(Debug, Deserialize)]
pub struct AisTrackParams {
    pub mmsi: String,
}

pub async fn gev_ais_live_track(Query(p): Query<AisTrackParams>) -> Result<Json<Value>, Response> {
    if ais::api_key().is_none() {
        return Err(ais_err(StatusCode::SERVICE_UNAVAILABLE, "ais not configured"));
    }
    let store = ais::tracks();
    let tracks = store.read().unwrap_or_else(|e| e.into_inner());
    match track_lookup(&tracks, p.mmsi.trim()) {
        Some(body) => Ok(Json(body)),
        None => Err(ais_err(StatusCode::NOT_FOUND, "unknown mmsi")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::sources::ais::{self as mmsrc, Transport, VesselRec};
    use std::collections::HashMap;

    // ---------- maxRows ----------

    #[test]
    fn max_rows_defaults_clamps_and_rejects_garbage() {
        assert_eq!(parse_max_rows(None), DEFAULT_MAX_ROWS);
        assert_eq!(parse_max_rows(Some("")), DEFAULT_MAX_ROWS, "empty → default");
        assert_eq!(parse_max_rows(Some("5000")), 5000);
        assert_eq!(parse_max_rows(Some("50001")), MAX_ROWS_CAP, "over cap → cap");
        assert_eq!(parse_max_rows(Some("99999999999999999999")), DEFAULT_MAX_ROWS, "overflow → default");
        assert_eq!(parse_max_rows(Some("0")), 1, "clamp to ≥1");
        assert_eq!(parse_max_rows(Some("-5")), DEFAULT_MAX_ROWS, "negative unparseable → default");
        assert_eq!(parse_max_rows(Some("abc")), DEFAULT_MAX_ROWS);
    }

    // ---------- degraded envelopes (contract §1, field-for-field) ----------

    fn assert_empty_envelope_shape(env: &Value, status: &str) {
        let mut keys: Vec<&str> = env.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "lastMessageAt", "newestPositionAt", "nextAttemptAt",
                "reconnectAttempt", "refreshing", "rows", "silentForMs", "status",
            ]
        );
        assert_eq!(env["status"], status);
        assert_eq!(env["refreshing"], true, "{status} ⇒ client renders stale chip");
        assert_eq!(env["rows"], json!([]));
        assert_eq!(env["newestPositionAt"], Value::Null);
        assert_eq!(env["lastMessageAt"], Value::Null);
        assert_eq!(env["nextAttemptAt"], Value::Null);
        assert_eq!(env["silentForMs"], Value::Null);
        assert_eq!(env["reconnectAttempt"], 0);
    }

    #[test]
    fn missing_key_envelope_matches_contract() {
        assert_empty_envelope_shape(&missing_key_envelope(), "missing-key");
    }

    #[test]
    fn connecting_envelope_matches_contract() {
        assert_empty_envelope_shape(&connecting_envelope(), "connecting");
    }

    // ---------- live passthrough + maxRows truncation ----------

    fn live_envelope(n: usize) -> Value {
        let mut recs: HashMap<String, VesselRec> = HashMap::new();
        let mut tr = HashMap::new();
        for i in 0..n {
            let mmsi = format!("{i:09}");
            mmsrc::upsert_position(
                &mut recs, &mut tr, &mmsi,
                25.0 + i as f64 * 0.001, -80.0,
                Some(10.0), Some(90.0), Some(45.0),
                1_700_000_000,
            );
        }
        mmsrc::vessels_envelope(&recs, Transport::Live, Some(1_700_000_000), None, 0, 1_700_000_000_000)
    }

    #[test]
    fn live_envelope_passthrough_truncates_rows_only() {
        let stored = live_envelope(100);
        let out = truncate_rows(stored.clone(), 12);
        assert_eq!(out["rows"].as_array().unwrap().len(), 12);
        // Every other top-level field passes through untouched.
        for key in [
            "newestPositionAt", "refreshing", "status", "lastMessageAt",
            "nextAttemptAt", "silentForMs", "reconnectAttempt",
        ] {
            assert_eq!(out[key], stored[key], "{key} must pass through");
        }
        assert_eq!(out["refreshing"], false, "live ⇒ not refreshing");
        assert_eq!(out["status"], "live");
        // Row field names stay contract-shaped after truncation.
        let mut rkeys: Vec<&str> = out["rows"][0].as_object().unwrap().keys().map(|s| s.as_str()).collect();
        rkeys.sort_unstable();
        assert_eq!(
            rkeys,
            vec![
                "course", "destination", "heading", "imo", "last_position_epoch",
                "lat", "lon", "mmsi", "name", "speed", "type_specific",
            ]
        );
    }

    #[test]
    fn truncate_rows_noop_below_cap_and_tolerates_missing_rows() {
        let stored = live_envelope(5);
        let out = truncate_rows(stored.clone(), 12_000);
        assert_eq!(out["rows"].as_array().unwrap().len(), 5);
        // A corrupt-ish envelope without rows must not panic.
        let no_rows = json!({"status": "live"});
        assert_eq!(truncate_rows(no_rows, 10), json!({"status": "live"}));
    }

    // ---------- track ----------

    #[test]
    fn track_lookup_unknown_mmsi_is_none() {
        let tracks: HashMap<String, VecDeque<TrackPoint>> = HashMap::new();
        assert!(track_lookup(&tracks, "123456789").is_none());
    }

    #[test]
    fn track_lookup_emits_contract_samples() {
        let mut recs: HashMap<String, VesselRec> = HashMap::new();
        let mut tr: HashMap<String, VecDeque<TrackPoint>> = HashMap::new();
        mmsrc::upsert_position(&mut recs, &mut tr, "987654321", 25.7, -80.1, Some(5.0), None, None, 1_700_000_000);
        mmsrc::upsert_position(&mut recs, &mut tr, "987654321", 25.8, -80.2, Some(5.0), None, None, 1_700_000_030);
        let body = track_lookup(&tr, "987654321").expect("known mmsi");
        let samples = body["samples"].as_array().unwrap();
        assert_eq!(samples.len(), 2);
        for s in samples {
            let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(|k| k.as_str()).collect();
            keys.sort_unstable();
            assert_eq!(keys, vec!["lat", "lon", "t"], "exactly {{lat,lon,t}}");
        }
        assert_eq!(samples[0]["lat"], 25.7);
        assert_eq!(samples[0]["lon"], -80.1);
        assert_eq!(samples[0]["t"], 1_700_000_000, "epoch SECONDS");
        assert_eq!(samples[1]["t"], 1_700_000_030);
        // A trimmed/empty ring is as good as unknown → 404.
        let mut empty: HashMap<String, VecDeque<TrackPoint>> = HashMap::new();
        empty.insert("555".into(), VecDeque::new());
        assert!(track_lookup(&empty, "555").is_none());
    }
}
