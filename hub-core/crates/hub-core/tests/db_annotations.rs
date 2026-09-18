//! GEV P8: `annotations_v1` CRUD against a real PostgreSQL.
//!
//! Harness note (P8 plan ruling 3): `#[sqlx::test]` is not used anywhere in
//! this workspace, and a plain `cargo test` on the dev Mac has no reachable
//! PG (Postgres runs in docker on the VMs). We therefore follow the repo's
//! established DB-test pattern (`tests/tier_enforcement.rs`): connect from
//! `DATABASE_URL` when it is exported, otherwise print `SKIP` and pass. Real
//! verification runs against the test VM, e.g.:
//!
//! ```text
//! ssh -N -L 15432:172.30.2.10:5432 Debian-test &
//! DATABASE_URL=postgres://intelhub:<pw>@127.0.0.1:15432/intelhub \
//!   cargo test --package hub-core --test db_annotations
//! ```
//!
//! Isolation for the parallel test run comes from a dedicated PG schema per
//! test (connection-level `search_path`), with the full migration set applied
//! — so these tests exercise `0022_annotations_v1.sql` in sequence, not a
//! hand-copied DDL stub.

use chrono::{Duration, Utc};
use hub_core::db::annotations::{
    create_annotation, delete_annotation, get_annotation, list_annotations, patch_annotation,
    AnnotationInsert, AnnotationPatch, AnnotationRow, BBox,
};
use serde_json::{json, Value as JsonValue};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Executor, PgPool};
use uuid::Uuid;

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

/// `None` means DATABASE_URL is unset → caller prints SKIP and returns.
///
/// The schema name is derived from the test name (deterministic) and recreated
/// from scratch, so a schema left behind by an earlier panicking run is cleaned
/// up on the next run instead of accumulating.
async fn fresh_db(test_name: &str) -> Option<TestDb> {
    let url = database_url()?;
    let schema = format!("annot_test_{test_name}");
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

    // Every pooled connection gets search_path pinned to the test schema, so
    // the unqualified table names in db::annotations resolve there.
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
                    "SKIP db_annotations::{}: DATABASE_URL unset (no direct PG access)",
                    $name
                );
                return;
            }
        }
    };
}

fn annotation(shape: &str, geometry: JsonValue) -> AnnotationInsert {
    AnnotationInsert {
        id: None,
        agent_id: None,
        shape: shape.to_string(),
        label: None,
        color: None,
        geometry,
        ttl_ms: None,
        meta: None,
    }
}

fn pin(lon: f64, lat: f64) -> JsonValue {
    json!({ "vertices": [{ "lon": lon, "lat": lat }] })
}

async fn fetch(pool: &PgPool, id: Uuid) -> AnnotationRow {
    get_annotation(pool, id)
        .await
        .expect("get_annotation")
        .expect("row must exist")
}

// ── create ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_returns_row_with_generated_id() {
    let db = require_db!("create_returns_row_with_generated_id");
    let row = create_annotation(&db.pool, annotation("pin", pin(1.0, 2.0)))
        .await
        .expect("create");
    assert_eq!(row.shape, "pin");
    assert_eq!(row.color, "primary", "color defaults to primary");
    assert_eq!(row.meta, json!({}), "meta defaults to empty object");
    assert!(row.ttl_ms.is_none());
    assert!(row.expires_at.is_none());
    assert_eq!(row.geometry, pin(1.0, 2.0));

    let again = fetch(&db.pool, row.id).await;
    assert_eq!(again.id, row.id, "generated id round-trips through GET");
    db.cleanup().await;
}

#[tokio::test]
async fn create_uses_supplied_id() {
    let db = require_db!("create_uses_supplied_id");
    let id = Uuid::new_v4();
    let mut spec = annotation("line", json!({ "vertices": [
        { "lon": 0.0, "lat": 0.0 }, { "lon": 1.0, "lat": 1.0 }
    ]}));
    spec.id = Some(id);
    let row = create_annotation(&db.pool, spec).await.expect("create");
    assert_eq!(row.id, id);
    assert_eq!(row.shape, "line");
    db.cleanup().await;
}

#[tokio::test]
async fn create_sets_expires_at_from_ttl_ms() {
    let db = require_db!("create_sets_expires_at_from_ttl_ms");
    let mut spec = annotation("pin", pin(3.0, 4.0));
    spec.ttl_ms = Some(60_000);
    let row = create_annotation(&db.pool, spec).await.expect("create");
    assert_eq!(row.ttl_ms, Some(60_000));
    let expires = row.expires_at.expect("ttl_ms implies expires_at");
    let delta = (expires - row.created_at).num_seconds();
    assert!(
        (58..=62).contains(&delta),
        "expires_at must be created_at + 60s (±clock skew), got {delta}s"
    );
    db.cleanup().await;
}

// ── list ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_filters_expired() {
    let db = require_db!("list_filters_expired");
    let live = create_annotation(&db.pool, annotation("pin", pin(5.0, 5.0)))
        .await
        .expect("create live");
    let mut expired = annotation("pin", pin(6.0, 6.0));
    expired.ttl_ms = Some(-1_000); // expires_at = now - 1s
    let dead = create_annotation(&db.pool, expired).await.expect("create dead");
    assert!(dead.expires_at.is_some());

    let rows = list_annotations(&db.pool, None, None, 100).await.expect("list");
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    assert!(ids.contains(&live.id), "live row must be listed");
    assert!(!ids.contains(&dead.id), "expired row must be filtered out");
    db.cleanup().await;
}

#[tokio::test]
async fn list_since_filter() {
    let db = require_db!("list_since_filter");
    let fresh = create_annotation(&db.pool, annotation("pin", pin(7.0, 7.0)))
        .await
        .expect("create fresh");
    let old = create_annotation(&db.pool, annotation("pin", pin(8.0, 8.0)))
        .await
        .expect("create old");
    // created_at is DB-owned (DEFAULT now()); backdate it directly.
    sqlx::query(r#"UPDATE annotations_v1 SET created_at = now() - interval '30 minutes' WHERE id = $1"#)
        .bind(old.id)
        .execute(&db.pool)
        .await
        .expect("backdate");

    let since = Utc::now() - Duration::minutes(15);
    let rows = list_annotations(&db.pool, Some(since), None, 100).await.expect("list");
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    assert!(ids.contains(&fresh.id), "row created now must survive since=now-15m");
    assert!(!ids.contains(&old.id), "row backdated 30m must be filtered by since=now-15m");
    db.cleanup().await;
}

#[tokio::test]
async fn list_bbox_filter() {
    let db = require_db!("list_bbox_filter");
    let inside = create_annotation(&db.pool, annotation("pin", pin(5.0, 5.0)))
        .await
        .expect("create inside");
    let outside = create_annotation(&db.pool, annotation("pin", pin(50.0, 50.0)))
        .await
        .expect("create outside");

    let bbox = BBox { south: 0.0, west: 0.0, north: 10.0, east: 10.0 };
    let rows = list_annotations(&db.pool, None, Some(bbox), 100).await.expect("list bbox");
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    assert!(ids.contains(&inside.id), "vertex inside the window must be listed");
    assert!(!ids.contains(&outside.id), "vertex outside the window must be filtered out");

    // No bbox → both visible (sanity: the filter, not the data, did the work).
    let all = list_annotations(&db.pool, None, None, 100).await.expect("list all");
    assert_eq!(all.len(), 2);
    db.cleanup().await;
}

// ── patch ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn patch_label_and_color() {
    let db = require_db!("patch_label_and_color");
    let mut spec = annotation("area", json!({ "vertices": [
        { "lon": 0.0, "lat": 0.0 }, { "lon": 1.0, "lat": 0.0 }, { "lon": 1.0, "lat": 1.0 }
    ]}));
    spec.label = Some("draft".to_string());
    let row = create_annotation(&db.pool, spec).await.expect("create");

    let patched = patch_annotation(
        &db.pool,
        row.id,
        AnnotationPatch {
            label: Some(Some("alpha".to_string())),
            color: Some("red".to_string()),
            ttl_ms: None,
        },
    )
    .await
    .expect("patch")
    .expect("row exists");

    assert_eq!(patched.label.as_deref(), Some("alpha"));
    assert_eq!(patched.color, "red");
    assert_eq!(patched.geometry, row.geometry, "geometry must never change on PATCH");
    assert_eq!(patched.created_at, row.created_at, "created_at is immutable");
    assert_eq!(patched.ttl_ms, None);
    db.cleanup().await;
}

#[tokio::test]
async fn patch_clear_label_using_double_option() {
    let db = require_db!("patch_clear_label_using_double_option");
    let mut spec = annotation("pin", pin(9.0, 9.0));
    spec.label = Some("keep".to_string());
    let row = create_annotation(&db.pool, spec).await.expect("create");
    assert_eq!(row.label.as_deref(), Some("keep"));

    // Some(None) = explicit JSON null → clear the column.
    let cleared = patch_annotation(
        &db.pool,
        row.id,
        AnnotationPatch { label: Some(None), color: None, ttl_ms: None },
    )
    .await
    .expect("patch clear")
    .expect("row exists");
    assert_eq!(cleared.label, None, "Some(None) must clear label");

    let stored = fetch(&db.pool, row.id).await;
    assert_eq!(stored.label, None);
    db.cleanup().await;
}

// ── delete ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn delete_returns_true_then_false() {
    let db = require_db!("delete_returns_true_then_false");
    let row = create_annotation(&db.pool, annotation("pin", pin(11.0, 11.0)))
        .await
        .expect("create");
    assert!(
        delete_annotation(&db.pool, row.id).await.expect("first delete"),
        "first delete must report a removed row"
    );
    assert!(
        !delete_annotation(&db.pool, row.id).await.expect("second delete"),
        "second delete must report not-found"
    );
    assert!(get_annotation(&db.pool, row.id).await.expect("get").is_none());
    db.cleanup().await;
}
