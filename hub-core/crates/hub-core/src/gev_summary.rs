//! GEV P9 T2: regional brief summary endpoint (spec §3.3 / plan Task 2).
//!
//! `GET /api/v1/gev/summary?entity_id`
//!
//! STUB (2026-09-18): the plan assumed `acled`/`reliefweb`/`gdelt` upstream
//! collectors already exist in hub-core — they do **not** (verified: no
//! `*acled*`/`*reliefweb*`/`*gdelt*` modules under `crates/hub-core/src/`).
//! Per plan Task 2 + spec §3.3 "空载防御", this returns 200 with
//! `bullets: []` + `sources: ["cache"]` and never invents fake upstream
//! calls (P8 lesson: 失败源保持可见 — degraded sources stay visible as a
//! user decision, we don't fake working aggregators).
//!
//! Cache: Redis `cockpit:summary:<entity_id>`, TTL 15 min (900s). On miss it
//! builds the stub and writes it, so the 200-with-empty-bullets shape is
//! stable and cheap for the console. Full aggregation (three-source parallel
//! fetch + dedupe + sort) is deferred until those upstreams stabilize.
//!
//! Response shape (spec §3.3 SummaryResponse): `entity_id`, `generated_at`
//! (RFC3339), `sources`, `bullets[]`, `next_refresh_after` (RFC3339,
//! generated_at + 15 min — the console uses it to schedule the next ask).

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::AppState;

pub const SUMMARY_CACHE_PREFIX: &str = "cockpit:summary:";
/// 15-minute Redis TTL (spec §3.3 + plan Task 2).
pub const SUMMARY_TTL_SECS: u64 = 900;
const REDIS_BUDGET_MS: u64 = 2000;
/// Cap entity_id length to keep Redis keys sane (also mirrors the geocode
/// query cap; entity ids like "ICAO:ZBAA" / "flight:UAL123" are short).
const MAX_ENTITY_ID_LEN: usize = 256;

#[derive(Deserialize)]
pub struct SummaryParams {
    entity_id: String,
}

fn gev_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn json_response(bytes: Vec<u8>, cache: &'static str) -> Response {
    let mut hm = HeaderMap::new();
    hm.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    hm.insert("x-summary-cache", HeaderValue::from_static(cache));
    (hm, bytes).into_response()
}

/// Redis cache key for a summary (spec §3.3: `cockpit:summary:<entity_id>`).
pub fn summary_cache_key(entity_id: &str) -> String {
    format!("{SUMMARY_CACHE_PREFIX}{entity_id}")
}

/// Build the empty-summary stub (no upstream sources exist yet). `sources`
/// is `["cache"]` per spec §3.3 空载防御 — signals "no upstream data" without
/// erroring, so the cockpit never retry-storms.
pub fn build_stub_summary(entity_id: &str, generated_at: &str, next_refresh_after: &str) -> Value {
    json!({
        "entity_id": entity_id,
        "generated_at": generated_at,
        "sources": ["cache"],
        "bullets": [],
        "next_refresh_after": next_refresh_after,
    })
}

pub async fn gev_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummaryParams>,
) -> Result<Response, Response> {
    let entity_id = params.entity_id.trim().to_string();
    if entity_id.is_empty() {
        return Err(gev_err(StatusCode::BAD_REQUEST, "entity_id required"));
    }
    if entity_id.chars().count() > MAX_ENTITY_ID_LEN {
        return Err(gev_err(StatusCode::BAD_REQUEST, "entity_id too long"));
    }

    let key = summary_cache_key(&entity_id);
    // Double Option: redis-rs maps Nil → Ok(vec![]) for Vec<u8>, so a single
    // Option would read every miss as hit+empty-body (GEV P3 lesson).
    let cached: Option<Option<Vec<u8>>> = state
        .redis_timed(redis::cmd("GET").arg(&key).clone(), REDIS_BUDGET_MS)
        .await;
    if let Some(Some(bytes)) = cached {
        return Ok(json_response(bytes, "hit"));
    }

    // No upstream sources exist yet → build the stub, cache it, return 200.
    let now = chrono::Utc::now();
    let generated_at = now.to_rfc3339();
    let next_refresh_after =
        (now + chrono::Duration::seconds(SUMMARY_TTL_SECS as i64)).to_rfc3339();
    let payload = build_stub_summary(&entity_id, &generated_at, &next_refresh_after);
    let bytes = serde_json::to_vec(&payload)
        .map_err(|e| gev_err(StatusCode::INTERNAL_SERVER_ERROR, &format!("encode: {e}")))?;
    let _: Option<String> = state
        .redis_timed(
            redis::cmd("SETEX")
                .arg(&key)
                .arg(SUMMARY_TTL_SECS)
                .arg(bytes.as_slice())
                .clone(),
            REDIS_BUDGET_MS,
        )
        .await;
    Ok(json_response(bytes, "miss"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_matches_spec_prefix() {
        assert_eq!(summary_cache_key("ICAO:ZBAA"), "cockpit:summary:ICAO:ZBAA");
        assert_eq!(
            summary_cache_key("flight:UAL123"),
            "cockpit:summary:flight:UAL123"
        );
    }

    #[test]
    fn stub_summary_has_contract_shape() {
        let v = build_stub_summary(
            "ICAO:ZBAA",
            "2026-09-18T00:00:00+00:00",
            "2026-09-18T00:15:00+00:00",
        );
        assert_eq!(v["entity_id"], "ICAO:ZBAA");
        assert_eq!(v["generated_at"], "2026-09-18T00:00:00+00:00");
        assert_eq!(v["sources"], json!(["cache"]));
        assert_eq!(v["bullets"], json!([]));
        assert_eq!(v["next_refresh_after"], "2026-09-18T00:15:00+00:00");
    }

    #[test]
    fn ttl_is_15_minutes() {
        assert_eq!(SUMMARY_TTL_SECS, 900);
    }
}
