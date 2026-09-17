//! AISStream.io live vessel positions (GEV P3 T1, 2026-09-17).
//!
//! Long-lived `wss://stream.aisstream.io/v0/stream` connection held inside
//! the Source tick loop (tokio-tungstenite — the one approved new crate;
//! `rustls-tls-native-roots` keeps the rust:trixie build image OpenSSL-free).
//! In-memory mmsi→record accumulation (adsb.rs shape: apply/merge → prune →
//! full-rewrite envelope), 15s tick, snapshot → Redis `hub:globe:vessels`
//! (SETEX 300s, expiry = death detector). Positions NEVER touch PG (globe
//! spec §2.2 parity). Per-MMSI last-50-position ring lives in a process-wide
//! `Arc<RwLock<..>>` exposed via `tracks()` for the T2 REST layer
//! (`GET /api/ais-live/track?mmsi=`).
//!
//! Env-gated (opensky.rs pattern): without `AISSTREAM_API_KEY` (or
//! `HUB_AISSTREAM_API_KEY`, which wins) the scheduler registry does NOT
//! instantiate this source — one startup log line; the T2 REST layer answers
//! `status:"missing-key"` (vendor chip copy exists, contracts.md §5).
//!
//! Health cell: written by the scheduler exactly like adsb (source_loop →
//! geo::report_health); this collector always returns Ok — the authoritative
//! transport state rides in the envelope's `status` field (transportStatus
//! enum, contracts.md §1), because the collector's own backoff cadence
//! (1s→60s + jitter, auth-failed 1h) must not fight the scheduler's
//! 30s→600s sweep backoff.
//!
//! Protocol notes (aisstream.io official docs, verified 2026-09-17):
//! - Subscription message MUST arrive within 3s of the handshake; frames
//!   from the server are BINARY WebSocket frames holding UTF-8 JSON
//!   (we accept Text too and decode both).
//! - `BoundingBoxes` is a list of corner PAIRS: `[[[lat,lon],[lat,lon]]]`.
//!   The flat `[[-90,-180,90,180]]` form in the task brief is NOT the
//!   documented shape and would be rejected ("invalid subscriptions do not"
//!   return a SubscriptionConfirmation) — we send `[[[-90,-180],[90,180]]]`.
//! - `Sog`/`Cog` are doubles ALREADY in knots/degrees (schema explorer:
//!   "Speed over ground in knots", "Course over ground in degrees"; example
//!   `"Sog": 12.4`) — no ×10 scaling. `TrueHeading` 511 = not available.
//! - ShipStaticData identity fields parsed defensively (`ShipName`|`Name`,
//!   `ImoNumber`|`Imo`, `TypeName`|`Type`) — the docs' schema explorer is
//!   JS-rendered and the protobuf JSON tags vary between aisstream releases;
//!   the emitted CONTRACT rows are unaffected (fixed field names, §1).

use futures::future::BoxFuture;
use futures::stream::StreamExt;
use futures::FutureExt;
use futures::SinkExt;
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::MaybeTlsStream;

use crate::error::Result;

use super::super::{Ctx, Signal};

pub const VESSELS_KEY: &str = "hub:globe:vessels";

const WS_URL: &str = "wss://stream.aisstream.io/v0/stream";
/// Snapshot TTL: the globe only loses vessels if the collector itself is
/// dead for >5min (adsb.rs parity), not because one tick hiccuped.
const SNAPSHOT_TTL_SECS: usize = 300;
/// Source tick: drain the socket, prune, full-rewrite the envelope.
const TICK_SECS: u64 = 15;
/// Drop vessels whose freshest position is older than this (adsb STALE_SECS).
const STALE_SECS: i64 = 600;
/// >120s without ANY frame → envelope status `stale` (socket kept serving
/// the cached rows — the client renders a degraded chip, contracts.md §1).
const SILENT_STALE_SECS: i64 = 120;
/// >300s without ANY frame → the socket is dead: close, reconnect with
/// exponential backoff. The 120..300s window is the visible `stale` band.
const SILENT_KILL_SECS: i64 = 300;
/// Per-MMSI track ring depth (T2 `/api/ais-live/track` consumer).
const TRACK_CAP: usize = 50;
/// Exponential reconnect backoff: 1s → 2s → … → 60s cap (+ jitter).
const BACKOFF_MIN_SECS: u64 = 1;
const BACKOFF_MAX_SECS: u64 = 60;
/// auth-failed NEVER fast-retries: one re-attempt per hour (a rejected key
/// is a config problem, not a transient outage).
const AUTH_RETRY_SECS: i64 = 3600;
/// Handshake ceiling.
const DIAL_TIMEOUT_SECS: u64 = 10;
/// One tick never spends more than this draining the stream, so the 15s
/// cadence holds even under a message flood.
const DRAIN_BUDGET_MS: u64 = 3000;
/// An idle gap this long ends a drain burst (stream is event-driven).
const DRAIN_IDLE_MS: u64 = 200;
/// Safety valve per tick.
const DRAIN_MAX_MSGS: usize = 5000;

type WsStream = tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type TrackMap = HashMap<String, VecDeque<TrackPoint>>;

static TRACKS: OnceLock<Arc<RwLock<TrackMap>>> = OnceLock::new();

/// Per-MMSI last-50-position ring, shared with the REST layer (T2 consumes
/// via `tracks()`). Process-wide (not per-source-instance) so a future
/// re-instantiated source or the REST handler sees the same map.
pub fn tracks() -> Arc<RwLock<TrackMap>> {
    TRACKS
        .get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
        .clone()
}

/// AISSTREAM_API_KEY (HUB_-prefixed wins — opensky.rs pattern). Empty values
/// are treated as unset. Read at every tick so a key rotation in secrets.env
/// (after a restart) is the only flow — no reload magic needed.
pub fn api_key() -> Option<String> {
    std::env::var("HUB_AISSTREAM_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("AISSTREAM_API_KEY").ok().filter(|s| !s.is_empty()))
}

/// transportStatus enum (contracts.md §1). `missing-key` is intentionally
/// absent: without a key the collector is never registered and the T2 REST
/// layer synthesizes that status itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transport {
    Live,
    Connecting,
    Reconnecting,
    Stale,
    Down,
    AuthFailed,
}

impl Transport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Transport::Live => "live",
            Transport::Connecting => "connecting",
            Transport::Reconnecting => "reconnecting",
            Transport::Stale => "stale",
            Transport::Down => "down",
            Transport::AuthFailed => "auth-failed",
        }
    }
    /// Contract: `refreshing:true` ⇒ the client renders the snapshot stale
    /// (vesselSnapshot: `stale: Boolean(payload?.refreshing)`). Only `live`
    /// is a non-refreshing transport.
    pub fn refreshing(&self) -> bool {
        !matches!(self, Transport::Live)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackPoint {
    pub lat: f64,
    pub lon: f64,
    /// Unix seconds.
    pub t: i64,
}

/// Accumulated per-MMSI vessel state. `has_pos` guards against 0.0/0.0
/// (Gulf of Guinea) leaking in from a static-data-only vessel.
#[derive(Clone, Debug, Default)]
pub struct VesselRec {
    pub has_pos: bool,
    pub lat: f64,
    pub lon: f64,
    pub name: Option<String>,
    pub imo: Option<String>,
    pub ship_type: Option<String>,
    pub destination: Option<String>,
    /// Knots — upstream units, client converts (contracts.md §1).
    pub speed: Option<f64>,
    /// Degrees.
    pub course: Option<f64>,
    /// Degrees. Distinct observation from course.
    pub heading: Option<f64>,
    /// Unix seconds of the freshest PositionReport (receiver time — the AIS
    /// slot `Timestamp` is seconds-within-the-minute, NOT epoch).
    pub epoch: Option<i64>,
}

/// One decoded stream frame.
#[derive(Debug, PartialEq)]
pub enum ParsedMsg {
    Position {
        mmsi: String,
        lat: f64,
        lon: f64,
        speed: Option<f64>,
        course: Option<f64>,
        heading: Option<f64>,
    },
    Static {
        mmsi: String,
        name: Option<String>,
        imo: Option<String>,
        ship_type: Option<String>,
        destination: Option<String>,
    },
    AuthRejected,
    Other,
}

/// AUTH_TEXT semantics (contracts.md §1 degraded enum): upstream rejected
/// the subscription key. aisstream documents an invalid key as a CONNECTION
/// CLOSE cause; client implementations in the wild additionally report a
/// text/JSON error frame, so we match the common rejection vocabulary
/// defensively — case-insensitive substring over the frame text and over a
/// top-level `{"error": "..."}` string value.
const AUTH_TEXT: &[&str] = &[
    "unauthorized",
    "not authorized",
    "authentication failed",
    "auth failed",
    "invalid api key",
    "invalid key",
    "access denied",
    "forbidden",
];

pub fn is_auth_text(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    AUTH_TEXT.iter().any(|p| t.contains(p))
}

fn f64_of(v: &serde_json::Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64())
}

fn i64_of(v: &serde_json::Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

fn str_of(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(k).and_then(|x| x.as_str()))
        .map(|s| s.trim().trim_matches('@').trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Pure frame parser (aisstream JSON → ParsedMsg). Never panics; malformed
/// frames degrade to `Other`.
pub fn parse_ws_text(text: &str) -> ParsedMsg {
    if is_auth_text(text) {
        return ParsedMsg::AuthRejected;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return ParsedMsg::Other;
    };
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        return if is_auth_text(err) { ParsedMsg::AuthRejected } else { ParsedMsg::Other };
    }
    let msg = &v["Message"];
    let meta = &v["MetaData"];
    match v.get("MessageType").and_then(|m| m.as_str()) {
        Some("PositionReport") => {
            let pr = &msg["PositionReport"];
            let Some(mmsi) = i64_of(pr, "UserID")
                .or_else(|| i64_of(meta, "MMSI"))
                .map(|m| m.to_string())
            else {
                return ParsedMsg::Other;
            };
            let (Some(lat), Some(lon)) = (
                f64_of(pr, "Latitude").or_else(|| f64_of(meta, "Latitude")),
                f64_of(pr, "Longitude").or_else(|| f64_of(meta, "Longitude")),
            ) else {
                return ParsedMsg::Other;
            };
            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                return ParsedMsg::Other;
            }
            // Sog is double knots / Cog is double degrees per the schema
            // explorer — pass through, only range-guarding absurd values.
            let speed = f64_of(pr, "Sog").filter(|s| (0.0..=110.0).contains(s));
            let course = f64_of(pr, "Cog").filter(|c| (0.0..=360.0).contains(c));
            // AIS 511 = heading not available.
            let heading = i64_of(pr, "TrueHeading")
                .filter(|h| (0..=359).contains(h))
                .map(|h| h as f64);
            ParsedMsg::Position { mmsi, lat, lon, speed, course, heading }
        }
        Some("ShipStaticData") => {
            let sd = &msg["ShipStaticData"];
            let Some(mmsi) = i64_of(sd, "UserID")
                .or_else(|| i64_of(meta, "MMSI"))
                .map(|m| m.to_string())
            else {
                return ParsedMsg::Other;
            };
            let name = str_of(sd, &["ShipName", "Name"]);
            let imo = i64_of(sd, "ImoNumber")
                .or_else(|| i64_of(sd, "Imo"))
                .filter(|n| *n > 0)
                .map(|n| n.to_string());
            let ship_type = str_of(sd, &["TypeName"]).or_else(|| {
                i64_of(sd, "Type").map(|t| t.to_string())
            });
            let destination = str_of(sd, &["Destination"]);
            ParsedMsg::Static { mmsi, name, imo, ship_type, destination }
        }
        _ => ParsedMsg::Other,
    }
}

/// Position merge — fresher-wins on epoch (out-of-order duplicate frames
/// cannot move a vessel backwards) + track ring append (cap 50, front-drop).
pub fn upsert_position(
    recs: &mut HashMap<String, VesselRec>,
    tracks: &mut TrackMap,
    mmsi: &str,
    lat: f64,
    lon: f64,
    speed: Option<f64>,
    course: Option<f64>,
    heading: Option<f64>,
    epoch: i64,
) {
    let rec = recs.entry(mmsi.to_string()).or_default();
    if let Some(cur) = rec.epoch {
        if epoch < cur {
            return; // stale duplicate — keep the fresher sighting
        }
    }
    rec.has_pos = true;
    rec.lat = lat;
    rec.lon = lon;
    if let Some(s) = speed {
        rec.speed = Some(s);
    }
    if let Some(c) = course {
        rec.course = Some(c);
    }
    if let Some(h) = heading {
        rec.heading = Some(h);
    }
    rec.epoch = Some(epoch);
    let ring = tracks.entry(mmsi.to_string()).or_default();
    ring.push_back(TrackPoint { lat, lon, t: epoch });
    while ring.len() > TRACK_CAP {
        ring.pop_front();
    }
}

/// Static identity merge — supplements the SAME mmsi record; never invents
/// a position.
pub fn upsert_static(
    recs: &mut HashMap<String, VesselRec>,
    mmsi: &str,
    name: Option<String>,
    imo: Option<String>,
    ship_type: Option<String>,
    destination: Option<String>,
) {
    let rec = recs.entry(mmsi.to_string()).or_default();
    if let Some(n) = name {
        rec.name = Some(n);
    }
    if let Some(i) = imo {
        rec.imo = Some(i);
    }
    if let Some(t) = ship_type {
        rec.ship_type = Some(t);
    }
    if let Some(d) = destination {
        rec.destination = Some(d);
    }
}

/// STALE_SECS prune (adsb parity): a vessel unseen for >600s leaves both the
/// snapshot and the track ring.
pub fn prune(recs: &mut HashMap<String, VesselRec>, tracks: &mut TrackMap, now: i64) {
    recs.retain(|_, r| matches!(r.epoch, Some(e) if now - e <= STALE_SECS));
    tracks.retain(|m, _| recs.contains_key(m));
}

/// Exponential reconnect backoff (pure): attempt 1→1s, 2→2s, 3→4s … ≥7→60s.
pub fn backoff_secs(attempt: u32) -> u64 {
    let shift = attempt.saturating_sub(1).min(6);
    (BACKOFF_MIN_SECS << shift).min(BACKOFF_MAX_SECS)
}

/// Deterministic jitter 0..=base/4 seconds from sub-second nanos (no rand
/// crate — zero-new-crate constraint; caller passes timestamp nanos).
pub fn jitter_ms(nanos: u64, base_secs: u64) -> u64 {
    nanos % (base_secs.saturating_mul(250).saturating_add(1))
}

/// Envelope status for the connected case (pure). `silent_secs` = now minus
/// last frame epoch; `None` = socket fresh, no frames yet (grace).
pub fn status_connected(silent_secs: Option<i64>, auth_failed: bool) -> Transport {
    if auth_failed {
        Transport::AuthFailed
    } else if silent_secs.map_or(false, |s| s > SILENT_STALE_SECS) {
        Transport::Stale
    } else {
        Transport::Live
    }
}

/// Envelope status for the disconnected case (pure).
pub fn status_disconnected(ever_connected: bool) -> Transport {
    if ever_connected {
        Transport::Reconnecting
    } else {
        Transport::Connecting
    }
}

/// Full-rewrite envelope (contracts.md §1, field-for-field). Rows are
/// mmsi-sorted for deterministic output (adsb hex-sort parity).
///
/// Unit/encoding decisions pinned by the vendor consumer:
/// - `newestPositionAt` / `lastMessageAt` → ISO 8601 strings or null
///   (`vesselSnapshot` does `Date.parse` on them; a bare epoch-seconds
///   NUMBER would be misread as ms by `epoch(v)` scale=1).
/// - `nextAttemptAt` → epoch MS number (the layer treats it as a deadline).
/// - `speed` stays knots, `course`/`heading` degrees (client converts).
pub fn vessels_envelope(
    recs: &HashMap<String, VesselRec>,
    status: Transport,
    last_msg_epoch: Option<i64>,
    next_attempt_epoch: Option<i64>,
    reconnect_attempt: u32,
    now_ms: i64,
) -> serde_json::Value {
    let mut rows: Vec<(&String, &VesselRec)> = recs.iter().filter(|(_, r)| r.has_pos).collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    let iso = |e: i64| {
        chrono::DateTime::from_timestamp(e, 0)
            .map(|d| d.to_rfc3339_opts(chrono::format::SecondsFormat::Secs, true))
    };
    let newest = rows.iter().filter_map(|(_, r)| r.epoch).max();
    let silent_ms = last_msg_epoch.map(|t| (now_ms - t * 1000).max(0));
    json!({
        "rows": rows.iter().map(|(mmsi, r)| json!({
            "mmsi": mmsi,
            "lat": r.lat,
            "lon": r.lon,
            "name": r.name,
            "imo": r.imo,
            "type_specific": r.ship_type,
            "destination": r.destination,
            "speed": r.speed,
            "course": r.course,
            "heading": r.heading,
            "last_position_epoch": r.epoch,
        })).collect::<Vec<_>>(),
        "newestPositionAt": newest.and_then(iso),
        "refreshing": status.refreshing(),
        "status": status.as_str(),
        "lastMessageAt": last_msg_epoch.and_then(iso),
        "nextAttemptAt": next_attempt_epoch.map(|e| e * 1000),
        "silentForMs": silent_ms,
        "reconnectAttempt": reconnect_attempt,
    })
}

/// Dial outcome classification.
enum DialError {
    /// Handshake rejected the credentials (HTTP 401/403 or auth frame).
    Auth,
    /// Rate limited at the handshake; seconds until retry (Retry-After).
    RateLimited(i64),
    /// Anything else (TLS, DNS, timeout, HTTP 5xx).
    Transport(String),
}

/// One dial: TLS handshake (10s ceiling) + subscribe frame within the
/// documented 3s post-handshake window.
async fn dial(key: &str) -> std::result::Result<WsStream, DialError> {
    let connect = tokio_tungstenite::connect_async(WS_URL);
    let (mut sock, _resp) = match tokio::time::timeout(Duration::from_secs(DIAL_TIMEOUT_SECS), connect).await {
        Ok(Ok(x)) => x,
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(r))) => {
            let status = r.status().as_u16();
            let retry_after = r
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<i64>().ok());
            tracing::warn!(status, "aisstream handshake rejected");
            return Err(match status {
                401 | 403 => DialError::Auth,
                429 => DialError::RateLimited(retry_after.unwrap_or(BACKOFF_MAX_SECS as i64)),
                _ => DialError::Transport(format!("handshake HTTP {status}")),
            });
        }
        Ok(Err(e)) => return Err(DialError::Transport(e.to_string())),
        Err(_) => return Err(DialError::Transport("handshake timeout".into())),
    };
    // Documented subscription shape: corner PAIRS [[[lat,lon],[lat,lon]]] —
    // see module docstring for why the flat 4-tuple form is not sent.
    let sub = json!({
        "APIKey": key,
        "BoundingBoxes": [[[-90.0, -180.0], [90.0, 180.0]]],
        "FilterMessageTypes": ["PositionReport", "ShipStaticData"],
    });
    sock.send(Message::Text(sub.to_string().into()))
        .await
        .map_err(|e| DialError::Transport(format!("subscribe send failed: {e}")))?;
    Ok(sock)
}

/// Per-source state. Lives behind one Mutex; the tick TAKES the whole
/// struct, works on it, and puts it back — a std MutexGuard is never held
/// across .await (the Source future must stay Send).
#[derive(Default)]
struct Inner {
    sock: Option<WsStream>,
    recs: HashMap<String, VesselRec>,
    ever_connected: bool,
    auth_failed: bool,
    last_msg_epoch: Option<i64>,
    reconnect_attempt: u32,
    next_attempt_epoch: Option<i64>,
}

pub struct Ais {
    inner: Mutex<Option<Inner>>,
}

impl Default for Ais {
    fn default() -> Self {
        Self { inner: Mutex::new(None) }
    }
}

impl super::super::Source for Ais {
    fn name(&self) -> &'static str {
        "ais"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(TICK_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let Some(key) = api_key() else {
                // registry() gates this; defensive only.
                tracing::info!("ais: no AISSTREAM_API_KEY — tick skipped (collector should not be registered)");
                return Ok(vec![]);
            };
            let now = chrono::Utc::now().timestamp();
            let now_ms = chrono::Utc::now().timestamp_millis();
            let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;

            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .unwrap_or_default();

            // ---- Phase 1: drain the live socket (event-driven burst) ----
            // Parsed frames are buffered locally and applied AFTER the read
            // loop: the tracks map is a std RwLock whose guard is !Send and
            // must never be held across an await.
            if inner.sock.is_some() {
                let mut sock = inner.sock.take().unwrap();
                let mut drop_socket = false;
                let mut msgs = 0usize;
                let mut parsed: Vec<ParsedMsg> = Vec::new();
                let budget = tokio::time::Instant::now() + Duration::from_millis(DRAIN_BUDGET_MS);
                while msgs < DRAIN_MAX_MSGS && tokio::time::Instant::now() < budget {
                    match tokio::time::timeout(Duration::from_millis(DRAIN_IDLE_MS), sock.next()).await {
                        // Idle gap → the burst is over; snapshot time.
                        Err(_) => break,
                        Ok(None) => {
                            drop_socket = true;
                            break;
                        }
                        Ok(Some(Err(e))) => {
                            tracing::warn!(error = %e, "ais stream error");
                            drop_socket = true;
                            break;
                        }
                        Ok(Some(Ok(msg))) => {
                            msgs += 1;
                            let text = match msg {
                                Message::Text(t) => Some(t.to_string()),
                                Message::Binary(b) => Some(String::from_utf8_lossy(&b).into_owned()),
                                Message::Close(_) => {
                                    drop_socket = true;
                                    None
                                }
                                // tokio-tungstenite auto-queues pong on
                                // read; the flush below actually sends it.
                                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => None,
                            };
                            if let Some(text) = text {
                                match parse_ws_text(&text) {
                                    p @ ParsedMsg::Position { .. } | p @ ParsedMsg::Static { .. } => {
                                        parsed.push(p);
                                    }
                                    ParsedMsg::AuthRejected => {
                                        tracing::warn!("aisstream rejected the API key — status auth-failed; retry in 1h");
                                        inner.auth_failed = true;
                                        inner.next_attempt_epoch = Some(now + AUTH_RETRY_SECS);
                                        drop_socket = true;
                                        break;
                                    }
                                    ParsedMsg::Other => {}
                                }
                            }
                            // Send any queued pong without blocking.
                            let _ = sock.flush().await;
                        }
                    }
                }
                // Apply buffered frames — the tracks lock is held only for
                // this synchronous pass (guard is !Send, never across await).
                if !parsed.is_empty() {
                    inner.last_msg_epoch = Some(now);
                    let store = tracks();
                    let mut tracks = store.write().unwrap_or_else(|p| p.into_inner());
                    for p in parsed {
                        match p {
                            ParsedMsg::Position { mmsi, lat, lon, speed, course, heading } => {
                                upsert_position(
                                    &mut inner.recs, &mut tracks,
                                    &mmsi, lat, lon, speed, course, heading, now,
                                );
                            }
                            ParsedMsg::Static { mmsi, name, imo, ship_type, destination } => {
                                upsert_static(&mut inner.recs, &mmsi, name, imo, ship_type, destination);
                            }
                            ParsedMsg::AuthRejected | ParsedMsg::Other => {}
                        }
                    }
                }
                if drop_socket {
                    drop(sock);
                } else {
                    inner.sock = Some(sock);
                }
                if msgs > 0 {
                    tracing::debug!(msgs, "ais stream burst drained");
                }
            }

            // ---- Phase 2: kill a long-silent socket (dead upstream) ----
            if let Some(sock) = inner.sock.take() {
                let silent = inner.last_msg_epoch.map(|t| now - t);
                if silent.map_or(false, |s| s > SILENT_KILL_SECS) {
                    tracing::warn!(silent_secs = silent, "ais stream silent > 300s; dropping socket");
                    drop(sock);
                    inner.reconnect_attempt += 1;
                    let base = backoff_secs(inner.reconnect_attempt);
                    let wait = base + jitter_ms(nanos, base) / 1000;
                    inner.next_attempt_epoch = Some(now + wait as i64);
                } else {
                    inner.sock = Some(sock);
                }
            }

            // ---- Phase 3: (re)connect when due ---------------------------
            if inner.sock.is_none() && !inner.auth_failed {
                let due = inner.next_attempt_epoch.map_or(true, |t| now >= t);
                if due {
                    match dial(&key).await {
                        Ok(sock) => {
                            tracing::info!(attempt = inner.reconnect_attempt, "ais stream connected");
                            inner.sock = Some(sock);
                            inner.ever_connected = true;
                            inner.reconnect_attempt = 0;
                            inner.next_attempt_epoch = None;
                        }
                        Err(DialError::Auth) => {
                            tracing::warn!("aisstream handshake rejected the API key — status auth-failed; retry in 1h");
                            inner.auth_failed = true;
                            inner.next_attempt_epoch = Some(now + AUTH_RETRY_SECS);
                        }
                        Err(DialError::RateLimited(wait)) => {
                            tracing::warn!(wait_secs = wait, "aisstream handshake rate-limited (429)");
                            inner.reconnect_attempt += 1;
                            inner.next_attempt_epoch = Some(now + wait.max(1));
                        }
                        Err(DialError::Transport(e)) => {
                            inner.reconnect_attempt += 1;
                            let base = backoff_secs(inner.reconnect_attempt);
                            let wait = base + jitter_ms(nanos, base) / 1000;
                            tracing::warn!(attempt = inner.reconnect_attempt, wait_secs = wait, error = %e, "aisstream dial failed");
                            inner.next_attempt_epoch = Some(now + wait as i64);
                        }
                    }
                }
            }

            // ---- Phase 4: prune + full-rewrite envelope ------------------
            {
                let store = tracks();
                let mut tracks = store.write().unwrap_or_else(|p| p.into_inner());
                prune(&mut inner.recs, &mut tracks, now);
            }
            let status = if let Some(sock_status) = inner.sock.as_ref().map(|_| {
                status_connected(inner.last_msg_epoch.map(|t| now - t), inner.auth_failed)
            }) {
                sock_status
            } else if inner.auth_failed {
                Transport::AuthFailed
            } else {
                status_disconnected(inner.ever_connected)
            };
            let envelope = vessels_envelope(
                &inner.recs,
                status,
                inner.last_msg_epoch,
                inner.next_attempt_epoch,
                inner.reconnect_attempt,
                now_ms,
            );
            // Redis write failure is visible but not fatal (adsb parity):
            // the 300s TTL expiry is the death detector.
            let wrote: Option<String> = ctx
                .state
                .redis_timed(
                    redis::cmd("SETEX").arg(VESSELS_KEY).arg(SNAPSHOT_TTL_SECS).arg(envelope.to_string()).clone(),
                    2000,
                )
                .await;
            if wrote.is_none() {
                tracing::warn!("ais snapshot write failed (redis down?)");
            }

            // Put the state back (incl. the live socket).
            *self.inner.lock().unwrap_or_else(|p| p.into_inner()) = Some(inner);
            Ok(vec![])
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos_report(mmsi: i64, sog: f64, cog: f64, hdg: i64, lat: f64, lon: f64) -> String {
        json!({
            "MessageType": "PositionReport",
            "Message": {"PositionReport": {
                "MessageID": 1, "UserID": mmsi, "Valid": true,
                "Sog": sog, "Cog": cog, "TrueHeading": hdg,
                "Latitude": lat, "Longitude": lon, "Timestamp": 42,
            }},
            "MetaData": {"MMSI": mmsi},
        })
        .to_string()
    }

    fn static_data(mmsi: i64, name: &str, imo: i64, type_name: &str, dest: &str) -> String {
        json!({
            "MessageType": "ShipStaticData",
            "Message": {"ShipStaticData": {
                "MessageID": 5, "UserID": mmsi, "Valid": true,
                "ShipName": name, "ImoNumber": imo,
                "TypeName": type_name, "Type": 70, "Destination": dest,
            }},
        })
        .to_string()
    }

    // ---------- frame parsing ----------

    #[test]
    fn parse_position_report_knots_and_degrees_unscaled() {
        // Official docs example values: Sog 12.4 / Cog 86.7 are ALREADY
        // knots/degrees (schema: number<double>) — no ×10 anywhere.
        let p = parse_ws_text(&pos_report(368207620, 12.4, 86.7, 87, 25.7617, -80.1918));
        match p {
            ParsedMsg::Position { mmsi, lat, lon, speed, course, heading } => {
                assert_eq!(mmsi, "368207620");
                assert!((lat - 25.7617).abs() < 1e-9);
                assert!((lon - (-80.1918)).abs() < 1e-9);
                assert!((speed.unwrap() - 12.4).abs() < 1e-9, "knots passthrough");
                assert!((course.unwrap() - 86.7).abs() < 1e-9, "degrees passthrough");
                assert!((heading.unwrap() - 87.0).abs() < 1e-9);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn parse_position_report_heading_511_is_unavailable() {
        let p = parse_ws_text(&pos_report(123456789, 0.0, 360.0, 511, 1.0, 2.0));
        match p {
            ParsedMsg::Position { speed, course, heading, .. } => {
                assert_eq!(heading, None, "AIS 511 = heading not available");
                assert_eq!(speed, Some(0.0));
                assert_eq!(course, Some(360.0));
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn parse_position_report_drops_bad_values_not_the_position() {
        // Sog 999 / Cog 400 are not real AIS values → the field degrades to
        // None (contract: speed/course nullable) but the POSITION is kept —
        // a bogus scalar must not lose a real fix.
        match parse_ws_text(&pos_report(1, 999.0, 10.0, 10, 1.0, 2.0)) {
            ParsedMsg::Position { speed, course, heading, .. } => {
                assert_eq!(speed, None);
                assert_eq!(course, Some(10.0));
                assert_eq!(heading, Some(10.0));
            }
            other => panic!("unexpected parse: {other:?}"),
        }
        match parse_ws_text(&pos_report(1, 10.0, 400.0, 10, 1.0, 2.0)) {
            ParsedMsg::Position { speed, course, .. } => {
                assert_eq!(speed, Some(10.0));
                assert_eq!(course, None);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
        // A position outside the planet is dropped wholesale.
        assert_eq!(parse_ws_text(&pos_report(1, 10.0, 10.0, 10, 91.0, 2.0)), ParsedMsg::Other);
    }

    #[test]
    fn parse_position_report_falls_back_to_metadata_position() {
        let mut v: serde_json::Value = serde_json::from_str(&pos_report(211123456, 5.0, 10.0, 20, 53.5, 10.0)).unwrap();
        v["Message"]["PositionReport"]["Latitude"] = serde_json::Value::Null;
        v["Message"]["PositionReport"]["Longitude"] = serde_json::Value::Null;
        v["MetaData"]["Latitude"] = json!(53.5);
        v["MetaData"]["Longitude"] = json!(10.0);
        match parse_ws_text(&v.to_string()) {
            ParsedMsg::Position { mmsi, lat, lon, .. } => {
                assert_eq!(mmsi, "211123456");
                assert_eq!((lat, lon), (53.5, 10.0));
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn parse_static_data_trims_name_and_maps_fields() {
        let p = parse_ws_text(&static_data(636092423, "MAERSK ETA @", 9501234, "CargoShips", "SINGAPORE"));
        match p {
            ParsedMsg::Static { mmsi, name, imo, ship_type, destination } => {
                assert_eq!(mmsi, "636092423");
                assert_eq!(name.as_deref(), Some("MAERSK ETA"));
                assert_eq!(imo.as_deref(), Some("9501234"));
                assert_eq!(ship_type.as_deref(), Some("CargoShips"));
                assert_eq!(destination.as_deref(), Some("SINGAPORE"));
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn parse_static_data_imo_zero_is_unavailable() {
        let p = parse_ws_text(&static_data(1, "  ", 0, "", ""));
        match p {
            ParsedMsg::Static { name, imo, ship_type, destination, .. } => {
                assert_eq!(name, None, "blank/whitespace name → None");
                assert_eq!(imo, None, "IMO 0 = not available");
                assert_eq!(ship_type, Some("70".to_string()), "empty TypeName → numeric Type fallback");
                assert_eq!(destination, None);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn parse_subscription_confirmation_and_unknown_are_other() {
        let conf = json!({"MessageType": "SubscriptionConfirmation", "Message": {"CompressionEnabled": true}}).to_string();
        assert_eq!(parse_ws_text(&conf), ParsedMsg::Other);
        assert_eq!(parse_ws_text("not json at all"), ParsedMsg::Other);
        assert_eq!(parse_ws_text(""), ParsedMsg::Other);
    }

    // ---------- auth / rate-limit text classification ----------

    #[test]
    fn auth_text_classification() {
        assert!(is_auth_text("Unauthorized"));
        assert!(is_auth_text("HTTP 401: unauthorized access"));
        assert!(is_auth_text("{\"error\": \"invalid api key\"}"));
        assert!(is_auth_text("Authentication Failed"));
        assert!(is_auth_text("FORBIDDEN"));
        assert!(!is_auth_text("PositionReport"));
        assert!(!is_auth_text("{\"MessageType\": \"PositionReport\"}"));
        // error object with non-auth message → Other, not AuthRejected
        assert_eq!(parse_ws_text("{\"error\": \"slow down\"}"), ParsedMsg::Other);
        assert_eq!(parse_ws_text("{\"error\": \"Unauthorized\"}"), ParsedMsg::AuthRejected);
    }

    // ---------- merge semantics ----------

    #[test]
    fn position_merge_fresher_wins() {
        let mut recs = HashMap::new();
        let mut tr = TrackMap::new();
        upsert_position(&mut recs, &mut tr, "111", 10.0, 20.0, Some(5.0), None, None, 1000);
        // Older duplicate must NOT move the vessel backwards.
        upsert_position(&mut recs, &mut tr, "111", 99.0, 99.0, Some(50.0), None, None, 900);
        let r = &recs["111"];
        assert_eq!((r.lat, r.lon), (10.0, 20.0));
        assert_eq!(r.speed, Some(5.0));
        assert_eq!(r.epoch, Some(1000));
        // Fresher report replaces position AND fills new fields.
        upsert_position(&mut recs, &mut tr, "111", 11.0, 21.0, None, Some(45.0), Some(44.0), 1100);
        let r = &recs["111"];
        assert_eq!((r.lat, r.lon), (11.0, 21.0));
        assert_eq!(r.speed, Some(5.0), "absent fields keep the previous value");
        assert_eq!(r.course, Some(45.0));
        assert_eq!(r.heading, Some(44.0));
        assert_eq!(tr["111"].len(), 2, "stale duplicate appended no track point");
    }

    #[test]
    fn static_merge_supplements_without_inventing_position() {
        let mut recs = HashMap::new();
        let mut tr = TrackMap::new();
        upsert_position(&mut recs, &mut tr, "222", 1.0, 2.0, None, None, None, 500);
        upsert_static(&mut recs, "222", Some("EVER GIVEN".into()), Some("9811000".into()), Some("CargoShips".into()), Some("ROTTERDAM".into()));
        let r = &recs["222"];
        assert!(r.has_pos);
        assert_eq!(r.name.as_deref(), Some("EVER GIVEN"));
        assert_eq!(r.imo.as_deref(), Some("9811000"));
        assert_eq!(r.ship_type.as_deref(), Some("CargoShips"));
        assert_eq!(r.destination.as_deref(), Some("ROTTERDAM"));
        // Static-only vessel: identity kept, but has_pos stays false so the
        // row never renders at 0.0,0.0 (Gulf of Guinea guard).
        upsert_static(&mut recs, "333", Some("GHOST".into()), None, None, None);
        assert!(!recs["333"].has_pos);
        assert_eq!(recs["333"].name.as_deref(), Some("GHOST"));
    }

    // ---------- prune ----------

    #[test]
    fn prune_drops_older_than_600s_and_cleans_tracks() {
        let mut recs = HashMap::new();
        let mut tr = TrackMap::new();
        upsert_position(&mut recs, &mut tr, "old", 0.0, 0.0, None, None, None, 900);
        upsert_position(&mut recs, &mut tr, "edge", 0.0, 0.0, None, None, None, 1000);
        upsert_position(&mut recs, &mut tr, "fresh", 0.0, 0.0, None, None, None, 1500);
        prune(&mut recs, &mut tr, 1600);
        assert!(!recs.contains_key("old"), "700s old → pruned");
        assert!(recs.contains_key("edge"), "exactly 600s → kept");
        assert!(recs.contains_key("fresh"));
        assert!(!tr.contains_key("old"), "pruned vessel's ring cleaned too");
        assert!(tr.contains_key("edge"));
    }

    // ---------- track ring ----------

    #[test]
    fn track_ring_caps_at_50_front_drop() {
        let mut recs = HashMap::new();
        let mut tr = TrackMap::new();
        for t in 0..60 {
            upsert_position(&mut recs, &mut tr, "444", t as f64, 0.0, None, None, None, t);
        }
        let ring = &tr["444"];
        assert_eq!(ring.len(), TRACK_CAP);
        assert_eq!(ring.front().unwrap().t, 10, "oldest 10 dropped");
        assert_eq!(ring.back().unwrap().t, 59);
    }

    // ---------- backoff / jitter ----------

    #[test]
    fn backoff_grows_exponentially_and_caps_at_60() {
        assert_eq!(backoff_secs(1), 1);
        assert_eq!(backoff_secs(2), 2);
        assert_eq!(backoff_secs(3), 4);
        assert_eq!(backoff_secs(4), 8);
        assert_eq!(backoff_secs(5), 16);
        assert_eq!(backoff_secs(6), 32);
        assert_eq!(backoff_secs(7), 60, "cap");
        assert_eq!(backoff_secs(100), 60, "cap holds at high attempts");
    }

    #[test]
    fn jitter_is_bounded_and_deterministic() {
        for nanos in [0u64, 1, 999, 1_000_000_007, u64::MAX] {
            let j = jitter_ms(nanos, 60);
            assert!(j <= 60 * 250, "jitter ≤ base/4 seconds");
        }
        assert_eq!(jitter_ms(42, 1), 42 % 251);
        assert_eq!(jitter_ms(7, 0), 0);
    }

    // ---------- status state machine ----------

    #[test]
    fn status_connected_transitions() {
        assert_eq!(status_connected(Some(10), false), Transport::Live);
        assert_eq!(status_connected(Some(121), false), Transport::Stale, ">120s silent → stale");
        assert_eq!(status_connected(Some(120), false), Transport::Live, "boundary inclusive");
        assert_eq!(status_connected(None, false), Transport::Live, "fresh socket grace");
        assert_eq!(status_connected(Some(5), true), Transport::AuthFailed, "auth dominates");
    }

    #[test]
    fn status_disconnected_transitions() {
        assert_eq!(status_disconnected(false), Transport::Connecting, "never connected");
        assert_eq!(status_disconnected(true), Transport::Reconnecting);
    }

    #[test]
    fn transport_strings_match_contract_enum() {
        assert_eq!(Transport::Live.as_str(), "live");
        assert_eq!(Transport::Connecting.as_str(), "connecting");
        assert_eq!(Transport::Reconnecting.as_str(), "reconnecting");
        assert_eq!(Transport::Stale.as_str(), "stale");
        assert_eq!(Transport::Down.as_str(), "down");
        assert_eq!(Transport::AuthFailed.as_str(), "auth-failed");
        assert!(Transport::Live.refreshing() == false);
        for t in [Transport::Connecting, Transport::Reconnecting, Transport::Stale, Transport::Down, Transport::AuthFailed] {
            assert!(t.refreshing(), "{t:?} must set refreshing=true (client stale flag)");
        }
    }

    // ---------- envelope shape (contract §1, field-for-field) ----------

    #[test]
    fn envelope_fields_match_contract_exactly() {
        let mut recs = HashMap::new();
        let mut tr = TrackMap::new();
        upsert_position(&mut recs, &mut tr, "987654321", 25.7617, -80.1918, Some(12.4), Some(86.7), Some(87.0), 1_700_000_000);
        upsert_static(&mut recs, "987654321", Some("EXAMPLE VESSEL".into()), Some("9876543".into()), Some("PassengerShips".into()), Some("MIAMI".into()));
        let env = vessels_envelope(
            &recs,
            Transport::Live,
            Some(1_700_000_000),
            None,
            0,
            1_700_000_000_500,
        );
        // Top-level keys — exactly the contract set, nothing extra.
        let mut keys: Vec<&str> = env.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "lastMessageAt", "newestPositionAt", "nextAttemptAt",
                "reconnectAttempt", "refreshing", "rows", "silentForMs", "status",
            ]
        );
        assert_eq!(env["status"], "live");
        assert_eq!(env["refreshing"], false);
        assert_eq!(env["lastMessageAt"], "2023-11-14T22:13:20Z");
        assert_eq!(env["newestPositionAt"], "2023-11-14T22:13:20Z");
        assert_eq!(env["silentForMs"], 500);
        assert_eq!(env["reconnectAttempt"], 0);
        assert_eq!(env["nextAttemptAt"], serde_json::Value::Null);
        // Row keys — exactly the contract set.
        let row = &env["rows"][0];
        let mut rkeys: Vec<&str> = row.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        rkeys.sort_unstable();
        assert_eq!(
            rkeys,
            vec![
                "course", "destination", "heading", "imo", "last_position_epoch",
                "lat", "lon", "mmsi", "name", "speed", "type_specific",
            ]
        );
        assert_eq!(row["mmsi"], "987654321");
        assert!((row["speed"].as_f64().unwrap() - 12.4).abs() < 1e-9, "knots — client converts");
        assert!((row["course"].as_f64().unwrap() - 86.7).abs() < 1e-9);
        assert_eq!(row["heading"], 87.0);
        assert_eq!(row["last_position_epoch"], 1_700_000_000);
        assert_eq!(row["type_specific"], "PassengerShips");
    }

    #[test]
    fn envelope_degraded_status_and_next_attempt() {
        let recs = HashMap::new();
        let env = vessels_envelope(
            &recs,
            Transport::Reconnecting,
            Some(1_700_000_000),
            Some(1_700_000_030),
            3,
            (1_700_000_000 + 130) * 1000,
        );
        assert_eq!(env["status"], "reconnecting");
        assert_eq!(env["refreshing"], true, "non-live ⇒ client renders stale chip");
        assert_eq!(env["nextAttemptAt"], 1_700_000_030i64 * 1000, "epoch ms number");
        assert_eq!(env["silentForMs"], 130_000);
        assert_eq!(env["reconnectAttempt"], 3);
        assert_eq!(env["rows"].as_array().unwrap().len(), 0);
        assert_eq!(env["newestPositionAt"], serde_json::Value::Null);
        // Auth-failed envelope never fast-retries.
        let env = vessels_envelope(&recs, Transport::AuthFailed, None, Some(1_700_003_600), 1, 1_700_000_000_000);
        assert_eq!(env["status"], "auth-failed");
        assert_eq!(env["refreshing"], true);
    }

    #[test]
    fn envelope_rows_are_mmsi_sorted_and_skip_positionless() {
        let mut recs = HashMap::new();
        let mut tr = TrackMap::new();
        upsert_static(&mut recs, "555", Some("NO POS".into()), None, None, None);
        upsert_position(&mut recs, &mut tr, "999", 0.0, 0.0, None, None, None, 100);
        upsert_position(&mut recs, &mut tr, "111", 0.0, 0.0, None, None, None, 100);
        let env = vessels_envelope(&recs, Transport::Live, None, None, 0, 0);
        let ids: Vec<&str> = env["rows"].as_array().unwrap().iter().map(|r| r["mmsi"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["111", "999"], "mmsi-sorted; static-only vessel excluded");
    }

    // ---------- key selection (opensky pattern) ----------

    #[test]
    fn key_selection_prefers_hub_prefix_and_rejects_empty() {
        // The registry gate is one call to api_key(); exercise the pure
        // selection semantics without mutating the process env.
        fn select(hub: Option<&str>, bare: Option<&str>) -> Option<String> {
            hub.filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| bare.filter(|s| !s.is_empty()).map(str::to_string))
        }
        assert_eq!(select(Some("hub-key"), Some("bare-key")).as_deref(), Some("hub-key"), "HUB_ wins");
        assert_eq!(select(Some(""), Some("bare-key")).as_deref(), Some("bare-key"), "empty HUB_ falls through");
        assert_eq!(select(None, Some("bare-key")).as_deref(), Some("bare-key"));
        assert_eq!(select(Some(""), None), None);
        assert_eq!(select(None, None), None, "no key → collector not registered");
    }
}
