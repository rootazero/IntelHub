//! GEV P12 T2: aircraft track backfill proxies (OpenSky + adsb.lol).
//!
//! The vendor engine fetches `/api/opensky-track?icao24=<hex6>` when a user
//! clicks a plane (upstream `standalone.js:86` — the path literal is part of
//! the console ↔ hub contract and must not be renamed) and
//! `/api/adsblol/trace?hex=<hex>` as the long-tail fallback. Both upstreams
//! are proxied here so that:
//!
//! - the OpenSky OAuth secret never reaches the browser,
//! - a rate-limited upstream degrades to the last known track instead of an
//!   empty screen (serve-stale), and
//! - several console viewers share one upstream budget (in-process cache).
//!
//! `OpenSkyClient`:
//! - OAuth2 client-credentials token, cached until 60s before its nominal
//!   expiry ([`TOKEN_MARGIN_MS`]); concurrent refreshes coalesce onto a single
//!   upstream call (refresh mutex + double-check, same shape as T1's per-key
//!   coalescer in `gev_enrichment.rs`).
//! - adaptive cache TTL from `X-Rate-Limit-Remaining` ([`adaptive_ttl`]):
//!   >2400 ⇒ 9s, >1200 ⇒ 30s, >400 ⇒ 90s, else 5min.
//! - 429 ⇒ honour `Retry-After` (clamped to 5s..30min) as a global cooldown;
//!   while the cooldown is active a cached body is served with the stale
//!   marker instead of an error.
//! - upstream bodies are capped at 5MB ([`RESPONSE_CAP_BYTES`]) and each
//!   per-key cache at 200 entries, oldest-`at`-wins ([`TRACK_CACHE_MAX`]).
//!
//! Response bodies keep the upstream shape verbatim — the vendor reads
//! `path` (OpenSky) and `trace` + `timestamp` (adsb.lol readings) — and gain a
//! normalized `records` array (spec §3.1/§3.3):
//! `{ "records": [{observedAtMs, latitude, longitude, baroAltitudeM,
//! courseDeg, onGround}], "complete": false, ... }`.
//!
//! Contract: `docs/superpowers/specs/2026-09-21-gev-p12-flight-layer-design.md`
//! §3.1, §3.3, §4.B.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::json;
use tokio::sync::{Mutex, RwLock};

use crate::error::HubError;
use crate::state::AppState;

/// Healthy-budget tier of [`adaptive_ttl`] (and the default before the first
/// upstream response reveals the real remaining quota).
pub const OPENSKY_CACHE_MS: u64 = 9000;
/// adsb.lol trace cache TTL — fixed 60s (spec §4.B; adsb.lol publishes no
/// rate-limit header to adapt on).
pub const TRACK_CACHE_MS: u64 = 60_000;
/// Per-key cap for both track caches (spec §4.B).
pub const TRACK_CACHE_MAX: usize = 200;
/// Upstream body cap. A track is a few KB; anything approaching this is a
/// mirror bug and must not be cached or forwarded.
pub const RESPONSE_CAP_BYTES: usize = 5 * 1024 * 1024;
/// 429 cooldown floor / ceiling (spec §4.B).
pub const COOLDOWN_MIN_MS: u64 = 5_000;
pub const COOLDOWN_MAX_MS: u64 = 30 * 60 * 1000;
/// Per-request upstream budget. The vendor aborts its own fetch at 8s
/// (`tracking.js:484` `AbortSignal.timeout(8000)`), so a slower proxy reply
/// is useless — fail fast and let the client fall back to its local trail.
pub const UPSTREAM_TIMEOUT_MS: u64 = 8000;

/// Refresh the token this long before its nominal expiry (spec §4.B / monitor
/// parity: `TOKEN_MARGIN_SECS = 60`), so clock skew cannot retire a token
/// mid-flight.
const TOKEN_MARGIN_MS: u64 = 60_000;
/// `expires_in` fallback when the token endpoint omits it (OpenSky sends 1800).
const DEFAULT_TOKEN_LIFETIME_SECS: u64 = 1800;
/// `Retry-After` values at or above this are absolute epoch seconds rather
/// than delta-seconds (HTTP allows both; OpenSky sends delta-seconds, some
/// CDNs send an epoch). Real deltas are seconds..hours; real epochs are
/// >2001-09-09. See [`parse_retry_after_secs`].
const RETRY_AFTER_EPOCH_FLOOR: u64 = 1_000_000_000;

const DEFAULT_OPENSKY_AUTH_URL: &str =
    "https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token";
const DEFAULT_OPENSKY_API_BASE: &str = "https://opensky-network.org/api";
const DEFAULT_ADSB_LOL_BASE_URL: &str = "https://adsb.lol";

/// Fraction of a foot, for the readsb (`alt_baro` in feet) trace shape.
const FOOT_TO_M: f64 = 0.3048;

// ---------- pure helpers ----------

/// Adaptive cache TTL (ms) from OpenSky's `X-Rate-Limit-Remaining` header.
/// `None` (header absent) means "never seen a limit" ⇒ treat as the healthy
/// tier, i.e. the pre-header default.
pub fn adaptive_ttl(remaining: Option<u64>) -> u64 {
    let r = remaining.unwrap_or(u64::MAX);
    if r > 2400 {
        OPENSKY_CACHE_MS
    } else if r > 1200 {
        30_000
    } else if r > 400 {
        90_000
    } else {
        300_000
    }
}

/// Parse an HTTP `Retry-After` header into a millisecond delta. Accepts both
/// allowed forms: delta-seconds (`"30"` ⇒ 30_000) and absolute epoch seconds
/// (⇒ `epoch - now`, saturating at 0). Anything else — including the
/// HTTP-date form, which neither upstream sends — is `None` and the caller
/// falls back to its own default.
pub fn parse_retry_after_secs(v: &str, now_ms: u64) -> Option<u64> {
    let v = v.trim();
    let n = v.parse::<u64>().ok()?;
    if n >= RETRY_AFTER_EPOCH_FLOOR {
        Some(n.saturating_sub(now_ms / 1000).saturating_mul(1000))
    } else {
        Some(n.saturating_mul(1000))
    }
}

/// One normalized track waypoint (spec §3.1 wire names are camelCase).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TrackRecord {
    #[serde(rename = "observedAtMs")]
    pub observed_at_ms: u64,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(rename = "baroAltitudeM")]
    pub baro_altitude_m: Option<f64>,
    #[serde(rename = "courseDeg")]
    pub course_deg: Option<f64>,
    #[serde(rename = "onGround")]
    pub on_ground: bool,
}

/// Normalize an OpenSky `/tracks/all` `path` (array-of-arrays
/// `[time_epoch_s, lat, lon, baro_alt_m, true_track_deg, on_ground]`) into
/// [`TrackRecord`]s. Non-array, short, or non-finite rows are skipped — an
/// upstream format wobble must degrade the trail, never fail the response.
pub fn normalize_track_path(path: &[serde_json::Value]) -> Vec<TrackRecord> {
    let mut records = Vec::with_capacity(path.len());
    for waypoint in path {
        let Some(arr) = waypoint.as_array() else {
            continue;
        };
        if arr.len() < 3 {
            continue;
        }
        let (Some(time), Some(lat), Some(lon)) =
            (arr[0].as_f64(), arr[1].as_f64(), arr[2].as_f64())
        else {
            continue;
        };
        if !time.is_finite() || !lat.is_finite() || !lon.is_finite() {
            continue;
        }
        records.push(TrackRecord {
            observed_at_ms: ((time * 1000.0).max(0.0)) as u64,
            latitude: lat,
            longitude: lon,
            baro_altitude_m: arr
                .get(3)
                .and_then(|v| v.as_f64())
                .filter(|n| n.is_finite()),
            course_deg: arr
                .get(4)
                .and_then(|v| v.as_f64())
                .filter(|n| n.is_finite()),
            on_ground: arr.get(5).and_then(|v| v.as_bool()).unwrap_or(false),
        });
    }
    records
}

/// Normalize an adsb.lol tar1090/readsb `trace_full_<hex>.json` body:
/// `timestamp` is the epoch-second base and every row is
/// `[offset_s, lat, lon, alt_ft | "ground", gs_kt, track_deg, ...]`.
/// `base_ts_secs == None` (upstream shape drift) yields no records rather
/// than inventing timestamps.
pub fn normalize_readsb_trace(
    base_ts_secs: Option<u64>,
    trace: &[serde_json::Value],
) -> Vec<TrackRecord> {
    let Some(base) = base_ts_secs else {
        return Vec::new();
    };
    let mut records = Vec::with_capacity(trace.len());
    for row in trace {
        let Some(arr) = row.as_array() else {
            continue;
        };
        if arr.len() < 3 {
            continue;
        }
        let (Some(offset), Some(lat), Some(lon)) =
            (arr[0].as_f64(), arr[1].as_f64(), arr[2].as_f64())
        else {
            continue;
        };
        if !offset.is_finite() || !lat.is_finite() || !lon.is_finite() {
            continue;
        }
        let on_ground = arr.get(3).and_then(|v| v.as_str()) == Some("ground");
        let baro_altitude_m = if on_ground {
            None
        } else {
            arr.get(3)
                .and_then(|v| v.as_f64())
                .filter(|n| n.is_finite())
                .map(|ft| ft * FOOT_TO_M)
        };
        records.push(TrackRecord {
            observed_at_ms: ((base as f64 + offset) * 1000.0).max(0.0) as u64,
            latitude: lat,
            longitude: lon,
            baro_altitude_m,
            course_deg: arr
                .get(5)
                .and_then(|v| v.as_f64())
                .filter(|n| n.is_finite()),
            on_ground,
        });
    }
    records
}

/// Build the hub wire body from a raw upstream track body: upstream shape
/// verbatim, plus `records` + `complete`. Already-augmented bodies (what the
/// cache stores) pass through untouched, so a cache hit costs one `contains`
/// scan instead of a re-parse of up to 5MB.
pub fn augment_track_body(raw: &str) -> String {
    if raw.contains("\"records\"") {
        return raw.to_string();
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return json!({ "records": [], "complete": false }).to_string();
    };
    let Some(obj) = v.as_object() else {
        return json!({ "records": [], "complete": false }).to_string();
    };
    let mut out = obj.clone();
    let records = if let Some(path) = out.get("path").and_then(|p| p.as_array()) {
        normalize_track_path(path)
    } else if let Some(trace) = out.get("trace").and_then(|t| t.as_array()) {
        normalize_readsb_trace(out.get("timestamp").and_then(|t| t.as_u64()), trace)
    } else {
        Vec::new()
    };
    out.insert("records".to_string(), json!(records));
    out.insert("complete".to_string(), json!(false));
    serde_json::Value::Object(out).to_string()
}

/// One cached upstream body: when it was stored (`at`, epoch ms) and the
/// augmented JSON.
#[derive(Clone, Debug)]
pub struct CachedTrack {
    pub at: u64,
    pub body: String,
}

/// Drop oldest-`at` entries until the map is back within [`TRACK_CACHE_MAX`].
/// Returns the number evicted. Pure map operation so the eviction policy is
/// testable without the handler.
pub fn evict_oldest(map: &mut HashMap<String, CachedTrack>) -> usize {
    let mut evicted = 0;
    while map.len() > TRACK_CACHE_MAX {
        let Some(oldest) = map.iter().min_by_key(|(_, v)| v.at).map(|(k, _)| k.clone()) else {
            break;
        };
        map.remove(&oldest);
        evicted += 1;
    }
    evicted
}

/// Insert into a bounded track cache; oldest-`at` eviction keeps it at
/// [`TRACK_CACHE_MAX`].
async fn put_capped(
    map: &Mutex<HashMap<String, CachedTrack>>,
    key: &str,
    entry: CachedTrack,
) {
    let mut guard = map.lock().await;
    guard.insert(key.to_string(), entry);
    if guard.len() > TRACK_CACHE_MAX {
        evict_oldest(&mut guard);
    }
}

/// Fresh-cache lookup: `Some((body, at))` when the entry is younger than
/// `ttl_ms`.
async fn fresh_cached(
    map: &Mutex<HashMap<String, CachedTrack>>,
    key: &str,
    ttl_ms: u64,
) -> Option<(String, u64)> {
    let guard = map.lock().await;
    guard
        .get(key)
        .filter(|e| crate::gev_enrichment::now_ms().saturating_sub(e.at) < ttl_ms)
        .map(|e| (e.body.clone(), e.at))
}

// ---------- config ----------

/// Resolve the OAuth credential pair. Both names are accepted — the
/// `HUB_`-prefixed one wins — and blanks count as unset, matching the monitor
/// collector's convention (`monitor/sources/opensky.rs::oauth_creds`).
pub fn resolve_env_value(primary: Option<String>, fallback: Option<String>) -> Option<String> {
    let clean = |v: Option<String>| {
        v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    };
    clean(primary).or_else(|| clean(fallback))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Every injectable knob of the tracks proxy. `from_env` is the production
/// path; tests build one directly so no global env mutation is needed.
#[derive(Clone, Debug)]
pub struct TracksConfig {
    pub opensky_auth_url: String,
    /// Includes the `/api` suffix — `/tracks/all` is appended to it.
    pub opensky_api_base: String,
    pub adsblol_base_url: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

impl TracksConfig {
    pub fn from_env() -> Self {
        Self {
            opensky_auth_url: env_or("HUB_OPENSKY_AUTH_URL", DEFAULT_OPENSKY_AUTH_URL),
            opensky_api_base: env_or("HUB_OPENSKY_API_BASE", DEFAULT_OPENSKY_API_BASE),
            adsblol_base_url: env_or("HUB_ADSB_LOL_BASE_URL", DEFAULT_ADSB_LOL_BASE_URL),
            client_id: resolve_env_value(
                std::env::var("HUB_OPENSKY_CLIENT_ID").ok(),
                std::env::var("OPENSKY_CLIENT_ID").ok(),
            ),
            client_secret: resolve_env_value(
                std::env::var("HUB_OPENSKY_CLIENT_SECRET").ok(),
                std::env::var("OPENSKY_CLIENT_SECRET").ok(),
            ),
        }
    }

    pub fn has_credentials(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some()
    }
}

impl Default for TracksConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

// ---------- OpenSky OAuth client ----------

/// OAuth2 token cache + adaptive TTL + 429 cooldown for OpenSky `/tracks/all`.
pub struct OpenSkyClient {
    pub http: reqwest::Client,
    auth_url: String,
    api_base: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    token: RwLock<Option<String>>,
    /// Nominal expiry (epoch ms) of [`Self::token`]; `0` = no token yet.
    token_expiry: RwLock<u64>,
    /// Serializes refreshes so N concurrent misses produce one upstream call;
    /// the holder double-checks the cache after acquiring.
    refresh_lock: Mutex<()>,
    /// Cache TTL (ms) derived from the last `X-Rate-Limit-Remaining`.
    pub adaptive_ttl_ms: AtomicU64,
    /// Epoch ms before which upstream fetches are skipped (429 cooldown).
    pub cooldown_until: AtomicU64,
    /// Real upstream token requests (test/observability counter).
    auth_calls: AtomicU64,
}

impl OpenSkyClient {
    pub fn new(http: reqwest::Client, cfg: &TracksConfig) -> Self {
        Self {
            http,
            auth_url: cfg.opensky_auth_url.clone(),
            api_base: cfg.opensky_api_base.clone(),
            client_id: cfg.client_id.clone(),
            client_secret: cfg.client_secret.clone(),
            token: RwLock::new(None),
            token_expiry: RwLock::new(0),
            refresh_lock: Mutex::new(()),
            adaptive_ttl_ms: AtomicU64::new(OPENSKY_CACHE_MS),
            cooldown_until: AtomicU64::new(0),
            auth_calls: AtomicU64::new(0),
        }
    }

    /// `/tracks/all` URL for a validated `icao24`. `time=0` asks for the most
    /// recent track (spec §4.B).
    fn track_url(&self, icao24: &str) -> String {
        format!("{}/tracks/all?icao24={icao24}&time=0", self.api_base)
    }

    pub fn has_credentials(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some()
    }

    /// Upstream token requests issued (coalesced callers do not count).
    pub fn auth_calls(&self) -> u64 {
        self.auth_calls.load(Ordering::Relaxed)
    }

    /// Current cached token, or a fresh one.
    ///
    /// `Ok(None)` = OAuth not configured (handler ⇒ 503 "not configured");
    /// `Err` = configured but the refresh failed (handler ⇒ 503 "auth
    /// failed") — the two must stay distinguishable so a broken credential
    /// never masquerades as "unconfigured".
    pub async fn get_token(&self) -> Result<Option<String>, HubError> {
        if let Some(token) = self.cached_token().await {
            return Ok(Some(token));
        }
        let (Some(id), Some(secret)) = (self.client_id.as_ref(), self.client_secret.as_ref())
        else {
            return Ok(None);
        };
        // Coalesce: whoever holds `refresh_lock` refreshes; everyone else
        // blocks here and returns the token the winner just cached.
        let _guard = self.refresh_lock.lock().await;
        if let Some(token) = self.cached_token().await {
            return Ok(Some(token));
        }
        self.refresh(id, secret).await.map(Some)
    }

    /// Token still inside its 60s safety margin? Clears nothing; read-only.
    async fn cached_token(&self) -> Option<String> {
        let expiry = *self.token_expiry.read().await;
        if crate::gev_enrichment::now_ms().saturating_add(TOKEN_MARGIN_MS) >= expiry {
            return None;
        }
        self.token.read().await.clone()
    }

    async fn refresh(&self, id: &str, secret: &str) -> Result<String, HubError> {
        self.auth_calls.fetch_add(1, Ordering::Relaxed);
        let res = self
            .http
            .post(&self.auth_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", id),
                ("client_secret", secret),
            ])
            .timeout(Duration::from_millis(UPSTREAM_TIMEOUT_MS))
            .send()
            .await
            .map_err(|e| HubError::sensor(format!("opensky auth transport: {}", e.without_url())))?;
        if !res.status().is_success() {
            return Err(HubError::sensor(format!(
                "opensky auth HTTP {}",
                res.status().as_u16()
            )));
        }
        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|e| HubError::sensor(format!("opensky auth body: {e}")))?;
        let Some(token) = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        else {
            return Err(HubError::sensor("opensky auth response missing access_token"));
        };
        let expires_in = body
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TOKEN_LIFETIME_SECS);
        let token = token.to_string();
        *self.token.write().await = Some(token.clone());
        *self.token_expiry.write().await =
            crate::gev_enrichment::now_ms() + expires_in.saturating_mul(1000);
        Ok(token)
    }
}

// ---------- service ----------

/// Shared tracks-proxy state: OpenSky OAuth/cooldown + the two bounded caches
/// + the outbound HTTP client. Injected into `AppState` and shared by the two
/// handlers.
pub struct TracksService {
    pub opensky: Arc<OpenSkyClient>,
    /// OpenSky track bodies keyed by `icao24` (TTL = adaptive tier).
    pub cache: Mutex<HashMap<String, CachedTrack>>,
    /// adsb.lol trace bodies keyed by hex (fixed 60s TTL).
    pub adsblol_cache: Mutex<HashMap<String, CachedTrack>>,
    http: reqwest::Client,
    adsblol_base_url: String,
}

impl TracksService {
    /// Production constructor: env-driven config.
    pub fn new(http: reqwest::Client) -> Self {
        Self::with_config(http, TracksConfig::from_env())
    }

    pub fn with_config(http: reqwest::Client, cfg: TracksConfig) -> Self {
        Self {
            opensky: Arc::new(OpenSkyClient::new(http.clone(), &cfg)),
            cache: Mutex::new(HashMap::new()),
            adsblol_cache: Mutex::new(HashMap::new()),
            http,
            adsblol_base_url: cfg.adsblol_base_url,
        }
    }

    /// Store an OpenSky track body (bounded, oldest-`at` eviction).
    pub async fn cache_put(&self, key: &str, entry: CachedTrack) {
        put_capped(&self.cache, key, entry).await;
    }
}

// ---------- HTTP surface ----------

fn json_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// Track payload response. `cache` is the vendor-visible `X-ADS-B-Cache`
/// state (`HIT` / `MISS` / `STALE`, the values the engine already reads in
/// `data/militaryFlights.js` tests); a stale body also carries
/// `X-OpenSky-Stale: 1` (spec §3.1) and every non-`MISS` body carries its age
/// so the operator can see how old "last known" is.
fn track_response(cache: &'static str, age_ms: Option<u64>, body: &str) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header("X-ADS-B-Cache", cache);
    if let Some(age) = age_ms {
        builder = builder.header("X-ADS-B-Cache-Age-Ms", age.to_string());
    }
    if cache == "STALE" {
        builder = builder.header("X-OpenSky-Stale", "1");
    }
    match builder.body(Body::from(body.to_string())) {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(error = %e, "track response build failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "response build failed")
        }
    }
}

fn parse_query(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// 6-char lowercase hex ICAO24. Hand-rolled (`regex` is not a workspace
/// dependency); length first so `all()` is only reached for plausible input.
fn valid_icao24(hex: &str) -> bool {
    hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// adsb.lol accepts 6- or 7-hex-digit traces plus readsb's `~`-prefixed
/// non-ICAO addresses (`~abcdef`).
fn valid_trace_hex(hex: &str) -> bool {
    match hex.strip_prefix('~') {
        Some(rest) => rest.len() == 6 && rest.bytes().all(|b| b.is_ascii_hexdigit()),
        None => (hex.len() == 6 || hex.len() == 7) && hex.bytes().all(|b| b.is_ascii_hexdigit()),
    }
}

/// `GET /api/opensky-track?icao24=<hex6>` — cache → cooldown → OAuth → fetch.
pub async fn handle_opensky_track(query: &str, svc: &TracksService) -> Response {
    let params = parse_query(query);
    let icao24 = params
        .get("icao24")
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !valid_icao24(&icao24) {
        return json_error(StatusCode::BAD_REQUEST, "icao24 must be 6-char hex");
    }

    // 1. Fresh cache hit — the common path once anyone has looked at this hex.
    let ttl_ms = svc.opensky.adaptive_ttl_ms.load(Ordering::Relaxed);
    if let Some((body, at)) = fresh_cached(&svc.cache, &icao24, ttl_ms).await {
        let age = crate::gev_enrichment::now_ms().saturating_sub(at);
        return track_response("HIT", Some(age), &body);
    }

    // 2. 429 cooldown: serve this key's last known body, else be honest.
    let now = crate::gev_enrichment::now_ms();
    if now < svc.opensky.cooldown_until.load(Ordering::Relaxed) {
        if let Some(entry) = svc.cache.lock().await.get(&icao24).cloned() {
            return track_response(
                "STALE",
                Some(now.saturating_sub(entry.at)),
                &entry.body,
            );
        }
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "opensky upstream cooling down",
        );
    }

    // 3. OAuth token. Missing credentials are a 503, never a silent skip —
    //    the vendor treats 503 as "fall back to the local trail", so the user
    //    sees a track; a 200 with no data would look like a working upstream.
    let token = match svc.opensky.get_token().await {
        Ok(Some(token)) => token,
        Ok(None) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "OPENSKY_CLIENT_ID/SECRET not configured",
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "opensky auth failed");
            return json_error(StatusCode::SERVICE_UNAVAILABLE, "opensky auth failed");
        }
    };

    // 4. Upstream fetch.
    let res = match svc
        .opensky
        .http
        .get(svc.opensky.track_url(&icao24))
        .bearer_auth(&token)
        .timeout(Duration::from_millis(UPSTREAM_TIMEOUT_MS))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e.without_url(), icao24 = %icao24, "opensky track fetch failed");
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "opensky upstream unavailable",
            );
        }
    };

    // Adaptive TTL is read even from error responses: a 429 carries
    // `Remaining: 0`, which is exactly when the long tier matters.
    let remaining = res
        .headers()
        .get("X-Rate-Limit-Remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    svc.opensky
        .adaptive_ttl_ms
        .store(adaptive_ttl(remaining), Ordering::Relaxed);

    let status = res.status();
    if status.as_u16() == 429 {
        let retry_after = res
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| parse_retry_after_secs(v, now))
            .unwrap_or(30_000);
        let cooldown = retry_after.clamp(COOLDOWN_MIN_MS, COOLDOWN_MAX_MS);
        svc.opensky
            .cooldown_until
            .store(now.saturating_add(cooldown), Ordering::Relaxed);
        tracing::warn!(icao24 = %icao24, cooldown_ms = cooldown, "opensky 429 — cooldown armed");
        if let Some(entry) = svc.cache.lock().await.get(&icao24).cloned() {
            return track_response(
                "STALE",
                Some(crate::gev_enrichment::now_ms().saturating_sub(entry.at)),
                &entry.body,
            );
        }
        return json_error(StatusCode::TOO_MANY_REQUESTS, "rate limited");
    }

    if !status.is_success() {
        // 404 (hex not currently tracked) and 5xx pass through; the vendor
        // silently falls back to its locally accumulated trail either way.
        return json_error(
            status,
            &format!("track source HTTP {}", status.as_u16()),
        );
    }

    // 5. Cap + cache + serve.
    let body_bytes = match res.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, icao24 = %icao24, "opensky track body read failed");
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "opensky upstream unavailable",
            );
        }
    };
    if body_bytes.len() > RESPONSE_CAP_BYTES {
        tracing::warn!(icao24 = %icao24, bytes = body_bytes.len(), "opensky track body over cap");
        return json_error(StatusCode::BAD_GATEWAY, "track response too large");
    }
    let body = augment_track_body(&String::from_utf8_lossy(&body_bytes));
    svc.cache_put(
        &icao24,
        CachedTrack {
            at: crate::gev_enrichment::now_ms(),
            body: body.clone(),
        },
    )
    .await;
    track_response("MISS", None, &body)
}

/// `GET /api/adsblol/trace?hex=<hex6|hex7>` — readsb trace long-tail
/// fallback. No OAuth, no upstream rate-limit header, fixed 60s TTL.
pub async fn handle_adsblol_trace(query: &str, svc: &TracksService) -> Response {
    let params = parse_query(query);
    let hex = params
        .get("hex")
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !valid_trace_hex(&hex) {
        return json_error(StatusCode::BAD_REQUEST, "hex must be 6-7 char");
    }

    if let Some((body, at)) = fresh_cached(&svc.adsblol_cache, &hex, TRACK_CACHE_MS).await {
        let age = crate::gev_enrichment::now_ms().saturating_sub(at);
        return track_response("HIT", Some(age), &body);
    }

    // tar1090 shards traces by the last two hex digits.
    let suffix = &hex[hex.len() - 2..];
    let url = format!(
        "{}/data/traces/{}/trace_full_{}.json",
        svc.adsblol_base_url, suffix, hex
    );
    let res = match svc
        .http
        .get(&url)
        .header(header::USER_AGENT, "IntelHub/dev")
        .timeout(Duration::from_millis(UPSTREAM_TIMEOUT_MS))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e.without_url(), hex = %hex, "adsblol trace fetch failed");
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "adsblol upstream unavailable",
            );
        }
    };
    let status = res.status();
    if !status.is_success() {
        return json_error(status, &format!("adsblol trace HTTP {}", status.as_u16()));
    }
    let body_bytes = match res.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, hex = %hex, "adsblol trace body read failed");
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "adsblol upstream unavailable",
            );
        }
    };
    if body_bytes.len() > RESPONSE_CAP_BYTES {
        tracing::warn!(hex = %hex, bytes = body_bytes.len(), "adsblol trace body over cap");
        return json_error(StatusCode::BAD_GATEWAY, "trace response too large");
    }
    let body = augment_track_body(&String::from_utf8_lossy(&body_bytes));
    put_capped(
        &svc.adsblol_cache,
        &hex,
        CachedTrack {
            at: crate::gev_enrichment::now_ms(),
            body: body.clone(),
        },
    )
    .await;
    track_response("MISS", None, &body)
}

/// Dispatcher shared by the two axum handlers and directly testable with a
/// plain `"/api/opensky-track?icao24=..."` string.
pub async fn handle_route(path: &str, svc: &TracksService) -> Response {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    match route {
        "/api/opensky-track" => handle_opensky_track(query, svc).await,
        "/api/adsblol/trace" => handle_adsblol_trace(query, svc).await,
        _ => json_error(StatusCode::NOT_FOUND, "unknown tracks endpoint"),
    }
}

fn query_path(base: &str, query: Option<&str>) -> String {
    match query {
        Some(q) if !q.is_empty() => format!("{base}?{q}"),
        _ => base.to_string(),
    }
}

/// `GET /api/opensky-track` (vendor path literal — do not rename).
pub async fn gev_opensky_track(
    State(state): State<Arc<AppState>>,
    RawQuery(query): RawQuery,
) -> Response {
    handle_route(
        &query_path("/api/opensky-track", query.as_deref()),
        &state.tracks,
    )
    .await
}

/// `GET /api/adsblol/trace` (vendor path literal — do not rename).
pub async fn gev_adsblol_trace(
    State(state): State<Arc<AppState>>,
    RawQuery(query): RawQuery,
) -> Response {
    handle_route(
        &query_path("/api/adsblol/trace", query.as_deref()),
        &state.tracks,
    )
    .await
}

/// Boot hook (called from `server.rs`). Nothing to load — both caches are
/// in-memory and self-warming — but the configuration is logged so an
/// operator can see whether the OpenSky OAuth credentials that gate
/// `/api/opensky-track` are present, and which upstreams are in play.
pub async fn start(state: AppState) -> Result<(), HubError> {
    tracing::info!(
        target: "hub.boot",
        opensky_credentials = state.tracks.opensky.has_credentials(),
        opensky_upstream = %state.tracks.opensky.api_base,
        adsblol_upstream = %state.tracks.adsblol_base_url,
        "tracks proxy ready"
    );
    Ok(())
}
