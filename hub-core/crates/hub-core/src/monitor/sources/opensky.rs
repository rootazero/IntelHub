//! OpenSky air-activity: keyless hotspot summaries (existing P1 behavior,
//! unchanged without credentials) + GEV P2 OAuth full-vector layer
//! (env-gated, Ruling 3/4).
//!
//! Keyless channel (no `HUB_OPENSKY_CLIENT_ID`/`_SECRET`): 10 hotspot region
//! summaries (anonymous ~400 credits/day per current docs; 10 regions ×
//! 4/hour ≈ 960 — the anonymous bucket is shared per exit IP and upstream
//! 429s are handled by the existing break-and-degrade loop below).
//!
//! OAuth full-vector channel (credentials present): every tick also fetches
//! `GET /api/states/all` (whole world, 4 credits/call per current docs;
//! 300s cycle ⇒ 288 calls ⇒ 1152 credits/day < 4000 standard-user bucket),
//! parses the fixed state-vector arrays into `AdsbPoint`s, and writes an
//! independent snapshot at `hub:globe:aircraft:opensky` (TTL 120s, Ruling 3
//! — never overwrites adsb's `hub:globe:aircraft`). The REST layer merges
//! the two snapshots by hex at read time (`adsb::merge_globe_snapshots`,
//! fresher `age_s` wins; adsb wins when no age info). The keyless hotspot
//! loop keeps its current 900s cadence by running every 3rd tick.

use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::error::Result;

use super::super::{Ctx, Signal, Source};
use super::adsb::{self, AdsbPoint};

/// (lamin, lomin, lamax, lomax, label) — Crucix OPENSKY HOTSPOTS verbatim.
pub(crate) const HOTSPOTS: &[(f64, f64, f64, f64, &str)] = &[
    (12.0, 30.0, 42.0, 65.0, "Middle East"),
    (20.0, 115.0, 28.0, 125.0, "Taiwan Strait"),
    (44.0, 22.0, 53.0, 41.0, "Ukraine Region"),
    (53.0, 19.0, 60.0, 29.0, "Baltic Region"),
    (5.0, 105.0, 23.0, 122.0, "South China Sea"),
    (33.0, 124.0, 43.0, 132.0, "Korean Peninsula"),
    (18.0, -90.0, 30.0, -72.0, "Caribbean"),
    (-2.0, -5.0, 8.0, 10.0, "Gulf of Guinea"),
    (-38.0, 12.0, -28.0, 24.0, "Cape Route"),
    (5.0, 40.0, 15.0, 55.0, "Horn of Africa"),
];

const HIGH_ALT_M: f64 = 12000.0;

/// Ruling 3: OpenSky full snapshot lives at its OWN key — adsb's
/// `hub:globe:aircraft` (TTL 300s) is never touched by this collector.
pub(crate) const OPENSKY_AIRCRAFT_KEY: &str = "hub:globe:aircraft:opensky";
/// Ruling 3: 120s TTL. The full-vector tick runs every 300s; a slightly
/// early expiry just means the REST merge briefly falls back to adsb-only.
const OPENSKY_SNAPSHOT_TTL_SECS: usize = 120;
/// Ruling: 300s cycle = 288 ticks/day ⇒ 1152 credits/day < 4000 standard.
const FULL_TICK_SECS: u64 = 300;
/// With credentials the source ticks at 300s, but the keyless hotspot loop
/// keeps its historical 900s cadence by running every 3rd tick.
const HOTSPOT_EVERY: u64 = 3;

const TOKEN_URL: &str =
    "https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token";
const STATES_URL: &str = "https://opensky-network.org/api/states/all";
/// OAuth token cache: JSON {"token","fetched_at","expires_in"} — same shape
/// the controller ruling describes for collector_states.value, held in Redis
/// (no collector_states table exists in this codebase; Redis is the existing
/// cross-restart store for monitor state, cf. sweephist ring).
const TOKEN_KEY: &str = "hub:monitor:opensky:token";
/// Daily 429/degrade bookkeeping: `hub:monitor:opensky:state_429:{YYYYMMDD}`
/// holds JSON {"ticks": n, "count_429": n}, TTL 48h.
const RL_PREFIX: &str = "hub:monitor:opensky:state_429:";
/// Ruling: treat the token as expired 60s before its nominal expiry
/// (expires_in from upstream is ~1800s ⇒ ~29min effective cache lifetime).
const TOKEN_MARGIN_SECS: i64 = 60;
/// OpenSky velocity is m/s; AdsbPoint.gs is knots (adsb.lol semantics).
const MS_TO_KNOTS: f64 = 1.94384;

/// OAuth client-credentials pair, or None when unconfigured. Both the
/// `HUB_`-prefixed and bare names are accepted (brief says HUB_*; ruling says
/// bare) — HUB_* wins when both are set.
fn oauth_creds() -> Option<(String, String)> {
    let id = std::env::var("HUB_OPENSKY_CLIENT_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("OPENSKY_CLIENT_ID").ok().filter(|s| !s.is_empty()))?;
    let secret = std::env::var("HUB_OPENSKY_CLIENT_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("OPENSKY_CLIENT_SECRET").ok().filter(|s| !s.is_empty()))?;
    Some((id, secret))
}

/// Pure: is a token cached at `fetched_at` with upstream `expires_in` still
/// usable at `now`? 60s early-expiry margin (ruling), so a token never dies
/// mid-flight because the cache outlived it by clock skew.
pub(crate) fn token_cache_valid(fetched_at: i64, expires_in: i64, now: i64) -> bool {
    now < fetched_at + expires_in - TOKEN_MARGIN_SECS
}

/// Pure: degrade the full-vector layer to keyless for this cycle once the
/// daily 429 share exceeds 30% of ticks (strictly greater; exactly 30% does
/// not degrade).
pub(crate) fn should_degrade_keyless(ticks: u64, count_429: u64) -> bool {
    ticks > 0 && count_429.saturating_mul(10) > ticks.saturating_mul(3)
}

/// Pure: parse OpenSky `states` (fixed-position arrays, 17+ fields) into
/// `AdsbPoint`s. Field order per the REST docs: [0]icao24 [1]callsign
/// [2]origin_country [3]time_position [4]last_contact [5]lon [6]lat
/// [7]baro_altitude(m) [8]on_ground [9]velocity(m/s) [10]true_track
/// [11]vertical_rate [12]sensors [13]geo_altitude [14]squawk ...
///
/// Semantics aligned with the adsb snapshot: `on_ground` rows are dropped;
/// rows without lon/lat or without any usable timestamp (time_position,
/// falling back to last_contact) are dropped — freshness is the merge key,
/// a row with no time can never win. `seen` is seconds-before-`now`
/// (AdsbPoint convention). Military marking is an adsb.lol concept; OpenSky
/// rows are always `mil = false`.
pub(crate) fn parse_state_vectors(j: &serde_json::Value, now: i64) -> Vec<AdsbPoint> {
    let Some(states) = j.get("states").and_then(|s| s.as_array()) else {
        return Vec::new();
    };
    states
        .iter()
        .filter_map(|v| {
            let arr = v.as_array()?;
            let hex = arr.get(0).and_then(|x| x.as_str())?.to_lowercase();
            let flight = arr
                .get(1)
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            // Ground rows are out of snapshot scope (brief, adsb parity).
            if arr.get(8).and_then(|x| x.as_bool()).unwrap_or(false) {
                return None;
            }
            let lon = arr.get(5).and_then(|x| x.as_f64())?;
            let lat = arr.get(6).and_then(|x| x.as_f64())?;
            let tpos = arr
                .get(3)
                .and_then(|x| x.as_i64())
                .or_else(|| arr.get(4).and_then(|x| x.as_i64()))?;
            Some(AdsbPoint {
                hex,
                flight,
                lat,
                lon,
                alt_m: arr.get(7).and_then(|x| x.as_f64()).unwrap_or(0.0),
                gs: arr.get(9).and_then(|x| x.as_f64()).map(|v| v * MS_TO_KNOTS),
                track: arr.get(10).and_then(|x| x.as_f64()),
                squawk: arr.get(14).and_then(|x| x.as_str()).map(str::to_string),
                mil: false,
                seen: (now - tpos).max(0) as f64,
            })
        })
        .collect()
}

pub struct OpenSky {
    /// Tick counter driving the every-3rd-tick keyless hotspot cadence when
    /// OAuth credentials are configured.
    ticks: Mutex<u64>,
    /// Log the "no creds, full-vector shelved" notice once per process.
    no_creds_logged: AtomicBool,
}

impl Default for OpenSky {
    fn default() -> Self {
        Self { ticks: Mutex::new(0), no_creds_logged: AtomicBool::new(false) }
    }
}

impl OpenSky {
    /// Ruling 4: credentials switch the source to the 300s full-vector
    /// cycle; without them the historical 900s keyless cadence is untouched.
    fn interval() -> Duration {
        if oauth_creds().is_some() {
            Duration::from_secs(FULL_TICK_SECS)
        } else {
            Duration::from_secs(900)
        }
    }

    /// OAuth full-vector channel. NEVER returns Err: failures here (token
    /// endpoint down, 401, 429, bad JSON) must not poison the health cell or
    /// lose the keyless hotspot signals already collected — they log and
    /// skip the full-vector part for this cycle (ruling: degrade = keyless
    /// continues).
    async fn full_vectors(&self, ctx: &Ctx, id: &str, secret: &str) {
        let now = chrono::Utc::now().timestamp();
        let day = chrono::Utc::now().format("%Y%m%d").to_string();
        let rl_key = format!("{RL_PREFIX}{day}");

        // --- daily 429 bookkeeping + 30% degrade gate ---------------------
        let mut rl: serde_json::Value = ctx
            .state
            .redis_timed(redis::cmd("GET").arg(&rl_key).clone(), 2000)
            .await
            .flatten()
            .and_then(|s: String| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({"ticks": 0, "count_429": 0}));
        let ticks = rl["ticks"].as_u64().unwrap_or(0) + 1;
        let mut count_429 = rl["count_429"].as_u64().unwrap_or(0);
        rl["ticks"] = json!(ticks);
        if should_degrade_keyless(ticks, count_429) {
            let _: Option<()> = ctx
                .state
                .redis_timed(
                    redis::cmd("SET").arg(&rl_key).arg(rl.to_string()).arg("EX").arg(172_800).clone(),
                    2000,
                )
                .await;
            tracing::warn!(ticks, count_429, "opensky: daily 429 share > 30%; full-vector layer degraded to keyless this cycle");
            return;
        }

        // --- bearer token (cached in Redis, 60s early expiry) -------------
        let mut token = match self.bearer_token(ctx, id, secret, now).await {
            Some(t) => t,
            None => {
                tracing::warn!("opensky: token fetch failed; skipping full-vector this cycle");
                return;
            }
        };

        // --- states/all (401 → clear cache, re-auth once, retry once) -----
        let mut resp = match ctx.http.get(STATES_URL).bearer_auth(&token).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "opensky states/all transport failed");
                return;
            }
        };
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            let _: Option<()> = ctx
                .state
                .redis_timed(redis::cmd("DEL").arg(TOKEN_KEY).clone(), 2000)
                .await;
            match self.bearer_token(ctx, id, secret, now).await {
                Some(t) => token = t,
                None => return,
            }
            resp = match ctx.http.get(STATES_URL).bearer_auth(&token).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "opensky states/all retry transport failed");
                    return;
                }
            };
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                tracing::warn!("opensky states/all still 401 after token refresh; skipping full-vector this cycle");
                return;
            }
        }
        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            count_429 += 1;
            rl["count_429"] = json!(count_429);
            let _: Option<()> = ctx
                .state
                .redis_timed(
                    redis::cmd("SET").arg(&rl_key).arg(rl.to_string()).arg("EX").arg(172_800).clone(),
                    2000,
                )
                .await;
            let retry_after = resp
                .headers()
                .get("x-rate-limit-retry-after-seconds")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("?");
            tracing::warn!(retry_after, count_429, ticks, "opensky states/all 429");
            return;
        }
        if !status.is_success() {
            tracing::warn!(status = %status, "opensky states/all non-2xx; skipping full-vector this cycle");
            return;
        }
        // Persist the tick counter on the success path too, so the 30% ratio
        // stays honest across the day.
        let _: Option<()> = ctx
            .state
            .redis_timed(
                redis::cmd("SET").arg(&rl_key).arg(rl.to_string()).arg("EX").arg(172_800).clone(),
                2000,
            )
            .await;

        let Ok(j) = resp.json::<serde_json::Value>().await else {
            tracing::warn!("opensky states/all bad json body");
            return;
        };
        let rows: Vec<(AdsbPoint, i64)> = parse_state_vectors(&j, now)
            .into_iter()
            .map(|a| {
                let seen_abs = now - a.seen.round() as i64;
                (a, seen_abs)
            })
            .collect();
        let env = adsb::snapshot_envelope(&rows, now, "full", "opensky", FULL_TICK_SECS as i64);
        // Redis write failure is visible but not fatal — next tick rewrites.
        let wrote: Option<String> = ctx
            .state
            .redis_timed(
                redis::cmd("SETEX").arg(OPENSKY_AIRCRAFT_KEY).arg(OPENSKY_SNAPSHOT_TTL_SECS).arg(env.to_string()).clone(),
                2000,
            )
            .await;
        if wrote.is_none() {
            tracing::warn!("opensky snapshot write failed (redis down?)");
        }
    }

    /// Cached OAuth client-credentials token. Cache lives in Redis as JSON
    /// {"token","fetched_at","expires_in"}; expiry check via the pure
    /// `token_cache_valid` (60s margin). Returns None on any failure —
    /// callers skip the cycle.
    async fn bearer_token(&self, ctx: &Ctx, id: &str, secret: &str, now: i64) -> Option<String> {
        let cached = ctx
            .state
            .redis_timed(redis::cmd("GET").arg(TOKEN_KEY).clone(), 2000)
            .await;
        if let Some(v) = cached.flatten().and_then(|s: String| serde_json::from_str::<serde_json::Value>(&s).ok()) {
            let fetched_at = v.get("fetched_at").and_then(|x| x.as_i64());
            let expires_in = v.get("expires_in").and_then(|x| x.as_i64());
            if let (Some(fa), Some(ei)) = (fetched_at, expires_in) {
                if token_cache_valid(fa, ei, now) {
                    return v.get("token").and_then(|x| x.as_str()).map(str::to_string);
                }
            }
        }
        // Hand-written form body (grant_type=client_credentials) — zero extra
        // reqwest features, zero new crates (ruling).
        let resp = match ctx
            .http
            .post(TOKEN_URL)
            .basic_auth(id, Some(secret))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("grant_type=client_credentials")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "opensky token endpoint transport failed");
                return None;
            }
        };
        if !resp.status().is_success() {
            tracing::warn!(status = %resp.status(), "opensky token endpoint non-2xx");
            return None;
        }
        let j = match resp.json::<serde_json::Value>().await {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "opensky token endpoint bad json");
                return None;
            }
        };
        let token = j.get("access_token").and_then(|x| x.as_str())?.to_string();
        let expires_in = j.get("expires_in").and_then(|x| x.as_i64()).unwrap_or(1800);
        let cache = json!({"token": token, "fetched_at": now, "expires_in": expires_in});
        let _: Option<()> = ctx
            .state
            .redis_timed(
                redis::cmd("SET")
                    .arg(TOKEN_KEY)
                    .arg(cache.to_string())
                    .arg("EX")
                    .arg((expires_in + 60).max(300))
                    .clone(),
                2000,
            )
            .await;
        Some(token)
    }
}

impl Source for OpenSky {
    fn name(&self) -> &'static str {
        "opensky"
    }
    fn interval(&self) -> Duration {
        Self::interval()
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let creds = oauth_creds();
            if creds.is_none() && !self.no_creds_logged.swap(true, Ordering::SeqCst) {
                tracing::info!(
                    "opensky: no OAuth creds (HUB_OPENSKY_CLIENT_ID/SECRET) — full-vector layer shelved, keyless hotspot mode unchanged"
                );
            }
            // Hotspot cadence: every tick without creds (900s, historical);
            // every 3rd tick with creds (300s × 3 = 900s, same cadence).
            let tick = {
                let mut t = self.ticks.lock().unwrap_or_else(|p| p.into_inner());
                *t += 1;
                *t
            };
            let run_hotspots = creds.is_none() || tick % HOTSPOT_EVERY == 1;

            let hour_bucket = chrono::Utc::now().format("%Y%m%d%H").to_string();
            let mut out = Vec::new();
            if run_hotspots {
                for (lamin, lomin, lamax, lomax, label) in HOTSPOTS {
                    let url = format!(
                        "https://opensky-network.org/api/states/all?lamin={lamin}&lomin={lomin}&lamax={lamax}&lomax={lomax}"
                    );
                    let resp = match ctx.http.get(&url).send().await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(region = label, error = %e, "opensky region failed");
                            continue;
                        }
                    };
                    if resp.status().as_u16() == 429 || resp.status().as_u16() == 403 {
                        // anonymous quota exhausted — degrade the whole source till next tick
                        tracing::warn!(status = %resp.status(), "opensky rate-limited; backing off");
                        break;
                    }
                    if !resp.status().is_success() {
                        continue;
                    }
                    let Ok(j) = resp.json::<serde_json::Value>().await else {
                        continue;
                    };
                    let Some(states) = j.get("states").and_then(|s| s.as_array()) else {
                        continue;
                    };
                    let total = states.len();
                    let high_alt = states
                        .iter()
                        .filter(|s| {
                            s.get(7).and_then(|v| v.as_f64()).unwrap_or(0.0) > HIGH_ALT_M
                        })
                        .count();
                    let no_callsign = states
                        .iter()
                        .filter(|s| {
                            s.get(1)
                                .and_then(|v| v.as_str())
                                .map(|c| c.trim().is_empty())
                                .unwrap_or(true)
                        })
                        .count();
                    if total == 0 {
                        continue;
                    }
                    let (clat, clon) = ((lamin + lamax) / 2.0, (lomin + lomax) / 2.0);
                    out.push(
                        Signal::new(
                            "flight",
                            format!("{label}: {total} aircraft ({high_alt} high-altitude, {no_callsign} dark)"),
                            clat,
                            clon,
                            format!("{label}:{hour_bucket}"),
                        )
                        .severity(if high_alt >= 5 { "routine" } else { "info" })
                        .payload(serde_json::json!({
                            "region": label, "total": total, "high_altitude": high_alt,
                            "no_callsign": no_callsign,
                        })),
                    );
                }
            }
            // Full-vector channel (ruling 4: appended inside the source when
            // creds exist; keyless behavior above is untouched either way).
            if let Some((id, secret)) = creds {
                self.full_vectors(ctx, &id, &secret).await;
            }
            Ok(out)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 17-field state vector in documented field order.
    fn sv(icao: &str, callsign: &str, tpos: i64, lon: f64, lat: f64, baro: f64, on_ground: bool, vel_ms: f64, track: f64, squawk: Option<&str>) -> serde_json::Value {
        json!([
            icao, callsign, "United States", tpos, tpos + 2, lon, lat, baro,
            on_ground, vel_ms, track, 0.0, null, baro + 100.0, squawk, false, 0
        ])
    }

    #[test]
    fn parse_state_vectors_extracts_positions_and_converts_units() {
        let now = 1_700_000_000i64;
        let j = json!({"time": now, "states": [sv("A1B2C3", "UAL123  ", now - 12, 121.4, 31.2, 11277.0, false, 250.0, 92.0, Some("2000"))]});
        let ac = parse_state_vectors(&j, now);
        assert_eq!(ac.len(), 1);
        let a = &ac[0];
        assert_eq!(a.hex, "a1b2c3", "icao24 normalized to lowercase for hex-merge dedupe");
        assert_eq!(a.flight.as_deref(), Some("UAL123"), "callsign trimmed");
        assert!((a.lat - 31.2).abs() < 1e-9 && (a.lon - 121.4).abs() < 1e-9);
        assert!((a.alt_m - 11277.0).abs() < 1e-9, "baro_altitude already metres");
        assert!((a.gs.unwrap() - 250.0 * 1.94384).abs() < 0.01, "velocity m/s → knots");
        assert!((a.track.unwrap() - 92.0).abs() < 1e-9);
        assert_eq!(a.squawk.as_deref(), Some("2000"));
        assert!(!a.mil, "opensky rows are never mil");
        assert!((a.seen - 12.0).abs() < 1e-9, "seen = now - time_position");
    }

    #[test]
    fn parse_state_vectors_drops_ground_unpositioned_and_timeless_rows() {
        let now = 1_700_000_000i64;
        let mut no_time = sv("EEEEEE", "X", 0, 10.0, 10.0, 1000.0, false, 0.0, 0.0, None);
        no_time[3] = serde_json::Value::Null;
        no_time[4] = serde_json::Value::Null;
        let mut no_lon = sv("FFFFFF", "X", now, 10.0, 10.0, 1000.0, false, 0.0, 0.0, None);
        no_lon[5] = serde_json::Value::Null;
        let mut no_baro = sv("BBBBBB", "KEEP", now - 5, 10.0, 10.0, 0.0, false, 0.0, 0.0, None);
        no_baro[7] = serde_json::Value::Null;
        let j = json!({"states": [
            sv("AAAAAA", "OK", now - 5, 10.0, 10.0, 1000.0, true, 0.0, 0.0, None),   // on_ground → drop
            no_lon,                                                                  // no position → drop
            no_time,                                                                 // no timestamp → drop
            no_baro                                                                  // null baro → 0, kept
        ]});
        let ac = parse_state_vectors(&j, now);
        assert_eq!(ac.len(), 1);
        assert_eq!(ac[0].hex, "bbbbbb");
        assert_eq!(ac[0].alt_m, 0.0);
    }

    #[test]
    fn parse_state_vectors_falls_back_to_last_contact() {
        let now = 1_700_000_000i64;
        let mut v = sv("CCCCCC", "X", 0, 10.0, 10.0, 1000.0, false, 0.0, 0.0, None);
        v[3] = serde_json::Value::Null;
        v[4] = json!(now - 30);
        let ac = parse_state_vectors(&json!({"states": [v]}), now);
        assert_eq!(ac.len(), 1);
        assert!((ac[0].seen - 30.0).abs() < 1e-9, "last_contact fallback");
    }

    #[test]
    fn parse_state_vectors_handles_18th_category_field() {
        // Newer API appends an 18th field (category) — index-based parsing is unaffected.
        let now = 1_700_000_000i64;
        let mut v = sv("DDDDDD", "X", now - 1, 10.0, 10.0, 1000.0, false, 0.0, 0.0, None);
        v.as_array_mut().unwrap().push(json!(3));
        let ac = parse_state_vectors(&json!({"states": [v]}), now);
        assert_eq!(ac.len(), 1);
    }

    #[test]
    fn token_cache_valid_applies_sixty_second_margin() {
        let fetched_at = 1_000_000i64;
        let expires_in = 1800i64;
        // Well inside the window.
        assert!(token_cache_valid(fetched_at, expires_in, fetched_at + 900));
        // Inside the 60s early-expiry margin → treated as expired.
        assert!(!token_cache_valid(fetched_at, expires_in, fetched_at + expires_in - 30));
        // Past nominal expiry.
        assert!(!token_cache_valid(fetched_at, expires_in, fetched_at + expires_in + 1));
        // Just outside the margin → still valid.
        assert!(token_cache_valid(fetched_at, expires_in, fetched_at + expires_in - 61));
    }

    #[test]
    fn should_degrade_keyless_triggers_above_thirty_percent() {
        // Exactly 30% does not degrade (strictly-greater rule).
        assert!(!should_degrade_keyless(10, 3));
        assert!(!should_degrade_keyless(0, 0), "no ticks → nothing to ratio");
        assert!(!should_degrade_keyless(100, 0));
        // One tick over 30% degrades.
        assert!(should_degrade_keyless(10, 4));
        assert!(should_degrade_keyless(3, 1));
        assert!(should_degrade_keyless(288, 87), "daily-scale: 87/288 = 30.2%");
        assert!(!should_degrade_keyless(288, 86), "86/288 = 29.9%");
    }

    #[test]
    fn opensky_envelope_uses_own_coverage_and_cycle() {
        let now = 1_700_000_000i64;
        let ac = AdsbPoint {
            hex: "abc".into(),
            flight: Some("OSK1".into()),
            lat: 1.0,
            lon: 2.0,
            alt_m: 100.0,
            gs: Some(300.0),
            track: None,
            squawk: None,
            mil: false,
            seen: 20.0,
        };
        let env = adsb::snapshot_envelope(&[(ac, now - 20)], now, "full", "opensky", FULL_TICK_SECS as i64);
        assert_eq!(env["coverage"], "opensky");
        assert_eq!(env["cycle_secs"], 300);
        assert_eq!(env["last_tick"], "full");
        assert_eq!(env["count"], 1);
        assert_eq!(env["aircraft"][0]["age_s"], 20);
        assert_eq!(env["aircraft"][0]["mil"], false);
    }
}
