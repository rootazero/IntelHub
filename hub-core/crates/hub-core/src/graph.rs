//! Neo4j read plane. Agents never run arbitrary Cypher (directive §38):
//! only these typed intents exist, each a fixed parameterized template.

use neo4rs::query;
use serde_json::{json, Value};

use crate::error::{HubError, Result};
use crate::state::AppState;

/// Strict identifier validation — values are always bound as parameters,
/// but we additionally constrain shape to kill injection attempts early.
fn valid_term(s: &str) -> bool {
    let len = s.chars().count();
    (1..=256).contains(&len) && !s.contains(['\0', '\n', '\r'])
}

/// Find entities by name (case-insensitive contains), optionally by kind.
/// Returns each match with its Neo4j internal `id` (stable for the lifetime
/// of the database — survives REINDEX unlike properties), so callers don't
/// have to fall back to (kind, name) → re-CREATE round-trips just to learn
/// what to put in `from_kind`/`to_kind` of subsequent create_relationship calls.
pub async fn query_entity(state: &AppState, name: &str, kind: Option<&str>, limit: i64) -> Result<Value> {
    if !valid_term(name) {
        return Err(HubError::bad_request("invalid entity name"));
    }
    if let Some(k) = kind {
        if !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || k.len() > 64 {
            return Err(HubError::bad_request("invalid entity kind"));
        }
    }
    let limit = limit.clamp(1, 100);
    // Everything matched/bound via parameters — never interpolated.
    // `id(e)` is the Neo4j internal node id (Int64); `e.name` is the
    // natural key. Callers should prefer (kind, name) for follow-ups,
    // but `id` is exposed for diagnostics and log correlation.
    let cypher = "MATCH (e:Entity) \
                  WHERE toLower(e.name) CONTAINS toLower($name) \
                    AND ($kind = '' OR e.kind = $kind) \
                  RETURN id(e) AS id, e.name AS name, e.kind AS kind, properties(e) AS props \
                  LIMIT $limit";
    let q = query(cypher)
        .param("name", name)
        .param("kind", kind.unwrap_or(""))
        .param("limit", limit);
    run_rows(state, q, "query_entity", &["id", "name", "kind", "props"]).await
}

/// Relationships of a given entity name (both directions), bounded.
pub async fn query_relationship(state: &AppState, name: &str, limit: i64) -> Result<Value> {
    if !valid_term(name) {
        return Err(HubError::bad_request("invalid entity name"));
    }
    let limit = limit.clamp(1, 200);
    let cypher = "MATCH (e:Entity)-[r]-(o:Entity) \
                  WHERE toLower(e.name) = toLower($name) \
                  RETURN e.name AS from, type(r) AS rel, properties(r) AS props, o.name AS to \
                  LIMIT $limit";
    let q = query(cypher).param("name", name).param("limit", limit);
    run_rows(state, q, "query_relationship", &["from", "rel", "props", "to"]).await
}

/// Shortest path between two entities, depth-bounded (max 5) — fixed
/// template, parameters only.
pub async fn find_path(state: &AppState, from: &str, to: &str) -> Result<Value> {
    if !valid_term(from) || !valid_term(to) {
        return Err(HubError::bad_request("invalid entity name"));
    }
    let cypher = "MATCH p = shortestPath((a:Entity)-[*..5]-(b:Entity)) \
                  WHERE toLower(a.name) = toLower($from) AND toLower(b.name) = toLower($to) \
                  RETURN [n IN nodes(p) | n.name] AS nodes, \
                         [r IN relationships(p) | type(r)] AS relationships, \
                         length(p) AS hops \
                  LIMIT 5";
    let q = query(cypher).param("from", from).param("to", to);
    run_rows(state, q, "find_path", &["nodes", "relationships", "hops"]).await
}

/// Execute a template and extract its known RETURN columns (neo4rs rows are
/// accessed by key; our templates declare exactly these columns).
async fn run_rows(state: &AppState, q: neo4rs::Query, op: &str, cols: &[&str]) -> Result<Value> {
    let mut result = state
        .neo4j
        .execute(q)
        .await
        .map_err(|e| HubError::GraphUnavailable(format!("{op}: {e}")))?;
    let mut rows = Vec::new();
    loop {
        match result.next().await {
            Ok(Some(row)) => {
                let mut obj = serde_json::Map::new();
                for col in cols {
                    if let Ok(v) = row.get::<neo4rs::BoltType>(col) {
                        obj.insert((*col).to_string(), bolt_to_json(&v));
                    }
                }
                rows.push(Value::Object(obj));
            }
            Ok(None) => break,
            Err(e) => return Err(HubError::GraphUnavailable(format!("{op} stream: {e}"))),
        }
    }
    Ok(json!({ "op": op, "count": rows.len(), "rows": rows }))
}

/// Minimal BoltType → JSON conversion covering our templates' return shapes.
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

#[cfg(test)]
mod bolt_json_tests_placeholder {}
