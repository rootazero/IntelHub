//! Bearer-key auth: keys are SHA-256 hashed at rest, resolved to an
//! AgentIdentity inserted into request extensions. MCP tool handlers read it
//! back via RequestContext extensions → axum Parts (verified pattern in rmcp
//! official examples), giving unspoofable per-request attribution.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::state::AppState;
use crate::types::{AgentIdentity, RequestTrace};

pub fn hash_key(presented: &str) -> String {
    let mut h = Sha256::new();
    h.update(presented.as_bytes());
    hex_lower(&h.finalize())
}

pub fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    v.strip_prefix("Bearer ").map(|s| s.trim().to_string())
}

/// Resolve identity for a presented key. No caching layer in SP2A — the
/// lookup is a single indexed row read and Postgres absorbs it easily.
pub async fn authenticate(state: &AppState, presented: &str) -> Option<AgentIdentity> {
    let (agent_id, key_id, name) = crate::store::resolve_key(&state.pg, presented).await.ok()??;
    Some(AgentIdentity { agent_id, name, key_id })
}

/// Per-agent token bucket (Redis INCR + EXPIRE, 1-minute window).
pub async fn check_rate_limit(state: &AppState, agent: &AgentIdentity) -> bool {
    let limit = state.config.rate_limit_rpm as i64;
    let key = format!("hub:ratelimit:{}:{}", agent.agent_id, chrono::Utc::now().timestamp() / 60);
    let mut conn = state.redis.clone();
    let res: redis::RedisResult<(i64,)> = redis::pipe()
        .atomic()
        .cmd("INCR").arg(&key)
        .cmd("EXPIRE").arg(&key).arg(70).ignore()
        .query_async(&mut conn)
        .await;
    match res {
        Ok((n,)) => n <= limit,
        Err(_) => true, // redis down: degrade open, failure model §8 (budgets are SP2B)
    }
}

/// Axum middleware: authenticate + rate-limit + trace. Skipped for /healthz.
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }
    let Some(token) = bearer_token(req.headers()) else {
        return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response();
    };
    let Some(identity) = authenticate(&state, &token).await else {
        return (StatusCode::UNAUTHORIZED, "invalid api key").into_response();
    };
    if !check_rate_limit(&state, &identity).await {
        return (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response();
    }
    req.extensions_mut().insert(identity);
    req.extensions_mut().insert(RequestTrace::new());
    next.run(req).await
}
