//! GEV P12 T1: adsbdb.com flight enrichment proxy + 24h disk-persisted cache.
//!
//! Two upstream lookups, keyless (adsbdb is CC0):
//! - `GET /api/adsbdb/type/{hex6}`       → ICAO24 hex → aircraft type + registration
//! - `GET /api/adsbdb/route/{callsign}`  → callsign → airline + origin/destination
//!
//! The console (GEV `enrichment.js`) fetches these lazily per visible/tracked
//! aircraft. Without a cache that is one upstream request per plane per sweep —
//! the `EnrichmentService` here makes it one request per entity per 24h:
//!
//! - warm entries are served from an in-process map without touching the network,
//! - concurrent misses on the same key coalesce to a single upstream call,
//! - upstream 404s are negative-cached so unknown hexes don't hammer adsbdb,
//! - the map is persisted to disk (tmp+rename, atomic) on a 15s dirty flush so a
//!   hub-core restart keeps its warm set.
//!
//! Response contracts are in
//! `docs/superpowers/specs/2026-09-21-gev-p12-flight-layer-design.md` §3.2.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, RwLock};

use crate::error::HubError;
use crate::state::AppState;

/// Cache entry TTL: 24 hours. Route/aircraft assignments are stable on that
/// horizon.
pub const TTL_MS: u64 = 24 * 3600 * 1000;

/// Default on-disk cache path.
///
/// The design spec named `/var/lib/intelhub/adsbdb-cache.json`, but the
/// deployed `hub-core.service` runs with `ProtectSystem=strict` and
/// `ReadWritePaths=/home/zou/IntelHub/data ...` — a `/var/lib` path is
/// read-only there, so a write would fail at flush time. The default lives
/// under the writable data dir instead. Override with
/// `HUB_ADSBDB_CACHE_PATH`.
pub const DEFAULT_CACHE_PATH: &str = "/home/zou/IntelHub/data/adsbdb-cache.json";

/// Dirty-flush cadence (spec §4.A).
pub const FLUSH_INTERVAL_SECS: u64 = 15;

/// Per-request upstream budget.
const UPSTREAM_TIMEOUT_MS: u64 = 8000;

/// Resolved cache path: `HUB_ADSBDB_CACHE_PATH` when set, else
/// [`DEFAULT_CACHE_PATH`].
pub fn cache_path() -> PathBuf {
    std::env::var("HUB_ADSBDB_CACHE_PATH")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CACHE_PATH))
}

/// adsbdb API base URL. `HUB_ADSBDB_BASE_URL` lets tests/deployments point at
/// a mirror or a wiremock instance; defaults to the public API.
pub fn adsbdb_base_url() -> String {
    std::env::var("HUB_ADSBDB_BASE_URL")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://api.adsbdb.com".to_string())
}

// ---------- wire types ----------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AirportData {
    pub code: String,
    pub name: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RouteData {
    pub airline: Option<String>,
    pub origin: AirportData,
    pub destination: AirportData,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AircraftData {
    pub type_code: Option<String>,
    pub type_name: Option<String>,
    pub registration: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CachedRoute {
    pub at: u64,
    /// `None` = negative cache entry (upstream 404 / unparseable 200).
    pub data: Option<RouteData>,
}

#[derive(Clone, Debug)]
pub struct CachedAircraft {
    pub at: u64,
    pub data: Option<AircraftData>,
}

// ---------- cache ----------

/// In-process enrichment cache. Owned behind an `RwLock` by
/// [`EnrichmentService`]; the maps are plain `HashMap`s so single-threaded
/// tests can exercise them directly (the service adds the concurrency layer).
#[derive(Debug, Default)]
pub struct EnrichmentCache {
    pub routes: HashMap<String, CachedRoute>,
    pub aircraft: HashMap<String, CachedAircraft>,
    /// Set on every insert; the flusher swaps it false and persists when true.
    pub dirty: AtomicBool,
    /// Number of real upstream requests issued (test/observability counter —
    /// cache hits and validation rejections do not count).
    pub upstream_calls: AtomicU64,
}

impl EnrichmentCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// TTL freshness check, factored through [`FreshCheck`] so routes and
    /// aircraft share one implementation.
    pub fn is_fresh(&self, entry: &impl FreshCheck) -> bool {
        entry.is_fresh()
    }

    /// Load a persisted cache from disk. A missing file is an error
    /// (`NotFound`) — the caller decides whether that means "start empty".
    ///
    /// Async on purpose (spec §4.A): `tokio::fs` runs the read on the blocking
    /// pool, so a boot-time cache load never blocks a runtime worker.
    pub async fn load_from(path: &Path) -> Result<Self, std::io::Error> {
        let s = tokio::fs::read_to_string(path).await?;
        Self::from_persisted_json(&s)
    }

    /// Decode a persisted cache body into the in-memory shape. Split out so
    /// the wire→memory mapping is exercisable without touching disk.
    fn from_persisted_json(s: &str) -> Result<Self, std::io::Error> {
        let p: PersistedCache = serde_json::from_str(s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Self {
            routes: p
                .routes
                .into_iter()
                .map(|(k, v)| (k, CachedRoute { at: v.at, data: v.data }))
                .collect(),
            aircraft: p
                .aircraft
                .into_iter()
                .map(|(k, v)| (k, CachedAircraft { at: v.at, data: v.data }))
                .collect(),
            dirty: AtomicBool::new(false),
            upstream_calls: AtomicU64::new(0),
        })
    }

    /// Snapshot into the on-disk shape. Pure memory copy — no syscalls and no
    /// serialization — so it is safe to call while holding a read guard.
    fn to_persisted(&self) -> PersistedCache {
        PersistedCache {
            routes: self
                .routes
                .iter()
                .map(|(k, v)| (k.clone(), PersistedEntry { at: v.at, data: v.data.clone() }))
                .collect(),
            aircraft: self
                .aircraft
                .iter()
                .map(|(k, v)| (k.clone(), PersistedEntry { at: v.at, data: v.data.clone() }))
                .collect(),
        }
    }

    /// Serialize + atomically swap into place: write a sibling `.tmp` file,
    /// then `rename` over the target. `rename` is atomic on the same
    /// filesystem, so a crash mid-write can never leave a truncated cache.
    ///
    /// **Blocking**: only ever called inside
    /// [`Self::write_persisted`]'s `spawn_blocking` closure. Serialization is
    /// deliberately included — the flusher rewrites the whole cache on every
    /// dirty tick, so the serde pass belongs off the runtime too.
    fn write_persisted_blocking(p: &PersistedCache, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(p)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Run [`Self::write_persisted_blocking`] on the blocking pool.
    ///
    /// `tokio::fs::write` is itself a thin `spawn_blocking` wrapper; doing the
    /// whole serialize + write + rename in one `spawn_blocking` gives the same
    /// "no syscall on a runtime worker" guarantee *and* moves the full-cache
    /// serde pass off the worker (which `tokio::fs` alone would leave inline).
    async fn write_persisted(p: PersistedCache, path: PathBuf) -> Result<(), std::io::Error> {
        tokio::task::spawn_blocking(move || Self::write_persisted_blocking(&p, &path))
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?
    }

    /// Persist the cache atomically, off the async runtime. Used by the dirty
    /// flusher ([`EnrichmentService::spawn_flusher`]).
    pub async fn persist_to(&self, path: &Path) -> Result<(), std::io::Error> {
        Self::write_persisted(self.to_persisted(), path.to_path_buf()).await
    }

    /// Wire shape for `GET /api/adsbdb/type/{hex}` (spec §3.2). A negative
    /// entry and a miss both render `{found:false}` — the console treats both
    /// as "unknown".
    pub fn aircraft_response(&self, hex: &str) -> serde_json::Value {
        match self.aircraft.get(hex) {
            Some(CachedAircraft { data: Some(d), .. }) => json!({
                "found": true,
                "typeCode": d.type_code,
                "typeName": d.type_name,
                "registration": d.registration
            }),
            _ => json!({ "found": false }),
        }
    }
}

/// TTL helper implemented by both cache entry shapes.
pub trait FreshCheck {
    fn is_fresh(&self) -> bool;
}

impl FreshCheck for CachedRoute {
    fn is_fresh(&self) -> bool {
        now_ms().saturating_sub(self.at) < TTL_MS
    }
}

impl FreshCheck for CachedAircraft {
    fn is_fresh(&self) -> bool {
        now_ms().saturating_sub(self.at) < TTL_MS
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedCache {
    routes: HashMap<String, PersistedEntry<RouteData>>,
    aircraft: HashMap<String, PersistedEntry<AircraftData>>,
}

#[derive(Serialize, Deserialize)]
struct PersistedEntry<T> {
    at: u64,
    data: Option<T>,
}

/// Millisecond wall-clock. Single time source for the whole module (entry
/// timestamps + TTL math) — `SystemTime::now()` everywhere.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------- upstream parsing ----------

/// Extract `response.flightroute.{airline,origin,destination}` (spec §3.2).
/// Returns `None` when the route is absent or either endpoint is null — the
/// caller stores that as a negative cache entry.
pub fn parse_route(json: &serde_json::Value) -> Option<RouteData> {
    let fr = json.get("response")?.get("flightroute")?.as_object()?;
    if fr.get("origin").map(|v| v.is_null()).unwrap_or(true) {
        return None;
    }
    if fr.get("destination").map(|v| v.is_null()).unwrap_or(true) {
        return None;
    }
    let airport = |a: &serde_json::Value| AirportData {
        code: a
            .get("iata_code")
            .or_else(|| a.get("icao_code"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        name: a
            .get("municipality")
            .or_else(|| a.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        lat: a.get("latitude").and_then(|v| v.as_f64()).filter(|n| n.is_finite()),
        lon: a.get("longitude").and_then(|v| v.as_f64()).filter(|n| n.is_finite()),
    };
    Some(RouteData {
        airline: fr
            .get("airline")
            .and_then(|a| a.get("name"))
            .and_then(|v| v.as_str())
            .map(String::from),
        origin: airport(fr.get("origin").unwrap()),
        destination: airport(fr.get("destination").unwrap()),
    })
}

/// Extract `response.aircraft.{icao_type,manufacturer,type,registration}`
/// (spec §3.2). `typeName` is the `manufacturer` + `type` join the vendor UI
/// displays.
pub fn parse_aircraft(json: &serde_json::Value) -> Option<AircraftData> {
    let a = json.get("response")?.get("aircraft")?.as_object()?;
    let type_code = a.get("icao_type").and_then(|v| v.as_str()).map(String::from);
    let manufacturer = a.get("manufacturer").and_then(|v| v.as_str());
    let type_name_only = a.get("type").and_then(|v| v.as_str());
    let type_name = match (manufacturer, type_name_only) {
        (Some(m), Some(t)) => Some(format!("{m} {t}")),
        (None, Some(t)) => Some(t.to_string()),
        _ => None,
    };
    let registration = a.get("registration").and_then(|v| v.as_str()).map(String::from);
    Some(AircraftData { type_code, type_name, registration })
}

/// Wire shape for `GET /api/adsbdb/route/{callsign}` (spec §3.2).
pub fn route_response(data: Option<&RouteData>) -> serde_json::Value {
    match data {
        Some(d) => json!({
            "found": true,
            "airline": d.airline,
            "origin": {
                "code": d.origin.code,
                "name": d.origin.name,
                "lat": d.origin.lat,
                "lon": d.origin.lon
            },
            "destination": {
                "code": d.destination.code,
                "name": d.destination.name,
                "lat": d.destination.lat,
                "lon": d.destination.lon
            }
        }),
        None => json!({ "found": false }),
    }
}

// ---------- service ----------

/// Shared enrichment state: the cache plus the concurrency + persistence
/// machinery. Injected into [`AppState`] and shared across handlers.
pub struct EnrichmentService {
    cache: RwLock<EnrichmentCache>,
    /// Per-key fetch locks. A key is held only while its upstream fetch is in
    /// flight, so N concurrent misses on the same key produce one request.
    inflight: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    http: reqwest::Client,
    base_url: String,
}

impl EnrichmentService {
    /// Production constructor: shared HTTP client + `HUB_ADSBDB_BASE_URL`.
    pub fn new(http: reqwest::Client) -> Self {
        Self::with_base_url(http, adsbdb_base_url())
    }

    pub fn with_base_url(http: reqwest::Client, base_url: impl Into<String>) -> Self {
        Self {
            cache: RwLock::new(EnrichmentCache::new()),
            inflight: Mutex::new(HashMap::new()),
            http,
            base_url: base_url.into(),
        }
    }

    /// Upstream request counter (cache hits / 4xx validation excluded).
    pub async fn upstream_calls(&self) -> u64 {
        self.cache.read().await.upstream_calls.load(Ordering::Relaxed)
    }

    /// Persist through a short-lived snapshot: the read guard is released
    /// before the await, so a slow blocking write never stalls producers.
    pub async fn persist_to_path(&self, path: &Path) -> Result<(), std::io::Error> {
        let snapshot = { self.cache.read().await.to_persisted() };
        EnrichmentCache::write_persisted(snapshot, path.to_path_buf()).await
    }

    /// Boot: load the persisted cache (missing/corrupt = start empty, never
    /// fatal) and spawn the dirty flusher.
    ///
    /// Hosted on the service rather than on `AppState` so the whole
    /// disk → warm cache → flusher path is testable without booting PG/Redis/
    /// Neo4j. `flush_every` is [`FLUSH_INTERVAL_SECS`] in production; tests
    /// pass a short interval so the flush is observable without waiting 15s.
    pub async fn start(self: &Arc<Self>, path: PathBuf, flush_every: Duration) {
        match EnrichmentCache::load_from(&path).await {
            Ok(c) => {
                let routes = c.routes.len();
                let aircraft = c.aircraft.len();
                {
                    let mut w = self.cache.write().await;
                    w.routes = c.routes;
                    w.aircraft = c.aircraft;
                }
                tracing::info!(
                    target: "hub.boot",
                    routes, aircraft, path = %path.display(),
                    "adsbdb enrichment cache loaded"
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(
                    target: "hub.boot",
                    path = %path.display(),
                    "adsbdb enrichment cache not present — starting empty"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e, path = %path.display(),
                    "adsbdb enrichment cache load failed — starting empty"
                );
            }
        }
        self.spawn_flusher(path, flush_every);
    }

    async fn key_lock(&self, key: &str) -> Arc<Mutex<()>> {
        let mut m = self.inflight.lock().await;
        m.entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn cached_aircraft(&self, hex: &str) -> Option<Response> {
        let c = self.cache.read().await;
        match c.aircraft.get(hex) {
            Some(entry) if entry.is_fresh() => {
                Some(Json(c.aircraft_response(hex)).into_response())
            }
            _ => None,
        }
    }

    async fn cached_route(&self, callsign: &str) -> Option<Response> {
        let c = self.cache.read().await;
        match c.routes.get(callsign) {
            Some(entry) if entry.is_fresh() => {
                Some(Json(route_response(entry.data.as_ref())).into_response())
            }
            _ => None,
        }
    }

    async fn store_aircraft(&self, hex: &str, data: Option<AircraftData>) -> serde_json::Value {
        let mut c = self.cache.write().await;
        c.aircraft
            .insert(hex.to_string(), CachedAircraft { at: now_ms(), data });
        c.dirty.store(true, Ordering::Relaxed);
        c.aircraft_response(hex)
    }

    async fn store_route(&self, callsign: &str, data: Option<RouteData>) -> serde_json::Value {
        let mut c = self.cache.write().await;
        c.routes
            .insert(callsign.to_string(), CachedRoute { at: now_ms(), data });
        c.dirty.store(true, Ordering::Relaxed);
        route_response(c.routes.get(callsign).and_then(|e| e.data.as_ref()))
    }

    /// `GET /api/adsbdb/type/{hex}` core: validate → cache → coalesce → fetch.
    pub async fn aircraft(&self, raw_hex: &str) -> Response {
        let hex = raw_hex.trim().to_ascii_lowercase();
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return err(StatusCode::BAD_REQUEST, "invalid hex");
        }
        if let Some(hit) = self.cached_aircraft(&hex).await {
            return hit;
        }
        // Coalesce: only one task per key reaches the upstream fetch. The rest
        // block here, then hit the warm cache on the re-check inside the guard.
        let lock = self.key_lock(&format!("type:{hex}")).await;
        let _guard = lock.lock().await;
        if let Some(hit) = self.cached_aircraft(&hex).await {
            return hit;
        }
        self.fetch_aircraft(&hex).await
    }

    /// `GET /api/adsbdb/route/{callsign}` core.
    pub async fn route(&self, raw_callsign: &str) -> Response {
        let callsign = raw_callsign.trim().to_ascii_uppercase();
        if callsign.len() < 2
            || callsign.len() > 8
            || !callsign.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return err(StatusCode::BAD_REQUEST, "invalid callsign");
        }
        if let Some(hit) = self.cached_route(&callsign).await {
            return hit;
        }
        let lock = self.key_lock(&format!("route:{callsign}")).await;
        let _guard = lock.lock().await;
        if let Some(hit) = self.cached_route(&callsign).await {
            return hit;
        }
        self.fetch_route(&callsign).await
    }

    async fn fetch_aircraft(&self, hex: &str) -> Response {
        self.cache
            .read()
            .await
            .upstream_calls
            .fetch_add(1, Ordering::Relaxed);
        let url = format!("{}/v0/aircraft/{}", self.base_url, hex);
        let resp = match self
            .http
            .get(&url)
            .timeout(Duration::from_millis(UPSTREAM_TIMEOUT_MS))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e.without_url(), hex = %hex, "adsbdb aircraft fetch failed");
                return err(StatusCode::SERVICE_UNAVAILABLE, "adsbdb upstream unavailable");
            }
        };
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            // Negative cache — unknown hexes must not hammer adsbdb.
            let out = self.store_aircraft(hex, None).await;
            return Json(out).into_response();
        }
        if !status.is_success() {
            // Transient upstream failure: never cache, retry next request.
            tracing::warn!(status = %status, hex = %hex, "adsbdb aircraft upstream returned error");
            return err(StatusCode::SERVICE_UNAVAILABLE, "adsbdb upstream unavailable");
        }
        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, hex = %hex, "adsbdb aircraft body decode failed");
                return err(StatusCode::SERVICE_UNAVAILABLE, "adsbdb upstream unavailable");
            }
        };
        let out = self.store_aircraft(hex, parse_aircraft(&body)).await;
        Json(out).into_response()
    }

    async fn fetch_route(&self, callsign: &str) -> Response {
        self.cache
            .read()
            .await
            .upstream_calls
            .fetch_add(1, Ordering::Relaxed);
        let url = format!("{}/v0/callsign/{}", self.base_url, callsign);
        let resp = match self
            .http
            .get(&url)
            .timeout(Duration::from_millis(UPSTREAM_TIMEOUT_MS))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e.without_url(), callsign = %callsign, "adsbdb route fetch failed");
                return err(StatusCode::SERVICE_UNAVAILABLE, "adsbdb upstream unavailable");
            }
        };
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            let out = self.store_route(callsign, None).await;
            return Json(out).into_response();
        }
        if !status.is_success() {
            tracing::warn!(status = %status, callsign = %callsign, "adsbdb route upstream returned error");
            return err(StatusCode::SERVICE_UNAVAILABLE, "adsbdb upstream unavailable");
        }
        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, callsign = %callsign, "adsbdb route body decode failed");
                return err(StatusCode::SERVICE_UNAVAILABLE, "adsbdb upstream unavailable");
            }
        };
        let out = self.store_route(callsign, parse_route(&body)).await;
        Json(out).into_response()
    }

    /// Spawn the dirty-flush loop (`every` = [`FLUSH_INTERVAL_SECS`] in
    /// production). First tick is skipped (interval fires immediately
    /// otherwise).
    fn spawn_flusher(self: &Arc<Self>, path: PathBuf, every: Duration) {
        let svc = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(every);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await; // skip immediate
            loop {
                interval.tick().await;
                let dirty = svc.cache.read().await.dirty.swap(false, Ordering::Relaxed);
                if dirty {
                    if let Err(e) = svc.persist_to_path(&path).await {
                        tracing::warn!(error = %e, path = %path.display(), "adsbdb cache flush failed");
                    } else {
                        tracing::debug!(path = %path.display(), "adsbdb cache flushed");
                    }
                }
            }
        });
    }
}

// ---------- HTTP surface ----------

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// Path dispatcher shared by the axum handlers (and directly testable).
/// Mirrors the design spec's `route(req, state)` entry point but takes the
/// path + service so the caller can inject a wiremock-backed service.
pub async fn handle_route(path: &str, svc: &EnrichmentService) -> Response {
    if let Some(hex) = path.strip_prefix("/api/adsbdb/type/") {
        return svc.aircraft(hex).await;
    }
    if let Some(callsign) = path.strip_prefix("/api/adsbdb/route/") {
        return svc.route(callsign).await;
    }
    err(StatusCode::NOT_FOUND, "unknown adsbdb endpoint")
}

/// `GET /api/adsbdb/type/{hex}`
pub async fn gev_adsbdb_type(
    State(state): State<Arc<AppState>>,
    AxumPath(hex): AxumPath<String>,
) -> Response {
    handle_route(&format!("/api/adsbdb/type/{hex}"), &state.enrichment).await
}

/// `GET /api/adsbdb/route/{callsign}`
pub async fn gev_adsbdb_route(
    State(state): State<Arc<AppState>>,
    AxumPath(callsign): AxumPath<String>,
) -> Response {
    handle_route(&format!("/api/adsbdb/route/{callsign}"), &state.enrichment).await
}

/// Boot hook (called from `server.rs`): load the persisted cache and start
/// the dirty flusher at the production 15s cadence.
pub async fn start(state: AppState) -> Result<(), HubError> {
    state
        .enrichment
        .start(cache_path(), Duration::from_secs(FLUSH_INTERVAL_SECS))
        .await;
    Ok(())
}
