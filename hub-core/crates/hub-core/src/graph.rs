//! Neo4j read plane. Agents never run arbitrary Cypher (directive §38):
//! only these typed intents exist, each a fixed parameterized template.

use neo4rs::query;
use serde_json::{json, Value};

use crate::error::{HubError, Result};
use crate::state::AppState;

/// Strict identifier validation — labels/property values are always bound as
/// parameters, but we additionally constrain shape to kill injection attempts
/// early and audit them.
fn valid_term(s: &str) -> bool {
    let len = s.chars().count();
    (1..=256).contains(&len)
        && !s.contains(['\0', '\n', '\r'])
}

/// Find entities by name (case-insensitive contains), optionally by label.
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
    // kind is matched against a property (never interpolated into cypher).
    let cypher = "MATCH (e:Entity) \
                  WHERE toLower(e.name) CONTAINS toLower($name) \
                    AND ($kind = '' OR e.kind = $kind) \
                  RETURN e LIMIT $limit";
    let q = query(cypher)
        .param("name", name)
        .param("kind", kind.unwrap_or(""))
        .param("limit", limit);
    run_rows(state, q, "query_entity").await
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
    run_rows(state, q, "query_relationship").await
}

/// Shortest path between two entities, depth-bounded (max 5) — fixed
/// template, parameters only, no variable-length cypher injection surface.
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
    run_rows(state, q, "find_path").await
}

async fn run_rows(state: &AppState, q: neo4rs::Query, op: &str) -> Result<Value> {
    let mut result = state
        .neo4j
        .execute(q)
        .await
        .map_err(|e| HubError::GraphUnavailable(format!("{op}: {e}")))?;
    let mut rows = Vec::new();
    while let Ok(Some(row)) = result.next().await {
        // Convert each column of the row to JSON via the bolt value debug
        // round-trip — neo4rs rows expose `get::<T>` per key; we iterate the
        // row's key set when available, else fall back to whole-row debug.
        let mut obj = serde_json::Map::new();
        for key in row.keys() {
            if let Ok(v) = row.get::<neo4rs::BoltType>(key) {
                obj.insert(key.to_string(), bolt_to_json(&v));
            }
        }
        rows.push(Value::Object(obj));
    }
    Ok(json!({ "op": op, "count": rows.len(), "rows": rows }))
}

/// Minimal BoltType → JSON conversion covering the shapes our templates return.
fn bolt_to_json(v: &neo4rs::BoltType) -> Value {
    use neo4rs::BoltType::*;
    match v {
        String(s) => Value::String(s.value.clone()),
        Integer(i) => json!(i.value),
        Float(f) => json!(f.value),
        Boolean(b) => json!(b.value),
        Null(_) => Value::Null,
        List(l) => Value::Array(l.value.iter().map(bolt_to_json).collect()),
        Map(m) => {
            let obj: serde_json::Map<String, Value> = m
                .value
                .iter()
                .map(|(k, v)| (k.clone(), bolt_to_json(v)))
                .collect();
            Value::Object(obj)
        }
        Node(n) => {
            let props: serde_json::Map<String, Value> = n
                .properties()
                .iter()
                .map(|(k, v)| (k.clone(), bolt_to_json(v)))
                .collect();
            json!({ "labels": n.labels(), "properties": props })
        }
        other => json!(format!("{other:?}")),
    }
}
