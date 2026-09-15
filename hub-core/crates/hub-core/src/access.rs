//! Tier-based access control for OSINT data.
//!
//! Every query path that touches `geo_events` goes through `apply_tier_filter`.
//! This rewrites the SQL at the boundary, so a free-tier caller physically
//! cannot retrieve paid-source rows even via crafted SQL strings.
//!
//! When a free-tier caller attempts to read paid-source data (e.g., by
//! passing `source=monitor:x` to `/api/v1/events/recent`), the handler
//! returns 200 with `events: []` and `audit_on_reject` logs the attempt.
//! We deliberately do NOT return 403, which would leak source existence.

use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Tier;
use crate::types::{AgentIdentity, BusEvent};

/// Implemented by anything that carries a caller identity.
///
/// `Send + Sync` are required so `&dyn TierAccess` can be held across an
/// `.await` inside handler futures (otherwise the axum `Handler` future
/// becomes `!Send` and the route fails to register).
pub trait TierAccess: Send + Sync {
    fn tier(&self) -> Tier;
}

impl TierAccess for AgentIdentity {
    fn tier(&self) -> Tier {
        self.tier
    }
}

/// Rewrites a SQL fragment so a `free` caller only sees `free` rows,
/// `paid` sees `free+paid`, and `admin` sees everything.
///
/// Caller is responsible for any prior WHERE / ORDER BY / LIMIT clauses.
/// `base_query` should end with the FROM clause but NOT have a WHERE on
/// `tier_required` (we add it).
///
/// Example:
/// ```text
/// apply_tier_filter("SELECT id FROM geo_events WHERE source = $1", Tier::Free)
///   → "SELECT id FROM geo_events WHERE source = $1 AND tier_required = 'free'"
/// ```
pub fn apply_tier_filter(base_query: &str, tier: Tier) -> String {
    match tier {
        Tier::Admin => base_query.to_string(),
        Tier::Paid => {
            if base_query.to_uppercase().contains(" WHERE ") {
                format!("{base_query} AND tier_required IN ('free', 'paid')")
            } else {
                format!("{base_query} WHERE tier_required IN ('free', 'paid')")
            }
        }
        Tier::Free => {
            if base_query.to_uppercase().contains(" WHERE ") {
                format!("{base_query} AND tier_required = 'free'")
            } else {
                format!("{base_query} WHERE tier_required = 'free'")
            }
        }
    }
}

/// Writes an audit row when a tier-mismatch access is attempted (or for
/// admin actions like `set-tier`). Best-effort: errors are logged but do
/// not fail the user-facing request — auditing must never become a DoS
/// vector.
pub async fn audit_on_reject(
    pool: &PgPool,
    agent_id: Uuid,
    api_key_prefix: &str,
    action: &str,
    source_attempted: Option<&str>,
    request_path: Option<&str>,
    trace_id: Option<Uuid>,
    blocked: bool,
) {
    let result = sqlx::query(
        "INSERT INTO audit_log
            (ts, agent_id, api_key_prefix, action, source_attempted, request_path, trace_id, blocked)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(Utc::now())
    .bind(agent_id)
    .bind(api_key_prefix)
    .bind(action)
    .bind(source_attempted)
    .bind(request_path)
    .bind(trace_id)
    .bind(blocked)
    .execute(pool)
    .await;

    if let Err(e) = result {
        tracing::warn!(
            target: "access::audit",
            "audit_on_reject failed: {e}"
        );
    }
}

// ---------- tier resolution helpers (REST + MCP) ----------

/// Tier hierarchy: `admin > paid > free`.
pub fn tier_allows(caller: Tier, required: Tier) -> bool {
    fn rank(t: Tier) -> u8 {
        match t {
            Tier::Free => 0,
            Tier::Paid => 1,
            Tier::Admin => 2,
        }
    }
    rank(caller) >= rank(required)
}

/// Best-effort extraction of the monitor source name from an actor string
/// (`hub:monitor:x`, `monitor:x`) or a bare payload source (`x`).
fn candidate_source(raw: &str) -> &str {
    raw.rsplit(':').next().unwrap_or(raw)
}

/// `true` when `raw` (actor / source string) is readable by a caller at
/// `tier`. Non-monitor sources (e.g. `agent:pi`) are always allowed — tier
/// isolation only governs `geo_events` monitor sources.
pub fn source_allowed(caller: Tier, raw: &str) -> bool {
    match crate::config::monitor_metadata().get(candidate_source(raw)) {
        Some(meta) => tier_allows(caller, meta.tier_required),
        None => true,
    }
}

/// Post-filter a bus `BusEvent` (SSE stream) for a caller tier.
pub fn bus_event_allowed(caller: Tier, ev: &BusEvent) -> bool {
    let payload_src = ev.payload.get("source").and_then(|s| s.as_str());
    source_allowed(caller, payload_src.unwrap_or(&ev.actor))
}

/// Post-filter the `{count, items}` JSON returned by `store::recent_events`.
/// Bus events carry their monitor source in `payload.source` (falling back to
/// the `actor` string); rows above the caller's tier are dropped and `count`
/// recomputed. The `events` bus table has no `tier_required` column, so this
/// is a Rust-side filter rather than a SQL rewrite.
pub fn filter_bus_items(value: &mut Value, tier: Tier) {
    let Some(items) = value.get_mut("items").and_then(|v| v.as_array_mut()) else {
        return;
    };
    items.retain(|item| {
        let payload_src = item.pointer("/payload/source").and_then(|s| s.as_str());
        let actor = item.get("actor").and_then(|a| a.as_str()).unwrap_or("");
        source_allowed(tier, payload_src.unwrap_or(actor))
    });
    let n = items.len();
    value["count"] = serde_json::json!(n);
}

/// Resolves the caller's tier, rewrites `base_sql` via `apply_tier_filter`,
/// and writes an `audit_log` row when the caller explicitly attempted a
/// source above their tier. Returns the filtered SQL.
///
/// `base_sql` MUST end with its WHERE clause (no ORDER BY / LIMIT / GROUP BY):
/// `apply_tier_filter` appends the tier predicate at the very end.
pub async fn filter_query_by_tier(
    pool: &PgPool,
    caller: &dyn TierAccess,
    agent_id: Uuid,
    api_key_prefix: &str,
    base_sql: &str,
    source_attempted: Option<&str>,
    request_path: &str,
    trace_id: Option<Uuid>,
) -> String {
    let tier = caller.tier();
    let filtered = apply_tier_filter(base_sql, tier);

    if let Some(src) = source_attempted {
        if let Some(meta) = crate::config::monitor_metadata().get(candidate_source(src)) {
            if !tier_allows(tier, meta.tier_required) {
                audit_on_reject(
                    pool,
                    agent_id,
                    api_key_prefix,
                    "tier_mismatch_query",
                    Some(src),
                    Some(request_path),
                    trace_id,
                    true,
                )
                .await;
            }
        }
    }

    filtered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_passes_through() {
        let q = "SELECT * FROM geo_events WHERE source = $1";
        assert_eq!(
            apply_tier_filter(q, Tier::Admin),
            "SELECT * FROM geo_events WHERE source = $1"
        );
    }

    #[test]
    fn paid_adds_in_clause() {
        let q = "SELECT * FROM geo_events WHERE source = $1";
        let out = apply_tier_filter(q, Tier::Paid);
        assert!(out.contains("WHERE source = $1"));
        assert!(out.contains("tier_required IN ('free', 'paid')"));
        assert!(!out.contains(" WHERE WHERE "));
    }

    #[test]
    fn free_adds_eq_clause() {
        let q = "SELECT * FROM geo_events WHERE source = $1";
        let out = apply_tier_filter(q, Tier::Free);
        assert!(out.contains("tier_required = 'free'"));
        assert!(!out.contains(" WHERE WHERE "));
    }

    #[test]
    fn free_without_where_adds_where() {
        let q = "SELECT * FROM geo_events";
        let out = apply_tier_filter(q, Tier::Free);
        assert_eq!(out, "SELECT * FROM geo_events WHERE tier_required = 'free'");
    }

    #[test]
    fn case_insensitive_where_detection() {
        let q = "select * from geo_events where source = $1";
        let out = apply_tier_filter(q, Tier::Paid);
        assert!(out.contains(" AND tier_required IN ('free', 'paid')"));
    }
}
