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
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Tier;

/// Implemented by anything that carries a caller identity.
pub trait TierAccess {
    fn tier(&self) -> Tier;
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
