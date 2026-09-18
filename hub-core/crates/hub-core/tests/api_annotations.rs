//! GEV P8: `/api/v1/annotations/*` HTTP contract tests.
//!
//! Two layers:
//!
//! 1. **Pure parse tests** (always run) — `parse_bbox` / `parse_since` are the
//!    one place where a silent mis-parse turns a filtered query into an
//!    unfiltered one, so they are pinned directly. No DB, no server.
//! 2. **HTTP round-trips** (DB-gated) — a real `axum::serve` on an ephemeral
//!    loopback port hit with `reqwest`, backed by a per-test PG schema (same
//!    harness as `tests/db_annotations.rs`, plan ruling 3: no `#[sqlx::test]`).
//!    When `DATABASE_URL` is unset the test prints `SKIP` and passes, exactly
//!    like the Task 1 DB tests — real verification runs on the test VM:
//!
//!    ```text
//!    ssh -N -L 15432:172.30.2.10:5432 Debian-test &
//!    DATABASE_URL=postgres://intelhub:<pw>@127.0.0.1:15432/intelhub \
//!      cargo test --package hub-core --test api_annotations -- --nocapture
//!    ```
//!
//! 401 rejection is exercised without a DB at all (the handler-local guard
//! answers before the pool is touched), so that contract holds on any machine.

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use hub_core::api::annotations::{parse_bbox, parse_since, AnnotationsState};
use serde_json::{json, Value as JsonValue};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Executor, PgPool};
use uuid::Uuid;

// ── pure parse tests (always run) ─────────────────────────────────────────

#[test]
fn parse_bbox_accepts_south_west_north_east() {
    let b = parse_bbox("0,0,10,10").expect("plain bbox");
    assert_eq!((b.south, b.west, b.north, b.east), (0.0, 0.0, 10.0, 10.0));

    let b2 = parse_bbox(" -5.5 , 10.25 , 20 , 30.75 ").expect("spaces + fractions");
    assert_eq!(
        (b2.south, b2.west, b2.north, b2.east),
        (-5.5, 10.25, 20.0, 30.75)
    );
}

#[test]
fn parse_bbox_rejects_malformed_instead_of_silently_dropping_filter() {
    assert!(parse_bbox("1,2,3").is_err(), "3 components");
    assert!(parse_bbox("1,2,3,4,5").is_err(), "5 components");
    assert!(parse_bbox("a,b,c,d").is_err(), "non-numeric");
    assert!(parse_bbox("").is_err(), "empty");
    assert!(parse_bbox("nan,0,1,1").is_err(), "non-finite");
    assert!(parse_bbox("10,0,0,10").is_err(), "south > north");
    assert!(parse_bbox("0,10,10,0").is_err(), "west > east");
}

#[test]
fn parse_since_accepts_documented_formats() {
    let now = Utc::now();
    assert!(parse_since("now").expect("now") >= now - Duration::seconds(1));

    let one_hour_ago = parse_since("-1h").expect("-1h");
    let delta = Utc::now().signed_duration_since(one_hour_ago).num_seconds();
    assert!((3595..=3605).contains(&delta), "got {delta}s lookback");

    let one_minute_ago = parse_since("-30m").expect("-30m");
    let delta = Utc::now().signed_duration_since(one_minute_ago).num_seconds();
    assert!((1795..=1805).contains(&delta), "got {delta}s lookback");

    let rfc = parse_since("2026-09-18T00:00:00Z").expect("rfc3339");
    assert_eq!(rfc.to_rfc3339(), "2026-09-18T00:00:00+00:00");

    let unix = parse_since("1758153600").expect("unix seconds");
    assert_eq!(unix.timestamp(), 1758153600);
}

#[test]
fn parse_since_rejects_garbage() {
    assert!(parse_since("yesterday").is_err());
    assert!(parse_since("").is_err());
    assert!(parse_since("-h").is_err(), "missing number");
    assert!(parse_since("-1w").is_err(), "week unit not supported");
}

/// Pins the PATCH wire contract: serde's blanket `Option` impl collapses a
/// JSON `null` to `None`, indistinguishable from an absent key. `PatchBody`
/// must keep them apart, or "clear this label" silently becomes a no-op.
#[test]
fn patch_body_double_option_wire_semantics() {
    use hub_core::api::annotations::PatchBody;

    let absent: PatchBody = serde_json::from_value(json!({})).expect("empty body");
    assert_eq!(absent.label, None, "absent label stays None (untouched)");

    let null: PatchBody = serde_json::from_value(json!({ "label": null })).expect("null label");
    assert_eq!(null.label, Some(None), "explicit null → Some(None) (clear)");

    let value: PatchBody =
        serde_json::from_value(json!({ "label": "x", "ttl_ms": 1000 })).expect("value body");
    assert_eq!(value.label, Some(Some("x".to_string())));
    assert_eq!(value.ttl_ms, Some(Some(1000)));
    assert_eq!(value.color, None);

    let null_ttl: PatchBody =
        serde_json::from_value(json!({ "ttl_ms": null })).expect("null ttl");
    assert_eq!(null_ttl.ttl_ms, Some(None));
}

// ── harness ────────────────────────────────────────────────────────────────

/// A fresh schema + pool for one test. `admin` shares the server but keeps the
/// default `search_path` so it can drop the schema afterwards.
struct TestDb {
    pool: PgPool,
    admin: PgPool,
    schema: String,
}

impl TestDb {
    async fn cleanup(self) {
        self.pool.close().await;
        self.admin
            .execute(format!(r#"DROP SCHEMA IF EXISTS "{}" CASCADE"#, self.schema).as_str())
            .await
            .expect("drop test schema");
    }
}

fn database_url() -> Option<String> {
    std::env::var("DATABASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn fresh_db(test_name: &str) -> Option<TestDb> {
    let url = database_url()?;
    let schema = format!("annot_api_test_{test_name}");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("DATABASE_URL admin connect");
    admin
        .execute(format!(r#"DROP SCHEMA IF EXISTS "{schema}" CASCADE"#).as_str())
        .await
        .expect("drop stale test schema");
    admin
        .execute(format!(r#"CREATE SCHEMA "{schema}""#).as_str())
        .await
        .expect("create test schema");

    let opts: PgConnectOptions = url.parse().expect("parse DATABASE_URL");
    let opts = opts.options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .expect("connect test schema pool");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("run migrations into test schema");

    Some(TestDb { pool, admin, schema })
}

macro_rules! require_db {
    ($name:literal) => {
        match fresh_db($name).await {
            Some(db) => db,
            None => {
                eprintln!(
                    "SKIP api_annotations::{}: DATABASE_URL unset (no direct PG access)",
                    $name
                );
                return;
            }
        }
    };
}

/// A pool that is never connected (queries connect lazily). Used by the
/// auth-rejection tests, whose request is rejected before the pool is touched.
fn lazy_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://intelhub:intelhub@127.0.0.1:5432/intelhub")
        .expect("lazy pool parses")
}

/// The annotation router mounted on a bare pool state (production uses
/// `Arc<AppState>`; `AnnotationsState: FromRef<Arc<AppState>>` bridges them).
fn app(pool: PgPool) -> axum::Router {
    hub_core::api::annotations::router::<AnnotationsState>().with_state(AnnotationsState(pool))
}

/// Serve `app` on an ephemeral loopback port, returning its base URL.
async fn spawn(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test server");
    });
    format!("http://{addr}")
}

/// Seed one live agent + api key, returning the bearer token.
async fn seed_key(pool: &PgPool, token: &str) {
    let agent_id = Uuid::new_v4();
    sqlx::query("INSERT INTO agents (agent_id, name) VALUES ($1, $2)")
        .bind(agent_id)
        .bind(format!("p8-api-{agent_id}"))
        .execute(pool)
        .await
        .expect("insert agent");
    sqlx::query("INSERT INTO api_keys (key_id, agent_id, key_hash, label) VALUES ($1, $2, $3, $4)")
        .bind(Uuid::new_v4())
        .bind(agent_id)
        .bind(hub_core::auth::hash_key(token))
        .bind("p8-api-test")
        .execute(pool)
        .await
        .expect("insert api key");
}

const KEY: &str = "ihk_p8_api_test_key";

fn pin(lon: f64, lat: f64) -> JsonValue {
    json!({ "vertices": [{ "lon": lon, "lat": lat, "height": 0 }] })
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn body_json(resp: reqwest::Response) -> JsonValue {
    resp.json().await.expect("json body")
}

// ── auth guard (no DB needed) ──────────────────────────────────────────────

#[tokio::test]
async fn unauthorized_create_returns_401() {
    let base = spawn(app(lazy_pool())).await;
    let resp = client()
        .post(format!("{base}/api/v1/annotations"))
        .json(&json!({ "shape": "pin", "geometry": pin(1.0, 2.0) }))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(body_json(resp).await["error"], "unauthorized");
}

#[tokio::test]
async fn unauthorized_patch_returns_401() {
    let base = spawn(app(lazy_pool())).await;
    let resp = client()
        .patch(format!("{base}/api/v1/annotations/{}", Uuid::new_v4()))
        .json(&json!({ "label": "x" }))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unauthorized_delete_returns_401() {
    let base = spawn(app(lazy_pool())).await;
    let resp = client()
        .delete(format!("{base}/api/v1/annotations/{}", Uuid::new_v4()))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── HTTP round-trips (DB-gated) ────────────────────────────────────────────

#[tokio::test]
async fn bogus_bearer_returns_401() {
    let db = require_db!("bogus_bearer_returns_401");
    seed_key(&db.pool, KEY).await;
    let base = spawn(app(db.pool.clone())).await;
    let resp = client()
        .post(format!("{base}/api/v1/annotations"))
        .bearer_auth("ihk_not_a_real_key")
        .json(&json!({ "shape": "pin", "geometry": pin(1.0, 2.0) }))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    db.cleanup().await;
}

#[tokio::test]
async fn list_empty_initially() {
    let db = require_db!("list_empty_initially");
    let base = spawn(app(db.pool.clone())).await;
    let resp = client()
        .get(format!("{base}/api/v1/annotations"))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["annotations"], json!([]));
    db.cleanup().await;
}

#[tokio::test]
async fn create_then_get_roundtrip() {
    let db = require_db!("create_then_get_roundtrip");
    seed_key(&db.pool, KEY).await;
    let base = spawn(app(db.pool.clone())).await;
    let c = client();

    let resp = c
        .post(format!("{base}/api/v1/annotations"))
        .bearer_auth(KEY)
        .json(&json!({
            "shape": "pin",
            "label": "alpha",
            "color": "red",
            "geometry": pin(1.0, 2.0),
            "meta": { "note": "drawn" }
        }))
        .send()
        .await
        .expect("post");
    assert_eq!(resp.status(), StatusCode::CREATED, "create must be 201");
    let row = body_json(resp).await;

    // Task 3 consumes this exact JSON shape — pin all ten fields.
    for k in [
        "id", "agent_id", "shape", "label", "color", "geometry", "ttl_ms", "meta", "created_at",
        "expires_at",
    ] {
        assert!(row.get(k).is_some(), "row must expose field {k}");
    }
    assert_eq!(row["shape"], "pin");
    assert_eq!(row["label"], "alpha");
    assert_eq!(row["color"], "red");
    assert_eq!(row["geometry"], pin(1.0, 2.0));
    assert_eq!(row["meta"], json!({ "note": "drawn" }));
    assert!(row["ttl_ms"].is_null());
    assert!(row["expires_at"].is_null());

    let id = row["id"].as_str().expect("id is a string").to_string();
    let got = c
        .get(format!("{base}/api/v1/annotations/{id}"))
        .send()
        .await
        .expect("get");
    assert_eq!(got.status(), StatusCode::OK);
    let fetched = body_json(got).await;
    assert_eq!(fetched["id"], row["id"]);
    assert_eq!(fetched["label"], "alpha");
    db.cleanup().await;
}

#[tokio::test]
async fn get_unknown_id_returns_404() {
    let db = require_db!("get_unknown_id_returns_404");
    let base = spawn(app(db.pool.clone())).await;
    let resp = client()
        .get(format!("{base}/api/v1/annotations/{}", Uuid::new_v4()))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    db.cleanup().await;
}

#[tokio::test]
async fn create_invalid_shape_returns_400() {
    let db = require_db!("create_invalid_shape_returns_400");
    seed_key(&db.pool, KEY).await;
    let base = spawn(app(db.pool.clone())).await;
    let resp = client()
        .post(format!("{base}/api/v1/annotations"))
        .bearer_auth(KEY)
        .json(&json!({ "shape": "cube", "geometry": pin(1.0, 2.0) }))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "invalid_shape");
    db.cleanup().await;
}

#[tokio::test]
async fn patch_label_and_color() {
    let db = require_db!("patch_label_and_color");
    seed_key(&db.pool, KEY).await;
    let base = spawn(app(db.pool.clone())).await;
    let c = client();

    let created = body_json(
        c.post(format!("{base}/api/v1/annotations"))
            .bearer_auth(KEY)
            .json(&json!({ "shape": "area", "label": "draft", "geometry": pin(3.0, 3.0) }))
            .send()
            .await
            .expect("post"),
    )
    .await;
    let id = created["id"].as_str().expect("id").to_string();

    let resp = c
        .patch(format!("{base}/api/v1/annotations/{id}"))
        .bearer_auth(KEY)
        .json(&json!({ "label": "foo", "color": "red" }))
        .send()
        .await
        .expect("patch");
    assert_eq!(resp.status(), StatusCode::OK);
    let patched = body_json(resp).await;
    assert_eq!(patched["label"], "foo");
    assert_eq!(patched["color"], "red");
    assert_eq!(patched["geometry"], created["geometry"], "geometry is immutable");

    let fetched = body_json(
        c.get(format!("{base}/api/v1/annotations/{id}"))
            .send()
            .await
            .expect("get"),
    )
    .await;
    assert_eq!(fetched["label"], "foo");
    db.cleanup().await;
}

#[tokio::test]
async fn patch_clears_label_double_option() {
    let db = require_db!("patch_clears_label_double_option");
    seed_key(&db.pool, KEY).await;
    let base = spawn(app(db.pool.clone())).await;
    let c = client();

    let created = body_json(
        c.post(format!("{base}/api/v1/annotations"))
            .bearer_auth(KEY)
            .json(&json!({ "shape": "pin", "label": "keep", "geometry": pin(4.0, 4.0) }))
            .send()
            .await
            .expect("post"),
    )
    .await;
    let id = created["id"].as_str().expect("id").to_string();
    assert_eq!(created["label"], "keep");

    // Omitted label must be left untouched.
    let untouched = body_json(
        c.patch(format!("{base}/api/v1/annotations/{id}"))
            .bearer_auth(KEY)
            .json(&json!({ "color": "blue" }))
            .send()
            .await
            .expect("patch color"),
    )
    .await;
    assert_eq!(untouched["label"], "keep", "omitted label stays");
    assert_eq!(untouched["color"], "blue");

    // Explicit JSON null clears it.
    let cleared = body_json(
        c.patch(format!("{base}/api/v1/annotations/{id}"))
            .bearer_auth(KEY)
            .json(&json!({ "label": null }))
            .send()
            .await
            .expect("patch clear"),
    )
    .await;
    assert!(cleared["label"].is_null(), "explicit null clears label");

    let fetched = body_json(
        c.get(format!("{base}/api/v1/annotations/{id}"))
            .send()
            .await
            .expect("get"),
    )
    .await;
    assert!(fetched["label"].is_null());
    db.cleanup().await;
}

#[tokio::test]
async fn delete_roundtrip() {
    let db = require_db!("delete_roundtrip");
    seed_key(&db.pool, KEY).await;
    let base = spawn(app(db.pool.clone())).await;
    let c = client();

    let created = body_json(
        c.post(format!("{base}/api/v1/annotations"))
            .bearer_auth(KEY)
            .json(&json!({ "shape": "pin", "geometry": pin(5.0, 5.0) }))
            .send()
            .await
            .expect("post"),
    )
    .await;
    let id = created["id"].as_str().expect("id").to_string();

    let del = c
        .delete(format!("{base}/api/v1/annotations/{id}"))
        .bearer_auth(KEY)
        .send()
        .await
        .expect("delete");
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    let gone = c
        .get(format!("{base}/api/v1/annotations/{id}"))
        .send()
        .await
        .expect("get after delete");
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    let again = c
        .delete(format!("{base}/api/v1/annotations/{id}"))
        .bearer_auth(KEY)
        .send()
        .await
        .expect("second delete");
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
    db.cleanup().await;
}

#[tokio::test]
async fn bbox_filter() {
    let db = require_db!("bbox_filter");
    seed_key(&db.pool, KEY).await;
    let base = spawn(app(db.pool.clone())).await;
    let c = client();

    let inside = body_json(
        c.post(format!("{base}/api/v1/annotations"))
            .bearer_auth(KEY)
            .json(&json!({ "shape": "pin", "geometry": pin(5.0, 5.0) }))
            .send()
            .await
            .expect("post inside"),
    )
    .await;
    let outside = body_json(
        c.post(format!("{base}/api/v1/annotations"))
            .bearer_auth(KEY)
            .json(&json!({ "shape": "pin", "geometry": pin(50.0, 50.0) }))
            .send()
            .await
            .expect("post outside"),
    )
    .await;

    // bbox is south,west,north,east → [lon 0..10] x [lat 0..10]
    let filtered = body_json(
        c.get(format!("{base}/api/v1/annotations"))
            .query(&[("bbox", "0,0,10,10")])
            .send()
            .await
            .expect("get bbox"),
    )
    .await;
    let ids: Vec<&str> = filtered["annotations"]
        .as_array()
        .expect("array")
        .iter()
        .map(|r| r["id"].as_str().expect("id"))
        .collect();
    assert!(ids.contains(&inside["id"].as_str().unwrap()), "inside must match");
    assert!(!ids.contains(&outside["id"].as_str().unwrap()), "outside must not match");

    // Sanity: without the filter both rows are visible.
    let all = body_json(
        c.get(format!("{base}/api/v1/annotations"))
            .send()
            .await
            .expect("get all"),
    )
    .await;
    assert_eq!(all["annotations"].as_array().expect("array").len(), 2);

    // Malformed bbox must 400, not silently return everything.
    let bad = c
        .get(format!("{base}/api/v1/annotations"))
        .query(&[("bbox", "1,2,3")])
        .send()
        .await
        .expect("get bad bbox");
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(bad).await["error"], "invalid_bbox");
    db.cleanup().await;
}

#[tokio::test]
async fn since_filter() {
    let db = require_db!("since_filter");
    seed_key(&db.pool, KEY).await;
    let base = spawn(app(db.pool.clone())).await;
    let c = client();

    let row = body_json(
        c.post(format!("{base}/api/v1/annotations"))
            .bearer_auth(KEY)
            .json(&json!({ "shape": "pin", "geometry": pin(6.0, 6.0) }))
            .send()
            .await
            .expect("post"),
    )
    .await;

    // Relative lookback includes the row.
    let recent = body_json(
        c.get(format!("{base}/api/v1/annotations"))
            .query(&[("since", "-1h")])
            .send()
            .await
            .expect("get since -1h"),
    )
    .await;
    let ids: Vec<&str> = recent["annotations"]
        .as_array()
        .expect("array")
        .iter()
        .map(|r| r["id"].as_str().expect("id"))
        .collect();
    assert!(ids.contains(&row["id"].as_str().unwrap()), "-1h must include the new row");

    // A future `since` excludes it.
    let future = (Utc::now() + Duration::seconds(60)).to_rfc3339();
    let none = body_json(
        c.get(format!("{base}/api/v1/annotations"))
            .query(&[("since", future.as_str())])
            .send()
            .await
            .expect("get future since"),
    )
    .await;
    assert_eq!(none["annotations"], json!([]));

    // Garbage since must 400, not silently fall back to the 24h default.
    let bad = c
        .get(format!("{base}/api/v1/annotations"))
        .query(&[("since", "yesterday")])
        .send()
        .await
        .expect("get bad since");
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(bad).await["error"], "invalid_since");
    db.cleanup().await;
}
