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
];

/// Property allowlist (§61 Level 2 schema validation).
const ATTR_ALLOWLIST: &[&str] = &[
    "description", "url", "country", "confidence", "severity", "first_seen",
    "last_seen", "tags", "source",
];

fn valid_kind(kind: &str) -> Result<&str> {
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
fn normalize_name(raw: &str) -> Result<String> {
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
    let list: Vec<String> = aliases
        .unwrap_or_default()
        .into_iter()
        .filter_map(|a| normalize_name(&a).ok())
        .take(10)
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
            let q = neo4rs::query(
                "MERGE (e:Entity {kind: $kind, name: $name})
                 SET e.aliases = $aliases, e.attributes = $attributes",
            )
            .param("kind", op["kind"].as_str().unwrap_or(""))
            .param("name", op["name"].as_str().unwrap_or(""))
            .param("aliases", op["aliases"].to_string())
            .param("attributes", op["attributes"].to_string());
            state.neo4j.run(q).await?;
        }
        Some("create_relationship") => {
            // rel type interpolated ONLY after whitelist validation.
            let rel = valid_rel(op["rel_type"].as_str().unwrap_or(""))?;
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
                    let q = neo4rs::query(
                        "MATCH (c:Claim {claim_id: $cid}), (e:Entity {kind: $kind, name: $name})
                         MERGE (c)-[:ABOUT]->(e)",
                    )
                    .param("cid", op["claim_id"].as_str().unwrap_or(""))
                    .param("kind", ent["kind"].as_str().unwrap_or(""))
                    .param("name", ent["name"].as_str().unwrap_or(""));
                    state.neo4j.run(q).await?;
                }
            }
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

    let (entity_id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO entities (entity_id, kind, name, aliases, attributes, created_by)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (kind, name) DO UPDATE
           SET aliases = entities.aliases || EXCLUDED.aliases,
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
        "type": "create_entity", "kind": kind, "name": name,
        "aliases": aliases, "attributes": attributes,
    });
    let synced = graph_write(state, op).await;

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
        sqlx::query(
            "INSERT INTO claim_evidence (claim_id, document_id) VALUES ($1,$2)
             ON CONFLICT DO NOTHING",
        )
        .bind(claim_id)
        .bind(doc_id)
        .execute(&state.pg)
        .await?;
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
    let from: Option<(Uuid,)> =
        sqlx::query_as("SELECT entity_id FROM entities WHERE kind=$1 AND name=$2")
            .bind(&from_kind)
            .bind(&from_name)
            .fetch_optional(&state.pg)
            .await?;
    let to: Option<(Uuid,)> =
        sqlx::query_as("SELECT entity_id FROM entities WHERE kind=$1 AND name=$2")
            .bind(&to_kind)
            .bind(&to_name)
            .fetch_optional(&state.pg)
            .await?;
    let (Some((from_id,)), Some((to_id,))) = (from, to) else {
        return Err(HubError::bad_request(
            "both relationship endpoints must exist (create them with create_entity first)",
        ));
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
        "from_kind": from_kind, "from_name": from_name,
        "to_kind": to_kind, "to_name": to_name,
        "rel_type": rel_type, "attributes": attributes,
    });
    let synced = graph_write(state, op).await;

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

use crate::types::BusEvent;
