//! P13 T4: 3rd live-aircraft source for the GEV flights layer.
//!
//! **Coverage gap.** T1's adsb.lol collector rotates through ten
//! conflict/OSINT hotspots (`opensky::HOTSPOTS`) — none of them in North
//! America — so the globe has no US feed at all. This collector polls the
//! same keyless adsb.lol `point` endpoint around six US hubs and publishes a
//! THIRD snapshot at `hub:globe:aircraft:adsbx` (TTL 300s), merged at read
//! time by `adsb::merge_globe_snapshots` (priority adsb > adsbx > opensky).
//!
//! **Naming.** The module is `adsbexchange`, per the P13 plan §Task 4 file
//! contract, but the plan's own strategy decision is "extend adsb.lol poll
//! radius": a real ADSB Exchange RapidAPI key would be shelved-by-design in
//! this deployment (no key configured, no new external dependency). So the
//! upstream here is adsb.lol, not adsbexchange.com — the module name is the
//! plan's slot name, not the upstream's.
//!
//! **Rate discipline** (carry-forward of the T1 controller ruling). Measured
//! upstream quota is roughly one token per 20–30s after a burst of 4–8, and
//! the egress IP is shared with the adsb rotation plus OpenSky. Six requests
//! fired back-to-back trip 429s — probed 2026-09-21 from the test VM: three
//! rapid `point` calls returned empty 429 bodies. So the hub sweep is paced
//! through the SHARED `Ctx::limiter` token bucket (`ADSBX_HUB_GAP` = 25s):
//! six requests spread over ~125s, then ~175s idle at the 300s interval,
//! i.e. ~1.2 req/min on top of the rotation's ~4 req/min.
//!
//! **Freshness.** The vendor coasts an aircraft at most 300s past its newest
//! fix (`vendor motion.js::staleCoastLimitSeconds`, `maximumSec: 300`), so
//! the snapshot is a FULL REWRITE every tick — never cumulative — and rows
//! with `seen > ADSBX_MAX_SEEN_SECS` are dropped. A cumulative snapshot
//! (T1's `apply_tick` shape) was rejected for this source: at a 300s tick the
//! rotation would need 30min to re-visit a hub, so most rows would carry
//! `age_s` far beyond the vendor's coast cap and be invisible anyway.
//!
//! Snapshot-only: no geo `Signal`s (celestrak/cctv precedent) — this feeds a
//! map layer, not the event pipeline.

use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::Value;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::limiter::RateLimiter;
use super::super::{Ctx, Signal, Source};
use super::adsb::{self, AdsbPoint};

/// Third snapshot key. T1 owns `hub:globe:aircraft` (rotation, TTL 300s) and
/// OpenSky owns `hub:globe:aircraft:opensky` (TTL 120s); this collector
/// overwrites neither (Ruling 3 pattern).
pub const ADSBX_AIRCRAFT_KEY: &str = "hub:globe:aircraft:adsbx";
/// Snapshot TTL = one interval. The sweep rewrites the whole envelope every
/// tick, so a single missed tick still leaves a valid (if 5min old) snapshot.
pub const ADSBX_TTL_SECS: usize = 300;
/// Poll cadence, aligned with the vendor's 300s coast cap.
pub const ADSBX_INTERVAL_SECS: u64 = 300;
/// Drop a row whose freshest transponder message is older than this. The
/// vendor's coast cap is 300s, but a row already 60s+ cold at poll time is
/// off the live picture and would only inflate `count`.
pub const ADSBX_MAX_SEEN_SECS: f64 = 60.0;
/// Inter-request gap inside one sweep, applied through the shared per-host
/// limiter (`RateLimiter::wait`) so the adsb rotation and this sweep together
/// stay inside the measured upstream refill rate.
pub const ADSBX_HUB_GAP: Duration = Duration::from_secs(25);
/// `point`-query radius in nautical miles (readsb contract, max 250).
pub const ADSBX_RADIUS_NM: u32 = 50;

/// (label, lat, lon) — eight US hubs chosen to fill the coverage the
/// hotspot-only rotation leaves on the table (plan §Task 4 verbatim list),
/// extended with West Coast hubs (LAX, SFO) so California's two largest
/// metros (LA basin + SF Bay, ~18M population, top-10 US pax airports) sit
/// inside a 50 nm capture radius. The previous six-hub set stopped at PHX
/// (lon -112) — 50 nm east of LA — so every LAX/SFO/SAN/SJC transponder
/// was invisible to the snapshot.
///
/// Rate impact: 8 hubs × 25 s gap = 200 s sweep, still under the 300 s
/// interval cap; combined with the rotation's ~4 req/min the measured
/// adsb.lol quota is ~5.6 req/min (was 5.2). The shared per-host limiter
/// (`Ctx::limiter`) absorbs the extra two slots without 429 risk — the
/// 25 s gap was calibrated for the worst-case burst, not the average.
pub const ADSBX_HUBS: &[(&str, f64, f64)] = &[
    ("ATL", 33.6407, -84.4277),
    ("JFK", 40.6413, -73.7781),
    ("ORD", 41.9742, -87.9073),
    ("DFW", 32.8998, -97.0403),
    ("DEN", 39.8561, -104.6737),
    ("PHX", 33.4342, -112.0080),
    ("LAX", 33.9425, -118.4081),
    ("SFO", 37.6189, -122.3750),
];

/// adsb.lol API base. `HUB_ADSBX_BASE_URL` lets tests point at a wiremock
/// instance instead of the public API (adsbdb precedent,
/// `HUB_ADSBDB_BASE_URL`).
pub fn adsbx_base_url() -> String {
    std::env::var("HUB_ADSBX_BASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://api.adsb.lol".to_string())
}

/// `{base}/v2/point/{lat}/{lon}/{radius_nm}` — readsb point query.
pub fn hub_url(base_url: &str, lat: f64, lon: f64) -> String {
    format!(
        "{}/v2/point/{:.4}/{:.4}/{}",
        base_url.trim_end_matches('/'),
        lat,
        lon,
        ADSBX_RADIUS_NM
    )
}

/// Hand-rolled ICAO-24 validator (no `regex` crate): exactly six ASCII hex
/// digits. Rows that fail are dropped before the snapshot — they can never be
/// enriched or tracked downstream (same rule as `gev_enrichment.rs` /
/// `gev_tracks.rs`).
pub fn is_icao_hex(s: &str) -> bool {
    s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Normalise one adsb.lol `{"ac":[...]}` body into points, dropping rows whose
/// hex is not a valid ICAO-24. Reuses T1's `parse_ac_array`, so row semantics
/// (ft→m altitude, `seen` = f64::MAX when absent, `flight` trimmed) stay
/// single-sourced with the rotation collector.
pub fn parse_adsbx_response(j: &Value) -> Vec<AdsbPoint> {
    adsb::parse_ac_array(j)
        .into_iter()
        .filter(|a| is_icao_hex(&a.hex))
        .collect()
}

/// Drop rows whose last transponder message is older than
/// `ADSBX_MAX_SEEN_SECS`. A row with no upstream `seen` parses to f64::MAX
/// (T1 semantics) and is dropped here: an unaged fix cannot be trusted live.
pub fn filter_stale(points: Vec<AdsbPoint>) -> Vec<AdsbPoint> {
    points
        .into_iter()
        .filter(|a| a.seen <= ADSBX_MAX_SEEN_SECS)
        .collect()
}

/// Deduplicate the hub union by hex, freshest (lowest `seen`) wins.
/// Delegates to T1's `merge_aircraft` so the tie-break and the hex-sorted
/// output ordering stay single-sourced.
pub fn dedup_merge(a: &[AdsbPoint], b: &[AdsbPoint]) -> Vec<AdsbPoint> {
    adsb::merge_aircraft(vec![a.to_vec(), b.to_vec()])
}

/// Full-rewrite envelope in T1's exact row shape (`hex/flight/lat/lon/alt_m/
/// gs/track/squawk/mil/age_s`) so the console aircraft adapter needs no new
/// mapping path.
pub fn snapshot_envelope(points: &[AdsbPoint], now: i64) -> Value {
    let rows: Vec<(AdsbPoint, i64)> = points
        .iter()
        .cloned()
        .map(|a| {
            let seen_abs = now - a.seen.round() as i64;
            (a, seen_abs)
        })
        .collect();
    adsb::snapshot_envelope(&rows, now, "us-hubs", "adsbx", ADSBX_INTERVAL_SECS as i64)
}

/// `SETEX <key> 300 <envelope>` — factored out of the fetch loop so the
/// key + TTL write contract is asserted without a live Redis.
pub fn snapshot_setex_cmd(envelope: &Value) -> redis::Cmd {
    redis::cmd("SETEX")
        .arg(ADSBX_AIRCRAFT_KEY)
        .arg(ADSBX_TTL_SECS)
        .arg(envelope.to_string())
        .clone()
}

/// Fetch one hub. `None` = failed (transport / non-2xx / non-JSON). Per-hub
/// isolation is deliberate: the upstream throttles in bursts, so one 429'd
/// hub must not cost the other five.
pub async fn fetch_hub(
    client: &reqwest::Client,
    base_url: &str,
    label: &str,
    lat: f64,
    lon: f64,
) -> Option<Vec<AdsbPoint>> {
    let url = hub_url(base_url, lat, lon);
    match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(j) => Some(parse_adsbx_response(&j)),
            Err(e) => {
                tracing::warn!(hub = label, error = %e, "adsbx upstream bad json");
                None
            }
        },
        Ok(r) => {
            tracing::warn!(hub = label, status = %r.status(), "adsbx upstream non-2xx");
            None
        }
        Err(e) => {
            tracing::warn!(hub = label, error = %e, "adsbx upstream failed");
            None
        }
    }
}

/// Sweep every US hub once. Returns `(hex-deduped fresh points, failed hubs)`.
/// `limiter`/`gap` pace the sweep through the shared per-host token bucket;
/// pass `None` to skip the sleep (tests, and any future caller that already
/// holds the host budget).
pub async fn fetch_hubs(
    client: &reqwest::Client,
    base_url: &str,
    limiter: Option<&RateLimiter>,
    gap: Duration,
) -> (Vec<AdsbPoint>, usize) {
    let mut batch: Vec<AdsbPoint> = Vec::new();
    let mut failed = 0usize;
    for (label, lat, lon) in ADSBX_HUBS {
        if let Some(l) = limiter {
            l.wait(base_url, gap).await;
        }
        match fetch_hub(client, base_url, label, *lat, *lon).await {
            Some(mut pts) => batch.append(&mut pts),
            None => failed += 1,
        }
    }
    (filter_stale(dedup_merge(&batch, &[])), failed)
}

pub struct AdsbExchange {
    base_url: String,
}

impl Default for AdsbExchange {
    fn default() -> Self {
        Self { base_url: adsbx_base_url() }
    }
}

impl AdsbExchange {
    /// Explicit base URL (wiremock tests / mirror deployment).
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into() }
    }
}

impl Source for AdsbExchange {
    fn name(&self) -> &'static str {
        "adsbx"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(ADSBX_INTERVAL_SECS)
    }

    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let now = chrono::Utc::now().timestamp();
            let (points, failed) =
                fetch_hubs(&ctx.http, &self.base_url, Some(&ctx.limiter), ADSBX_HUB_GAP).await;

            // Every hub down ⇒ Err, so the scheduler's 30s→600s backoff (the
            // 429 cooldown) fires. A partial sweep is a normal tick: the
            // healthy hubs' aircraft still land in the snapshot.
            if failed == ADSBX_HUBS.len() {
                return Err(HubError::sensor(format!(
                    "adsbx: all {} us hubs failed",
                    ADSBX_HUBS.len()
                )));
            }

            let count = points.len();
            let envelope = snapshot_envelope(&points, now);
            // Redis write failure must be visible but must NOT fail the tick:
            // the previous snapshot is still inside its 300s TTL, and backoff
            // is reserved for upstream failures.
            let wrote: Option<String> = ctx
                .state
                .redis_timed(snapshot_setex_cmd(&envelope), 2000)
                .await;
            if wrote.is_none() {
                tracing::warn!("adsbx snapshot write failed (redis down?)");
            }
            tracing::info!(
                hubs = ADSBX_HUBS.len(),
                failed,
                aircraft = count,
                "adsbx us-hub sweep"
            );
            // Snapshot-only feed: no geo Signals (celestrak/cctv precedent).
            Ok(Vec::new())
        }
        .boxed()
    }
}
