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
    let (agent_id, key_id, name, tier_str) =
        crate::store::resolve_key(&state.pg, presented).await.ok()??;
    let tier = crate::config::Tier::parse(&tier_str).unwrap_or(crate::config::Tier::Free);
    let key_prefix = presented.chars().take(12).collect();
    Some(AgentIdentity { agent_id, name, key_id, admin: false, tier, key_prefix })
}

/// Constant-time-ish comparison for the admin token (no early exit on mismatch).
pub fn token_matches(configured: &str, presented: &str) -> bool {
    let (a, b) = (configured.as_bytes(), presented.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Per-agent token bucket (Redis INCR + EXPIRE, 1-minute window).
pub async fn check_rate_limit(state: &AppState, agent: &AgentIdentity) -> bool {
    let limit = state.config.rate_limit_rpm as i64;
    let key = format!("hub:ratelimit:{}:{}", agent.agent_id, chrono::Utc::now().timestamp() / 60);
    let mut conn = state.redis().await;
    let res: redis::RedisResult<(i64,)> = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        redis::pipe()
            .atomic()
            .cmd("INCR").arg(&key)
            .cmd("EXPIRE").arg(&key).arg(70).ignore()
            .query_async(&mut conn),
    )
    .await
    .unwrap_or_else(|_| Err(redis::RedisError::from((
        redis::ErrorKind::IoError,
        "ratelimit redis timeout",
    ))));
    match res {
        Ok((n,)) => n <= limit,
        Err(_) => true, // redis down: degrade open, failure model §8 (budgets are SP2B)
    }
}

/// Axum middleware: authenticate + rate-limit + trace. Only /api/* and /mcp
/// are protected; static console assets and /healthz are public (the SPA
/// shell holds no data — every data request still requires a key, §24).
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if path == "/healthz" || (!path.starts_with("/api/") && !path.starts_with("/mcp")) {
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
    // SP2B Level 3: X-Admin-Token upgrades this request to admin (flag only,
    // token value never stored/logged).
    let mut identity = identity;
    if let (Some(cfg), Some(hdr)) = (
        state.config.admin_token.as_deref(),
        req.headers().get("x-admin-token").and_then(|v| v.to_str().ok()),
    ) {
        identity.admin = token_matches(cfg, hdr);
    }
    req.extensions_mut().insert(identity);
    req.extensions_mut().insert(RequestTrace::new());
    next.run(req).await
}
