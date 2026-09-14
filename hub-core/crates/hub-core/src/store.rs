//! Postgres repository layer. Runtime-checked queries (no compile-time DB
//! needed for Docker builds). API-facing reads return serde_json::Value;
//! internal paths use typed returns.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{HubError, Result};
use crate::types::EvidenceEvent;

// ---------- agents / api_keys ----------

pub async fn create_agent(pg: &PgPool, name: &str, version: Option<&str>) -> Result<(Uuid, String)> {
    let agent_id = Uuid::new_v4();
    sqlx::query("INSERT INTO agents (agent_id, name, version) VALUES ($1, $2, $3)")
        .bind(agent_id)
        .bind(name)
        .bind(version)
        .execute(pg)
        .await?;
    let key = format!("ihk_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let key_hash = crate::auth::hash_key(&key);
    sqlx::query("INSERT INTO api_keys (key_id, agent_id, key_hash, label) VALUES ($1, $2, $3, $4)")
        .bind(Uuid::new_v4())
        .bind(agent_id)
        .bind(key_hash)
        .bind("initial")
        .execute(pg)
        .await?;
    Ok((agent_id, key))
}

/// Rotate (replace) the API key for an existing agent. The previous key is
/// soft-revoked (`revoked = true`) — its row stays in `api_keys` for audit /
/// trace purposes — and a fresh key is minted. The agent_id is preserved so
/// MCP clients that hold the agent_id still see continuity. No restart is
/// needed: hub-core looks keys up by hash on every request.
///
/// Returns `(agent_id, new_api_key)` on success.
pub async fn rotate_agent_key(pg: &PgPool, name: &str) -> Result<(Uuid, String)> {
    // Look up the existing agent. `agents.name` is UNIQUE so this is one row.
    let row = sqlx::query("SELECT agent_id FROM agents WHERE name = $1")
        .bind(name)
        .fetch_optional(pg)
        .await?;
    let row = row.ok_or_else(|| {
        HubError::bad_request(format!(
            "agent '{name}' is not provisioned yet — use create-agent first"
        ))
    })?;
    use sqlx::Row;
    let agent_id: Uuid = row.get(0);

    // Soft-revoke all existing live keys for this agent. Auth's resolve_key
    // filters `revoked = false` so old keys stop working the instant the
    // UPDATE commits — well within the next MCP request.
    sqlx::query("UPDATE api_keys SET revoked = true WHERE agent_id = $1 AND revoked = false")
        .bind(agent_id)
        .execute(pg)
        .await?;

    // Mint a fresh key + insert as the new live row.
    let key = format!("ihk_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let key_hash = crate::auth::hash_key(&key);
    sqlx::query("INSERT INTO api_keys (key_id, agent_id, key_hash, label) VALUES ($1, $2, $3, $4)")
        .bind(Uuid::new_v4())
        .bind(agent_id)
        .bind(key_hash)
        .bind("rotated")
        .execute(pg)
        .await?;
    Ok((agent_id, key))
}

/// Resolve a presented bearer key to (agent_id, key_id, name).
pub async fn resolve_key(pg: &PgPool, presented: &str) -> Result<Option<(Uuid, Uuid, String)>> {
    let hash = crate::auth::hash_key(presented);
    let row = sqlx::query(
        "SELECT k.agent_id, k.key_id, a.name FROM api_keys k \
         JOIN agents a ON a.agent_id = k.agent_id \
         WHERE k.key_hash = $1 AND k.revoked = false",
    )
    .bind(hash)
    .fetch_optional(pg)
    .await?;
    Ok(row.map(|r| {
        use sqlx::Row;
        (r.get::<Uuid, _>(0), r.get::<Uuid, _>(1), r.get::<String, _>(2))
    }))
}

// ---------- investigations ----------

#[allow(clippy::too_many_arguments)]
pub async fn create_investigation(
    pg: &PgPool,
    title: &str,
    question: Option<&str>,
    target: Option<&str>,
    hypothesis: Option<&str>,
    created_by: &str,
) -> Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO investigations (investigation_id, title, question, target, hypothesis, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(title)
    .bind(question)
    .bind(target)
    .bind(hypothesis)
    .bind(created_by)
    .execute(pg)
    .await?;
    Ok(id)
}

/// Seed an investigation created from a Radar geo event (console "convert to
/// investigation"): capture the event as an evidence document and record a
/// system finding bound to it (§29 evidence chain), so a converted
/// investigation never opens empty. Best-effort — callers log and continue on
/// failure. Returns false when the event no longer exists.
pub async fn seed_from_geo_event(pg: &PgPool, investigation_id: Uuid, event_id: Uuid) -> Result<bool> {
    let row = sqlx::query(
        "SELECT source, kind, title, lat, lon, severity, occurred_at, payload \
         FROM geo_events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_optional(pg)
    .await?;
    let Some(row) = row else { return Ok(false) };
    use sqlx::Row;
    let source: String = row.get(0);
    let kind: String = row.get(1);
    let title: String = row.get(2);
    let lat: Option<f64> = row.get(3);
    let lon: Option<f64> = row.get(4);
    let severity: String = row.get(5);
    let occurred: DateTime<Utc> = row.get(6);
    let payload: Value = row.get(7);
    let coord = match (lat, lon) {
        (Some(a), Some(o)) => format!("{a:.4},{o:.4}"),
        _ => "n/a".to_string(),
    };
    let content = format!(
        "Radar event ({kind}) — {title}\nsource: {source}\nseverity: {severity}\noccurred_at: {}\ncoordinates: {coord}\npayload: {payload}",
        occurred.to_rfc3339(),
    );
    let hash = {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(content.as_bytes()))
    };
    let url = format!("radar://event/{event_id}");
    let doc_id: Uuid = sqlx::query_scalar(
        "INSERT INTO documents (document_id, source_id, url_canonical, url_original, title, content_hash, retrieved_at, published_at, lang, content_text, embedding_status) \
         VALUES ($1, NULL, $2, $2, $3, $4, now(), $5, 'und', $6, 'SKIPPED') \
         ON CONFLICT (content_hash) DO UPDATE SET retrieved_at = EXCLUDED.retrieved_at \
         RETURNING document_id",
    )
    .bind(Uuid::new_v4())
    .bind(&url)
    .bind(&title)
    .bind(&hash)
    .bind(occurred)
    .bind(&content)
    .fetch_one(pg)
    .await?;
    create_finding(
        pg,
        investigation_id,
        &format!("Radar signal: {title}"),
        &format!(
            "Geo event {event_id} ({kind}, severity {severity}) observed by {source} at {} ({coord}) — converted to an investigation from the Radar console.",
            occurred.to_rfc3339(),
        ),
        &[(doc_id, "supports".to_string())],
        "system:radar",
        None,
        None,
    )
    .await?;
    Ok(true)
}

pub async fn update_investigation(
    pg: &PgPool,
    id: Uuid,
    title: Option<&str>,
    question: Option<&str>,
    target: Option<&str>,
    hypothesis: Option<&str>,
    status: Option<&str>,
) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE investigations SET \
           title = COALESCE($2, title), \
           question = COALESCE($3, question), \
           target = COALESCE($4, target), \
           hypothesis = COALESCE($5, hypothesis), \
           status = COALESCE($6, status), \
           updated_at = now() \
         WHERE investigation_id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(question)
    .bind(target)
    .bind(hypothesis)
    .bind(status)
    .execute(pg)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn get_investigation(pg: &PgPool, id: Uuid) -> Result<Value> {
    let row = sqlx::query(
        "SELECT investigation_id, title, question, target, hypothesis, status, created_by, created_at, updated_at \
         FROM investigations WHERE investigation_id = $1",
    )
    .bind(id)
    .fetch_optional(pg)
    .await?
    .ok_or_else(|| HubError::NotFound(format!("investigation {id}")))?;
    use sqlx::Row;
    Ok(json!({
        "investigation_id": row.get::<Uuid, _>(0),
        "title": row.get::<String, _>(1),
        "question": row.get::<Option<String>, _>(2),
        "target": row.get::<Option<String>, _>(3),
        "hypothesis": row.get::<Option<String>, _>(4),
        "status": row.get::<String, _>(5),
        "created_by": row.get::<String, _>(6),
        "created_at": row.get::<DateTime<Utc>, _>(7),
        "updated_at": row.get::<DateTime<Utc>, _>(8),
    }))
}

pub async fn list_investigations(pg: &PgPool, status: Option<&str>, limit: i64) -> Result<Value> {
    let rows = sqlx::query(
        "SELECT investigation_id, title, status, created_by, created_at, updated_at \
         FROM investigations \
         WHERE ($1::text IS NULL OR status = $1) \
         ORDER BY updated_at DESC LIMIT $2",
    )
    .bind(status)
    .bind(limit)
    .fetch_all(pg)
    .await?;
    use sqlx::Row;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "investigation_id": r.get::<Uuid, _>(0),
                "title": r.get::<String, _>(1),
                "status": r.get::<String, _>(2),
                "created_by": r.get::<String, _>(3),
                "created_at": r.get::<DateTime<Utc>, _>(4),
                "updated_at": r.get::<DateTime<Utc>, _>(5),
            })
        })
        .collect();
    Ok(json!({ "count": items.len(), "items": items }))
}

// ---------- tasks ----------

pub async fn create_task(
    pg: &PgPool,
    investigation_id: Option<Uuid>,
    kind: &str,
    created_by: &str,
    detail: Value,
) -> Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tasks (task_id, investigation_id, kind, status, created_by, detail) \
         VALUES ($1, $2, $3, 'running', $4, $5)",
    )
    .bind(id)
    .bind(investigation_id)
    .bind(kind)
    .bind(created_by)
    .bind(detail)
    .execute(pg)
    .await?;
    Ok(id)
}

pub async fn finish_task(pg: &PgPool, id: Uuid, status: &str, detail: Value) -> Result<()> {
    sqlx::query("UPDATE tasks SET status = $2, detail = detail || $3, updated_at = now() WHERE task_id = $1")
        .bind(id)
        .bind(status)
        .bind(detail)
        .execute(pg)
        .await?;
    Ok(())
}

pub async fn get_task(pg: &PgPool, id: Uuid) -> Result<Value> {
    let row = sqlx::query(
        "SELECT task_id, investigation_id, kind, status, created_by, created_at, updated_at, detail \
         FROM tasks WHERE task_id = $1",
    )
    .bind(id)
    .fetch_optional(pg)
    .await?
    .ok_or_else(|| HubError::NotFound(format!("task {id}")))?;
    use sqlx::Row;
    Ok(json!({
        "task_id": row.get::<Uuid, _>(0),
        "investigation_id": row.get::<Option<Uuid>, _>(1),
        "kind": row.get::<String, _>(2),
        "status": row.get::<String, _>(3),
        "created_by": row.get::<String, _>(4),
        "created_at": row.get::<DateTime<Utc>, _>(5),
        "updated_at": row.get::<DateTime<Utc>, _>(6),
        "detail": row.get::<Value, _>(7),
    }))
}

// ---------- sources / documents ----------

pub async fn get_or_create_source(pg: &PgPool, origin: &str, base_url: Option<&str>) -> Result<Uuid> {
    if let Some(row) = sqlx::query("SELECT source_id FROM sources WHERE origin = $1 AND base_url IS NOT DISTINCT FROM $2")
        .bind(origin)
        .bind(base_url)
        .fetch_optional(pg)
        .await?
    {
        use sqlx::Row;
        return Ok(row.get(0));
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO sources (source_id, origin, base_url) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(origin)
        .bind(base_url)
        .execute(pg)
        .await?;
    Ok(id)
}

pub struct InsertedDocument {
    pub document_id: Uuid,
    pub content_hash: String,
    pub duplicate: bool,
}

/// Persist a normalized EvidenceEvent. Exact dedupe via content_hash UNIQUE:
/// on conflict we return the existing document marked duplicate.
#[allow(clippy::too_many_arguments)]
pub async fn insert_document(
    pg: &PgPool,
    ev: &EvidenceEvent,
    source_id: Uuid,
    url_canonical: &str,
    title: Option<&str>,
    simhash: i64,
    raw_path: &str,
    parent_task: Option<Uuid>,
) -> Result<InsertedDocument> {
    let id = Uuid::new_v4();
    let res = sqlx::query(
        "INSERT INTO documents (document_id, source_id, url_canonical, url_original, title, \
         content_hash, simhash, retrieved_at, published_at, raw_path, content_text, parent_task, metadata, provenance) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) \
         ON CONFLICT (content_hash) DO NOTHING",
    )
    .bind(id)
    .bind(source_id)
    .bind(url_canonical)
    .bind(&ev.url)
    .bind(title)
    .bind(&ev.content_hash)
    .bind(simhash)
    .bind(ev.retrieved_at)
    .bind(ev.published_at)
    .bind(raw_path)
    .bind(&ev.content)
    .bind(parent_task)
    .bind(&ev.metadata)
    .bind(&ev.provenance)
    .execute(pg)
    .await?;

    if res.rows_affected() == 1 {
        return Ok(InsertedDocument {
            document_id: id,
            content_hash: ev.content_hash.clone(),
            duplicate: false,
        });
    }
    // exact duplicate: fetch existing
    let row = sqlx::query("SELECT document_id FROM documents WHERE content_hash = $1")
        .bind(&ev.content_hash)
        .fetch_one(pg)
        .await?;
    use sqlx::Row;
    Ok(InsertedDocument {
        document_id: row.get(0),
        content_hash: ev.content_hash.clone(),
        duplicate: true,
    })
}

pub async fn get_document(pg: &PgPool, id: Uuid) -> Result<Value> {
    let row = sqlx::query(
        "SELECT document_id, url_canonical, title, content_hash, retrieved_at, published_at, \
                raw_path, metadata, provenance, embedding_status, left(content_text, 200000) \
         FROM documents WHERE document_id = $1",
    )
    .bind(id)
    .fetch_optional(pg)
    .await?
    .ok_or_else(|| HubError::NotFound(format!("document {id}")))?;
    use sqlx::Row;
    Ok(json!({
        "document_id": row.get::<Uuid, _>(0),
        "url": row.get::<String, _>(1),
        "title": row.get::<Option<String>, _>(2),
        "content_hash": row.get::<String, _>(3),
        "retrieved_at": row.get::<DateTime<Utc>, _>(4),
        "published_at": row.get::<Option<DateTime<Utc>>, _>(5),
        "raw_path": row.get::<Option<String>, _>(6),
        "metadata": row.get::<Value, _>(7),
        "provenance": row.get::<Value, _>(8),
        "embedding_status": row.get::<String, _>(9),
        // dual-key: MCP agents consume "content"; the console DocumentDetail
        // interface expects "content_text" (mismatch left the page crashing
        // on undefined.slice since SP3)
        "content": row.get::<Option<String>, _>(10),
        "content_text": row.get::<Option<String>, _>(10),
    }))
}

pub async fn keyword_search(pg: &PgPool, query: &str, limit: i64) -> Result<Value> {
    let rows = sqlx::query(
        "SELECT document_id, url_canonical, title, retrieved_at, \
                ts_rank(content_tsv, plainto_tsquery('simple', $1)) AS rank, \
                left(content_text, 500) AS snippet \
         FROM documents \
         WHERE content_tsv @@ plainto_tsquery('simple', $1) \
         ORDER BY rank DESC LIMIT $2",
    )
    .bind(query)
    .bind(limit)
    .fetch_all(pg)
    .await?;
    use sqlx::Row;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "document_id": r.get::<Uuid, _>(0),
                "url": r.get::<String, _>(1),
                "title": r.get::<Option<String>, _>(2),
                "retrieved_at": r.get::<DateTime<Utc>, _>(3),
                "rank": r.get::<f32, _>(4),
                "snippet": r.get::<Option<String>, _>(5),
            })
        })
        .collect();
    Ok(json!({ "mode": "keyword", "count": items.len(), "items": items }))
}

/// Recent simhashes for near-dup detection (bounded scan window).
pub async fn recent_simhashes(pg: &PgPool, window: i64) -> Result<Vec<(Uuid, i64)>> {
    let rows = sqlx::query(
        "SELECT document_id, simhash FROM documents WHERE simhash IS NOT NULL \
         ORDER BY created_at DESC LIMIT $1",
    )
    .bind(window)
    .fetch_all(pg)
    .await?;
    use sqlx::Row;
    Ok(rows
        .iter()
        .map(|r| (r.get::<Uuid, _>(0), r.get::<i64, _>(1)))
        .collect())
}

pub async fn enqueue_embedding_job(pg: &PgPool, document_id: Uuid, model: &str, force: bool, trace_id: Option<Uuid>) -> Result<()> {
    sqlx::query(
        "INSERT INTO embedding_jobs (job_id, document_id, status, model, force, trace_id)
         VALUES ($1, $2, 'PENDING', $3, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(document_id)
    .bind(model)
    .bind(force)
    .bind(trace_id)
    .execute(pg)
    .await?;
    Ok(())
}

// ---------- findings ----------

pub async fn create_finding(
    pg: &PgPool,
    investigation_id: Uuid,
    title: &str,
    claim_text: &str,
    evidence: &[(Uuid, String)], // (document_id, relation)
    created_by: &str,
    agent_id: Option<Uuid>,
    task_id: Option<Uuid>,
) -> Result<Uuid> {
    if evidence.is_empty() {
        return Err(HubError::bad_request(
            "create_finding requires at least one evidence document (directive §29)",
        ));
    }
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO findings (finding_id, investigation_id, title, claim_text, created_by, agent_id, task_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(id)
    .bind(investigation_id)
    .bind(title)
    .bind(claim_text)
    .bind(created_by)
    .bind(agent_id)
    .bind(task_id)
    .execute(pg)
    .await?;
    for (doc_id, relation) in evidence {
        let rel = if relation == "contradicts" { "contradicts" } else { "supports" };
        sqlx::query(
            "INSERT INTO finding_evidence (finding_id, document_id, relation) VALUES ($1,$2,$3)",
        )
        .bind(id)
        .bind(doc_id)
        .bind(rel)
        .execute(pg)
        .await?;
    }
    Ok(id)
}

pub async fn list_findings(pg: &PgPool, investigation_id: Uuid) -> Result<Value> {
    let rows = sqlx::query(
        "SELECT f.finding_id, f.title, f.claim_text, f.created_by, f.created_at, \
                COUNT(*) FILTER (WHERE fe.relation = 'supports') AS supporting, \
                COUNT(*) FILTER (WHERE fe.relation = 'contradicts') AS contradicting \
         FROM findings f LEFT JOIN finding_evidence fe ON fe.finding_id = f.finding_id \
         WHERE f.investigation_id = $1 \
         GROUP BY f.finding_id ORDER BY f.created_at DESC",
    )
    .bind(investigation_id)
    .fetch_all(pg)
    .await?;
    use sqlx::Row;
    // The console workspace renders each finding's evidence chain inline, so
    // the list must carry the evidence rows themselves — counts alone left
    // every card showing "0 linked evidence" even when §29 chains existed.
    let ids: Vec<Uuid> = rows.iter().map(|r| r.get::<Uuid, _>(0)).collect();
    let mut ev_map: std::collections::HashMap<Uuid, Vec<Value>> = std::collections::HashMap::new();
    if !ids.is_empty() {
        let ev_rows = sqlx::query(
            "SELECT fe.finding_id, fe.document_id, fe.relation, d.title, d.url_canonical \
             FROM finding_evidence fe JOIN documents d ON d.document_id = fe.document_id \
             WHERE fe.finding_id = ANY($1) \
             ORDER BY fe.relation DESC",
        )
        .bind(&ids)
        .fetch_all(pg)
        .await?;
        for er in &ev_rows {
            ev_map.entry(er.get::<Uuid, _>(0)).or_default().push(json!({
                "document_id": er.get::<Uuid, _>(1),
                "relation": er.get::<String, _>(2),
                "title": er.get::<Option<String>, _>(3),
                "url": er.get::<String, _>(4),
            }));
        }
    }
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            let fid = r.get::<Uuid, _>(0);
            json!({
                "finding_id": fid,
                "title": r.get::<String, _>(1),
                "claim_text": r.get::<String, _>(2),
                "created_by": r.get::<String, _>(3),
                "created_at": r.get::<DateTime<Utc>, _>(4),
                "supporting_evidence": r.get::<i64, _>(5),
                "contradicting_evidence": r.get::<i64, _>(6),
                "evidence": ev_map.get(&fid).cloned().unwrap_or_default(),
            })
        })
        .collect();
    Ok(json!({ "count": items.len(), "items": items }))
}

// ---------- tool_calls / audit / events ----------

#[allow(clippy::too_many_arguments)]
pub async fn record_tool_call(
    pg: &PgPool,
    agent_id: Option<Uuid>,
    session_id: Option<&str>,
    request_id: &str,
    trace_id: &str,
    tool: &str,
    args_digest: Option<&str>,
    status: &str,
    latency_ms: i32,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO tool_calls (tool_call_id, agent_id, session_id, request_id, trace_id, tool, args_digest, status, latency_ms) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(Uuid::new_v4())
    .bind(agent_id)
    .bind(session_id)
    .bind(request_id)
    .bind(trace_id)
    .bind(tool)
    .bind(args_digest)
    .bind(status)
    .bind(latency_ms)
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn record_audit(
    pg: &PgPool,
    actor: &str,
    action: &str,
    object_type: Option<&str>,
    object_id: Option<&str>,
    result: &str,
    detail: Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_records (audit_id, actor, action, object_type, object_id, result, detail) \
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(Uuid::new_v4())
    .bind(actor)
    .bind(action)
    .bind(object_type)
    .bind(object_id)
    .bind(result)
    .bind(detail)
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn persist_event(pg: &PgPool, ev: &crate::types::BusEvent) -> Result<()> {
    sqlx::query(
        "INSERT INTO events (event_id, event_type, ts, actor, investigation_id, trace_id, payload) \
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(ev.event_id)
    .bind(&ev.event_type)
    .bind(ev.ts)
    .bind(&ev.actor)
    .bind(ev.investigation_id)
    .bind(&ev.trace_id)
    .bind(&ev.payload)
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn recent_events(pg: &PgPool, limit: i64) -> Result<Value> {
    let rows = sqlx::query(
        "SELECT event_id, event_type, ts, actor, investigation_id, trace_id, payload \
         FROM events ORDER BY ts DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pg)
    .await?;
    use sqlx::Row;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "event_id": r.get::<Uuid, _>(0),
                "event_type": r.get::<String, _>(1),
                "ts": r.get::<DateTime<Utc>, _>(2),
                "actor": r.get::<String, _>(3),
                "investigation_id": r.get::<Option<Uuid>, _>(4),
                "trace_id": r.get::<Option<String>, _>(5),
                "payload": r.get::<Value, _>(6),
            })
        })
        .collect();
    Ok(json!({ "count": items.len(), "items": items }))
}

// ---------- entities (SP9 KG memory) ----------
//
// Helpers consumed by `crate::graph_v2::resolve` (Task 2) and later Tasks 3-5.
// Migrated columns (valid_from/valid_until/merged_into/resolution_method/
// resolution_confidence) live on `entities`; the new `entity_aliases` table
// is the join surface for resolution.
//
// All queries are runtime-checked (sqlx::query / sqlx::query_as) so Docker
// builds do not require a live PG.

pub mod entities {
    use chrono::{DateTime, Utc};
    use sqlx::PgPool;
    use uuid::Uuid;

    use crate::error::HubError;

    /// Name normalization: trim + collapse internal whitespace + drop control
    /// chars + cap 256. Mirrors `graphw::normalize_name` but does not reject
    /// control chars (resolver silently strips; the write-plane rejects).
    pub fn normalize_name(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut last_space = true;
        for ch in s.chars() {
            if ch.is_control() {
                continue;
            }
            if ch.is_whitespace() {
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            } else {
                out.push(ch);
                last_space = false;
            }
        }
        let trimmed = out.trim();
        if trimmed.chars().count() > 256 {
            trimmed.chars().take(256).collect()
        } else {
            trimmed.to_string()
        }
    }

    /// Exact (kind, normalized-name) lookup. Caller normalizes.
    pub async fn find_by_kind_name(
        pool: &PgPool,
        kind: &str,
        name_norm: &str,
    ) -> Result<Option<Uuid>, HubError> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT entity_id FROM entities WHERE kind = $1 AND lower(name) = lower($2) LIMIT 1",
        )
        .bind(kind)
        .bind(name_norm)
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// Exact alias lookup. The aliases table has UNIQUE(kind, alias_norm).
    pub async fn lookup_alias_exact(
        pool: &PgPool,
        kind: &str,
        alias_norm: &str,
    ) -> Result<Option<Uuid>, HubError> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT entity_id FROM entity_aliases WHERE kind = $1 AND alias_norm = $2 LIMIT 1",
        )
        .bind(kind)
        .bind(alias_norm)
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// Single row from `find_candidates_by_kind`.
    pub struct Candidate {
        pub entity_id: Uuid,
        pub name_norm: String,
    }

    /// Pull all live (non-merged) candidates for a kind, capped at 5000.
    /// The jaro_winkler scan is O(n) over the result.
    pub async fn find_candidates_by_kind(
        pool: &PgPool,
        kind: &str,
    ) -> Result<Vec<Candidate>, HubError> {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT entity_id, lower(name) FROM entities WHERE kind = $1 AND merged_into IS NULL LIMIT 5000",
        )
        .bind(kind)
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(entity_id, name_norm)| Candidate { entity_id, name_norm })
            .collect())
    }

    /// Record an alias for an existing entity. ON CONFLICT DO NOTHING so a
    /// concurrent resolver can't double-insert.
    pub async fn write_alias(
        pool: &PgPool,
        entity_id: Uuid,
        alias: &str,
        alias_norm: &str,
        kind: &str,
        source: &str,
        confidence: f64,
    ) -> Result<(), HubError> {
        sqlx::query(
            "INSERT INTO entity_aliases (entity_id, alias, alias_norm, kind, source, confidence) \
             VALUES ($1,$2,$3,$4,$5,$6) \
             ON CONFLICT (kind, alias_norm) DO NOTHING",
        )
        .bind(entity_id)
        .bind(alias)
        .bind(alias_norm)
        .bind(kind)
        .bind(source)
        .bind(confidence)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Insert a new entity + its initial self-alias inside one transaction.
    /// ON CONFLICT (kind, name) DO UPDATE returns the existing id so the
    /// function is idempotent for retries.
    pub async fn create_with_alias(
        pool: &PgPool,
        kind: &str,
        name: &str,
        name_norm: &str,
        source: &str,
        confidence: f64,
    ) -> Result<Uuid, HubError> {
        let mut tx = pool.begin().await?;
        let row: (Uuid,) = sqlx::query_as(
            "INSERT INTO entities (kind, name) VALUES ($1,$2) \
             ON CONFLICT (kind, name) DO UPDATE SET kind = EXCLUDED.kind \
             RETURNING entity_id",
        )
        .bind(kind)
        .bind(name)
        .fetch_one(&mut *tx)
        .await?;
        let eid = row.0;
        sqlx::query(
            "INSERT INTO entity_aliases (entity_id, alias, alias_norm, kind, source, confidence) \
             VALUES ($1,$2,$3,$4,$5,$6) \
             ON CONFLICT (kind, alias_norm) DO NOTHING",
        )
        .bind(eid)
        .bind(name)
        .bind(name_norm)
        .bind(kind)
        .bind(source)
        .bind(confidence)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(eid)
    }

    #[allow(dead_code)]
    pub struct ResolutionHit {
        pub entity_id: Uuid,
        pub score: f64,
        pub matched_at: DateTime<Utc>,
    }
}
