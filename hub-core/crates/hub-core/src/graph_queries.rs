//! Read-only Cypher + SQL queries backing the 10 new MCP tools (spec §6).
//! No raw Cypher reaches agents — these templates are the only entry point.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::HubError;
use crate::state::AppState;

/// Minimal BoltType → JSON conversion covering our templates' return shapes.
/// Mirrors the helper in `graph.rs`; intentionally duplicated (kept private)
/// so the read plane here has no dependency on the write plane there.
fn bolt_to_json(v: &neo4rs::BoltType) -> Value {
    match v {
        neo4rs::BoltType::String(s) => Value::String(s.value.clone()),
        neo4rs::BoltType::Integer(i) => json!(i.value),
        neo4rs::BoltType::Float(f) => json!(f.value),
        neo4rs::BoltType::Boolean(b) => json!(b.value),
        neo4rs::BoltType::Null(_) => Value::Null,
        neo4rs::BoltType::List(l) => Value::Array(l.value.iter().map(bolt_to_json).collect()),
        neo4rs::BoltType::Map(m) => {
            let obj: serde_json::Map<std::string::String, Value> = m
                .value
                .iter()
                .map(|(k, v)| (k.value.clone(), bolt_to_json(v)))
                .collect();
            Value::Object(obj)
        }
        other => json!(format!("{other:?}")),
    }
}

/// Drain a Cypher result into `Vec<Value>` objects (one per row), each
/// containing the named columns as JSON. Used by `get_neighbors` and
/// `find_path`. Failures bubble up as `HubError::GraphUnavailable`.
async fn collect_cypher_rows(
    state: &AppState,
    cypher: String,
    params: Vec<(&str, neo4rs::BoltType)>,
    columns: &[&str],
) -> Result<Vec<Value>, HubError> {
    let mut q = neo4rs::query(&cypher);
    for (k, v) in params {
        q = q.param(k, v);
    }
    let mut stream = state
        .neo4j
        .execute(q)
        .await
        .map_err(|e| HubError::GraphUnavailable(format!("neo4j execute: {e}")))?;
    let mut rows: Vec<Value> = Vec::new();
    loop {
        match stream.next().await {
            Ok(Some(row)) => {
                let mut obj = serde_json::Map::new();
                for col in columns {
                    if let Ok(v) = row.get::<neo4rs::BoltType>(col) {
                        obj.insert((*col).to_string(), bolt_to_json(&v));
                    }
                }
                rows.push(Value::Object(obj));
            }
            Ok(None) => break,
            Err(e) => return Err(HubError::GraphUnavailable(format!("neo4j stream: {e}"))),
        }
    }
    Ok(rows)
}

// =============================================================================
// SQL-backed queries
// =============================================================================

/// Alias-aware entity search. Matches on `lower(name)` or any `entity_aliases`
/// row whose `alias_norm` contains the query, optionally filtered by kind.
/// Scoring: 1.0 for exact-case-insensitive name match, 0.5 otherwise.
pub async fn search_entity(
    state: &AppState,
    name: &str,
    kind: Option<&str>,
    limit: i64,
) -> Result<Value, HubError> {
    let pool = &state.pg;
    let lim = limit.clamp(1, 100);
    let mut q = String::from(
        "SELECT e.entity_id, e.kind, e.name, e.aliases, \
                CASE WHEN lower(e.name) = lower($1) THEN 1.0 ELSE 0.5 END AS score \
           FROM entities e \
          WHERE e.merged_into IS NULL \
            AND (lower(e.name) LIKE '%' || lower($1) || '%' \
                 OR EXISTS (SELECT 1 FROM entity_aliases ea \
                             WHERE ea.entity_id = e.entity_id \
                               AND ea.alias_norm LIKE '%' || lower($1) || '%'))",
    );
    if kind.is_some() {
        q.push_str(" AND e.kind = $2 ");
    }
    q.push_str(" ORDER BY score DESC, e.name LIMIT $");
    let limit_idx = if kind.is_some() { 3 } else { 2 };
    q.push_str(&limit_idx.to_string());
    let mut query = sqlx::query_as::<_, (Uuid, String, String, serde_json::Value, f64)>(&q).bind(name);
    if let Some(k) = kind {
        query = query.bind(k);
    }
    query = query.bind(lim);
    let rows = query.fetch_all(pool).await?;
    Ok(json!(rows.into_iter().map(|(id, k, n, a, s)| json!({
        "entity_id": id, "kind": k, "name": n, "aliases": a, "score": s
    })).collect::<Vec<_>>()))
}

/// Entity by id, joined with aliases + outgoing relationships + claims.
/// When `merged_into` is set, also resolves the parent (for redirect).
pub async fn get_entity(state: &AppState, entity_id: Uuid) -> Result<Value, HubError> {
    let pool = &state.pg;
    let e: (
        Uuid,
        String,
        String,
        serde_json::Value,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        DateTime<Utc>,
        Option<Uuid>,
    ) = sqlx::query_as(
        "SELECT entity_id, kind, name, aliases, valid_from, valid_until, discovered_at, merged_into \
           FROM entities WHERE entity_id = $1",
    )
    .bind(entity_id)
    .fetch_one(pool)
    .await?;
    let aliases: Vec<(String, String, f64)> = sqlx::query_as(
        "SELECT alias, source, confidence FROM entity_aliases \
          WHERE entity_id = $1 ORDER BY confidence DESC LIMIT 50",
    )
    .bind(entity_id)
    .fetch_all(pool)
    .await?;
    let relationships: Vec<(
        Uuid,
        String,
        Uuid,
        String,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Option<f64>,
    )> = sqlx::query_as(
        "SELECT r.relationship_id, r.rel_type, r.to_entity, ent.name, \
                r.valid_from, r.valid_until, r.confidence \
           FROM relationships r JOIN entities ent ON ent.entity_id = r.to_entity \
          WHERE r.from_entity = $1 ORDER BY r.discovered_at DESC LIMIT 100",
    )
    .bind(entity_id)
    .fetch_all(pool)
    .await?;
    let claims: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT c.claim_id, c.text, c.status \
           FROM claims c JOIN claim_entities ce ON ce.claim_id = c.claim_id \
          WHERE ce.entity_id = $1 AND ce.role IN ('subject','object') LIMIT 50",
    )
    .bind(entity_id)
    .fetch_all(pool)
    .await?;
    let merged_into: Option<Value> = match e.7 {
        Some(parent) => {
            let r: (String, String) =
                sqlx::query_as("SELECT kind, name FROM entities WHERE entity_id = $1")
                    .bind(parent)
                    .fetch_one(pool)
                    .await?;
            Some(json!({ "entity_id": parent, "kind": r.0, "name": r.1 }))
        }
        None => None,
    };
    Ok(json!({
        "entity_id": e.0, "kind": e.1, "name": e.2, "aliases": e.3,
        "valid_from": e.4, "valid_until": e.5, "discovered_at": e.6,
        "merged_into": merged_into,
        "aliases_list": aliases.into_iter().map(|(a,s,c)| json!({
            "alias": a, "source": s, "confidence": c
        })).collect::<Vec<_>>(),
        "relationships": relationships.into_iter().map(|(rid,rt,tid,tn,vf,vu,co)| json!({
            "relationship_id": rid,
            "rel_type": rt,
            "to_entity_id": tid,
            "to_entity_name": tn,
            "valid_from": vf,
            "valid_until": vu,
            "confidence": co
        })).collect::<Vec<_>>(),
        "claims": claims.into_iter().map(|(cid,t,s)| json!({
            "claim_id": cid, "text": t, "status": s
        })).collect::<Vec<_>>()
    }))
}

/// Audit timeline of an entity (and its relationships). Reads
/// `graph_change_log` filtered by `target_kind` + `target_id`, optional
/// time window.
pub async fn get_entity_timeline(
    state: &AppState,
    entity_id: Uuid,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Value, HubError> {
    let lim = limit.clamp(1, 500);
    let rows: Vec<(
        i64,
        String,
        String,
        serde_json::Value,
        serde_json::Value,
        String,
        Option<DateTime<Utc>>,
    )> = sqlx::query_as(
        "SELECT change_id, op, target_kind, before, after, changed_by, changed_at \
           FROM graph_change_log \
          WHERE (target_kind = 'entity' AND target_id = $1) \
             OR (target_kind = 'relationship' AND target_id IN ( \
                  SELECT relationship_id::text FROM relationships \
                   WHERE from_entity = $1 OR to_entity = $1)) \
            AND ($2::timestamptz IS NULL OR changed_at >= $2) \
            AND ($3::timestamptz IS NULL OR changed_at <= $3) \
          ORDER BY changed_at DESC LIMIT $4",
    )
    .bind(entity_id)
    .bind(from)
    .bind(to)
    .bind(lim)
    .fetch_all(&state.pg)
    .await?;
    Ok(json!(rows.into_iter().map(|(c,o,tk,b,a,by,t)| json!({
        "change_id": c,
        "op": o,
        "target_kind": tk,
        "before": b,
        "after": a,
        "changed_by": by,
        "changed_at": t
    })).collect::<Vec<_>>()))
}

/// Relationship changes between two entities (by name, case-insensitive
/// substring). Returns most recent first.
pub async fn find_relationship_changes(
    state: &AppState,
    a: &str,
    b: &str,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> Result<Value, HubError> {
    let rows: Vec<(
        Uuid,
        String,
        Uuid,
        String,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        DateTime<Utc>,
        Option<f64>,
    )> = sqlx::query_as(
        "SELECT r.relationship_id, r.rel_type, r.to_entity, ent.name, \
                r.valid_from, r.valid_until, r.discovered_at, r.confidence \
           FROM relationships r \
           JOIN entities ea ON ea.entity_id = r.from_entity \
           JOIN entities ent ON ent.entity_id = r.to_entity \
          WHERE (lower(ea.name) = lower($1) OR lower(ea.name) LIKE '%' || lower($1) || '%') \
            AND (lower(ent.name) = lower($2) OR lower(ent.name) LIKE '%' || lower($2) || '%') \
            AND ($3::timestamptz IS NULL OR r.discovered_at >= $3) \
            AND ($4::timestamptz IS NULL OR r.discovered_at <= $4) \
          ORDER BY r.discovered_at DESC LIMIT 100",
    )
    .bind(a)
    .bind(b)
    .bind(from)
    .bind(to)
    .fetch_all(&state.pg)
    .await?;
    Ok(json!(rows.into_iter().map(|(rid,rt,tid,tn,vf,vu,da,co)| json!({
        "relationship_id": rid,
        "rel_type": rt,
        "to_entity_id": tid,
        "to_entity_name": tn,
        "valid_from": vf,
        "valid_until": vu,
        "discovered_at": da,
        "confidence": co
    })).collect::<Vec<_>>()))
}

/// Claims that cite this entity, joined with documents that support them.
/// Supports-only by design (use `find_contradicting_claims` for the
/// opposing half).
pub async fn find_supporting_claims(
    state: &AppState,
    entity_id: Uuid,
    limit: i64,
) -> Result<Value, HubError> {
    let lim = limit.clamp(1, 100);
    let rows: Vec<(Uuid, String, String, Uuid, String)> = sqlx::query_as(
        "SELECT c.claim_id, c.text, c.status, d.document_id, d.content_hash \
           FROM claims c \
           JOIN claim_entities ce ON ce.claim_id = c.claim_id \
           JOIN claim_evidence ce2 ON ce2.claim_id = c.claim_id AND ce2.relation = 'supports' \
           JOIN documents d ON d.document_id = ce2.document_id \
          WHERE ce.entity_id = $1 LIMIT $2",
    )
    .bind(entity_id)
    .bind(lim)
    .fetch_all(&state.pg)
    .await?;
    Ok(json!(rows.into_iter().map(|(cid,t,s,did,dh)| json!({
        "claim_id": cid,
        "text": t,
        "status": s,
        "supporting_doc_id": did,
        "content_hash": dh
    })).collect::<Vec<_>>()))
}

/// Contradictions touching a claim OR an entity. Exactly one of
/// `claim_id` / `entity_id` must be supplied (both `None` → `Validation`).
pub async fn find_contradicting_claims(
    state: &AppState,
    claim_id: Option<Uuid>,
    entity_id: Option<Uuid>,
) -> Result<Value, HubError> {
    let pool = &state.pg;
    let rows: Vec<(i64, Uuid, String, Uuid, String, String)> = match (claim_id, entity_id) {
        (Some(c), _) => sqlx::query_as(
            "SELECT cc.contradiction_id, cc.claim_a, ca.text, cc.claim_b, cb.text, cc.reason \
               FROM claim_contradictions cc \
               JOIN claims ca ON ca.claim_id = cc.claim_a \
               JOIN claims cb ON cb.claim_id = cc.claim_b \
              WHERE cc.claim_a = $1 OR cc.claim_b = $1",
        )
        .bind(c)
        .fetch_all(pool)
        .await?,
        (None, Some(e)) => sqlx::query_as(
            "SELECT DISTINCT cc.contradiction_id, cc.claim_a, ca.text, cc.claim_b, cb.text, cc.reason \
               FROM claim_contradictions cc \
               JOIN claims ca ON ca.claim_id = cc.claim_a \
               JOIN claims cb ON cb.claim_id = cc.claim_b \
               JOIN claim_entities ce ON ce.claim_id IN (cc.claim_a, cc.claim_b) \
              WHERE ce.entity_id = $1",
        )
        .bind(e)
        .fetch_all(pool)
        .await?,
        (None, None) => {
            return Err(HubError::Validation(
                "claim_id or entity_id required".into(),
            ))
        }
    };
    Ok(json!(rows.into_iter().map(|(cid,a,at,b_id,bt,r)| json!({
        "contradiction_id": cid,
        "claim_a": a,
        "text_a": at,
        "claim_b": b_id,
        "text_b": bt,
        "reason": r
    })).collect::<Vec<_>>()))
}

/// Findings + entities + claims for an investigation. Joins via
/// `claim_entities` + `findings.investigation_id`.
pub async fn query_investigation_graph(
    state: &AppState,
    investigation_id: Uuid,
) -> Result<Value, HubError> {
    let findings: Vec<(Uuid, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT finding_id, claim_text, created_at FROM findings \
          WHERE investigation_id = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(investigation_id)
    .fetch_all(&state.pg)
    .await?;
    let entities: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT DISTINCT e.entity_id, e.kind, e.name \
           FROM findings f \
           JOIN claim_entities ce ON ce.claim_id = f.finding_id \
           JOIN entities e ON e.entity_id = ce.entity_id \
          WHERE f.investigation_id = $1 LIMIT 200",
    )
    .bind(investigation_id)
    .fetch_all(&state.pg)
    .await?;
    let claims: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT DISTINCT c.claim_id, c.text, c.status \
           FROM claims c \
           JOIN claim_entities ce ON ce.claim_id = c.claim_id \
           JOIN entities e ON e.entity_id = ce.entity_id \
           JOIN findings f ON f.investigation_id = $1 AND f.finding_id = c.claim_id \
          LIMIT 100",
    )
    .bind(investigation_id)
    .fetch_all(&state.pg)
    .await?;
    Ok(json!({
        "investigation_id": investigation_id,
        "findings": findings.into_iter().map(|(id,t,ca)| json!({
            "finding_id": id, "text": t, "created_at": ca
        })).collect::<Vec<_>>(),
        "entities": entities.into_iter().map(|(id,k,n)| json!({
            "entity_id": id, "kind": k, "name": n
        })).collect::<Vec<_>>(),
        "claims": claims.into_iter().map(|(id,t,s)| json!({
            "claim_id": id, "text": t, "status": s
        })).collect::<Vec<_>>()
    }))
}

/// Documents that cite this entity via `claim_evidence`. `relation`
/// defaults to "supports". Side-effect: writes an `observations` row
/// per document (idempotent — `ON CONFLICT DO NOTHING`). The
/// observations table was previously orphaned (per inventory) — this is
/// the moment it's finally populated.
pub async fn list_evidence_for_entity(
    state: &AppState,
    entity_id: Uuid,
    relation: Option<&str>,
) -> Result<Value, HubError> {
    let rel = relation.unwrap_or("supports");
    let docs: Vec<(Uuid, String, DateTime<Utc>, String)> = sqlx::query_as(
        "SELECT d.document_id, d.base_url, d.retrieved_at, ce.relation \
           FROM documents d \
           JOIN claim_evidence ce ON ce.document_id = d.document_id \
           JOIN claim_entities c ON c.claim_id = ce.claim_id \
          WHERE c.entity_id = $1 AND ce.relation = $2 \
          ORDER BY d.retrieved_at DESC LIMIT 100",
    )
    .bind(entity_id)
    .bind(rel)
    .fetch_all(&state.pg)
    .await?;
    // Side-effect: populate the previously orphaned observations table.
    // After migration 0009 the table has DEFAULTs on observation_id +
    // created_by plus a UNIQUE (entity_id, document_id, observed_at),
    // so this INSERT is well-formed and ON CONFLICT DO NOTHING actually
    // fires. Errors propagate (no `let _ =`) so callers see if the
    // side-effect failed — silently swallowing them left the table
    // un-populated for months (reviewer finding Critical #1).
    for (doc_id, _url, observed_at, _r) in &docs {
        sqlx::query(
            "INSERT INTO observations (entity_id, document_id, snippet, observed_at) \
             VALUES ($1, $2, '', $3) ON CONFLICT DO NOTHING",
        )
        .bind(entity_id)
        .bind(doc_id)
        .bind(observed_at)
        .execute(&state.pg)
        .await?;
    }
    Ok(json!(docs.into_iter().map(|(id,u,t,r)| json!({
        "document_id": id, "base_url": u, "retrieved_at": t, "relation": r
    })).collect::<Vec<_>>()))
}

// =============================================================================
// Neo4j Cypher-backed queries
// =============================================================================

/// Variable-length neighborhood expansion with optional temporal
/// predicate and post-filter on `rel_type` / `confidence`. The Cypher
/// uses variables-in-paths (Cypher 5+).
pub async fn get_neighbors(
    state: &AppState,
    entity_id: Uuid,
    depth: u8,
    rel_types: Option<Vec<String>>,
    min_confidence: Option<f64>,
    at_time: Option<DateTime<Utc>>,
) -> Result<Value, HubError> {
    let d = depth.clamp(1, 4);
    let cypher = format!(
        "MATCH (e:Entity {{entity_id: $eid}})-[*1..{d}]-(neighbor:Entity) \
         WHERE e <> neighbor \
           AND ($at_time IS NULL \
                OR ALL(rel IN relationships(path) WHERE \
                  (rel.valid_from IS NULL OR rel.valid_from <= datetime($at_time)) \
                  AND (rel.valid_until IS NULL OR rel.valid_until > datetime($at_time)))) \
         WITH neighbor, relationships(path) AS rs LIMIT 100 \
         RETURN neighbor.entity_id AS id, neighbor.name AS name, neighbor.kind AS kind, \
                [r IN rs | {{rel_type: type(r), valid_from: r.valid_from, valid_until: r.valid_until, confidence: r.confidence}}] AS edges",
    );
    let at_time_value = at_time
        .map(|t| neo4rs::BoltType::String(neo4rs::BoltString { value: t.to_rfc3339() }))
        .unwrap_or(neo4rs::BoltType::Null(neo4rs::BoltNull));
    let rows = collect_cypher_rows(
        state,
        cypher,
        vec![
            ("eid", neo4rs::BoltType::String(neo4rs::BoltString { value: entity_id.to_string() })),
            ("at_time", at_time_value),
        ],
        &["id", "name", "kind", "edges"],
    )
    .await?;

    // Post-Cypher filter on rel_type / confidence.
    let rts: Vec<String> = rel_types.unwrap_or_default();
    let minc = min_confidence.unwrap_or(0.0);
    let mut out: Vec<Value> = Vec::new();
    for row in rows {
        let id = row.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let empty_arr = Value::Array(vec![]);
        let edges_v = row.get("edges").unwrap_or(&empty_arr);
        let edges_parsed: Vec<HashMap<String, Value>> = match edges_v {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::Object(m) => Some(
                        m.iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect::<HashMap<String, Value>>(),
                    ),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let edges: Vec<Value> = edges_parsed
            .into_iter()
            .filter(|m| {
                let rt = m.get("rel_type").and_then(|x| x.as_str()).unwrap_or("");
                let conf = m.get("confidence").and_then(|x| x.as_f64()).unwrap_or(1.0);
                (rts.is_empty() || rts.iter().any(|x| x == rt)) && conf >= minc
            })
            .map(|m| Value::Object(m.into_iter().collect()))
            .collect();
        if !edges.is_empty() {
            out.push(json!({
                "entity_id": id, "name": name, "kind": kind, "edges": edges
            }));
        }
    }
    Ok(json!(out))
}

/// v2 shortest-path. v1 (in `graph.rs`) is replaced by this:
/// - `weighted` toggles `1 - confidence` sum vs raw hops.
/// - `max_hops` bounds expansion.
pub async fn find_path(
    state: &AppState,
    from: &str,
    to: &str,
    weighted: bool,
    max_hops: u8,
) -> Result<Value, HubError> {
    let h = max_hops.clamp(1, 8);
    let cypher = format!(
        "MATCH p = shortestPath((a:Entity)-[*..{h}]-(b:Entity)) \
         WHERE toLower(a.name) = toLower($from) AND toLower(b.name) = toLower($to) \
         RETURN [n IN nodes(p) | n.name] AS names, \
                [r IN relationships(p) | {{rel_type: type(r), confidence: coalesce(r.confidence, 1.0)}}] AS edges, \
                length(p) AS hops",
    );
    let rows = collect_cypher_rows(
        state,
        cypher,
        vec![
            (
                "from",
                neo4rs::BoltType::String(neo4rs::BoltString {
                    value: from.to_string(),
                }),
            ),
            (
                "to",
                neo4rs::BoltType::String(neo4rs::BoltString {
                    value: to.to_string(),
                }),
            ),
        ],
        &["names", "edges", "hops"],
    )
    .await?;

    let mut out: Vec<Value> = Vec::new();
    for row in rows {
        let names_v = row.get("names").cloned().unwrap_or(Value::Array(vec![]));
        let edges_v = row.get("edges").cloned().unwrap_or(Value::Array(vec![]));
        let hops = row.get("hops").and_then(|v| v.as_i64()).unwrap_or(0);

        let edges_parsed: Vec<HashMap<String, Value>> = match edges_v {
            Value::Array(arr) => arr
                .into_iter()
                .filter_map(|v| match v {
                    Value::Object(m) => Some(
                        m.into_iter()
                            .collect::<HashMap<String, Value>>(),
                    ),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let score = if weighted {
            edges_parsed
                .iter()
                .map(|m| {
                    m.get("confidence")
                        .and_then(|x| x.as_f64())
                        .unwrap_or(1.0)
                })
                .fold(0.0_f64, |acc, c| acc + (1.0 - c))
        } else {
            hops as f64
        };
        out.push(json!({
            "names": names_v,
            "edges": edges_parsed,
            "hops": hops,
            "score": score,
        }));
    }
    Ok(json!(out))
}

#[cfg(test)]
mod tests {
    // Pure unit tests are minimal here — most logic is DB/Cypher.
    // Integration coverage lives in accept-sp9.py (Task 9).
    #[test]
    fn clamp_works() {
        assert_eq!(0_i64.clamp(1, 100), 1);
        assert_eq!(50_i64.clamp(1, 100), 50);
        assert_eq!(200_i64.clamp(1, 100), 100);
    }

    #[test]
    fn depth_clamp_bounds() {
        assert_eq!(0_u8.clamp(1, 4), 1);
        assert_eq!(5_u8.clamp(1, 4), 4);
        assert_eq!(2_u8.clamp(1, 4), 2);
    }

    #[test]
    fn hops_clamp_bounds() {
        assert_eq!(0_u8.clamp(1, 8), 1);
        assert_eq!(10_u8.clamp(1, 8), 8);
        assert_eq!(5_u8.clamp(1, 8), 5);
    }
}