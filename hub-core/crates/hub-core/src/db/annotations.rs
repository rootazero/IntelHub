//! GEV P8: `annotations_v1` CRUD + bbox query.
//!
//! World-anchored marks drawn on the globe (pin / line / area) are stored here
//! so they survive across sessions — a deliberate step past the vendored
//! engine's in-memory annotation store. The `geometry` column is the engine's
//! own annotation geometry shape, stored verbatim:
//!
//! ```json
//! { "vertices": [ { "lon": 12.3, "lat": 45.6, "height": 0 } ] }
//! ```
//!
//! TTL semantics: `ttl_ms = NULL` means persistent; otherwise `expires_at` is
//! derived at insert/patch time (`now() + ttl_ms`) and the DB CHECK constraint
//! keeps the two columns in lockstep. Reads filter out rows whose `expires_at`
//! has passed, but they are not deleted here (GC is out of scope for P8).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use uuid::Uuid;

/// One persisted annotation, as returned to the REST layer.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AnnotationRow {
    pub id: Uuid,
    pub agent_id: Option<String>,
    pub shape: String,
    pub label: Option<String>,
    pub color: String,
    pub geometry: JsonValue,
    pub ttl_ms: Option<i32>,
    pub meta: JsonValue,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Create payload (REST `POST /api/v1/annotations` body). `id` is optional —
/// the client may supply a stable id (offline retry) or let the DB mint one.
#[derive(Debug, Deserialize)]
pub struct AnnotationInsert {
    pub id: Option<Uuid>,
    pub agent_id: Option<String>,
    pub shape: String,
    pub label: Option<String>,
    pub color: Option<String>,
    pub geometry: JsonValue,
    pub ttl_ms: Option<i32>,
    pub meta: Option<JsonValue>,
}

/// Partial-update payload (REST `PATCH /api/v1/annotations/{id}` body).
///
/// `label` / `ttl_ms` use the double-Option idiom: the outer `Option` says
/// "field present in the request?", the inner says "value or explicit null".
/// `Some(None)` clears the column, `None` leaves it untouched. `geometry` is
/// deliberately absent — geometry is immutable, change means delete+recreate.
#[derive(Debug, Deserialize, Default)]
pub struct AnnotationPatch {
    pub label: Option<Option<String>>,
    pub color: Option<String>,
    pub ttl_ms: Option<Option<i32>>,
}

/// Axis-aligned query window. `south <= north`, `west <= east`.
#[derive(Debug, Deserialize)]
pub struct BBox {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

/// Build the bbox predicate over the engine geometry's vertex list.
///
/// The P8 plan sketched this as `geometry @> jsonb_build_object('south', ...)`,
/// but the pinned geometry contract (design §3.1) is `{vertices:[{lon,lat}]}`
/// — there are no bbox keys on the row. The filter therefore tests whether any
/// vertex falls inside the window, which is what the REST `?bbox=` param and
/// the planned tests mean by "annotation is in the box". The GIN index stays
/// useful for containment probes but this scan does not depend on it (P9 perf
/// decision, design §260).
const BBOX_PREDICATE: &str = r#"
              AND EXISTS (
                  SELECT 1
                  FROM jsonb_array_elements(geometry->'vertices') AS v
                  WHERE (v->>'lon')::float8 BETWEEN $2 AND $4
                    AND (v->>'lat')::float8 BETWEEN $3 AND $5
              )"#;

const SELECT_COLS: &str = "id, agent_id, shape, label, color, geometry, ttl_ms, meta, created_at, expires_at";

/// Insert one annotation, returning the stored row (server-side defaults and
/// `created_at` / `expires_at` resolved).
pub async fn create_annotation(
    pool: &PgPool,
    spec: AnnotationInsert,
) -> Result<AnnotationRow, sqlx::Error> {
    let id = spec.id.unwrap_or_else(Uuid::new_v4);
    let color = spec.color.unwrap_or_else(|| "primary".to_string());
    let meta = spec.meta.unwrap_or_else(|| serde_json::json!({}));
    let expires_at =
        spec.ttl_ms.map(|ttl| Utc::now() + chrono::Duration::milliseconds(ttl as i64));
    sqlx::query_as::<_, AnnotationRow>(
        r#"
        INSERT INTO annotations_v1 (id, agent_id, shape, label, color, geometry, ttl_ms, meta, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id, agent_id, shape, label, color, geometry, ttl_ms, meta, created_at, expires_at
        "#,
    )
    .bind(id)
    .bind(spec.agent_id)
    .bind(&spec.shape)
    .bind(spec.label)
    .bind(color)
    .bind(&spec.geometry)
    .bind(spec.ttl_ms)
    .bind(&meta)
    .bind(expires_at)
    .fetch_one(pool)
    .await
}

/// Fetch one annotation by id (`None` = no such row).
pub async fn get_annotation(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<AnnotationRow>, sqlx::Error> {
    sqlx::query_as::<_, AnnotationRow>(
        r#"
        SELECT id, agent_id, shape, label, color, geometry, ttl_ms, meta, created_at, expires_at
        FROM annotations_v1 WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// List live annotations created at/after `since` (default: last 24h),
/// optionally restricted to a lon/lat window, newest first.
pub async fn list_annotations(
    pool: &PgPool,
    since: Option<DateTime<Utc>>,
    bbox: Option<BBox>,
    limit: i64,
) -> Result<Vec<AnnotationRow>, sqlx::Error> {
    let since = since.unwrap_or_else(|| Utc::now() - chrono::Duration::hours(24));
    let limit = limit.clamp(1, 500);
    if let Some(b) = bbox {
        let sql = format!(
            r#"
            SELECT {SELECT_COLS}
            FROM annotations_v1
            WHERE created_at >= $1
              AND (expires_at IS NULL OR expires_at > now()){BBOX_PREDICATE}
            ORDER BY created_at DESC
            LIMIT $6
            "#
        );
        sqlx::query_as::<_, AnnotationRow>(&sql)
            .bind(since)
            .bind(b.west)
            .bind(b.south)
            .bind(b.east)
            .bind(b.north)
            .bind(limit)
            .fetch_all(pool)
            .await
    } else {
        let sql = format!(
            r#"
            SELECT {SELECT_COLS}
            FROM annotations_v1
            WHERE created_at >= $1
              AND (expires_at IS NULL OR expires_at > now())
            ORDER BY created_at DESC
            LIMIT $2
            "#
        );
        sqlx::query_as::<_, AnnotationRow>(&sql)
            .bind(since)
            .bind(limit)
            .fetch_all(pool)
            .await
    }
}

/// Apply a partial update. Returns `Ok(None)` when the id does not exist.
/// Only `label` / `color` / `ttl_ms` (+ derived `expires_at`) can change.
pub async fn patch_annotation(
    pool: &PgPool,
    id: Uuid,
    patch: AnnotationPatch,
) -> Result<Option<AnnotationRow>, sqlx::Error> {
    let clear_label = patch.label.as_ref().is_some_and(|o| o.is_none());
    let new_label = patch.label.and_then(|o| o);
    let clear_ttl = patch.ttl_ms.as_ref().is_some_and(|o| o.is_none());
    let new_ttl = patch.ttl_ms.and_then(|o| o);
    let expires_at = new_ttl.map(|ttl| Utc::now() + chrono::Duration::milliseconds(ttl as i64));

    sqlx::query_as::<_, AnnotationRow>(
        r#"
        UPDATE annotations_v1 SET
            label      = CASE WHEN $2 THEN NULL::text ELSE COALESCE($3, label) END,
            color      = COALESCE($4, color),
            ttl_ms     = CASE WHEN $5 THEN NULL::integer ELSE COALESCE($6, ttl_ms) END,
            expires_at = CASE
                WHEN $5 THEN NULL::timestamptz
                WHEN $6 IS NOT NULL THEN $7
                ELSE expires_at
            END
        WHERE id = $1
        RETURNING id, agent_id, shape, label, color, geometry, ttl_ms, meta, created_at, expires_at
        "#,
    )
    .bind(id)
    .bind(clear_label)
    .bind(new_label)
    .bind(patch.color)
    .bind(clear_ttl)
    .bind(new_ttl)
    .bind(expires_at)
    .fetch_optional(pool)
    .await
}

/// Hard-delete one annotation. `true` = a row was removed, `false` = not found.
pub async fn delete_annotation(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query("DELETE FROM annotations_v1 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected > 0)
}
