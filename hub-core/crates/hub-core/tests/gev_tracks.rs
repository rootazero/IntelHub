//! GEV P12 T2: aircraft track backfill proxies — OpenSky OAuth/adaptive-TTL/
//! cooldown/serve-stale, adsb.lol trace, cache bounds, and wire shapes.
//!
//! All upstream I/O is served by wiremock (every endpoint is
//! constructor-injected via [`TracksConfig`]), so these run without network or
//! live PG/Redis/Neo4j.
//!
//! Covers the spec §4.B test list:
//! `oauth_token_cache_within_expiry`, `oauth_inflight_coalesce`,
//! `adaptive_ttl_4_tiers`, `cooldown_429_honors_retry_after`,
//! `cooldown_429_clamps_to_5s_floor_and_30min_ceiling`,
//! `serve_stale_during_cooldown`, `cache_lru_eviction`, `response_cap_5mb`,
//! `normalizes_track_path`, `env_missing_returns_503` — plus the wire-shape,
//! adsb.lol readsb, 404 passthrough, and eviction/decoding edge cases.

use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::http::StatusCode;
use hub_core::gev_enrichment::now_ms;
use hub_core::gev_tracks::{
    adaptive_ttl, augment_track_body, evict_oldest, handle_route, normalize_readsb_trace,
    normalize_track_path, parse_retry_after_secs, resolve_env_value, CachedTrack, TracksConfig,
    TracksService, COOLDOWN_MAX_MS, COOLDOWN_MIN_MS, OPENSKY_CACHE_MS, RESPONSE_CAP_BYTES,
    TRACK_CACHE_MAX,
};
use serde_json::json;
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------- fixtures ----------

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

/// Service wired to `server`. Endpoint layout mirrors production:
/// `POST {base}/auth/token`, `GET {base}/api/tracks/all`,
/// `GET {base}/adsblol/data/traces/{xx}/trace_full_{hex}.json`.
fn service_for(server: &MockServer, creds: Option<(&str, &str)>) -> TracksService {
    let base = server.uri();
    TracksService::with_config(
        http_client(),
        TracksConfig {
            opensky_auth_url: format!("{base}/auth/token"),
            opensky_api_base: format!("{base}/api"),
            adsblol_base_url: format!("{base}/adsblol"),
            client_id: creds.map(|(id, _)| id.to_string()),
            client_secret: creds.map(|(_, secret)| secret.to_string()),
        },
    )
}

fn track_service(server: &MockServer) -> TracksService {
    service_for(server, Some(("client-id", "client-secret")))
}

/// OAuth token endpoint. `expect` is the exact number of upstream auth
/// requests allowed — wiremock fails the test on drop if it differs, which is
/// how the two coalescing tests are enforced.
async fn mount_oauth(server: &MockServer, expect: u64) {
    Mock::given(method("POST"))
        .and(path("/auth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-1",
            "expires_in": 1800
        })))
        .expect(expect)
        .mount(server)
        .await;
}

async fn mount_opensky_track(
    server: &MockServer,
    icao24: &str,
    template: ResponseTemplate,
    expect: u64,
) {
    Mock::given(method("GET"))
        .and(path("/api/tracks/all"))
        .and(query_param("icao24", icao24))
        .respond_with(template)
        .expect(expect)
        .mount(server)
        .await;
}

fn opensky_body() -> serde_json::Value {
    json!({
        "icao24": "4ca9b1",
        "path": [
            [1726845215.0, 40.69, -74.17, 10500.0, 91.0, false],
            [1726845300.0, 40.70, -74.18, 10600.0, 92.0, false]
        ],
        "startTime": 1726845215,
        "endTime": 1726845300
    })
}

async fn body_json(res: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn cache_header(res: &axum::response::Response) -> Option<&str> {
    res.headers().get("X-ADS-B-Cache").and_then(|v| v.to_str().ok())
}

// ---------- pure helpers ----------

#[test]
fn adaptive_ttl_4_tiers() {
    assert_eq!(adaptive_ttl(Some(3000)), 9000);
    assert_eq!(adaptive_ttl(Some(2000)), 30_000);
    assert_eq!(adaptive_ttl(Some(800)), 90_000);
    assert_eq!(adaptive_ttl(Some(200)), 300_000);
    assert_eq!(adaptive_ttl(None), 9000);
    // Tier boundaries are exclusive (spec §4.B: `>2400 / >1200 / >400 / else`).
    assert_eq!(adaptive_ttl(Some(2401)), OPENSKY_CACHE_MS);
    assert_eq!(adaptive_ttl(Some(2400)), 30_000);
    assert_eq!(adaptive_ttl(Some(1200)), 90_000);
    assert_eq!(adaptive_ttl(Some(400)), 300_000);
    // Exhausted quota ⇒ the 5-minute tier, the one that matters on a 429.
    assert_eq!(adaptive_ttl(Some(0)), 300_000);
}

#[test]
fn parse_retry_after_secs_delta_and_epoch() {
    let now = 1_800_000_000_000u64; // epoch ms
    // Delta-seconds (what OpenSky sends).
    assert_eq!(parse_retry_after_secs("30", now), Some(30_000));
    assert_eq!(parse_retry_after_secs("0", now), Some(0));
    // Absolute epoch seconds (what some CDNs send).
    let future = ((now / 1000) + 60).to_string();
    assert_eq!(parse_retry_after_secs(&future, now), Some(60_000));
    // An epoch already in the past saturates to 0; the caller clamps it up.
    assert_eq!(parse_retry_after_secs("1800000000", now), Some(0));
    // Garbage (including the HTTP-date form, which neither upstream sends).
    assert_eq!(parse_retry_after_secs("invalid", now), None);
    assert_eq!(parse_retry_after_secs("Wed, 21 Oct 2026 07:28:00 GMT", now), None);
    assert_eq!(parse_retry_after_secs("", now), None);
}

#[test]
fn normalizes_track_path() {
    let path = vec![
        json!([1726845215.0, 40.69, -74.17, 10500.0, 91.0, false]),
        json!([1726845300.0, 40.70, -74.18, 10600.0, 92.0, false]),
    ];
    let records = normalize_track_path(&path);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].observed_at_ms, 1726845215000);
    assert!((records[0].latitude - 40.69).abs() < 0.01);
    assert_eq!(records[1].baro_altitude_m, Some(10600.0));
    assert_eq!(records[0].course_deg, Some(91.0));
    assert!(!records[0].on_ground);

    // Wire names are camelCase (spec §3.1 + accept-sp6 check_43).
    let wire = serde_json::to_value(&records[0]).unwrap();
    assert_eq!(wire["observedAtMs"], json!(1726845215000u64));
    assert_eq!(wire["baroAltitudeM"], json!(10500.0));
    assert_eq!(wire["courseDeg"], json!(91.0));
    assert_eq!(wire["onGround"], json!(false));
    assert!(wire.get("observed_at_ms").is_none(), "snake_case must not leak");
}

#[test]
fn normalize_track_path_skips_malformed_rows() {
    let path = vec![
        json!("not-an-array"),
        json!([1.0, 2.0]),          // too short
        json!([1.0, null, 3.0]),    // null latitude
        json!([1.0, 40.0, -74.0]),  // minimal valid row
    ];
    let records = normalize_track_path(&path);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].observed_at_ms, 1000);
    assert_eq!(records[0].latitude, 40.0);
    assert_eq!(records[0].baro_altitude_m, None);
    assert_eq!(records[0].course_deg, None);
    assert!(!records[0].on_ground);
}

#[test]
fn normalize_readsb_trace_uses_timestamp_base() {
    let trace = vec![
        json!([0.0, 40.69, -74.17, 35000.0, 400.0, 91.0, 0, 0]),
        json!([5.0, 40.70, -74.18, "ground", 0.0, 92.0, 0, 0]),
    ];
    let records = normalize_readsb_trace(Some(1_726_845_000), &trace);
    assert_eq!(records.len(), 2);
    // Offsets are relative to `timestamp` (spec §3.3 readsb shape).
    assert_eq!(records[0].observed_at_ms, 1_726_845_000_000);
    assert_eq!(records[1].observed_at_ms, 1_726_845_005_000);
    // readsb altitudes are feet; "ground" is a marker, not an altitude.
    assert!((records[0].baro_altitude_m.unwrap() - 10668.0).abs() < 1.0);
    assert_eq!(records[1].baro_altitude_m, None);
    assert!(records[1].on_ground);
    // No base timestamp (upstream shape drift) ⇒ no records, never invented ones.
    assert!(normalize_readsb_trace(None, &trace).is_empty());
}

#[test]
fn augment_track_body_preserves_upstream_shape() {
    let raw = opensky_body().to_string();
    let out: serde_json::Value = serde_json::from_str(&augment_track_body(&raw)).unwrap();
    assert_eq!(
        out["path"].as_array().unwrap().len(),
        1 + 1,
        "vendor reads `path` verbatim"
    );
    assert_eq!(out["records"].as_array().unwrap().len(), 2);
    assert_eq!(out["complete"], json!(false));
    assert_eq!(out["startTime"], json!(1726845215), "unknown keys survive");

    // Already-augmented bodies (what the cache stores) pass through untouched.
    let once = augment_track_body(&raw);
    assert_eq!(augment_track_body(&once), once);

    // Unparseable / non-object bodies degrade to an empty envelope, never panic.
    let empty: serde_json::Value = serde_json::from_str(&augment_track_body("not json")).unwrap();
    assert_eq!(empty["records"], json!([]));
    let arr: serde_json::Value = serde_json::from_str(&augment_track_body("[1,2,3]")).unwrap();
    assert_eq!(arr["records"], json!([]));
}

#[test]
fn evict_oldest_drops_by_at() {
    let mut map = std::collections::HashMap::new();
    for i in 0..(TRACK_CACHE_MAX + 3) {
        map.insert(format!("{i:06x}"), CachedTrack { at: i as u64, body: "{}".to_string() });
    }
    assert_eq!(evict_oldest(&mut map), 3);
    assert_eq!(map.len(), TRACK_CACHE_MAX);
    assert!(map.get("000000").is_none());
    assert!(map.get("000001").is_none());
    assert!(map.get("000002").is_none());
    assert!(map.contains_key(&format!("{:06x}", TRACK_CACHE_MAX + 2)));
}

#[test]
fn env_prefixed_credential_wins() {
    // `HUB_*` wins over the bare name (monitor-collector convention).
    assert_eq!(
        resolve_env_value(Some("hub-id".into()), Some("bare-id".into())).as_deref(),
        Some("hub-id")
    );
    // Blank counts as unset in either slot.
    assert_eq!(
        resolve_env_value(Some("   ".into()), Some("bare".into())).as_deref(),
        Some("bare")
    );
    assert_eq!(resolve_env_value(Some(String::new()), None), None);
    assert_eq!(resolve_env_value(None, Some("bare".into())).as_deref(), Some("bare"));
    assert_eq!(resolve_env_value(None, None), None);
}

// ---------- OAuth ----------

#[tokio::test]
async fn oauth_token_cache_within_expiry() {
    let server = MockServer::start().await;
    mount_oauth(&server, 1).await;
    let svc = track_service(&server);

    let first = svc.opensky.get_token().await.unwrap();
    assert_eq!(first.as_deref(), Some("tok-1"));
    // Second call is served from the cache; the mock's `expect(1)` fails the
    // test if a second upstream auth request is issued.
    let second = svc.opensky.get_token().await.unwrap();
    assert_eq!(second, first);
    assert_eq!(svc.opensky.auth_calls(), 1);
}

#[tokio::test]
async fn oauth_inflight_coalesce() {
    let server = MockServer::start().await;
    mount_oauth(&server, 1).await;
    let svc = std::sync::Arc::new(track_service(&server));

    let calls: Vec<_> = (0..10)
        .map(|_| {
            let svc = svc.clone();
            async move { svc.opensky.get_token().await }
        })
        .collect();
    let tokens = futures::future::join_all(calls).await;
    assert!(
        tokens.iter().all(|t| t.as_ref().unwrap().as_deref() == Some("tok-1")),
        "every coalesced caller receives the refreshed token"
    );
    assert_eq!(
        svc.opensky.auth_calls(),
        1,
        "10 concurrent refreshes must coalesce to a single upstream call"
    );
}

// ---------- OpenSky handler ----------

#[tokio::test]
async fn opensky_track_returns_records_and_preserves_path() {
    let server = MockServer::start().await;
    mount_oauth(&server, 1).await;
    mount_opensky_track(
        &server,
        "4ca9b1",
        ResponseTemplate::new(200)
            .insert_header("X-Rate-Limit-Remaining", "3000")
            .set_body_json(opensky_body()),
        1,
    )
    .await;

    let svc = track_service(&server);
    let res = handle_route("/api/opensky-track?icao24=4CA9B1", &svc).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(cache_header(&res), Some("MISS"));
    let body = body_json(res).await;
    // Both shapes: raw `path` for the vendor engine, `records` for the
    // documented hub contract (spec §3.1 / accept-sp6 check_43).
    assert_eq!(body["path"].as_array().unwrap().len(), 2);
    assert_eq!(body["records"].as_array().unwrap().len(), 2);
    assert_eq!(body["records"][0]["observedAtMs"], json!(1726845215000u64));
    assert_eq!(body["records"][0]["latitude"], json!(40.69));
    assert_eq!(body["records"][0]["onGround"], json!(false));
    assert_eq!(body["complete"], json!(false));
    // `X-Rate-Limit-Remaining: 3000` selects the healthy 9s tier.
    assert_eq!(svc.opensky.adaptive_ttl_ms.load(Ordering::Relaxed), OPENSKY_CACHE_MS);

    // Second call is a fresh cache HIT (the upstream mock allows only one).
    let res = handle_route("/api/opensky-track?icao24=4ca9b1", &svc).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(cache_header(&res), Some("HIT"));
    assert!(res.headers().get("X-ADS-B-Cache-Age-Ms").is_some());
}

#[tokio::test]
async fn opensky_404_passes_through() {
    let server = MockServer::start().await;
    mount_oauth(&server, 1).await;
    mount_opensky_track(
        &server,
        "4ca9b1",
        ResponseTemplate::new(404).set_body_string("no track"),
        1,
    )
    .await;

    let svc = track_service(&server);
    let res = handle_route("/api/opensky-track?icao24=4ca9b1", &svc).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let body = body_json(res).await;
    assert_eq!(body["error"], json!("track source HTTP 404"));
    assert_eq!(
        svc.opensky.cooldown_until.load(Ordering::Relaxed),
        0,
        "a 404 is not a rate limit — no cooldown"
    );
}

#[tokio::test]
async fn opensky_track_invalid_icao24_400() {
    let server = MockServer::start().await;
    let svc = track_service(&server);
    for bad in ["", "4ca9b", "4ca9b1f", "zzzzzz", "4ca9%20"] {
        let res = handle_route(&format!("/api/opensky-track?icao24={bad}"), &svc).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "icao24={bad:?}");
    }
    let res = handle_route("/api/opensky-track", &svc).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(svc.opensky.auth_calls(), 0, "validation must not reach upstream");
}

#[tokio::test]
async fn cooldown_429_honors_retry_after() {
    let server = MockServer::start().await;
    mount_oauth(&server, 1).await;
    mount_opensky_track(
        &server,
        "4ca9b1",
        ResponseTemplate::new(429).insert_header("Retry-After", "30"),
        1,
    )
    .await;

    let svc = track_service(&server);
    let before = now_ms();
    let res = handle_route("/api/opensky-track?icao24=4ca9b1", &svc).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let cooldown = svc.opensky.cooldown_until.load(Ordering::Relaxed);
    let expected = before + 30_000;
    assert!(
        cooldown.abs_diff(expected) < 1_000,
        "cooldown={cooldown} expected≈{expected}"
    );
}

#[tokio::test]
async fn cooldown_429_clamps_to_5s_floor_and_30min_ceiling() {
    // Retry-After: 7200 (2h) → clamped down to the 30min ceiling.
    let server = MockServer::start().await;
    mount_oauth(&server, 1).await;
    mount_opensky_track(
        &server,
        "4ca9b1",
        ResponseTemplate::new(429).insert_header("Retry-After", "7200"),
        1,
    )
    .await;
    let svc = track_service(&server);
    let before = now_ms();
    handle_route("/api/opensky-track?icao24=4ca9b1", &svc).await;
    let cooldown = svc.opensky.cooldown_until.load(Ordering::Relaxed);
    assert!(
        cooldown >= before + COOLDOWN_MAX_MS - 1_000,
        "ceiling must still leave a full-length cooldown"
    );
    assert!(cooldown.saturating_sub(now_ms()) <= COOLDOWN_MAX_MS);

    // Retry-After: 0 → clamped up to the 5s floor (never a no-op cooldown).
    let server = MockServer::start().await;
    mount_oauth(&server, 1).await;
    mount_opensky_track(
        &server,
        "4ca9b1",
        ResponseTemplate::new(429).insert_header("Retry-After", "0"),
        1,
    )
    .await;
    let svc = track_service(&server);
    let before = now_ms();
    handle_route("/api/opensky-track?icao24=4ca9b1", &svc).await;
    let cooldown = svc.opensky.cooldown_until.load(Ordering::Relaxed);
    assert!(cooldown >= before + COOLDOWN_MIN_MS - 1_000, "5s floor");
    assert!(cooldown <= now_ms() + COOLDOWN_MIN_MS + 1_000);

    // Retry-After absent → 30s default.
    let server = MockServer::start().await;
    mount_oauth(&server, 1).await;
    mount_opensky_track(&server, "4ca9b1", ResponseTemplate::new(429), 1).await;
    let svc = track_service(&server);
    let before = now_ms();
    handle_route("/api/opensky-track?icao24=4ca9b1", &svc).await;
    let cooldown = svc.opensky.cooldown_until.load(Ordering::Relaxed);
    assert!(
        cooldown.abs_diff(before + 30_000) < 1_000,
        "missing Retry-After falls back to 30s"
    );
}

#[tokio::test]
async fn serve_stale_during_cooldown() {
    // No mocks: any upstream call would be an unmatched 404 and fail the
    // assertions below, so this proves the cooldown path never fetches.
    let server = MockServer::start().await;
    let svc = track_service(&server);
    let cached = augment_track_body(&opensky_body().to_string());
    svc.cache_put(
        "4ca9b1",
        CachedTrack { at: now_ms() - 120_000, body: cached },
    )
    .await;
    svc.opensky
        .cooldown_until
        .store(now_ms() + 60_000, Ordering::Relaxed);

    let res = handle_route("/api/opensky-track?icao24=4ca9b1", &svc).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(cache_header(&res), Some("STALE"));
    assert_eq!(res.headers().get("X-OpenSky-Stale").unwrap(), "1");
    assert!(res.headers().get("X-ADS-B-Cache-Age-Ms").is_some());
    let body = body_json(res).await;
    assert_eq!(body["records"].as_array().unwrap().len(), 2);

    // A key with nothing cached gets an honest 503 during the cooldown.
    let res = handle_route("/api/opensky-track?icao24=abcdef", &svc).await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(res).await;
    assert_eq!(body["error"], json!("opensky upstream cooling down"));
}

#[tokio::test]
async fn cache_lru_eviction() {
    let server = MockServer::start().await;
    let svc = track_service(&server);
    for i in 0..TRACK_CACHE_MAX {
        svc.cache_put(
            &format!("{i:06x}"),
            CachedTrack { at: i as u64, body: "{}".to_string() },
        )
        .await;
    }
    assert_eq!(svc.cache.lock().await.len(), TRACK_CACHE_MAX);

    // One more entry evicts the oldest-by-`at` (`000000` had at = 0).
    svc.cache_put("ffffff", CachedTrack { at: 999_999, body: "{}".to_string() })
        .await;
    let cache = svc.cache.lock().await;
    assert_eq!(cache.len(), TRACK_CACHE_MAX);
    assert!(cache.get("000000").is_none(), "oldest entry must be evicted");
    assert!(cache.get("ffffff").is_some(), "newest entry must survive");
    assert!(cache.get("000001").is_some(), "only the oldest is evicted");
}

#[tokio::test]
async fn response_cap_5mb() {
    let server = MockServer::start().await;
    mount_oauth(&server, 1).await;
    mount_opensky_track(
        &server,
        "4ca9b1",
        ResponseTemplate::new(200).set_body_bytes(vec![b'x'; RESPONSE_CAP_BYTES + 1]),
        1,
    )
    .await;

    let svc = track_service(&server);
    let res = handle_route("/api/opensky-track?icao24=4ca9b1", &svc).await;
    assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
    assert!(
        svc.cache.lock().await.is_empty(),
        "an over-cap body must never be cached"
    );
}

#[tokio::test]
async fn env_missing_returns_503() {
    let server = MockServer::start().await;
    // No credentials = the same code path `TracksConfig::from_env()` takes when
    // OPENSKY_CLIENT_ID/SECRET are absent. Never a silent skip: the vendor
    // treats 503 as "fall back to the local trail" and the operator sees the
    // real reason in the body.
    let svc = service_for(&server, None);
    assert!(!svc.opensky.has_credentials());
    let res = handle_route("/api/opensky-track?icao24=4ca9b1", &svc).await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(res).await;
    assert!(
        body["error"].as_str().unwrap().contains("not configured"),
        "body must name the missing credential: {body}"
    );
}

// ---------- adsb.lol trace ----------

#[tokio::test]
async fn adsblol_trace_invalid_hex_400() {
    let server = MockServer::start().await;
    let svc = track_service(&server);
    for bad in ["", "x", "abcde", "zzzzzz", "4ca9b1c1"] {
        let res = handle_route(&format!("/api/adsblol/trace?hex={bad}"), &svc).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "hex={bad:?}");
    }
    let res = handle_route("/api/adsblol/trace", &svc).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 7-char and readsb `~`-prefixed non-ICAO shapes pass validation — with no
    // mock mounted they reach upstream and surface its 404, not a 400.
    for ok in ["4ca9b1", "4ca9b1c", "~abcdef"] {
        let res = handle_route(&format!("/api/adsblol/trace?hex={ok}"), &svc).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "hex={ok:?} must validate");
    }
}

#[tokio::test]
async fn adsblol_trace_serves_readsb_shape_with_records() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/adsblol/data/traces/b1/trace_full_4ca9b1\.json$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "icao": "4ca9b1",
            "timestamp": 1726845000,
            "trace": [
                [0.0, 40.69, -74.17, 35000.0, 400.0, 91.0, 0, 0],
                [5.0, 40.70, -74.18, 34000.0, 395.0, 92.0, 0, 0]
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let svc = track_service(&server);
    let res = handle_route("/api/adsblol/trace?hex=4CA9B1", &svc).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(cache_header(&res), Some("MISS"));
    let body = body_json(res).await;
    // Vendor shape (`timestamp` + `trace`) survives; `records` is added.
    assert_eq!(body["timestamp"], json!(1726845000));
    assert_eq!(body["trace"].as_array().unwrap().len(), 2);
    assert_eq!(body["records"].as_array().unwrap().len(), 2);
    assert_eq!(body["records"][0]["observedAtMs"], json!(1726845000000u64));
    assert_eq!(body["records"][1]["latitude"], json!(40.70));

    // Second call is a 60s-cache HIT (the upstream mock allows only one).
    let res = handle_route("/api/adsblol/trace?hex=4ca9b1", &svc).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(cache_header(&res), Some("HIT"));
}

#[tokio::test]
async fn adsblol_trace_404_passes_through() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/adsblol/data/traces/b1/trace_full_4ca9b1\.json$"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let svc = track_service(&server);
    let res = handle_route("/api/adsblol/trace?hex=4ca9b1", &svc).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let body = body_json(res).await;
    assert_eq!(body["error"], json!("adsblol trace HTTP 404"));
}

// ---------- dispatcher ----------

#[tokio::test]
async fn unknown_route_404() {
    let server = MockServer::start().await;
    let svc = track_service(&server);
    let res = handle_route("/api/not-a-track-endpoint", &svc).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
