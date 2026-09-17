//! GEV P3 T5+T6: traffic domain REST endpoints.
//!
//! Three routes under `/api/v1/gev/` (the console adapter's T7 path-rewrite
//! maps the engine's `/api/overpass` + `/api/tomtom/*` onto these):
//!
//! - `POST /api/v1/gev/overpass` — Overpass QL proxy. Form body (`data=`),
//!   `[out:json]` whitelist, Redis cache 300s, upstream failure passthrough.
//! - `GET  /api/v1/gev/tomtom/status` — `{"hasKey": bool}`, always 200.
//! - `GET  /api/v1/gev/tomtom/flow/{z}/{x}/{y}` — TomTom flow MVT tile
//!   (the engine's `.pbf` suffix is stripped by the console adapter — axum
//!   forbids a second segment param; upstream TomTom URL keeps its .pbf)
//!   proxy, zoom 8..16, Redis cache 120s (aligned with the engine's
//!   flowSource TTL), key never leaves the server.
//!
//! Contracts: `.superpowers/sdd/2026-09-17-gev-p3/contracts.md` §2. The
//! engine reads `x-overpass-cache` / `x-overpass-upstream` for timing
//! (ingestion.js:92-93) — both are always set on overpass responses.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::state::AppState;

pub const OVERPASS_CACHE_PREFIX: &str = "hub:gev:overpass:";
pub const OVERPASS_CACHE_TTL_SECS: u64 = 300;
pub const FLOW_CACHE_PREFIX: &str = "hub:gev:tomtom:flow:";
pub const FLOW_CACHE_TTL_SECS: u64 = 120;

const DEFAULT_OVERPASS_URL: &str = "https://overpass-api.de/api/interpreter";
const OVERPASS_TIMEOUT_SECS: u64 = 30;
/// Abuse guard alongside the `[out:json]` whitelist: a bbox road query is a
/// few hundred bytes; anything bigger is not a roads query.
const MAX_OVERPASS_BODY_BYTES: usize = 16 * 1024;

/// Redis GET/SET budget, same 2s degrade-to-None budget the GEV P2 starlink
/// proxy uses.
const REDIS_BUDGET_MS: u64 = 2000;

/// Shared HTTP client for these upstreams: honest UA (overpass-api.de rate
/// policy distinguishes generic scripts), 30s timeout (contract: overpass
/// timeout is 30s — the engine sends `[timeout:25]` upstream of that).
fn traffic_http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(OVERPASS_TIMEOUT_SECS))
            .user_agent("intelhub-gev-traffic/1.0")
            .build()
            .expect("traffic http client build")
    })
}

fn gev_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// reqwest's `Error` Display embeds the full request URL — for the TomTom
/// flow proxy that URL carries `?key=<SECRET>`, so a naive `%e` logging field
/// would write the API key into hub-core's log (T5+T6 review M1). Strip the
/// URL before logging. Applied uniformly (overpass too) so no log point has
/// to reason about which URLs are secret-bearing.
fn redact_reqwest_error(e: reqwest::Error) -> String {
    e.without_url().to_string()
}

// ---------- T6a: TomTom key resolution (opensky.rs env-gate precedent) ----------

/// `HUB_TOMTOM_API_KEY` wins over the bare name when both are set; empty
/// values are ignored (secrets.env templates ship keys with empty values —
/// see the 410-empty-keys incident).
pub fn tomtom_api_key() -> Option<String> {
    std::env::var("HUB_TOMTOM_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("TOMTOM_API_KEY").ok().filter(|s| !s.is_empty()))
}

pub async fn gev_tomtom_status() -> Json<serde_json::Value> {
    Json(json!({ "hasKey": tomtom_api_key().is_some() }))
}

// ---------- T6b: flow tile proxy ----------

/// Web Mercator XYZ validity, mirroring the engine's `isValidTileCoord`
/// (gev-engine/src/data/tomtomTiles.js): integer z in [8,16], x/y in
/// [0, 2^z).
pub fn valid_flow_tile(z: i64, x: i64, y: i64) -> bool {
    if !(8..=16).contains(&z) {
        return false;
    }
    let max = 1_i64 << z;
    (0..max).contains(&x) && (0..max).contains(&y)
}

/// TomTom Traffic Flow Tiles endpoint. Scheme confirmed against the vendor
/// tree (gev-engine/src/data/tomtomTiles.js file header: "the scheme TomTom's
/// `traffic/map/4/tile/flow` endpoints use"). `relative0` = absolute speed
/// rendered relative to free-flow; `thickness=10` matches the engine's road
/// polyline budget. The key is query-param material for TomTom and must
/// never appear in any hub response — it only ever exists in this URL.
pub fn flow_tile_upstream_url(key: &str, z: i64, x: i64, y: i64) -> String {
    format!(
        "https://api.tomtom.com/traffic/map/4/tile/flow/relative0/{z}/{x}/{y}.pbf?key={key}&thickness=10"
    )
}

pub fn flow_cache_key(z: i64, x: i64, y: i64) -> String {
    format!("{FLOW_CACHE_PREFIX}{z}/{x}/{y}")
}

/// 503 body when no TomTom key is configured — factored out so the shape is
/// unit-testable without an AppState.
pub fn flow_not_configured_response() -> Response {
    gev_err(StatusCode::SERVICE_UNAVAILABLE, "tomtom not configured")
}

fn mvt_response(bytes: Bytes, content_type: Option<&str>, cache_header: &'static str) -> Response {
    let mut hm = HeaderMap::new();
    hm.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_str(content_type.unwrap_or("application/vnd.mapbox-vector-tile"))
            .unwrap_or(HeaderValue::from_static("application/vnd.mapbox-vector-tile")),
    );
    hm.insert("x-tomtom-cache", HeaderValue::from_static(cache_header));
    (hm, bytes).into_response()
}

pub async fn gev_tomtom_flow(
    State(state): State<Arc<AppState>>,
    Path((z, x, y)): Path<(i64, i64, i64)>,
) -> Result<Response, Response> {
    if !valid_flow_tile(z, x, y) {
        return Err(gev_err(StatusCode::BAD_REQUEST, "invalid tile coordinate"));
    }
    let key = match tomtom_api_key() {
        Some(k) => k,
        None => return Err(flow_not_configured_response()),
    };
    let cache_key = flow_cache_key(z, x, y);
    let cached: Option<Vec<u8>> = state
        .redis_timed(redis::cmd("GET").arg(&cache_key).clone(), REDIS_BUDGET_MS)
        .await;
    if let Some(bytes) = cached {
        return Ok(mvt_response(Bytes::from(bytes), None, "hit"));
    }
    let url = flow_tile_upstream_url(&key, z, x, y);
    let resp = match traffic_http().get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %redact_reqwest_error(e), z, x, y, "tomtom flow upstream fetch failed");
            return Err(gev_err(StatusCode::BAD_GATEWAY, "tomtom upstream failed"));
        }
    };
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body = resp.bytes().await.map_err(|e| {
        tracing::warn!(error = %redact_reqwest_error(e), "tomtom flow body read failed");
        gev_err(StatusCode::BAD_GATEWAY, "tomtom upstream failed")
    })?;
    if !status.is_success() {
        // Passthrough upstream status + error body — mirrors the overpass
        // contract (client treats non-2xx per tile and tolerates partial
        // failure, flowSource.js:78-83).
        tracing::warn!(status = %status, "tomtom flow upstream returned error");
        return Ok((status, body).into_response());
    }
    let bytes = Bytes::from(body.to_vec());
    let _: Option<String> = state
        .redis_timed(
            redis::cmd("SETEX")
                .arg(&cache_key)
                .arg(FLOW_CACHE_TTL_SECS)
                .arg(bytes.as_ref())
                .clone(),
            REDIS_BUDGET_MS,
        )
        .await;
    Ok(mvt_response(bytes, content_type.as_deref(), "miss"))
}

// ---------- T5: overpass proxy ----------

pub fn overpass_upstream_url() -> String {
    std::env::var("OVERPASS_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_OVERPASS_URL.to_string())
}

/// Ordered endpoint list for the proxy: env override (single) or the
/// shared fallback chain (installations.rs owns the list — 2026-09-17
/// overpass-api.de Apache-406 blocks our egress; mirrors probed 200 from
/// the 315 egress). An Apache 406 / transport error falls through to the
/// next mirror; other non-2xx (e.g. 429 rate limit, 400 bad query) are
/// per-contract passthroughs.
pub fn overpass_endpoints() -> Vec<String> {
    for var in ["HUB_OVERPASS_URL", "OVERPASS_URL"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return vec![v.to_string()];
            }
        }
    }
    crate::monitor::sources::installations::endpoints()
}

/// Host portion of the upstream URL for the `x-overpass-upstream` header
/// (engine reads it for timing). Falls back to the raw string when the URL
/// doesn't parse.
fn upstream_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| url.to_string())
}

/// Validate + extract the Overpass QL query from a form body.
///
/// Contract (§2): the body must be `application/x-www-form-urlencoded` with
/// a leading `data=` field; after percent-decoding the query must start with
/// `[out:json]` (whitelist — this proxy only serves the engine's road
/// queries, never arbitrary QL). Returns the decoded query.
pub fn validate_overpass_form(body: &str) -> Result<String, &'static str> {
    if body.len() > MAX_OVERPASS_BODY_BYTES {
        return Err("overpass query too large");
    }
    if !body.starts_with("data=") {
        return Err("form body must start with data=");
    }
    let query = url::form_urlencoded::parse(body.as_bytes())
        .find(|(k, _)| k == "data")
        .map(|(_, v)| v.into_owned())
        .ok_or("missing data= field")?;
    if !query.trim_start().starts_with("[out:json]") {
        return Err("only [out:json] queries are allowed");
    }
    Ok(query)
}

/// Cache key = sha256 hex of the decoded query body (contracts.md §2).
pub fn overpass_cache_key(query: &str) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(query.as_bytes());
    format!("{OVERPASS_CACHE_PREFIX}{:x}", hasher.finalize())
}

/// Upstream failure passthrough: same status code, upstream body verbatim.
/// Factored out so the shape is unit-testable.
pub fn upstream_error_response(status: StatusCode, body: Bytes) -> Response {
    (status, body).into_response()
}

fn overpass_response(
    bytes: Bytes,
    content_type: Option<&str>,
    cache: &'static str,
    upstream_host: &str,
) -> Response {
    let mut hm = HeaderMap::new();
    hm.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_str(content_type.unwrap_or("application/json"))
            .unwrap_or(HeaderValue::from_static("application/json")),
    );
    hm.insert("x-overpass-cache", HeaderValue::from_static(cache));
    hm.insert(
        "x-overpass-upstream",
        HeaderValue::from_str(upstream_host).unwrap_or(HeaderValue::from_static("unknown")),
    );
    (hm, bytes).into_response()
}

pub async fn gev_overpass(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Response> {
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !ct.starts_with("application/x-www-form-urlencoded") {
        return Err(gev_err(
            StatusCode::BAD_REQUEST,
            "content-type must be application/x-www-form-urlencoded",
        ));
    }
    let body_str = std::str::from_utf8(&body)
        .map_err(|_| gev_err(StatusCode::BAD_REQUEST, "form body must be utf-8"))?;
    let query = validate_overpass_form(body_str)
        .map_err(|m| gev_err(StatusCode::BAD_REQUEST, m))?;

    let cache_key = overpass_cache_key(&query);
    let upstream = overpass_upstream_url();
    let host = upstream_host(&upstream);
    let cached: Option<Vec<u8>> = state
        .redis_timed(redis::cmd("GET").arg(&cache_key).clone(), REDIS_BUDGET_MS)
        .await;
    if let Some(bytes) = cached {
        return Ok(overpass_response(
            Bytes::from(bytes),
            Some("application/json"),
            "hit",
            &host,
        ));
    }

    // Walk the mirror chain: transport errors and Apache-level 406 egress
    // blocks fall through; genuine API errors (400/429/…) passthrough.
    let mut last_transport: Option<String> = None;
    let mut resp = None;
    let mut used_host = host.clone();
    for url in &overpass_endpoints() {
        match traffic_http()
            .post(url)
            .header(
                axum::http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(query.clone())
            .send()
            .await
        {
            Ok(r) if r.status().as_u16() == 406 => {
                tracing::warn!(endpoint = %upstream_host(url), "overpass 406 egress block — next mirror");
                last_transport = Some(format!("HTTP 406 ({})", upstream_host(url)));
            }
            Ok(r) => {
                used_host = upstream_host(url);
                resp = Some(r);
                break;
            }
            Err(e) => {
                let redacted = redact_reqwest_error(e);
                tracing::warn!(error = %redacted, endpoint = %upstream_host(url), "overpass mirror transport failed — next");
                last_transport = Some(redacted);
            }
        }
    }
    let Some(resp) = resp else {
        return Err(gev_err(
            StatusCode::BAD_GATEWAY,
            "overpass upstream failed on all mirrors",
        ));
    };
    let host = used_host;
    let _ = last_transport;
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = resp.bytes().await.map_err(|e| {
        tracing::warn!(error = %redact_reqwest_error(e), "overpass body read failed");
        gev_err(StatusCode::BAD_GATEWAY, "overpass upstream failed")
    })?;
    if !status.is_success() {
        // Contract: upstream non-2xx passes status + error body through
        // (engine reports "Overpass API returned {status}").
        tracing::warn!(status = %status, "overpass upstream returned error");
        return Ok(upstream_error_response(status, Bytes::from(bytes.to_vec())));
    }
    let bytes = Bytes::from(bytes.to_vec());
    let _: Option<String> = state
        .redis_timed(
            redis::cmd("SETEX")
                .arg(&cache_key)
                .arg(OVERPASS_CACHE_TTL_SECS)
                .arg(bytes.as_ref())
                .clone(),
            REDIS_BUDGET_MS,
        )
        .await;
    Ok(overpass_response(bytes, content_type.as_deref(), "miss", &host))
}

#[cfg(test)]
mod tests {
    use super::redact_reqwest_error;

    /// T5+T6 review M1 regression: reqwest's `Error` Display embeds the full
    /// request URL. For the TomTom flow proxy that URL carries
    /// `?key=<SECRET>` — a naive `%e` log field would write the key into
    /// hub-core's log. `redact_reqwest_error` (without_url) must strip it.
    ///
    /// The error is built deterministically against 127.0.0.1:1 (connection
    /// refused — no external network), which still attaches the request URL
    /// to the error, exactly like a real upstream failure would.
    #[tokio::test]
    async fn reqwest_error_redaction_drops_key_and_url() {
        let url = "http://127.0.0.1:1/traffic/map/4/tile/flow/relative0/12/100/200.pbf?key=SECRET_TOMTOM_KEY&thickness=10";
        let err = reqwest::Client::builder()
            .build()
            .expect("client builds")
            .get(url)
            .send()
            .await
            .expect_err("connection to 127.0.0.1:1 must fail");

        // Sanity: this error shape really does carry the key-bearing URL —
        // without this the redaction assertion below would be vacuous.
        assert!(err.url().is_some(), "reqwest error must carry the request URL");
        let raw = err.to_string();
        assert!(
            raw.contains("SECRET_TOMTOM_KEY"),
            "sanity: naive Display leaks the key (found: {raw})"
        );

        let redacted = redact_reqwest_error(err);
        assert!(
            !redacted.contains("SECRET_TOMTOM_KEY"),
            "redacted error must not contain the TomTom key (found: {redacted})"
        );
        assert!(
            !redacted.contains("127.0.0.1"),
            "redacted error must not contain the request URL at all (found: {redacted})"
        );
    }
}
