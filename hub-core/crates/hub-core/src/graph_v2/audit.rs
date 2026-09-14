//! graph_v2 audit-row helper.
//!
//! The change_log → Neo4j mirror worker reads rows from `graph_change_log`
//! and translates them to v1 op shapes. Every v2 write that wants its
//! Neo4j mirror to stay consistent (or recover from a cold mirror) MUST
//! emit a corresponding audit row.
//!
//! v1's `create_claim` at graphw.rs:378 emits an audit row of shape
//! `(insert, claim, $1, {text, status, entities})`. Future v2 sites that
//! create claims (e.g. an auto-extraction pipeline that picks claims
//! out of documents) should use the helper here so the audit shape stays
//! in sync with phase 3's translate arm.
//!
//! The compiler's existing 4 inline `INSERT INTO graph_change_log`
//! sites are intentionally NOT refactored to call this helper:
//!   - They each have a different `after` shape (entity / relationship /
//!     claim-evidence / finding-about) that translate_change_log
//!     disambiguates by JSON field presence, not by target_kind alone.
//!   - Inlining keeps each dispatch branch self-contained and easy to
//!     audit on its own (low coupling, high cohesion per branch).
//!
//! This helper exists solely so future v2 claim sites have a single,
//! well-documented write site that's guaranteed to match what
//! translate_change_log expects.

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// Emit a `(insert, claim, $1, {text, status, entities})` audit row.
///
/// Call AFTER the canonical `INSERT INTO claims` so the mirror worker
/// can replay to Neo4j on the next 15s tick. The v1 `create_claim` op
/// shape in `try_graph_write` will MERGE the :Claim node and any
/// :ABOUT edges for the entities listed.
///
/// * `text` — the claim text (8-4000 chars, same validation as v1)
/// * `entities` — optional `[{kind, name}, ...]` pairs (from the
///   caller's `claim_entities` join, if any). Pass `None` or `Some(vec![])`
///   for claims with no entity associations.
/// * `actor` — the agent/role performing the write
/// * `task` — optional task_id for traceability (mirrors the existing
///   v2 compiler pattern)
pub async fn emit_claim_audit(
    pool: &PgPool,
    claim_id: Uuid,
    text: &str,
    entities: Option<&[Value]>,
    actor: &str,
    task: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    let entities_json = match entities {
        Some(arr) if !arr.is_empty() => Value::Array(arr.to_vec()),
        _ => Value::Array(Vec::new()),
    };
    let after = json!({
        "text": text,
        "status": "unverified",
        "entities": entities_json,
    });
    sqlx::query(
        "INSERT INTO graph_change_log
            (op, target_kind, target_id, before, after, changed_by, task_id)
         VALUES ('insert','claim',$1, NULL, $2, $3, $4)",
    )
    .bind(claim_id.to_string())
    .bind(&after)
    .bind(actor)
    .bind(task)
    .execute(pool)
    .await?;
    Ok(())
}
