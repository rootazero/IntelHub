//! GEV P8: `/api/v1/annotations/*` REST endpoints.
//!
//! Routes (all under the `/api/*` prefix that `auth::auth_middleware`
//! protects at the server level — see `server.rs`):
//!
//! - `GET    /api/v1/annotations`        — list (since / bbox / limit)
//! - `POST   /api/v1/annotations`        — create (Bearer)
//! - `GET    /api/v1/annotations/{id}`   — single row
//! - `PATCH  /api/v1/annotations/{id}`   — partial update, label/color/ttl_ms (Bearer)
//! - `DELETE /api/v1/annotations/{id}`   — hard delete (Bearer)
//!
//! # Auth
//!
//! In production every `/api/*` request is already authenticated by
//! `auth::auth_middleware` (server.rs) before it reaches a handler, and the
//! resolved `AgentIdentity` is injected into request extensions. The
//! `require_bearer` guard here is a second, handler-local gate for the write
//! verbs: it re-resolves `Authorization: Bearer ihk_*` against `api_keys` so
//! the write contract still holds if this module is ever mounted outside the
//! main app (e.g. a test router or a future split). Reads stay unauthenticated
//! at the handler level and rely on the outer middleware.
//!
//! The plan sketched a `auth::require_bearer(&AppState, &HeaderMap)` helper;
//! no such function exists in the crate (`auth` exposes `bearer_token` +
//! `authenticate`, and `store::resolve_key` is the pool-level primitive), so
//! this module owns a small pool-scoped equivalent instead.

use std::sync::Arc;

use axum::{
    extract::{FromRef, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::annotations::{self, AnnotationInsert, AnnotationPatch, BBox};
use crate::state::AppState;

/// Default page size for `GET /api/v1/annotations` when `?limit=` is absent.
/// The DB layer clamps to `1..=500` regardless.
const DEFAULT_LIST_LIMIT: i64 = 100;

/// Shapes the engine can draw; mirrors the `annotations_v1.shape` CHECK.
const VALID_SHAPES: [&str; 3] = ["pin", "line", "area"];

/// Substates the annotation handlers depend on: just the Postgres pool.
///
/// Production routers carry `Arc<AppState>`; the integration tests carry a
/// per-test `PgPool`. Extracting only the pool keeps the handlers exercisable
/// end-to-end (real HTTP + real PG) without a live Redis/Neo4j, which the full
/// `AppState` would require.
#[derive(Clone)]
pub struct AnnotationsState(pub PgPool);

impl FromRef<Arc<AppState>> for AnnotationsState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        AnnotationsState(state.pg.clone())
    }
}

/// Query string for the list endpoint. `since` is kept as a raw `String`
/// because `serde_urlencoded` (axum's `Query` backend) has no built-in
/// `DateTime` support — `parse_since` handles the formats below.
#[derive(Debug, Deserialize, Default)]
pub struct ListParams {
    /// RFC3339, unix seconds, `now`, or a relative lookback like `-30m`/`-24h`.
    pub since: Option<String>,
    /// `south,west,north,east`.
    pub bbox: Option<String>,
    /// Max rows; DB clamps to `1..=500`.
    pub limit: Option<i64>,
}

/// Wire body for `PATCH /api/v1/annotations/{id}`.
///
/// The db layer's `AnnotationPatch` uses the double-`Option` idiom to tell
/// "field absent" (`None`) from "explicit JSON null" (`Some(None)`), but
/// serde's blanket `Deserialize for Option<T>` collapses JSON `null` to `None`
/// — making the two indistinguishable. Each nullable field therefore gets
/// `#[serde(default, deserialize_with = "double_option")]`, which keeps the
/// `None` default for an absent key while wrapping any present value (including
/// `null`) in `Some`. Converted into the db-layer type via `From`.
#[derive(Debug, Default, Deserialize)]
pub struct PatchBody {
    #[serde(default, deserialize_with = "double_option")]
    pub label: Option<Option<String>>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub ttl_ms: Option<Option<i32>>,
}

impl From<PatchBody> for AnnotationPatch {
    fn from(b: PatchBody) -> Self {
        AnnotationPatch { label: b.label, color: b.color, ttl_ms: b.ttl_ms }
    }
}

/// `Some(inner)` for any present field value (including JSON `null`), so the
/// outer `Option` records presence and the inner one records null-vs-value.
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

/// Route table for the five annotation endpoints.
///
/// Generic over the router state `S` so the same table can be merged into the
/// production `Router<Arc<AppState>>` and mounted on a bare `PgPool` in tests.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AnnotationsState: FromRef<S>,
{
    Router::new()
        .route("/api/v1/annotations", get(list_handler).post(create_handler))
        .route(
            "/api/v1/annotations/{id}",
            get(get_handler).patch(patch_handler).delete(delete_handler),
        )
}

// ---------- handlers ----------

pub async fn list_handler(
    State(st): State<AnnotationsState>,
    Query(params): Query<ListParams>,
) -> Response {
    let since = match params.since.as_deref().map(parse_since).transpose() {
        Ok(v) => v,
        Err(msg) => return err_json(StatusCode::BAD_REQUEST, "invalid_since", msg),
    };
    let bbox = match params.bbox.as_deref().map(parse_bbox).transpose() {
        Ok(v) => v,
        Err(msg) => return err_json(StatusCode::BAD_REQUEST, "invalid_bbox", msg),
    };
    let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT);
    match annotations::list_annotations(&st.0, since, bbox, limit).await {
        Ok(rows) => Json(json!({ "annotations": rows })).into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
}

pub async fn create_handler(
    State(st): State<AnnotationsState>,
    headers: HeaderMap,
    Json(spec): Json<AnnotationInsert>,
) -> Response {
    if let Err(resp) = require_bearer(&st.0, &headers).await {
        return resp;
    }
    if !VALID_SHAPES.contains(&spec.shape.as_str()) {
        return err_json(
            StatusCode::BAD_REQUEST,
            "invalid_shape",
            format!("shape must be one of pin/line/area, got {:?}", spec.shape),
        );
    }
    match annotations::create_annotation(&st.0, spec).await {
        Ok(row) => (StatusCode::CREATED, Json(row)).into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
}

pub async fn get_handler(State(st): State<AnnotationsState>, Path(id): Path<Uuid>) -> Response {
    match annotations::get_annotation(&st.0, id).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => not_found(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
}

pub async fn patch_handler(
    State(st): State<AnnotationsState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchBody>,
) -> Response {
    if let Err(resp) = require_bearer(&st.0, &headers).await {
        return resp;
    }
    match annotations::patch_annotation(&st.0, id, body.into()).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => not_found(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
}

pub async fn delete_handler(
    State(st): State<AnnotationsState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    if let Err(resp) = require_bearer(&st.0, &headers).await {
        return resp;
    }
    match annotations::delete_annotation(&st.0, id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, "db", e),
    }
}

// ---------- auth ----------

/// Handler-local Bearer gate for write verbs. Returns `Ok(())` only when the
/// presented key resolves to a live, non-revoked `api_keys` row.
///
/// See the module doc: production also authenticates via `auth_middleware`
/// before the handler runs; this guard is deliberate defense-in-depth.
async fn require_bearer(pool: &PgPool, headers: &HeaderMap) -> Result<(), Response> {
    let Some(token) = crate::auth::bearer_token(headers) else {
        return Err(unauthorized("write operations require an Authorization: Bearer <ihk_...> header"));
    };
    if token.is_empty() {
        return Err(unauthorized("bearer token is empty"));
    }
    match crate::store::resolve_key(pool, &token).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(unauthorized("unknown or revoked api key")),
        Err(e) => Err(err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            "db",
            format!("auth lookup failed: {e}"),
        )),
    }
}

fn unauthorized(msg: &str) -> Response {
    err_json(StatusCode::UNAUTHORIZED, "unauthorized", msg)
}

// ---------- query parsing (pure; directly unit-tested) ----------

/// Parse `?bbox=south,west,north,east`.
///
/// Rejects malformed input instead of silently dropping the filter — a typo
/// that turns a filtered query into an unfiltered one is a data-correctness
/// hazard, not a convenience.
pub fn parse_bbox(s: &str) -> Result<BBox, String> {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(format!("bbox must be 'south,west,north,east' (4 numbers), got {s:?}"));
    }
    let mut nums = [0f64; 4];
    for (i, p) in parts.iter().enumerate() {
        nums[i] = p
            .parse::<f64>()
            .map_err(|_| format!("bbox component {p:?} is not a number"))?;
    }
    let [south, west, north, east] = nums;
    if !nums.iter().all(|v| v.is_finite()) {
        return Err(format!("bbox components must be finite numbers, got {s:?}"));
    }
    if south > north {
        return Err(format!("bbox south ({south}) must be <= north ({north})"));
    }
    if west > east {
        return Err(format!("bbox west ({west}) must be <= east ({east})"));
    }
    Ok(BBox { south, west, north, east })
}

/// Parse `?since=`. Accepts:
/// - `now`
/// - relative lookback `-<n><s|m|h|d>` (e.g. `-30m`, `-24h`, `-2d`)
/// - RFC3339 (`2026-09-18T00:00:00Z`, offsets allowed)
/// - unix seconds
pub fn parse_since(s: &str) -> Result<DateTime<Utc>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("since must not be empty".to_string());
    }
    if t.eq_ignore_ascii_case("now") {
        return Ok(Utc::now());
    }
    if let Some(rest) = t.strip_prefix('-') {
        let split = rest
            .find(|c: char| !c.is_ascii_digit())
            .ok_or_else(|| format!("relative since must end in s/m/h/d, got {t:?}"))?;
        let (num, unit) = rest.split_at(split);
        if num.is_empty() {
            return Err(format!("relative since is missing a number: {t:?}"));
        }
        let n: i64 = num
            .parse()
            .map_err(|_| format!("invalid relative since number: {t:?}"))?;
        let dur = match unit {
            "s" => Duration::seconds(n),
            "m" => Duration::minutes(n),
            "h" => Duration::hours(n),
            "d" => Duration::days(n),
            other => {
                return Err(format!("unknown relative unit {other:?} (use s/m/h/d) in {t:?}"))
            }
        };
        return Ok(Utc::now() - dur);
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(t) {
        return Ok(dt.with_timezone(&Utc));
    }
    if t.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(secs) = t.parse::<i64>() {
            if let Some(dt) = Utc.timestamp_opt(secs, 0).single() {
                return Ok(dt);
            }
        }
    }
    Err(format!(
        "since must be RFC3339, unix seconds, 'now', or '-<n><s|m|h|d>', got {t:?}"
    ))
}

// ---------- responses ----------

fn not_found() -> Response {
    err_json(StatusCode::NOT_FOUND, "missing", "annotation not found")
}

fn err_json(status: StatusCode, code: &str, msg: impl std::fmt::Display) -> Response {
    (status, Json(json!({ "error": code, "message": msg.to_string() }))).into_response()
}
