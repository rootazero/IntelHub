//! Graph write plane (directive §38) — typed graph intents.
//!
//! Agent → Typed Intent → schema validation → Level 2 authorization (policy.rs)
//! → PG canonical write (§36) → Neo4j parameterized MERGE (relationship memory,
//! §38) → audit + events. When Neo4j is unavailable the validated op lands in
//! graph_sync_queue (§72: graph updates are NEVER lost) and a replay worker
//! retries with backoff.

use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::{HubError, Result};
use crate::state::AppState;

/// §38 kind enum — entities outside this vocabulary are rejected.
pub const ENTITY_KINDS: &[&str] = &[
    "person", "org", "domain", "ip", "location", "event", "infrastructure",
    "software", "handle", "email", "phone", "crypto_wallet",
];

/// Relationship vocabulary — interpolated into Cypher ONLY after whitelist check.
pub const REL_TYPES: &[&str] = &[
    "controls", "owns", "communicates_with", "resolves_to", "located_in",
    "affiliated_with", "uses", "hosts", "registered_by", "related_to",
    // 2026-09-14: graph_v2 resolve.rs merge op (dropped entity → kept entity)
    // surfaces as a :MERGED_INTO edge. Same Entity↔Entity vocabulary.
    "MERGED_INTO",
];

/// Property allowlist (§61 Level 2 schema validation).
const ATTR_ALLOWLIST: &[&str] = &[
    "description", "url", "country", "confidence", "severity", "first_seen",
    "last_seen", "tags", "source",
];

pub(crate) fn valid_kind(kind: &str) -> Result<&str> {
    ENTITY_KINDS
        .iter()
        .find(|k| **k == kind)
        .copied()
        .ok_or_else(|| HubError::bad_request(format!("invalid entity kind '{kind}'")))
}

fn valid_rel(rel: &str) -> Result<&str> {
    REL_TYPES
        .iter()
        .find(|r| **r == rel)
        .copied()
        .ok_or_else(|| HubError::bad_request(format!("invalid relationship type '{rel}'")))
}

/// Name normalization: trim + collapse internal whitespace + cap 256 chars.
/// (Trim-only; full NFC noted as spec deviation — no extra dependency.)
pub(crate) fn normalize_name(raw: &str) -> Result<String> {
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let name: String = collapsed.chars().take(256).collect();
    if name.is_empty() {
        return Err(HubError::bad_request("entity name must not be empty"));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(HubError::bad_request("entity name contains control characters"));
    }
    Ok(name)
}

fn filter_attributes(attrs: Option<Value>) -> Result<Value> {
    match attrs {
        None => Ok(json!({})),
        Some(Value::Object(map)) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if ATTR_ALLOWLIST.contains(&k.as_str()) {
                    out.insert(k, v);
                }
            }
            Ok(Value::Object(out))
        }
        Some(_) => Err(HubError::bad_request("attributes must be an object")),
    }
}

fn filter_aliases(aliases: Option<Vec<String>>) -> Result<Value> {
    // 2026-09-15 fix (A-005/B-005/C-001): dedup with order preservation
    // before the JSON serialize. Without this, every create_entity re-seed
    // appends the full alias list via `entities.aliases || EXCLUDED.aliases`
    // and accumulates duplicates that bubble up to the /entities/:id UI.
    // 2026-09-16 fix (e2e BRICS A3): reject when callers send >10 aliases
    // instead of silently truncating — silent truncation hides caller
    // bugs (alias list leaked from a hot loop) and means the persisted
    // record no longer matches what the caller sent. Schema says max 10.
    const MAX_ALIASES: usize = 10;
    let raw = aliases.unwrap_or_default();
    if raw.len() > MAX_ALIASES {
        return Err(crate::error::HubError::bad_request(format!(
            "create_entity aliases cap is {max} (got {got})",
            max = MAX_ALIASES,
            got = raw.len()
        )));
    }
    let mut seen = std::collections::HashSet::new();
    let list: Vec<String> = raw
        .into_iter()
        .filter_map(|a| normalize_name(&a).ok())
        .filter(|a| seen.insert(a.clone()))
        .collect();
    Ok(json!(list))
}

/// Execute a validated graph op against Neo4j, or enqueue it when the graph is
/// unavailable. Returns true when the graph write succeeded synchronously.
async fn graph_write(state: &AppState, op: Value) -> bool {
    match try_graph_write(state, &op).await {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, "neo4j write failed — enqueueing for replay");
            let _ = sqlx::query(
                "INSERT INTO graph_sync_queue (op_id, op, last_error) VALUES ($1,$2,$3)",
            )
            .bind(Uuid::new_v4())
            .bind(&op)
            .bind(e.to_string())
            .execute(&state.pg)
            .await;
            false
        }
    }
}

async fn try_graph_write(state: &AppState, op: &Value) -> Result<()> {
    match op.get("type").and_then(|t| t.as_str()) {
        Some("create_entity") => {
            // 2026-09-14 fix: the op JSON now carries entity_id (from the PG
            // INSERT ... RETURNING). Legacy queue rows (pre-fix) lack it, so
            // the SET clause for entity_id is conditional — replay of an old
            // op must not fail on a missing param.
            let eid = op["entity_id"].as_str().unwrap_or("");
            if eid.is_empty() {
                let q = neo4rs::query(
                    "MERGE (e:Entity {kind: $kind, name: $name})
                     SET e.aliases = $aliases, e.attributes = $attributes",
                )
                .param("kind", op["kind"].as_str().unwrap_or(""))
                .param("name", op["name"].as_str().unwrap_or(""))
                .param("aliases", op["aliases"].to_string())
                .param("attributes", op["attributes"].to_string());
                state.neo4j.run(q).await?;
            } else {
                let q = neo4rs::query(
                    "MERGE (e:Entity {kind: $kind, name: $name})
                     SET e.entity_id = $eid, e.aliases = $aliases, e.attributes = $attributes",
                )
                .param("kind", op["kind"].as_str().unwrap_or(""))
                .param("name", op["name"].as_str().unwrap_or(""))
                .param("eid", eid)
                .param("aliases", op["aliases"].to_string())
                .param("attributes", op["attributes"].to_string());
                state.neo4j.run(q).await?;
            }
        }
        Some("create_relationship") => {
            // rel type interpolated ONLY after whitelist validation.
            let rel = valid_rel(op["rel_type"].as_str().unwrap_or(""))?;
            // 2026-09-14 fix: ops now carry from_id/to_id (+ relationship_id)
            // so the matched endpoints get their PG uuid backfilled and the
            // edge gets its canonical relationship_id. Legacy ops lack them;
            // fall back to the old statement so replay keeps working.
            let from_id = op["from_id"].as_str().unwrap_or("");
            let to_id = op["to_id"].as_str().unwrap_or("");
            let rel_id = op["relationship_id"].as_str().unwrap_or("");
            let has_ids = !from_id.is_empty() && !to_id.is_empty() && !rel_id.is_empty();
            if has_ids {
                let cypher = format!(
                    "MATCH (a:Entity {{kind: $fk, name: $fn}}), (b:Entity {{kind: $tk, name: $tn}})
                     SET a.entity_id = $from_id, b.entity_id = $to_id
                     MERGE (a)-[r:{rel}]->(b)
                     SET r.attributes = $attributes, r.relationship_id = $rel_id"
                );
                let q = neo4rs::query(&cypher)
                    .param("fk", op["from_kind"].as_str().unwrap_or(""))
                    .param("fn", op["from_name"].as_str().unwrap_or(""))
                    .param("tk", op["to_kind"].as_str().unwrap_or(""))
                    .param("tn", op["to_name"].as_str().unwrap_or(""))
                    .param("from_id", from_id)
                    .param("to_id", to_id)
                    .param("rel_id", rel_id)
                    .param("attributes", op["attributes"].to_string());
                state.neo4j.run(q).await?;
            } else {
                let cypher = format!(
                    "MATCH (a:Entity {{kind: $fk, name: $fn}}), (b:Entity {{kind: $tk, name: $tn}})
                     MERGE (a)-[r:{rel}]->(b)
                     SET r.attributes = $attributes"
                );
                let q = neo4rs::query(&cypher)
                    .param("fk", op["from_kind"].as_str().unwrap_or(""))
                    .param("fn", op["from_name"].as_str().unwrap_or(""))
                    .param("tk", op["to_kind"].as_str().unwrap_or(""))
                    .param("tn", op["to_name"].as_str().unwrap_or(""))
                    .param("attributes", op["attributes"].to_string());
                state.neo4j.run(q).await?;
            }
        }
        Some("create_claim") => {
            let q = neo4rs::query(
                "MERGE (c:Claim {claim_id: $cid}) SET c.text = $text, c.status = $status",
            )
            .param("cid", op["claim_id"].as_str().unwrap_or(""))
            .param("text", op["text"].as_str().unwrap_or(""))
            .param("status", op["status"].as_str().unwrap_or("unverified"));
            state.neo4j.run(q).await?;
            if let Some(entities) = op.get("entities").and_then(|e| e.as_array()) {
                for ent in entities {
                    // 2026-09-14 phase 3 fix: the audit path may reference
                    // entities that were just created in PG and haven't
                    // been mirrored to Neo4j yet (no (insert, entity)
                    // change_log row fired in time). MERGE both endpoints
                    // so the :ABOUT edge materialises regardless of the
                    // entity mirror race. If the entity later gets a
                    // proper entity_id from create_entity, the SET on
                    // merge updates the same node — no duplication.
                    let q = neo4rs::query(
                        "MERGE (e:Entity {kind: $kind, name: $name})
                         WITH e
                         MATCH (c:Claim {claim_id: $cid})
                         MERGE (c)-[:ABOUT]->(e)",
                    )
                    .param("cid", op["claim_id"].as_str().unwrap_or(""))
                    .param("kind", ent["kind"].as_str().unwrap_or(""))
                    .param("name", ent["name"].as_str().unwrap_or(""));
                    state.neo4j.run(q).await?;
                }
            }
        }
        // 2026-09-14: mirror path for graph_v2's claim_evidence / finding_about
        // / contradiction / merge ops. See translate_change_log in this file
        // and docs/superpowers/specs/2026-09-14-intelhub-graph-v2-mirror-design.md.
        Some("create_document") => {
            let q = neo4rs::query(
                "MERGE (d:Document {document_id: $did})
                 SET d.url_canonical = $url, d.title = $title, d.source_id = $sid",
            )
            .param("did", op["document_id"].as_str().unwrap_or(""))
            .param("url", op["url_canonical"].as_str().unwrap_or(""))
            .param("title", op["title"].as_str().unwrap_or(""))
            .param("sid", op["source_id"].as_str().unwrap_or(""));
            state.neo4j.run(q).await?;
        }
        Some("create_finding") => {
            let q = neo4rs::query(
                "MERGE (f:Finding {finding_id: $fid})
                 SET f.title = $title, f.claim_text = $claim_text,
                     f.source_confidence = $sc, f.claim_confidence = $cc,
                     f.investigation_id = $iid",
            )
            .param("fid", op["finding_id"].as_str().unwrap_or(""))
            .param("title", op["title"].as_str().unwrap_or(""))
            .param("claim_text", op["claim_text"].as_str().unwrap_or(""))
            .param("sc", op["source_confidence"].as_f64().unwrap_or(0.0))
            .param("cc", op["claim_confidence"].as_f64().unwrap_or(0.0))
            .param("iid", op["investigation_id"].as_str().unwrap_or(""));
            state.neo4j.run(q).await?;
        }
        Some("create_contradiction") => {
            // :Contradiction node + 2 :INVOLVES edges to the disputed claims.
            // INVOLVES is interpolated after an inline whitelist check.
            let q = neo4rs::query(
                "MERGE (c:Contradiction {contradiction_id: $cid})
                 SET c.claim_a = $a, c.claim_b = $b, c.reason = $reason
                 WITH c
                 MATCH (a:Claim {claim_id: $a}), (b:Claim {claim_id: $b})
                 MERGE (a)-[:INVOLVES]->(c)
                 MERGE (b)-[:INVOLVES]->(c)",
            )
            .param("cid", op["contradiction_id"].as_str().unwrap_or(""))
            .param("a", op["claim_a"].as_str().unwrap_or(""))
            .param("b", op["claim_b"].as_str().unwrap_or(""))
            .param("reason", op["reason"].as_str().unwrap_or(""));
            state.neo4j.run(q).await?;
        }
        Some("link_claim_evidence") => {
            // Claim → Document via :SUPPORTS or :CONTRADICTS. The relation
            // string is interpolated into Cypher ONLY after whitelist check
            // (inline; not a general Entity↔Entity rel type).
            let rel = match op["relation"].as_str().unwrap_or("") {
                "contradicts" => "CONTRADICTS",
                _ => "SUPPORTS",
            };
            let cypher = format!(
                "MERGE (d:Document {{document_id: $did}})
                 MERGE (c:Claim {{claim_id: $cid}})
                 MERGE (c)-[r:{rel}]->(d)"
            );
            let q = neo4rs::query(&cypher)
                .param("did", op["document_id"].as_str().unwrap_or(""))
                .param("cid", op["claim_id"].as_str().unwrap_or(""));
            state.neo4j.run(q).await?;
        }
        Some("link_finding_about_entity") => {
            // Finding → Entity :ABOUT edge. v2 only — v1 doesn't write
            // finding→entity links (finding_evidence is finding→doc only).
            let q = neo4rs::query(
                "MERGE (f:Finding {finding_id: $fid})
                 WITH f
                 MATCH (e:Entity {entity_id: $eid})
                 MERGE (f)-[:ABOUT]->(e)",
            )
            .param("fid", op["finding_id"].as_str().unwrap_or(""))
            .param("eid", op["entity_id"].as_str().unwrap_or(""));
            state.neo4j.run(q).await?;
        }
        Some("link_finding_evidence") => {
            // Finding → Document :SUPPORTS edge. Backfill source is PG
            // finding_evidence; future writes go through v1's
            // create_finding path (if it exists) or graph_v2's link
            // evidence flow. v2 doesn't currently expose finding-evidence
            // linking, so this op is mostly for backfill + extensibility.
            let q = neo4rs::query(
                "MERGE (f:Finding {finding_id: $fid})
                 MERGE (d:Document {document_id: $did})
                 MERGE (f)-[:SUPPORTS]->(d)",
            )
            .param("fid", op["finding_id"].as_str().unwrap_or(""))
            .param("did", op["document_id"].as_str().unwrap_or(""));
            state.neo4j.run(q).await?;
        }
        other => {
            return Err(HubError::bad_request(format!(
                "unknown graph op {other:?}"
            )))
        }
    }
    Ok(())
}

// ---------- typed intents (called by MCP tools) ----------

#[derive(Debug, Deserialize)]
pub struct EntityIntent {
    pub kind: String,
    pub name: String,
    pub aliases: Option<Vec<String>>,
    pub attributes: Option<Value>,
}

pub async fn create_entity(state: &AppState, actor: &str, intent: EntityIntent) -> Result<Value> {
    let kind = valid_kind(&intent.kind)?.to_string();
    let name = normalize_name(&intent.name)?;
    let aliases = filter_aliases(intent.aliases)?;
    let attributes = filter_attributes(intent.attributes)?;

    // 2026-09-15 fix (A-005/B-005/C-001): use array_distinct() so the merge
    // of the existing and incoming aliases deduplicates in-Postgres.
    // Previously `entities.aliases || EXCLUDED.aliases` concatenated and
    // accumulated duplicates on every re-seed (BRICS 12×, NASA FIRMS 82×).
    let (entity_id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO entities (entity_id, kind, name, aliases, attributes, created_by)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (kind, name) DO UPDATE
           SET aliases = (
                 SELECT to_jsonb(array_agg(DISTINCT v))
                 FROM jsonb_array_elements_text(
                   COALESCE(entities.aliases, '[]'::jsonb) ||
                   COALESCE(EXCLUDED.aliases, '[]'::jsonb)
                 ) AS v
               ),
               attributes = entities.attributes || EXCLUDED.attributes
         RETURNING entity_id",
    )
    .bind(Uuid::new_v4())
    .bind(&kind)
    .bind(&name)
    .bind(&aliases)
    .bind(&attributes)
    .bind(actor)
    .fetch_one(&state.pg)
    .await?;

    let op = json!({
        "type": "create_entity",
        "entity_id": entity_id.to_string(),
        "kind": kind, "name": name,
        "aliases": aliases, "attributes": attributes,
    });
    let synced = graph_write(state, op).await;

    // 2026-09-16 fix (e2e BRICS B-Audit): the v1 create_entity path
    // historically skipped the graph_change_log audit row that
    // v1 create_claim writes (graphw.rs:511) and that the v2 compiler
    // arms (graph_v2/compiler.rs) write. get_entity_timeline reads
    // graph_change_log to surface the entity creation event; without
    // this INSERT, the timeline returned [] for entities that had
    // been alive for hours. Mirror job also relies on the change_log
    // to back-fill Neo4j when the in-line graph_write fails.
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by)
         VALUES ('insert','entity',$1, NULL, jsonb_build_object('kind',$2,'name',$3,'aliases',$4::jsonb,'attributes',$5::jsonb), $6)",
    )
    .bind(entity_id.to_string())
    .bind(&kind)
    .bind(&name)
    .bind(&aliases)
    .bind(&attributes)
    .bind(actor)
    .execute(&state.pg)
    .await?;

    crate::events::publish(
        state,
        BusEvent::new(
            "GRAPH_ENTITY_CREATED",
            actor,
            json!({ "entity_id": entity_id, "kind": kind, "name": name, "graph_synced": synced }),
        ),
    )
    .await;
    Ok(json!({ "entity_id": entity_id, "kind": kind, "name": name, "graph_synced": synced }))
}

#[derive(Debug, Deserialize)]
pub struct ClaimEntityRef {
    pub kind: String,
    pub name: String,
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimIntent {
    pub text: String,
    /// Entities the claim is about (created if missing).
    pub entities: Option<Vec<ClaimEntityRef>>,
    /// §43: every claim must trace to evidence — at least one document.
    pub evidence_document_ids: Vec<String>,
}

pub async fn create_claim(state: &AppState, actor: &str, intent: ClaimIntent) -> Result<Value> {
    let text = intent.text.trim().to_string();
    if text.len() < 8 || text.len() > 4000 {
        return Err(HubError::bad_request("claim text must be 8–4000 chars"));
    }
    if intent.evidence_document_ids.is_empty() {
        return Err(HubError::policy_denied(
            "claims require at least one evidence document (§43)",
        ));
    }
    // Verify all evidence documents exist (fail fast, no orphan links).
    let mut doc_ids = Vec::new();
    for raw in &intent.evidence_document_ids {
        let id = Uuid::parse_str(raw)
            .map_err(|_| HubError::bad_request(format!("invalid document id '{raw}'")))?;
        let exists: Option<(Uuid,)> =
            sqlx::query_as("SELECT document_id FROM documents WHERE document_id = $1")
                .bind(id)
                .fetch_optional(&state.pg)
                .await?;
        if exists.is_none() {
            return Err(HubError::bad_request(format!("evidence document {id} not found")));
        }
        doc_ids.push(id);
    }

    let claim_id = Uuid::new_v4();
    sqlx::query("INSERT INTO claims (claim_id, text, created_by) VALUES ($1,$2,$3)")
        .bind(claim_id)
        .bind(&text)
        .bind(actor)
        .execute(&state.pg)
        .await?;
    for doc_id in &doc_ids {
        // PG row: idempotent via ON CONFLICT DO NOTHING (claim_evidence PK
        // is (claim_id, document_id, relation)). This is PG canonical —
        // duplicate inserts don't break anything.
        let _res = sqlx::query(
            "INSERT INTO claim_evidence (claim_id, document_id) VALUES ($1,$2)
             ON CONFLICT DO NOTHING",
        )
        .bind(claim_id)
        .bind(doc_id)
        .execute(&state.pg)
        .await?;

        // 2026-09-14: always emit per-evidence-row link_claim_evidence op so
        // the Neo4j mirror worker materializes the Claim→Document :SUPPORTS
        // edge for THIS claim. v1 ClaimIntent has no `relation` field
        // (mcp.rs:370) — the PG claim_evidence table has no relation column
        // either, so "supports" is the implicit default. Hardcoded to match.
        // graph_write itself is idempotent on the Neo4j side (MERGE on
        // both endpoints + edge), so re-running create_claim with the same
        // claim+doc pair is harmless — but here we run for every doc_id, so
        // a SECOND claim reusing the same docs ALSO gets its own edges.
        let link_op = json!({
            "type": "link_claim_evidence",
            "claim_id": claim_id,
            "document_id": doc_id,
            "relation": "supports",
        });
        graph_write(state, link_op).await;
    }

    // Resolve/create referenced entities + link them.
    let mut graph_entities = Vec::new();
    for ent in intent.entities.unwrap_or_default() {
        let e = create_entity(
            state,
            actor,
            EntityIntent {
                kind: ent.kind,
                name: ent.name,
                aliases: None,
                attributes: None,
            },
        )
        .await?;
        let role = ent.role.unwrap_or_else(|| "mentioned".into());
        if !["subject", "object", "mentioned"].contains(&role.as_str()) {
            return Err(HubError::bad_request(format!("invalid claim entity role '{role}'")));
        }
        sqlx::query(
            "INSERT INTO claim_entities (claim_id, entity_id, role) VALUES ($1,$2,$3)
             ON CONFLICT DO NOTHING",
        )
        .bind(claim_id)
        .bind(Uuid::parse_str(e["entity_id"].as_str().unwrap_or_default()).unwrap_or_default())
        .bind(&role)
        .execute(&state.pg)
        .await?;
        graph_entities.push(json!({ "kind": e["kind"], "name": e["name"] }));
    }

    let op = json!({
        "type": "create_claim", "claim_id": claim_id, "text": text,
        "status": "unverified", "entities": graph_entities,
    });
    let synced = graph_write(state, op).await;

    // 2026-09-14 phase 3: change_log audit row. If graph_write above
    // failed (Neo4j down etc.), the next mirror tick will retry the
    // mirror via translate_change_log's create_claim arm (matches
    // after={text,status} with neither `linked` nor `entity_id`).
    // Backfill scripts also replay this shape, so historical claims
    // gain a mirror if Neo4j ever goes cold.
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by)
         VALUES ('insert','claim',$1, NULL, jsonb_build_object('text',$2,'status',$3,'entities',$4::jsonb), $5)",
    )
    .bind(claim_id.to_string())
    .bind(&text)
    .bind("unverified")
    .bind(Value::Array(graph_entities.clone()))
    .bind(actor)
    .execute(&state.pg)
    .await?;

    crate::events::publish(
        state,
        BusEvent::new(
            "GRAPH_CLAIM_CREATED",
            actor,
            json!({ "claim_id": claim_id, "graph_synced": synced }),
        ),
    )
    .await;
    Ok(json!({ "claim_id": claim_id, "graph_synced": synced }))
}

#[derive(Debug, Deserialize)]
pub struct RelationshipIntent {
    pub from_kind: String,
    pub from_name: String,
    pub to_kind: String,
    pub to_name: String,
    pub rel_type: String,
    pub attributes: Option<Value>,
}

pub async fn create_relationship(
    state: &AppState,
    actor: &str,
    intent: RelationshipIntent,
) -> Result<Value> {
    let from_kind = valid_kind(&intent.from_kind)?.to_string();
    let to_kind = valid_kind(&intent.to_kind)?.to_string();
    let rel_type = valid_rel(&intent.rel_type)?.to_string();
    let from_name = normalize_name(&intent.from_name)?;
    let to_name = normalize_name(&intent.to_name)?;
    let attributes = filter_attributes(intent.attributes)?;

    // Both endpoints must exist in PG canonical store (graph mirrors it).
    // 2026-09-16 fix (e2e BRICS B-Rel-Race): the original lookup fired a
    // single SELECT and returned "endpoints must exist" if either row was
    // missing. With sqlx's pool (max 8 conns), the create_entity INSERT
    // can land on conn A while the relationship lookup hits conn B and
    // sees a snapshot before the insert committed — a true read-after-write
    // race. The e2e BRICS test hit this for BRICS→mBridge (entity just
    // created 1 call earlier). We retry up to 3× with exponential backoff
    // before failing; this absorbs the race without requiring the caller
    // to know about it. The error message also now reports WHICH endpoint
    // is missing so the caller can fix the typo.
    let mut from: Option<Uuid> = None;
    for attempt in 0..3u32 {
        let res = sqlx::query_as::<_, (Uuid,)>(
            "SELECT entity_id FROM entities WHERE kind=$1 AND name=$2",
        )
        .bind(&from_kind)
        .bind(&from_name)
        .fetch_optional(&state.pg)
        .await;
        if let Ok(Some((id,))) = res {
            from = Some(id);
            break;
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(50 * (1u64 << attempt))).await;
        }
    }
    let mut to: Option<Uuid> = None;
    for attempt in 0..3u32 {
        let res = sqlx::query_as::<_, (Uuid,)>(
            "SELECT entity_id FROM entities WHERE kind=$1 AND name=$2",
        )
        .bind(&to_kind)
        .bind(&to_name)
        .fetch_optional(&state.pg)
        .await;
        if let Ok(Some((id,))) = res {
            to = Some(id);
            break;
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(50 * (1u64 << attempt))).await;
        }
    }
    let (Some(from_id), Some(to_id)) = (from, to) else {
        let missing = match (from, to) {
            (None, None) => format!("from={from_kind}:{from_name} AND to={to_kind}:{to_name}"),
            (None, _) => format!("from={from_kind}:{from_name}"),
            (_, None) => format!("to={to_kind}:{to_name}"),
            _ => unreachable!(),
        };
        return Err(HubError::bad_request(format!(
            "create_relationship: missing endpoint entity: {missing}. \
             Call create_entity(kind, name) for the missing side first."
        )));
    };

    let (rel_id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO relationships (relationship_id, from_entity, to_entity, rel_type, attributes, created_by)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (from_entity, to_entity, rel_type) DO UPDATE
           SET attributes = relationships.attributes || EXCLUDED.attributes
         RETURNING relationship_id",
    )
    .bind(Uuid::new_v4())
    .bind(from_id)
    .bind(to_id)
    .bind(&rel_type)
    .bind(&attributes)
    .bind(actor)
    .fetch_one(&state.pg)
    .await?;

    let op = json!({
        "type": "create_relationship",
        "relationship_id": rel_id.to_string(),
        "from_id": from_id.to_string(),
        "to_id": to_id.to_string(),
        "from_kind": from_kind, "from_name": from_name,
        "to_kind": to_kind, "to_name": to_name,
        "rel_type": rel_type, "attributes": attributes,
    });
    let synced = graph_write(state, op).await;

    // 2026-09-16 fix (e2e BRICS B-Audit): same rationale as create_entity
    // above — v1 create_relationship was the second missing arm. The
    // change_log drives get_entity_timeline + the Neo4j mirror worker;
    // without this row, a relationship is "invisible" to both. The v2
    // compiler already does this; we're back-filling v1 parity.
    sqlx::query(
        "INSERT INTO graph_change_log (op, target_kind, target_id, before, after, changed_by)
         VALUES ('insert','relationship',$1, NULL, jsonb_build_object('from_kind',$3,'from_id',$4,'to_kind',$5,'to_id',$6,'rel_type',$7,'attributes',$8::jsonb), $2)",
    )
    .bind(rel_id.to_string())
    .bind(actor)
    .bind(&from_kind)
    .bind(from_id.to_string())
    .bind(&to_kind)
    .bind(to_id.to_string())
    .bind(&rel_type)
    .bind(&attributes)
    .execute(&state.pg)
    .await?;

    crate::events::publish(
        state,
        BusEvent::new(
            "GRAPH_REL_CREATED",
            actor,
            json!({
                "relationship_id": rel_id, "rel_type": rel_type,
                "from": format!("{from_kind}:{from_name}"),
                "to": format!("{to_kind}:{to_name}"),
                "graph_synced": synced,
            }),
        ),
    )
    .await;
    Ok(json!({ "relationship_id": rel_id, "graph_synced": synced }))
}

// ---------- replay worker (§72) ----------

pub async fn run_replay(state: AppState, ct: tokio_util::sync::CancellationToken) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = ct.cancelled() => break,
            _ = tick.tick() => {
                if let Err(e) = replay_once(&state).await {
                    tracing::warn!(error = %e, "graph replay tick failed");
                }
            }
        }
    }
}

async fn replay_once(state: &AppState) -> Result<()> {
    let pending: Vec<(Uuid, Value, i32)> = sqlx::query_as(
        "SELECT op_id, op, attempts FROM graph_sync_queue
         WHERE status = 'PENDING' ORDER BY created_at LIMIT 20",
    )
    .fetch_all(&state.pg)
    .await?;
    for (op_id, op, attempts) in pending {
        match try_graph_write(state, &op).await {
            Ok(()) => {
                sqlx::query(
                    "UPDATE graph_sync_queue SET status='DONE', processed_at=now() WHERE op_id=$1",
                )
                .bind(op_id)
                .execute(&state.pg)
                .await?;
            }
            Err(e) if attempts >= 4 => {
                sqlx::query(
                    "UPDATE graph_sync_queue SET status='FAILED', attempts=attempts+1, last_error=$2
                     WHERE op_id=$1",
                )
                .bind(op_id)
                .bind(e.to_string())
                .execute(&state.pg)
                .await?;
                let _ = crate::alerts::raise(
                    state,
                    crate::alerts::NewAlert {
                        severity: "warning",
                        source: "infra",
                        title: "Graph sync op failed permanently after 5 attempts",
                        body: Some(&e.to_string()),
                        task_id: None,
                        investigation_id: None,
                        entity_name: None,
                        evidence_id: None,
                        recommended_action: Some("Inspect graph_sync_queue and Neo4j health"),
                        dedupe_key: Some("graphsync:failed"),
                    },
                )
                .await;
            }
            Err(e) => {
                sqlx::query(
                    "UPDATE graph_sync_queue SET attempts=attempts+1, last_error=$2 WHERE op_id=$1",
                )
                .bind(op_id)
                .bind(e.to_string())
                .execute(&state.pg)
                .await?;
            }
        }
    }
    Ok(())
}

// ---------- change_log mirror worker (SP9 v2 path → Neo4j) ----------
//
// Background: graph_v2 (SP9 compiler) writes canonical state to PG + an
// audit row to graph_change_log, but never reached Neo4j. The v1 replay
// worker drains graph_sync_queue, so the natural fix is: this worker
// reads un-mirrored change_log rows, translates to v1 op shapes, and
// enqueues them into graph_sync_queue. Downstream v1 path handles
// Neo4j writes (MERGE-based, idempotent). See docs/superpowers/specs/
// 2026-09-14-intelhub-graph-v2-mirror-design.md for the full translation
// table and rationale.

pub async fn run_change_log_mirror(state: AppState, ct: tokio_util::sync::CancellationToken) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = ct.cancelled() => break,
            _ = tick.tick() => {
                if let Err(e) = mirror_change_log_once(&state).await {
                    tracing::warn!(error = %e, "change_log mirror tick failed");
                }
            }
        }
    }
}

async fn mirror_change_log_once(state: &AppState) -> Result<()> {
    // 1. Pull a batch of un-mirrored change_log rows.
    let rows: Vec<(i64, String, String, String, Value)> = sqlx::query_as(
        "SELECT change_id, op, target_kind, target_id, after
         FROM graph_change_log
         WHERE mirrored_at IS NULL
         ORDER BY change_id
         LIMIT 50",
    )
    .fetch_all(&state.pg)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }

    // 2. Pre-resolve kind/name for any row that produces an
    //    Entity↔Entity relationship (so the worker can construct
    //    v1 create_relationship ops without re-issuing per-op
    //    SELECTs): current rows (subject/object/predicate), merge
    //    rows (dropped + kept).
    let mut resolve_uuids: Vec<String> = Vec::new();
    for (_, op, tk, target_id, after) in &rows {
        if (op == "insert" || op == "update") && tk == "relationship" {
            if let Some(s) = after.get("subject").and_then(|v| v.as_str()) {
                resolve_uuids.push(s.to_string());
            }
            if let Some(o) = after.get("object").and_then(|v| v.as_str()) {
                resolve_uuids.push(o.to_string());
            }
        } else if op == "merge" && tk == "entity" {
            if !target_id.is_empty() {
                resolve_uuids.push(target_id.to_string());
            }
            if let Some(k) = after.get("merged_into").and_then(|v| v.as_str()) {
                if !k.is_empty() {
                    resolve_uuids.push(k.to_string());
                }
            }
        }
    }
    resolve_uuids.sort();
    resolve_uuids.dedup();
    let name_map: std::collections::HashMap<String, (String, String)> = if resolve_uuids.is_empty()
    {
        Default::default()
    } else {
        let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
            "SELECT entity_id, kind, name FROM entities
             WHERE entity_id::text = ANY($1)",
        )
        .bind(&resolve_uuids)
        .fetch_all(&state.pg)
        .await?;
        rows.into_iter()
            .map(|(id, k, n)| (id.to_string(), (k, n)))
            .collect()
    };

    // 3. Translate each row → one or more v1 ops, enqueue them.
    let mut all_queued: Vec<Uuid> = Vec::new();
    for (change_id, op, target_kind, target_id, after) in &rows {
        let ops = translate_change_log(op, target_kind, target_id, after, &name_map);
        for v1op in ops {
            let (op_id,): (Uuid,) = sqlx::query_as(
                "INSERT INTO graph_sync_queue (op_id, op) VALUES ($1, $2)
                 ON CONFLICT (op_id) DO NOTHING
                 RETURNING op_id",
            )
            .bind(Uuid::new_v4())
            .bind(&v1op)
            .fetch_one(&state.pg)
            .await?;
            all_queued.push(op_id);
        }
        // Mark change_log row as mirrored once its ops are durably queued.
        // We mark per-row even if no ops were produced (skipped kinds) so
        // the worker doesn't re-scan them forever.
        sqlx::query(
            "UPDATE graph_change_log SET mirrored_at = now() WHERE change_id = $1",
        )
        .bind(change_id)
        .execute(&state.pg)
        .await?;
    }

    if !all_queued.is_empty() {
        tracing::debug!(count = all_queued.len(), "change_log mirror enqueued ops");
    }
    Ok(())
}

fn translate_change_log(
    op: &str,
    target_kind: &str,
    target_id: &str,
    after: &Value,
    name_map: &std::collections::HashMap<String, (String, String)>,
) -> Vec<Value> {
    let mut out = Vec::new();
    match (op, target_kind) {
        ("insert" | "update", "entity") => {
            let kind = after
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = after
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if kind.is_empty() || name.is_empty() {
                tracing::warn!(target_id, "change_log entity row missing kind/name in after");
                return out;
            }
            out.push(json!({
                "type": "create_entity",
                "entity_id": target_id,
                "kind": kind,
                "name": name,
                "aliases": Value::Array(Vec::new()),
                "attributes": Value::Object(Default::default()),
            }));
        }
        ("insert" | "update", "relationship") => {
            let subject = after.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let object = after.get("object").and_then(|v| v.as_str()).unwrap_or("");
            let predicate = after
                .get("predicate")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if subject.is_empty() || object.is_empty() || predicate.is_empty() {
                tracing::warn!(target_id, "change_log rel row missing subject/object/predicate");
                return out;
            }
            let (from_kind, from_name) = name_map
                .get(subject)
                .cloned()
                .unwrap_or_else(|| (String::new(), String::new()));
            let (to_kind, to_name) = name_map
                .get(object)
                .cloned()
                .unwrap_or_else(|| (String::new(), String::new()));
            if from_kind.is_empty() || to_kind.is_empty() {
                tracing::warn!(
                    target_id,
                    subject,
                    object,
                    "change_log rel row: endpoint not in PG entities"
                );
                return out;
            }
            // No relationship_id in change_log — the v1 mirror path is
            // fine without it (MERGE on the (fk,fn,tk,tn,rel) tuple).
            out.push(json!({
                "type": "create_relationship",
                "from_id": subject,
                "to_id": object,
                "from_kind": from_kind, "from_name": from_name,
                "to_kind": to_kind, "to_name": to_name,
                "rel_type": predicate,
                "attributes": Value::Object(Default::default()),
            }));
        }
        // 2026-09-14: claim evidence + finding-about + contradiction +
        // entity-merge translations. The change_log op kind 'insert' with
        // target_kind 'claim' has THREE sub-shapes — disambiguate by after
        // JSON: {linked,relation} = link_evidence_to_claim;
        //         {entity_id,relation:"about"} = mark_finding_about_entity;
        //         {text,status,...} = create_claim (new in phase 3).
        ("insert" | "update", "claim") => {
            if let (Some(document_id), Some(relation)) = (
                after.get("linked").and_then(|v| v.as_str()),
                after.get("relation").and_then(|v| v.as_str()),
            ) {
                if !target_id.is_empty() && !document_id.is_empty() {
                    out.push(json!({
                        "type": "link_claim_evidence",
                        "claim_id": target_id,
                        "document_id": document_id,
                        "relation": relation,
                    }));
                } else {
                    tracing::warn!(target_id, "claim/link_evidence: empty uuid");
                }
            } else if let Some(entity_id) =
                after.get("entity_id").and_then(|v| v.as_str())
            {
                // target_id here is the finding_id (the v2 compiler binds
                // finding_id to the $1 column, not a claim_id).
                if !target_id.is_empty() && !entity_id.is_empty() {
                    out.push(json!({
                        "type": "link_finding_about_entity",
                        "finding_id": target_id,
                        "entity_id": entity_id,
                    }));
                } else {
                    tracing::warn!(target_id, "claim/finding_about: empty uuid");
                }
            } else if let Some(text) =
                after.get("text").and_then(|v| v.as_str())
            {
                // 2026-09-14 phase 3: bare claim audit row (from
                // create_claim in this file). target_id is the claim_id.
                // entities[] is optional — create_claim passes the graph
                // entities (kind/name pairs) when the claim references any.
                if !target_id.is_empty() && !text.is_empty() {
                    let status = after
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unverified");
                    let entities = after
                        .get("entities")
                        .cloned()
                        .unwrap_or(Value::Array(Vec::new()));
                    out.push(json!({
                        "type": "create_claim",
                        "claim_id": target_id,
                        "text": text,
                        "status": status,
                        "entities": entities,
                    }));
                } else {
                    tracing::warn!(target_id, "claim/create_claim: empty uuid or text");
                }
            } else {
                tracing::warn!(target_id, "claim change_log row with unrecognized after shape");
            }
        }
        ("contradict", "contradiction") => {
            // PG claim_contradictions uses BIGINT; change_log binds
            // contradiction_id::text so we keep the string form.
            let claim_a = after.get("a").and_then(|v| v.as_str()).unwrap_or("");
            let claim_b = after.get("b").and_then(|v| v.as_str()).unwrap_or("");
            let reason = after.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            if target_id.is_empty() || claim_a.is_empty() || claim_b.is_empty() {
                tracing::warn!(target_id, "contradict row missing ids");
                return out;
            }
            out.push(json!({
                "type": "create_contradiction",
                "contradiction_id": target_id,
                "claim_a": claim_a,
                "claim_b": claim_b,
                "reason": reason,
            }));
        }
        ("merge", "entity") => {
            // graph_v2 resolve::apply_merge: dropped entity gets
            // merged_into=kept. Surface as :MERGED_INTO edge (audit trail
            // — neither node is deleted, edges from dropped are kept for
            // history; the canonical kept node is what new queries should
            // follow).
            let dropped = target_id;
            let kept = after
                .get("merged_into")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if dropped.is_empty() || kept.is_empty() {
                tracing::warn!(dropped, "merge row missing merged_into");
                return out;
            }
            let (from_kind, from_name) = name_map
                .get(dropped)
                .cloned()
                .unwrap_or_default();
            let (to_kind, to_name) = name_map
                .get(kept)
                .cloned()
                .unwrap_or_default();
            if from_kind.is_empty() || to_kind.is_empty() {
                tracing::warn!(dropped, kept, "merge: endpoint not in PG entities");
                return out;
            }
            out.push(json!({
                "type": "create_relationship",
                "from_id": dropped, "to_id": kept,
                "from_kind": from_kind, "from_name": from_name,
                "to_kind": to_kind, "to_name": to_name,
                "rel_type": "MERGED_INTO",
                "attributes": Value::Object(Default::default()),
            }));
        }
        _ => {
            // Op kinds we don't mirror yet:
            //   merge + entity        → split / re-attach (modeling decision)
            //   contradict + contradiction → no :Contradiction node label yet
            //   any + claim           → claim↔document / finding↔entity
            //                           (needs new Neo4j node labels)
            //   temporal_close        → Neo4j has no edge valid_until yet
            // See design doc.
        }
    }
    out
}

// ============================================================
// Phase 3 — Neo4j ↔ PG reconciliation worker
// ============================================================
//
// The change_log mirror worker is one-way: PG → Neo4j. It never removes
// Neo4j nodes. If a PG row gets deleted (currently no production path
// does this, but admin tools or future retract endpoints could) the
// corresponding Neo4j node becomes an orphan.
//
// This worker closes the loop defensively. On startup + every hour, it
// diffs Neo4j against PG for the 5 main tables and DETACH DELETEs
// orphans. Idempotent — empty diffs are the steady state.
//
// Tables covered:
//   * entities            — (kind, name) → :Entity
//   * relationships       — (from_id, to_id, rel_type) → :REL
//   * claims              — claim_id → :Claim
//   * findings            — finding_id → :Finding
//   * documents           — document_id → :Document
//   * claim_evidence      — (claim_id, document_id, relation) → claim-doc edge
//   * finding_evidence    — (finding_id, document_id) → finding-doc :SUPPORTS
//   * finding_entities    — (finding_id, entity_id) → finding-entity :ABOUT
//
// Edges not in PG (e.g. :MERGED_INTO) are skipped — they're mirror
// audit edges with no PG source.

pub async fn run_reconcile(state: AppState, ct: tokio_util::sync::CancellationToken) {
    // First pass at startup after a short grace period (let the rest of
    // the stack come up), then every 15 minutes — matches the change_log
    // mirror cadence so cleanup lag is bounded by the same interval.
    // Hourly was the initial value; tightened to 15 min after the
    // first VM 410 startup showed orphan accumulation between hourly
    // ticks during heavy test churn.
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    if let Err(e) = reconcile_once(&state).await {
        tracing::warn!(error = %e, "initial reconcile failed");
    }
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(900));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ct.cancelled() => break,
            _ = tick.tick() => {
                if let Err(e) = reconcile_once(&state).await {
                    tracing::warn!(error = %e, "reconcile tick failed");
                }
            }
        }
    }
}

async fn reconcile_once(state: &AppState) -> Result<()> {
    // Open a run row for visibility / accept-sp10 check.
    let (run_id,): (i64,) =
        sqlx::query_as("INSERT INTO mirror_reconcile_runs DEFAULT VALUES RETURNING run_id")
            .fetch_one(&state.pg)
            .await?;

    // Helper: drain a Cypher RETURN into a Vec<Value>, one per row.
    async fn fetch_rows(
        state: &AppState,
        cypher: &str,
        cols: &[&str],
    ) -> Result<Vec<Value>> {
        let q = neo4rs::query(cypher);
        let mut stream = state.neo4j.execute(q).await?;
        let mut out: Vec<Value> = Vec::new();
        loop {
            match stream.next().await {
                Ok(Some(row)) => {
                    let mut obj = serde_json::Map::new();
                    for col in cols {
                        if let Ok(v) = row.get::<neo4rs::BoltType>(col) {
                            obj.insert((*col).to_string(), crate::graph_queries::bolt_to_json(&v));
                        }
                    }
                    out.push(Value::Object(obj));
                }
                Ok(None) => break,
                Err(e) => {
                    return Err(crate::error::HubError::GraphUnavailable(format!(
                        "neo4j stream: {e}"
                    )));
                }
            }
        }
        Ok(out)
    }
    fn s_of(v: &Value, key: &str) -> String {
        v.get(key)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    }

    // ---- 1. Orphan :Entity nodes (no PG row with same kind+name) ----
    let entity_pg: std::collections::HashSet<(String, String)> = {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT kind, name FROM entities")
                .fetch_all(&state.pg)
                .await?;
        rows.into_iter().collect()
    };
    let entity_neo: Vec<(String, String)> = fetch_rows(state, "MATCH (e:Entity) RETURN e.kind AS k, e.name AS n", &["k","n"])
        .await?
        .into_iter()
        .map(|r| (s_of(&r, "k"), s_of(&r, "n")))
        .filter(|(k, n)| !k.is_empty() && !n.is_empty())
        .collect();
    let mut orphan_entities: Vec<(String, String)> = Vec::new();
    for kn in &entity_neo {
        if !entity_pg.contains(kn) {
            orphan_entities.push(kn.clone());
        }
    }
    let mut total_removed = 0usize;
    for (k, n) in &orphan_entities {
        // Safe to DETACH DELETE here: orphans by definition have no PG
        // canonical counterparts, so no PG FK references them.
        let cypher = "MATCH (e:Entity {kind: $k, name: $n}) DETACH DELETE e";
        let q = neo4rs::query(cypher)
            .param("k", k.as_str())
            .param("n", n.as_str());
        state.neo4j.run(q).await?;
        total_removed += 1;
    }

    // ---- 2. Orphan :Claim nodes (no PG row with same claim_id) ----
    let claim_pg: std::collections::HashSet<String> = {
        let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT claim_id FROM claims")
            .fetch_all(&state.pg)
            .await?;
        rows.into_iter().map(|(c,)| c.to_string()).collect()
    };
    let claim_neo: Vec<String> = fetch_rows(state, "MATCH (c:Claim) RETURN c.claim_id AS cid", &["cid"])
        .await?
        .into_iter()
        .map(|r| s_of(&r, "cid"))
        .filter(|s| !s.is_empty())
        .collect();
    let mut orphan_claims: Vec<String> = Vec::new();
    for cid in &claim_neo {
        if !claim_pg.contains(cid) {
            orphan_claims.push(cid.clone());
        }
    }
    for cid in &orphan_claims {
        let cypher = "MATCH (c:Claim {claim_id: $cid}) DETACH DELETE c";
        let q = neo4rs::query(cypher).param("cid", cid.as_str());
        state.neo4j.run(q).await?;
        total_removed += 1;
    }

    // ---- 3. Orphan :Finding nodes ----
    let finding_pg: std::collections::HashSet<String> = {
        let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT finding_id FROM findings")
            .fetch_all(&state.pg)
            .await?;
        rows.into_iter().map(|(c,)| c.to_string()).collect()
    };
    let finding_neo: Vec<String> = fetch_rows(state, "MATCH (f:Finding) RETURN f.finding_id AS fid", &["fid"])
        .await?
        .into_iter()
        .map(|r| s_of(&r, "fid"))
        .filter(|s| !s.is_empty())
        .collect();
    let mut orphan_findings: Vec<String> = Vec::new();
    for fid in &finding_neo {
        if !finding_pg.contains(fid) {
            orphan_findings.push(fid.clone());
        }
    }
    for fid in &orphan_findings {
        let cypher = "MATCH (f:Finding {finding_id: $fid}) DETACH DELETE f";
        let q = neo4rs::query(cypher).param("fid", fid.as_str());
        state.neo4j.run(q).await?;
        total_removed += 1;
    }

    // ---- 4. Orphan :Document nodes ----
    let doc_pg: std::collections::HashSet<String> = {
        let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT document_id FROM documents")
            .fetch_all(&state.pg)
            .await?;
        rows.into_iter().map(|(c,)| c.to_string()).collect()
    };
    let doc_neo: Vec<String> = fetch_rows(state, "MATCH (d:Document) RETURN d.document_id AS did", &["did"])
        .await?
        .into_iter()
        .map(|r| s_of(&r, "did"))
        .filter(|s| !s.is_empty())
        .collect();
    let mut orphan_docs: Vec<String> = Vec::new();
    for did in &doc_neo {
        if !doc_pg.contains(did) {
            orphan_docs.push(did.clone());
        }
    }
    for did in &orphan_docs {
        let cypher = "MATCH (d:Document {document_id: $did}) DETACH DELETE d";
        let q = neo4rs::query(cypher).param("did", did.as_str());
        state.neo4j.run(q).await?;
        total_removed += 1;
    }

    // ---- 5. Stale claim-doc edges (claim_evidence) ----
    let edge_keys_pg: std::collections::HashSet<(String, String, String)> = {
        let rows: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT claim_id, document_id, relation FROM claim_evidence",
        )
        .fetch_all(&state.pg)
        .await?;
        rows.into_iter()
            .map(|(c, d, r)| (c.to_string(), d.to_string(), r))
            .collect()
    };
    let edge_rows_neo: Vec<(String, String, String)> = fetch_rows(
        state,
        "MATCH (c:Claim)-[r]->(d:Document)
         WHERE type(r) IN ['SUPPORTS','CONTRADICTS']
         RETURN c.claim_id AS cid, d.document_id AS did, type(r) AS rel",
        &["cid", "did", "rel"],
    )
    .await?
    .into_iter()
    .map(|r| (s_of(&r, "cid"), s_of(&r, "did"), s_of(&r, "rel")))
    .collect();
    let mut stale_claim_edges = 0usize;
    for (cid, did, rel) in &edge_rows_neo {
        let rel_norm = if rel == "CONTRADICTS" { "contradicts" } else { "supports" };
        let key = (cid.clone(), did.clone(), rel_norm.to_string());
        if !edge_keys_pg.contains(&key) {
            let cypher = format!(
                "MATCH (c:Claim {{claim_id: $cid}})-[r:{rel}]->(d:Document {{document_id: $did}}) DELETE r"
            );
            let q = neo4rs::query(&cypher)
                .param("cid", cid.as_str())
                .param("did", did.as_str());
            state.neo4j.run(q).await?;
            total_removed += 1;
            stale_claim_edges += 1;
        }
    }

    // ---- 6. Stale finding-doc edges (finding_evidence) ----
    let fedges_pg: std::collections::HashSet<(String, String)> = {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT finding_id, document_id FROM finding_evidence",
        )
        .fetch_all(&state.pg)
        .await?;
        rows.into_iter()
            .map(|(c, d)| (c.to_string(), d.to_string()))
            .collect()
    };
    let fedges_neo: Vec<(String, String)> = fetch_rows(
        state,
        "MATCH (f:Finding)-[:SUPPORTS]->(d:Document)
         RETURN f.finding_id AS fid, d.document_id AS did",
        &["fid", "did"],
    )
    .await?
    .into_iter()
    .map(|r| (s_of(&r, "fid"), s_of(&r, "did")))
    .filter(|(f, d)| !f.is_empty() && !d.is_empty())
    .collect();
    let mut stale_finding_doc_edges = 0usize;
    for (fid, did) in &fedges_neo {
        let key = (fid.clone(), did.clone());
        if !fedges_pg.contains(&key) {
            let cypher = "MATCH (f:Finding {finding_id: $fid})-[r:SUPPORTS]->(d:Document {document_id: $did}) DELETE r";
            let q = neo4rs::query(cypher)
                .param("fid", fid.as_str())
                .param("did", did.as_str());
            state.neo4j.run(q).await?;
            total_removed += 1;
            stale_finding_doc_edges += 1;
        }
    }

    // ---- 7. Stale finding-entity :ABOUT edges ----
    let fes_pg: std::collections::HashSet<(String, String)> = {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT finding_id, entity_id FROM finding_entities",
        )
        .fetch_all(&state.pg)
        .await?;
        rows.into_iter()
            .map(|(c, d)| (c.to_string(), d.to_string()))
            .collect()
    };
    let fes_neo: Vec<(String, String)> = fetch_rows(
        state,
        "MATCH (f:Finding)-[:ABOUT]->(e:Entity)
         RETURN f.finding_id AS fid, e.entity_id AS eid",
        &["fid", "eid"],
    )
    .await?
    .into_iter()
    .map(|r| (s_of(&r, "fid"), s_of(&r, "eid")))
    .filter(|(f, e)| !f.is_empty() && !e.is_empty())
    .collect();
    let mut stale_finding_entity_edges = 0usize;
    for (fid, eid) in &fes_neo {
        let key = (fid.clone(), eid.clone());
        if !fes_pg.contains(&key) {
            let cypher = "MATCH (f:Finding {finding_id: $fid})-[r:ABOUT]->(e:Entity {entity_id: $eid}) DELETE r";
            let q = neo4rs::query(cypher)
                .param("fid", fid.as_str())
                .param("eid", eid.as_str());
            state.neo4j.run(q).await?;
            total_removed += 1;
            stale_finding_entity_edges += 1;
        }
    }

    // ---- 8. 2026-09-15 fix (B-001, B-003, B-004): reverse backfill.
    //     The sections above only clean Neo4j → PG drift (orphans).
    //     This section closes the PG → Neo4j direction: when the
    //     graph_sync_queue replay fails (Neo4j unreachable during the
    //     window), the PG canonical row stays in PG and never gets
    //     mirrored. Reconciliation backfills these by enqueueing
    //     idempotent ops into graph_sync_queue (graph_write itself uses
    //     MERGE on both endpoints, so retries are safe).
    //
    //     Dedupe: op_id is UUID v5 from a fixed namespace + the natural
    //     key, so repeated reconcile ticks don't requeue the same
    //     backfill (ON CONFLICT (op_id) DO NOTHING).
    let mut backfilled_claims = 0usize;
    let mut backfilled_claim_edges = 0usize;
    let mut backfilled_docs = 0usize;
    // Fixed namespace UUID for backfill dedupe (random once, deterministic).
    let backfill_ns: Uuid = Uuid::parse_str("8a3d4f1e-9b27-4c5d-b1e6-7f8a9c0d2e3b").unwrap();

    // 8a. PG claims missing from Neo4j
    let claim_pg_ids: Vec<Uuid> = sqlx::query_as("SELECT claim_id FROM claims")
        .fetch_all(&state.pg).await?
        .into_iter().map(|(c,)| c).collect();
    let claim_neo_ids: std::collections::HashSet<String> = fetch_rows(
        state,
        "MATCH (c:Claim) RETURN c.claim_id AS cid",
        &["cid"],
    ).await?
    .into_iter().map(|r| s_of(&r, "cid"))
    .filter(|s| !s.is_empty()).collect();
    for cid in &claim_pg_ids {
        if claim_neo_ids.contains(&cid.to_string()) { continue; }
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT text, COALESCE(status::text, 'unverified') FROM claims WHERE claim_id = $1",
        ).bind(cid).fetch_optional(&state.pg).await?;
        if let Some((text, status)) = row {
            // claim_entities only carries entity_id; JOIN to get the kind/name
            // the create_claim op needs for the :ABOUT edge materialization.
            let ents: Vec<(String, String)> = sqlx::query_as(
                "SELECT e.kind, e.name FROM claim_entities ce
                 JOIN entities e ON e.entity_id = ce.entity_id
                 WHERE ce.claim_id = $1",
            ).bind(cid).fetch_all(&state.pg).await?;
            let mut op = json!({
                "type": "create_claim",
                "claim_id": cid.to_string(),
                "text": text,
                "status": status,
            });
            if !ents.is_empty() {
                let arr: Vec<Value> = ents.iter().map(|(k, n)| json!({"kind": k, "name": n})).collect();
                op.as_object_mut().unwrap().insert("entities".into(), json!(arr));
            }
            let op_id = Uuid::new_v5(&backfill_ns, format!("claim:{}", cid).as_bytes());
            let inserted = sqlx::query(
                "INSERT INTO graph_sync_queue (op_id, op) VALUES ($1, $2)
                 ON CONFLICT (op_id) DO NOTHING",
            ).bind(op_id).bind(&op).execute(&state.pg).await?;
            if inserted.rows_affected() > 0 { backfilled_claims += 1; }
        }
    }

    // 8b. PG claim_evidence rows missing from Neo4j :SUPPORTS edge
    let ce_pg: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT claim_id, document_id FROM claim_evidence",
    ).fetch_all(&state.pg).await?;
    let ce_neo: std::collections::HashSet<(String, String)> = fetch_rows(
        state,
        "MATCH (c:Claim)-[r:SUPPORTS]->(d:Document)
         RETURN c.claim_id AS cid, d.document_id AS did",
        &["cid", "did"],
    ).await?
    .into_iter().map(|r| (s_of(&r, "cid"), s_of(&r, "did")))
    .filter(|(c, d)| !c.is_empty() && !d.is_empty()).collect();
    for (cid, did) in &ce_pg {
        let key = (cid.to_string(), did.to_string());
        if ce_neo.contains(&key) { continue; }
        let op_id = Uuid::new_v5(&backfill_ns, format!("claim-edge:{}:{}", cid, did).as_bytes());
        let op = json!({
            "type": "link_claim_evidence",
            "claim_id": cid.to_string(),
            "document_id": did.to_string(),
            "relation": "supports",
        });
        let inserted = sqlx::query(
            "INSERT INTO graph_sync_queue (op_id, op) VALUES ($1, $2)
             ON CONFLICT (op_id) DO NOTHING",
        ).bind(op_id).bind(&op).execute(&state.pg).await?;
        if inserted.rows_affected() > 0 { backfilled_claim_edges += 1; }
    }

    // 8c. PG documents missing from Neo4j :Document
    let doc_pg_ids: Vec<Uuid> = sqlx::query_as("SELECT document_id FROM documents")
        .fetch_all(&state.pg).await?
        .into_iter().map(|(d,)| d).collect();
    let doc_neo_ids: std::collections::HashSet<String> = fetch_rows(
        state,
        "MATCH (d:Document) RETURN d.document_id AS did",
        &["did"],
    ).await?
    .into_iter().map(|r| s_of(&r, "did"))
    .filter(|s| !s.is_empty()).collect();
    for did in &doc_pg_ids {
        if doc_neo_ids.contains(&did.to_string()) { continue; }
        let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT url_canonical, title, source_id::text
             FROM documents WHERE document_id = $1",
        ).bind(did).fetch_optional(&state.pg).await?;
        if let Some((url, title, source_id)) = row {
            let op_id = Uuid::new_v5(&backfill_ns, format!("doc:{}", did).as_bytes());
            let op = json!({
                "type": "create_document",
                "document_id": did.to_string(),
                "url_canonical": url,
                "title": title.unwrap_or_default(),
                "source_id": source_id.unwrap_or_default(),
            });
            let inserted = sqlx::query(
                "INSERT INTO graph_sync_queue (op_id, op) VALUES ($1, $2)
                 ON CONFLICT (op_id) DO NOTHING",
            ).bind(op_id).bind(&op).execute(&state.pg).await?;
            if inserted.rows_affected() > 0 { backfilled_docs += 1; }
        }
    }

    if backfilled_claims + backfilled_claim_edges + backfilled_docs > 0 {
        tracing::info!(
            run_id,
            backfilled_claims, backfilled_claim_edges, backfilled_docs,
            "neo4j reconcile backfilled PG→Neo4j drift into graph_sync_queue"
        );
    }

    // Record run result.
    let details = json!({
        "orphan_entities": orphan_entities.len(),
        "orphan_claims": orphan_claims.len(),
        "orphan_findings": orphan_findings.len(),
        "orphan_documents": orphan_docs.len(),
        "stale_claim_doc_edges": stale_claim_edges,
        "stale_finding_doc_edges": stale_finding_doc_edges,
        "stale_finding_entity_edges": stale_finding_entity_edges,
        "backfilled_claims": backfilled_claims,
        "backfilled_claim_edges": backfilled_claim_edges,
        "backfilled_documents": backfilled_docs,
    });
    sqlx::query(
        "UPDATE mirror_reconcile_runs
            SET finished_at = now(),
                orphans_removed = $1,
                details = $2
          WHERE run_id = $3",
    )
    .bind(total_removed as i32)
    .bind(&details)
    .bind(run_id)
    .execute(&state.pg)
    .await?;

    tracing::info!(
        run_id,
        orphans_removed = total_removed,
        ?details,
        "neo4j reconcile pass complete"
    );

    // Notify only when orphans were removed — steady-state runs are
    // silent. The alert goes through the unified alerts engine so it
    // picks up dedupe, decay cooldown, webhook delivery, and console
    // visibility for free. dedupe_key keeps the alert cluster to one
    // open row per source-table-kind (e.g. "reconcile:orphan:claim")
    // so repeated runs BUMP rather than spam.
    if total_removed > 0 {
        let top_kind = pick_top_kind(&details);
        let body = format!(
            "Reconcile run #{run_id} removed {total_removed} orphans: {details}"
        );
        if let Err(e) = crate::alerts::raise(state, crate::alerts::NewAlert {
            severity: "warning",
            source: "infra",
            title: &format!("Neo4j mirror drift: {total_removed} orphans cleaned ({top_kind})"),
            body: Some(&body),
            task_id: None,
            investigation_id: None,
            entity_name: None,
            evidence_id: None,
            recommended_action: Some("review graph_change_log + mirror_reconcile_runs for root cause"),
            dedupe_key: Some(&format!("reconcile:orphan:{top_kind}")),
        }).await {
            tracing::warn!(error = %e, "reconcile orphan alert raise failed");
        }
    }

    Ok(())
}

/// Pick the largest orphan bucket so the alert title points operators
/// at the right table. Stable across runs that produce the same shape.
fn pick_top_kind(details: &Value) -> &'static str {
    let buckets: [(&str, usize); 7] = [
        ("entity", details.get("orphan_entities").and_then(|v| v.as_u64()).unwrap_or(0) as usize),
        ("claim", details.get("orphan_claims").and_then(|v| v.as_u64()).unwrap_or(0) as usize),
        ("finding", details.get("orphan_findings").and_then(|v| v.as_u64()).unwrap_or(0) as usize),
        ("document", details.get("orphan_documents").and_then(|v| v.as_u64()).unwrap_or(0) as usize),
        ("claim_doc_edge", details.get("stale_claim_doc_edges").and_then(|v| v.as_u64()).unwrap_or(0) as usize),
        ("finding_doc_edge", details.get("stale_finding_doc_edges").and_then(|v| v.as_u64()).unwrap_or(0) as usize),
        ("finding_entity_edge", details.get("stale_finding_entity_edges").and_then(|v| v.as_u64()).unwrap_or(0) as usize),
    ];
    buckets.into_iter().max_by_key(|(_, n)| *n).map(|(k, _)| k).unwrap_or("unknown")
}

use crate::types::BusEvent;
