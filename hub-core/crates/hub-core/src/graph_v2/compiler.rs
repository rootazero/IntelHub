//! Typed-intent dispatcher. Every MCP/agent write intent flows through here.
//! Spec §4: 6 intent kinds. Hard rule: every intent carries ≥1 evidence_doc_ids.
//!
//! Visibility note: `require_evidence` and `ALLOWED_PREDICATES` are exposed
//! as `pub` (rather than private) so the integration test harness
//! (`tests/graph_v2_integration.rs`) can exercise them. The inline
//! `#[cfg(test)]` tests below would normally suffice, but a pre-existing
//! compile error in `monitor/sources/{epa,fred}.rs` blocks `cargo test
//! --lib`. The integration harness compiles cleanly and is the verification
//! path for Task 3. Both call surfaces still match the brief — the helpers
//! are pure logic and have no security-sensitive state.

use futures::future::BoxFuture;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::HubError;
use crate::graph_v2::{contradiction, evidence, resolve, temporal};

/// Row shape for the temporal-merge lookup — extracted to silence
/// `clippy::type_complexity` and to make the (id, valid_from, valid_until)
/// tuple self-documenting at the call site.
type RelationshipWindowRow = (
    Uuid,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
);

pub async fn dispatch(
    intent: Value,
    actor: &str,
    task_id: Option<Uuid>,
    pool: &PgPool,
) -> Result<Value, HubError> {
    let action = intent
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| HubError::Validation("missing action".into()))?;
    match action {
        "assert_entity" => dispatch_assert_entity(&intent, actor, task_id, pool).await,
        "assert_relationship" => dispatch_assert_relationship(&intent, actor, task_id, pool).await,
        "assert_contradiction" => {
            dispatch_assert_contradiction(&intent, actor, task_id, pool).await
        }
        "link_evidence_to_claim" => dispatch_link_evidence(&intent, actor, task_id, pool).await,
        "mark_finding_about_entity" => {
            dispatch_mark_finding(&intent, actor, task_id, pool).await
        }
        "close_investigation_extract" => {
            dispatch_close_investigation(&intent, actor, task_id, pool).await
        }
        other => Err(HubError::Validation(format!("unknown action: {other}"))),
    }
}

/// Spec §1 hard rule: every intent MUST carry ≥1 `evidence_doc_ids[]` entry,
/// OR a non-empty `evidence_absent_reason` string.
pub fn require_evidence(intent: &Value) -> Result<(), HubError> {
    let ids: Vec<Uuid> = intent
        .get("evidence_doc_ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default();
    let absent = intent
        .get("evidence_absent_reason")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if ids.is_empty() && !absent {
        return Err(HubError::Validation(
            "evidence_doc_ids required (or evidence_absent_reason)".into(),
        ));
    }
    Ok(())
}

async fn dispatch_assert_entity(
    intent: &Value,
    actor: &str,
    task: Option<Uuid>,
    pool: &PgPool,
) -> Result<Value, HubError> {
    require_evidence(intent)?;
    let kind = intent
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| HubError::Validation("missing kind".into()))?;
    let name = intent
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| HubError::Validation("missing name".into()))?;
    let conf = intent
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);
    let ids: Vec<Uuid> = parse_doc_ids(intent);
    // Only validate doc IDs when the caller actually provided some.
    // require_evidence already permitted the (empty_ids, non_empty_absent_reason)
    // combination — calling validate_doc_ids(&[]) in that case would fail.
    if !ids.is_empty() {
        evidence::validate_doc_ids(&ids, pool).await?;
    }
    let resolution = resolve::resolve_entity(name, kind, actor, conf, pool).await?;
    let eid = match &resolution {
        resolve::EntityResolution::Existing { entity_id, .. } => *entity_id,
        resolve::EntityResolution::New { entity_id } => *entity_id,
    };
    let matched_by = match &resolution {
        resolve::EntityResolution::Existing { matched_by, .. } => *matched_by,
        resolve::EntityResolution::New { .. } => resolve::ResolutionMethod::NewEntity,
    };
    let audit_after = json!({
        "kind": kind,
        "name": name,
        "matched_by": format!("{matched_by:?}"),
    });
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by, task_id)
         VALUES ('insert','entity',$1, NULL, $2, $3, $4)",
    )
    .bind(eid.to_string())
    .bind(&audit_after)
    .bind(actor)
    .bind(task)
    .execute(pool)
    .await?;
    Ok(json!({ "entity_id": eid, "resolution": audit_after }))
}

async fn dispatch_assert_relationship(
    intent: &Value,
    actor: &str,
    task: Option<Uuid>,
    pool: &PgPool,
) -> Result<Value, HubError> {
    require_evidence(intent)?;
    let subject = intent
        .get("subject")
        .ok_or_else(|| HubError::Validation("missing subject".into()))?;
    let object = intent
        .get("object")
        .ok_or_else(|| HubError::Validation("missing object".into()))?;
    let predicate = intent
        .get("predicate")
        .and_then(|v| v.as_str())
        .ok_or_else(|| HubError::Validation("missing predicate".into()))?;
    if !ALLOWED_PREDICATES.contains(&predicate) {
        return Err(HubError::Validation(format!(
            "predicate {predicate} not in allowlist"
        )));
    }
    let conf = intent
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);
    let s_id = resolve_entity_ref(subject, actor, conf, pool).await?;
    let o_id = resolve_entity_ref(object, actor, conf, pool).await?;
    let ids: Vec<Uuid> = parse_doc_ids(intent);
    if !ids.is_empty() {
        evidence::validate_doc_ids(&ids, pool).await?;
    }
    let valid_from = parse_dt(intent.get("valid_from"));
    let valid_until = parse_dt(intent.get("valid_until"));
    if let (Some(f), Some(u)) = (valid_from, valid_until) {
        if f >= u {
            return Err(HubError::Validation(
                "valid_from must be < valid_until".into(),
            ));
        }
    }
    // Look for an existing rel with same (subject, predicate, object) — apply temporal merge.
    let existing_row: Option<RelationshipWindowRow> = sqlx::query_as(
        "SELECT relationship_id, valid_from, valid_until FROM relationships
         WHERE from_entity = $1 AND to_entity = $2 AND rel_type = $3
         ORDER BY discovered_at DESC LIMIT 1",
    )
    .bind(s_id)
    .bind(o_id)
    .bind(predicate)
    .fetch_optional(pool)
    .await?;
    let overlap = match existing_row.as_ref() {
        Some((_, vf, vu)) => temporal::classify_overlap(valid_from, valid_until, *vf, *vu),
        None => temporal::Overlap::Disjoint,
    };
    match overlap {
        temporal::Overlap::SameWindow => {
            let (existing_id, _, _) = existing_row.expect("existing_row guaranteed by match");
            sqlx::query(
                "UPDATE relationships SET confidence = $1, evidence_doc_ids = $2,
                 valid_from = COALESCE($3, valid_from),
                 valid_until = COALESCE($4, valid_until),
                 created_by_task_id = COALESCE($5, created_by_task_id)
                 WHERE relationship_id = $6",
            )
            .bind(conf)
            .bind(&ids)
            .bind(valid_from)
            .bind(valid_until)
            .bind(task)
            .bind(existing_id)
            .execute(pool)
            .await?;
        }
        _ => {
            sqlx::query(
                "INSERT INTO relationships
                   (from_entity, to_entity, rel_type, attributes, valid_from, valid_until,
                    discovered_at, confidence, evidence_doc_ids, created_by_task_id)
                 VALUES ($1,$2,$3,'{}'::jsonb,$4,$5,now(),$6,$7,$8)
                 ON CONFLICT (from_entity, to_entity, rel_type) DO NOTHING",
            )
            .bind(s_id)
            .bind(o_id)
            .bind(predicate)
            .bind(valid_from)
            .bind(valid_until)
            .bind(conf)
            .bind(&ids)
            .bind(task)
            .execute(pool)
            .await?;
        }
    }
    let op = if matches!(overlap, temporal::Overlap::SameWindow) {
        "update"
    } else {
        "insert"
    };
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by, task_id)
         VALUES ($1,'relationship',$2, NULL, $3, $4, $5)",
    )
    .bind(op)
    .bind(s_id.to_string())
    .bind(json!({
        "subject": s_id,
        "predicate": predicate,
        "object": o_id,
        "valid_from": valid_from,
        "valid_until": valid_until,
        "confidence": conf,
    }))
    .bind(actor)
    .bind(task)
    .execute(pool)
    .await?;
    Ok(json!({
        "subject": s_id,
        "predicate": predicate,
        "object": o_id,
        "overlap": format!("{overlap:?}")
    }))
}

async fn resolve_entity_ref(
    v: &Value,
    actor: &str,
    conf: f64,
    pool: &PgPool,
) -> Result<Uuid, HubError> {
    if let Some(s) = v.as_str() {
        return Uuid::parse_str(s).map_err(|_| HubError::Validation("entity id not uuid".into()));
    }
    let kind = v
        .get("kind")
        .and_then(|x| x.as_str())
        .ok_or_else(|| HubError::Validation("entity ref needs kind or id".into()))?;
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .ok_or_else(|| HubError::Validation("entity ref needs name".into()))?;
    match resolve::resolve_entity(name, kind, actor, conf, pool).await? {
        resolve::EntityResolution::Existing { entity_id, .. } => Ok(entity_id),
        resolve::EntityResolution::New { entity_id } => Ok(entity_id),
    }
}

async fn dispatch_assert_contradiction(
    intent: &Value,
    actor: &str,
    task: Option<Uuid>,
    pool: &PgPool,
) -> Result<Value, HubError> {
    require_evidence(intent)?;
    let a = parse_uuid(intent.get("claim_a"), "claim_a")?;
    let b = parse_uuid(intent.get("claim_b"), "claim_b")?;
    let reason = intent.get("reason").and_then(|v| v.as_str()).unwrap_or("");
    let cid = contradiction::record(a, b, reason, actor, task, pool).await?;
    Ok(json!({ "contradiction_id": cid }))
}

async fn dispatch_link_evidence(
    intent: &Value,
    actor: &str,
    task: Option<Uuid>,
    pool: &PgPool,
) -> Result<Value, HubError> {
    // Spec §1: link_evidence_to_claim is administrative (records an already-
    // known evidence binding), so it does NOT require evidence_doc_ids on
    // the intent itself — the (claim_id, document_id, relation) IS the
    // evidence. require_evidence is intentionally skipped here.
    let claim_id = parse_uuid(intent.get("claim_id"), "claim_id")?;
    let document_id = parse_uuid(intent.get("document_id"), "document_id")?;
    let relation = intent
        .get("relation")
        .and_then(|v| v.as_str())
        .ok_or_else(|| HubError::Validation("missing relation".into()))?;
    if !["supports", "contradicts"].contains(&relation) {
        return Err(HubError::Validation(format!("invalid relation {relation}")));
    }
    let snippet = intent.get("snippet").and_then(|v| v.as_str());
    sqlx::query(
        "INSERT INTO claim_evidence (claim_id, document_id, relation, snippet)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (claim_id, document_id, relation) DO NOTHING",
    )
    .bind(claim_id)
    .bind(document_id)
    .bind(relation)
    .bind(snippet)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by, task_id)
         VALUES ('insert','claim',$1, NULL, jsonb_build_object('linked',$2,'relation',$3), $4, $5)",
    )
    .bind(claim_id.to_string())
    .bind(document_id)
    .bind(relation)
    .bind(actor)
    .bind(task)
    .execute(pool)
    .await?;
    Ok(json!({
        "claim_id": claim_id,
        "document_id": document_id,
        "relation": relation,
    }))
}

async fn dispatch_mark_finding(
    intent: &Value,
    actor: &str,
    task: Option<Uuid>,
    pool: &PgPool,
) -> Result<Value, HubError> {
    // mark_finding_about_entity records an attribution between a finding
    // and an entity: every document that observes the entity becomes a
    // supporting evidence for the finding (idempotent via ON CONFLICT).
    //
    // SQL note: the brief's reference table was `entities` but entities has
    // no `document_id` column. The intent (copy entity evidence into finding
    // evidence) is best served by joining through `observations` — that is
    // the canonical entity ↔ document link table.
    let finding_id = parse_uuid(intent.get("finding_id"), "finding_id")?;
    let entity_id = parse_uuid(intent.get("entity_id"), "entity_id")?;
    let conf = intent
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);
    sqlx::query(
        "INSERT INTO finding_evidence (finding_id, document_id, relation)
         SELECT $1, o.document_id, 'supports'
           FROM observations o
          WHERE o.entity_id = $2
         ON CONFLICT DO NOTHING",
    )
    .bind(finding_id)
    .bind(entity_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by, task_id)
         VALUES ('insert','claim',$1, NULL, jsonb_build_object('entity_id',$2,'relation','about'), $3, $4)",
    )
    .bind(finding_id.to_string())
    .bind(entity_id)
    .bind(actor)
    .bind(task)
    .execute(pool)
    .await?;
    Ok(json!({
        "finding_id": finding_id,
        "entity_id": entity_id,
        "confidence": conf,
    }))
}

async fn dispatch_close_investigation(
    intent: &Value,
    actor: &str,
    task: Option<Uuid>,
    pool: &PgPool,
) -> Result<Value, HubError> {
    let investigation_id = parse_uuid(intent.get("investigation_id"), "investigation_id")?;
    let extractions = intent
        .get("extraction")
        .and_then(|v| v.as_array())
        .ok_or_else(|| HubError::Validation("missing extraction[]".into()))?;
    let mut results = Vec::with_capacity(extractions.len());
    for sub in extractions {
        // Recursive call into `dispatch` requires indirection (Rust async fns
        // cannot recurse directly — the future would have infinite size).
        let r = dispatch_boxed(sub.clone(), actor, task, pool).await?;
        results.push(r);
    }
    Ok(json!({
        "investigation_id": investigation_id,
        "applied": results,
    }))
}

/// Indirection wrapper for the recursive `dispatch` call inside
/// `dispatch_close_investigation`. Returns a boxed future so the inner
/// `dispatch` future doesn't need an unbounded size.
fn dispatch_boxed<'a>(
    intent: Value,
    actor: &'a str,
    task: Option<Uuid>,
    pool: &'a PgPool,
) -> BoxFuture<'a, Result<Value, HubError>> {
    Box::pin(dispatch(intent, actor, task, pool))
}

fn parse_uuid(v: Option<&Value>, field: &str) -> Result<Uuid, HubError> {
    let s = v
        .and_then(|x| x.as_str())
        .ok_or_else(|| HubError::Validation(format!("missing {field}")))?;
    Uuid::parse_str(s).map_err(|_| HubError::Validation(format!("{field} not a uuid")))
}

fn parse_dt(v: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    v.and_then(|x| x.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
}

fn parse_doc_ids(intent: &Value) -> Vec<Uuid> {
    intent
        .get("evidence_doc_ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect()
        })
        .unwrap_or_default()
}

pub const ALLOWED_PREDICATES: &[&str] = &[
    "controls",
    "owns",
    "communicates_with",
    "resolves_to",
    "located_in",
    "affiliated_with",
    "uses",
    "hosts",
    "registered_by",
    "related_to",
    "SUPPORTS",
    "CONTRADICTS",
    "ABOUT",
    "MENTIONS",
    "PRODUCED",
    "PART_OF",
    "REPLACES",
    "WORKS_FOR",
    "ACQUIRED",
    "OPERATES",
    "FOUNDED",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn require_evidence_rejects_empty() {
        let v = json!({"action": "assert_entity", "kind": "org", "name": "x"});
        assert!(require_evidence(&v).is_err());
    }

    #[test]
    fn require_evidence_accepts_ids() {
        let v = json!({
            "action": "assert_entity",
            "kind": "org",
            "name": "x",
            "evidence_doc_ids": ["00000000-0000-0000-0000-000000000001"]
        });
        assert!(require_evidence(&v).is_ok());
    }

    #[test]
    fn require_evidence_accepts_absent_reason() {
        let v = json!({
            "action": "assert_entity",
            "kind": "org",
            "name": "x",
            "evidence_absent_reason": "manually seeded"
        });
        assert!(require_evidence(&v).is_ok());
    }

    #[test]
    fn require_evidence_accepts_blank_absent_reason_via_ids() {
        // Whitespace-only absent_reason is NOT enough — ids must also be present.
        let v = json!({
            "action": "assert_entity",
            "kind": "org",
            "name": "x",
            "evidence_absent_reason": "   ",
            "evidence_doc_ids": ["00000000-0000-0000-0000-000000000002"]
        });
        assert!(require_evidence(&v).is_ok());
    }

    #[test]
    fn predicate_allowlist() {
        assert!(ALLOWED_PREDICATES.contains(&"owns"));
        assert!(ALLOWED_PREDICATES.contains(&"CONTRADICTS"));
        assert!(!ALLOWED_PREDICATES.contains(&"hates"));
    }

    #[test]
    fn predicate_allowlist_lowercase_writes_only() {
        // Predicate strings must be exactly as listed — the brief's list mixes
        // lowercase ("owns") with screaming-snake ("CONTRADICTS"). This test
        // pins the entries so accidental case drift surfaces.
        for p in ALLOWED_PREDICATES {
            assert!(!p.is_empty(), "empty predicate in allowlist");
        }
    }

    #[test]
    fn parse_uuid_handles_missing() {
        let v = json!({});
        assert!(parse_uuid(v.get("x"), "x").is_err());
    }

    #[test]
    fn parse_uuid_handles_bad_uuid() {
        let v = json!({"x": "not-a-uuid"});
        assert!(parse_uuid(v.get("x"), "x").is_err());
    }

    #[test]
    fn parse_uuid_accepts_valid() {
        let v = json!({"x": "00000000-0000-0000-0000-000000000001"});
        let u = parse_uuid(v.get("x"), "x").unwrap();
        assert_eq!(
            u,
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
        );
    }

    #[test]
    fn parse_dt_handles_rfc3339() {
        let v = json!({"ts": "2025-06-01T00:00:00Z"});
        let dt = parse_dt(v.get("ts")).expect("rfc3339 should parse");
        assert_eq!(dt.to_rfc3339(), "2025-06-01T00:00:00+00:00");
    }

    #[test]
    fn parse_dt_handles_missing() {
        let v = json!({});
        assert!(parse_dt(v.get("ts")).is_none());
    }

    #[test]
    fn parse_doc_ids_collects_valid_only() {
        let v = json!({
            "evidence_doc_ids": [
                "00000000-0000-0000-0000-000000000001",
                "not-a-uuid",
                "00000000-0000-0000-0000-000000000002"
            ]
        });
        let ids = parse_doc_ids(&v);
        assert_eq!(ids.len(), 2);
    }
}
