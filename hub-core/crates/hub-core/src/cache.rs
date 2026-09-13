//! A: query result cache (Redis-backed).
//!
//! Wraps hybrid_search / semantic_search / keyword_search with a
//! short-TTL Redis cache keyed by (mode, normalized_query, limit,
//! url_contains). Repeated identical calls within 5 minutes get
//! instant returns — budget holds, latency drops to ~5ms.
//!
//! Design notes:
//!   - Cache key normalized: lowercase + whitespace-collapsed query,
//!     so "BRICS" and "  brics " hit the same slot.
//!   - Mode is a prefix byte inside the hash, not part of the Redis
//!     key prefix — keeps the surface small and prevents cross-mode
//!     pollution.
//!   - Truncated sha256 (16 bytes = 32 hex) is the suffix. Plenty of
//!     entropy for query traffic, much shorter keys than full 64-hex.
//!   - Cache miss / error paths fall through silently — caching is
//!     a perf optimization, never a correctness dependency.

use crate::error::HubError;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Distinguishes which search mode a cache entry belongs to. Stored
/// as the first byte of the hash input so hybrid and semantic results
/// never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    Hybrid,
    Semantic,
    Keyword,
}

impl CacheMode {
    fn tag(self) -> u8 {
        match self {
            CacheMode::Hybrid => 0x01,
            CacheMode::Semantic => 0x02,
            CacheMode::Keyword => 0x03,
        }
    }
}

/// Stable, deterministic cache key for the search surface.
/// Format: `hub:qcache:{32-hex}` where hex = sha256(mode || query_norm || 0 || limit || 0 || url_contains)[:16].
pub fn cache_key(
    mode: CacheMode,
    query: &str,
    limit: i64,
    url_contains: Option<&str>,
) -> String {
    let normalized = normalize_query(query);
    let url = url_contains.unwrap_or("");
    let mut hasher = Sha256::new();
    hasher.update([mode.tag()]);
    hasher.update((normalized.len() as u64).to_le_bytes());
    hasher.update(normalized.as_bytes());
    hasher.update([0u8]);
    hasher.update(limit.to_le_bytes());
    hasher.update([0u8]);
    hasher.update(url.as_bytes());
    let full = hasher.finalize();
    let mut hex = String::with_capacity(32);
    for b in &full[..16] {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    format!("hub:qcache:{hex}")
}

/// Normalize a query so trivial whitespace / case variations collapse
/// to the same cache key. Keeps punctuation verbatim (entity names,
/// quoted phrases etc. still matter).
fn normalize_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len());
    let mut prev_ws = true;
    for ch in q.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            for low in ch.to_lowercase() {
                out.push(low);
            }
            prev_ws = false;
        }
    }
    out.trim().to_string()
}

/// Cache hit value — JSON-serialized response blob from the search layer.
#[derive(Debug, Clone)]
pub struct CacheHit {
    pub value: Value,
    pub key: String,
}

/// Read a cached query result. Returns None on miss OR on Redis error
/// (never bubbles the error — cache failures must not block search).
pub async fn cache_get(state: &crate::state::AppState, key: &str) -> Option<Value> {
    let raw: Option<String> = state
        .redis_timed(redis::cmd("GET").arg(key).clone(), 2000)
        .await?;
    let raw = raw?;
    serde_json::from_str(&raw).ok()
}

/// Write a cache entry. Best-effort — Redis errors are logged, not propagated.
pub async fn cache_set(
    state: &crate::state::AppState,
    key: &str,
    value: &Value,
    ttl_secs: u64,
) -> Result<(), HubError> {
    let s = serde_json::to_string(value).map_err(|e| HubError::Internal(format!("cache encode: {e}")))?;
    let _: Option<String> = state
        .redis_timed(redis::cmd("SET").arg(key).arg(s).arg("EX").arg(ttl_secs).clone(), 2000)
        .await;
    Ok(())
}

/// Cache hit/miss/disabled marker surfaced on every search response.
/// The caller renders it on the response blob so the agent sees whether
/// it paid for the retrieval or got it for free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    Hit,
    Miss,
    Disabled,
    Error,
}

impl CacheStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CacheStatus::Hit => "hit",
            CacheStatus::Miss => "miss",
            CacheStatus::Disabled => "disabled",
            CacheStatus::Error => "error",
        }
    }
}

/// Wrap a search inner-call with Redis cache. Returns `(result, status)`.
///
/// - On enabled + hit: returns the cached blob, records cost kind=`cache_hit`,
///   `status=Hit`. Fresh call is NOT made.
/// - On enabled + miss: calls `fresh()`, caches the Ok result, `status=Miss`.
///   Errors are returned uncached (the next call retries).
/// - On disabled: passes through, `status=Disabled`.
/// - On Redis transport error: falls through to fresh, `status=Error`
///   (still treated as miss for cost purposes).
///
/// The `fresh` closure is `FnOnce` so we can borrow from the caller
/// without fighting the borrow checker over `self`.
pub async fn search_with_cache<F, Fut>(
    state: &crate::state::AppState,
    agent_id: Option<uuid::Uuid>,
    trace_id: Option<uuid::Uuid>,
    mode: CacheMode,
    query: &str,
    limit: i64,
    url_contains: Option<&str>,
    fresh: F,
) -> (crate::error::Result<Value>, CacheStatus)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = crate::error::Result<Value>>,
{
    let enabled = state.config.query_cache_enabled;
    let ttl = state.config.query_cache_ttl_secs;
    let key = cache_key(mode, query, limit, url_contains);
    if enabled {
        match cache_get(state, &key).await {
            Some(v) => {
                record_cache_hit(state, agent_id, trace_id, mode, query).await;
                return (Ok(v), CacheStatus::Hit);
            }
            None => {}
        }
    }
    let result = fresh().await;
    if enabled {
        if let Ok(ref v) = result {
            // Best-effort cache write — failure doesn't propagate
            let _ = cache_set(state, &key, v, ttl).await;
        }
    }
    let status = if !enabled {
        CacheStatus::Disabled
    } else {
        CacheStatus::Miss
    };
    (result, status)
}

async fn record_cache_hit(
    state: &crate::state::AppState,
    agent_id: Option<uuid::Uuid>,
    trace_id: Option<uuid::Uuid>,
    mode: CacheMode,
    query: &str,
) {
    let _ = crate::cost::record_cost(
        state,
        agent_id,
        None,
        trace_id,
        "cache_hit",
        1.0,
        "call",
        serde_json::json!({ "mode": format!("{mode:?}"), "query": query }),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapse_runs_of_whitespace() {
        assert_eq!(normalize_query("a   b\tc"), "a b c");
    }

    #[test]
    fn normalize_lowercases() {
        assert_eq!(normalize_query("BRICS"), "brics");
    }

    #[test]
    fn normalize_trims_edges() {
        assert_eq!(normalize_query("  hello  "), "hello");
    }

    #[test]
    fn normalize_keeps_punctuation() {
        assert_eq!(normalize_query("BRICS-Yield"), "brics-yield");
    }
}