//! GEV P13 T4: 3rd live-aircraft source — expanded-radius adsb.lol US hubs.
//!
//! All upstream I/O is served by wiremock (`fetch_hubs` takes a base URL,
//! adsbdb precedent `HUB_ADSBDB_BASE_URL`), so this suite runs without
//! network or live PG/Redis/Neo4j. The Redis write contract is asserted
//! through the packed RESP command (`snapshot_setex_cmd`) — opening a real
//! Redis connection is the deploy acceptance suite's job.
//!
//! Coverage list (plan §Task 4): `parse_adsbx_response_normalises_records`,
//! `parse_empty_response_yields_empty_vec`,
//! `network_error_returns_empty_vec_no_panic`, `timestamp_zero_means_recent`,
//! `stale_records_filtered_out`, `merge_dedup_by_hex_picks_fresher_age`,
//! `merge_with_no_adsbx_keeps_existing`, `redis_writes_cached_snapshot`.

use std::time::Duration;

use hub_core::monitor::sources::adsb::{merge_globe_snapshots, AdsbPoint};
use hub_core::monitor::sources::adsbexchange::{
    dedup_merge, fetch_hubs, filter_stale, hub_url, is_icao_hex, parse_adsbx_response,
    snapshot_envelope, snapshot_setex_cmd, ADSBX_AIRCRAFT_KEY, ADSBX_HUBS, ADSBX_MAX_SEEN_SECS,
    ADSBX_RADIUS_NM, ADSBX_TTL_SECS,
};
use serde_json::{json, Value};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------- fixtures ----------

/// One adsb.lol `point` row, readsb field names (what the upstream really
/// sends). `seen` is seconds since the last transponder message.
fn ac(hex: &str, seen: f64) -> Value {
    json!({
        "hex": hex, "flight": format!("{}  ", hex.to_uppercase()),
        "lat": 33.6407, "lon": -84.4277, "alt_baro": 37000, "gs": 452.0,
        "track": 92.0, "squawk": "2000", "dbFlags": 0, "seen": seen,
    })
}

/// A full adsb.lol `point` body (shape verified live 2026-09-21).
fn body(rows: Vec<Value>) -> Value {
    json!({ "ac": rows, "msg": "No error", "now": 1_789_968_958_501i64, "total": 0, "ctime": 1_789_968_958_501i64, "ptime": 0 })
}

fn point(hex: &str, seen: f64) -> AdsbPoint {
    AdsbPoint {
        hex: hex.to_string(),
        flight: Some(hex.to_uppercase()),
        lat: 33.6407,
        lon: -84.4277,
        alt_m: 11277.6,
        gs: Some(452.0),
        track: Some(92.0),
        squawk: Some("2000".to_string()),
        mil: false,
        seen,
    }
}

/// One already-merged envelope row (the REST/merge row shape carries `age_s`).
fn row(hex: &str, age_s: i64) -> Value {
    json!({
        "hex": hex, "flight": hex.to_uppercase(), "lat": 33.6407, "lon": -84.4277,
        "alt_m": 11277, "gs": 452.0, "track": 92.0, "squawk": null, "mil": false,
        "age_s": age_s,
    })
}

fn env(coverage: &str, rows: Vec<Value>) -> Value {
    let count = rows.len();
    json!({
        "ts": "2026-09-21T00:00:00Z", "count": count, "coverage": coverage,
        "cycle_secs": 300, "last_tick": "us-hubs", "aircraft": rows,
    })
}

async fn mount(server: &MockServer, status: u16, body: Value) {
    Mock::given(method("GET"))
        .and(path_regex(r"^/v2/point/"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

// ---------- 1. parse ----------

#[test]
fn parse_adsbx_response_normalises_records() {
    let pts = parse_adsbx_response(&body(vec![ac("a1b2c3", 1.2)]));
    assert_eq!(pts.len(), 1);
    assert_eq!(pts[0].hex, "a1b2c3");
    assert_eq!(pts[0].flight.as_deref(), Some("A1B2C3"), "trailing pad trimmed");
    assert!((pts[0].alt_m - 11277.6).abs() < 0.5, "ft->m");
    assert_eq!(pts[0].gs, Some(452.0));
    assert_eq!(pts[0].track, Some(92.0));
    assert_eq!(pts[0].seen, 1.2);
    assert!(!pts[0].mil);

    // Hand-rolled ICAO-24 validator (no regex crate): 6 ASCII hex digits.
    assert!(is_icao_hex("a1B2c3") && is_icao_hex("ABCDEF") && is_icao_hex("012345"));
    assert!(!is_icao_hex("a1b2c"), "too short");
    assert!(!is_icao_hex("a1b2c3d"), "too long");
    assert!(!is_icao_hex("zzzzzz"), "non-hex");
    assert!(!is_icao_hex("a1b2c "), "space is not a hex digit");

    // Rows with a non-ICAO hex never reach the snapshot (they cannot be
    // enriched or tracked downstream).
    let bad = body(vec![ac("zzz", 1.0), ac("a1b2c3d4", 1.0), ac("GGBBCC", 1.0)]);
    assert!(parse_adsbx_response(&bad).is_empty());
    // Mixed batch keeps only the valid row.
    let mixed = body(vec![ac("zzz", 1.0), ac("a1b2c3", 2.0)]);
    let kept = parse_adsbx_response(&mixed);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].hex, "a1b2c3");
}

// ---------- 2. empty ----------

#[tokio::test]
async fn parse_empty_response_yields_empty_vec() {
    assert!(parse_adsbx_response(&body(vec![])).is_empty());
    assert!(parse_adsbx_response(&json!({})).is_empty(), "no `ac` key");
    assert!(parse_adsbx_response(&json!({"ac": null})).is_empty(), "null `ac`");

    // A 200 with zero traffic is a healthy hub, not a failure.
    let server = MockServer::start().await;
    mount(&server, 200, body(vec![])).await;
    let (pts, failed) = fetch_hubs(&reqwest::Client::new(), &server.uri(), None, Duration::ZERO).await;
    assert!(pts.is_empty());
    assert_eq!(failed, 0, "empty-but-200 hub is not a failure");
}

// ---------- 3. transport / protocol errors ----------

#[tokio::test]
async fn network_error_returns_empty_vec_no_panic() {
    // 500 on every hub: the sweep degrades to empty, never panics.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let (pts, failed) = fetch_hubs(&reqwest::Client::new(), &server.uri(), None, Duration::ZERO).await;
    assert!(pts.is_empty());
    assert_eq!(failed, ADSBX_HUBS.len(), "every hub counted as failed");

    // 200 with a non-JSON body is isolated per hub too.
    let server2 = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>nope</html>"))
        .mount(&server2)
        .await;
    let (pts2, failed2) = fetch_hubs(&reqwest::Client::new(), &server2.uri(), None, Duration::ZERO).await;
    assert!(pts2.is_empty());
    assert_eq!(failed2, ADSBX_HUBS.len());

    // Unreachable host (nothing listens on port 1) is a transport error.
    let (pts3, failed3) =
        fetch_hubs(&reqwest::Client::new(), "http://127.0.0.1:1", None, Duration::ZERO).await;
    assert!(pts3.is_empty());
    assert_eq!(failed3, ADSBX_HUBS.len());

    // Partial failure keeps the healthy hubs' aircraft (429 bursts are
    // per-hub, so one throttled hub must not cost the other five).
    let server3 = MockServer::start().await;
    mount(&server3, 200, body(vec![ac("a1b2c3", 1.0)])).await;
    let (pts4, failed4) = fetch_hubs(&reqwest::Client::new(), &server3.uri(), None, Duration::ZERO).await;
    assert_eq!(failed4, 0);
    assert_eq!(pts4.len(), 1, "6 hubs x 1 hex -> deduped to one row");
}

// ---------- 4. freshness: seen 0 ----------

#[test]
fn timestamp_zero_means_recent() {
    // `seen: 0` = message received just now ⇒ kept, age_s 0 in the envelope.
    let pts = filter_stale(parse_adsbx_response(&body(vec![ac("a1b2c3", 0.0)])));
    assert_eq!(pts.len(), 1);
    let env = snapshot_envelope(&pts, 1_789_968_958);
    assert_eq!(env["count"], 1);
    assert_eq!(env["aircraft"][0]["hex"], "a1b2c3");
    assert_eq!(env["aircraft"][0]["age_s"], 0);

    // A row that omits `seen` entirely parses to f64::MAX (T1 semantics) and
    // is NOT treated as recent — an unaged fix cannot be trusted as live.
    let no_seen = json!({"hex": "a1b2c3", "lat": 33.6, "lon": -84.4, "alt_baro": 1000});
    assert!(filter_stale(parse_adsbx_response(&body(vec![no_seen]))).is_empty());
}

// ---------- 5. freshness: stale ----------

#[test]
fn stale_records_filtered_out() {
    let pts = filter_stale(parse_adsbx_response(&body(vec![
        ac("aaaaaa", 61.0),
        ac("bbbbbb", 60.0),
        ac("cccccc", 5.0),
    ])));
    let hexes: Vec<&str> = pts.iter().map(|p| p.hex.as_str()).collect();
    assert_eq!(hexes, vec!["bbbbbb", "cccccc"], "61s dropped, 60s boundary kept");
    assert_eq!(ADSBX_MAX_SEEN_SECS, 60.0);
    // The dropped row never reaches the envelope, so `count` stays honest.
    let env = snapshot_envelope(&pts, 1_789_968_958);
    assert_eq!(env["count"], 2);
}

// ---------- 6. dedup + 3-source merge ----------

#[test]
fn merge_dedup_by_hex_picks_fresher_age() {
    // Unit level: the hub union keeps the freshest sighting per hex, and the
    // output is hex-sorted (deterministic envelope ordering).
    let m = dedup_merge(&[point("aaaaaa", 90.0), point("bbbbbb", 5.0)], &[point("aaaaaa", 2.0)]);
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].hex, "aaaaaa");
    assert_eq!(m[0].seen, 2.0, "fresher sighting wins");
    assert_eq!(m[1].hex, "bbbbbb");

    // REST level: adsbx replaces the rotation row only when genuinely fresher.
    let adsb = env("hotspots+mil+squawk", vec![row("aaaaaa", 10)]);
    let adsbx = env("adsbx", vec![row("aaaaaa", 3), row("dddddd", 4)]);
    let merged = merge_globe_snapshots(Some(&adsb), Some(&adsbx), None);
    assert_eq!(merged["count"], 2);
    let rows = merged["aircraft"].as_array().unwrap();
    assert_eq!(
        rows.iter().map(|r| r["hex"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["aaaaaa", "dddddd"]
    );
    let aa = rows.iter().find(|r| r["hex"] == "aaaaaa").unwrap();
    assert_eq!(aa["age_s"], 3, "adsbx row is fresher than the rotation row");
    // A stale adsbx row does NOT displace a fresher rotation row.
    let stale_adsbx = env("adsbx", vec![row("aaaaaa", 99)]);
    let merged2 = merge_globe_snapshots(Some(&adsb), Some(&stale_adsbx), None);
    assert_eq!(merged2["aircraft"][0]["age_s"], 10);
    assert_eq!(merged2["count"], 1);
}

// ---------- 7. merge with no adsbx ----------

#[test]
fn merge_with_no_adsbx_keeps_existing() {
    let adsb = env("hotspots+mil+squawk", vec![row("aaaaaa", 10)]);

    // Missing third snapshot ⇒ base envelope served verbatim (sp6's globe
    // contract: last_tick / cycle_secs / coverage unchanged).
    let m = merge_globe_snapshots(Some(&adsb), None, None);
    assert_eq!(m, adsb);

    // adsbx present but empty is likewise a no-op.
    let empty = env("adsbx", vec![]);
    assert_eq!(merge_globe_snapshots(Some(&adsb), Some(&empty), None), adsb);

    // adsbx-only (rotation + opensky both dead) still serves the 3rd source.
    let only = merge_globe_snapshots(None, Some(&env("adsbx", vec![row("dddddd", 4)])), None);
    assert_eq!(only["count"], 1);
    assert_eq!(only["coverage"], "adsbx");
    assert_eq!(only["aircraft"][0]["hex"], "dddddd");

    // Coverage concat is priority-ordered and idempotent (re-merging a
    // merged envelope never double-appends).
    let merged = merge_globe_snapshots(Some(&adsb), Some(&empty), Some(&env("opensky", vec![row("eeeeee", 2)])));
    assert_eq!(merged["coverage"], "hotspots+mil+squawk+opensky", "empty adsbx adds no suffix");
    let both = merge_globe_snapshots(Some(&adsb), Some(&env("adsbx", vec![row("dddddd", 4)])), Some(&env("opensky", vec![row("eeeeee", 2)])));
    assert_eq!(both["coverage"], "hotspots+mil+squawk+adsbx+opensky");
    let again = merge_globe_snapshots(Some(&both), Some(&env("adsbx", vec![row("dddddd", 4)])), Some(&env("opensky", vec![row("eeeeee", 2)])));
    assert_eq!(again["coverage"], "hotspots+mil+squawk+adsbx+opensky", "idempotent");

    // Both overlays dead and no base ⇒ the historical stale envelope.
    let dead = merge_globe_snapshots(None, None, None);
    assert_eq!(dead["stale"], true);
    assert!(dead["aircraft"].as_array().unwrap().is_empty());
}

// ---------- 8. Redis write contract ----------

#[test]
fn redis_writes_cached_snapshot() {
    assert_eq!(ADSBX_AIRCRAFT_KEY, "hub:globe:aircraft:adsbx", "own key — never clobbers hub:globe:aircraft");
    assert_eq!(ADSBX_TTL_SECS, 300);
    assert_eq!(ADSBX_RADIUS_NM, 50);
    assert_eq!(ADSBX_HUBS.len(), 8);

    // Point URL shape (readsb `point` query, radius in nautical miles).
    assert_eq!(
        hub_url("https://api.adsb.lol", 33.6407, -84.4277),
        "https://api.adsb.lol/v2/point/33.6407/-84.4277/50"
    );
    assert_eq!(
        hub_url("http://127.0.0.1:1234/", 40.6413, -73.7781),
        "http://127.0.0.1:1234/v2/point/40.6413/-73.7781/50",
        "trailing slash tolerated (wiremock base)"
    );

    // The envelope handed to SETEX is T1's exact row shape.
    let env = snapshot_envelope(&[point("a1b2c3", 7.0)], 1_789_968_958);
    assert_eq!(env["coverage"], "adsbx");
    assert_eq!(env["last_tick"], "us-hubs");
    assert_eq!(env["cycle_secs"], 300);
    assert_eq!(env["aircraft"][0]["age_s"], 7);
    assert_eq!(env["aircraft"][0]["alt_m"], 11278);
    assert!(env["aircraft"][0].get("seen").is_none(), "only age_s is published");

    // Packed RESP: `SETEX <key> 300 <json>` — the TTL is its own argument
    // ($3\r\n300\r\n), not a substring of the JSON payload.
    let packed = String::from_utf8_lossy(&snapshot_setex_cmd(&env).get_packed_command()).to_string();
    assert!(packed.starts_with("SETEX") || packed.contains("SETEX"), "packed={packed}");
    assert!(packed.contains(ADSBX_AIRCRAFT_KEY), "key must be in the SETEX");
    assert!(packed.contains("$3\r\n300\r\n"), "TTL 300s must be a discrete argument: {packed}");
    assert!(packed.contains("\"coverage\":\"adsbx\""), "envelope body is the command payload");
}
