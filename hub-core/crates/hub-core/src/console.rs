//! Console API (SP3) — read-only aggregation endpoints for the Unified
//! Console (directive §24: the frontend talks ONLY to the Hub API). Every
//! handler is L1 (policy registry default); all queries are runtime sqlx.

use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::Result;
use crate::state::AppState;

// ---------- §25 overview aggregate ----------

pub async fn overview(state: &AppState) -> Result<Value> {
    let health = crate::api::system_health(state).await;

    let (active_inv,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM investigations WHERE status = 'open'")
            .fetch_one(&state.pg)
            .await?;
    let inv_items = crate::store::list_investigations(&state.pg, Some("open"), 5).await?;

    let (open_alerts,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM alerts WHERE status = 'open'")
            .fetch_one(&state.pg)
            .await?;
    let recent_alerts = crate::alerts::list(state, Some("open"), None, None, 5).await?;

    let (docs_today,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM documents WHERE retrieved_at >= date_trunc('day', now() AT TIME ZONE 'UTC')",
    )
    .fetch_one(&state.pg)
    .await?;
    let (docs_total,): (i64,) = sqlx::query_as("SELECT count(*) FROM documents")
        .fetch_one(&state.pg)
        .await?;

    let (entities_total,): (i64,) = sqlx::query_as("SELECT count(*) FROM entities")
        .fetch_one(&state.pg)
        .await?;
    let (rels_total,): (i64,) = sqlx::query_as("SELECT count(*) FROM relationships")
        .fetch_one(&state.pg)
        .await?;
    let (graph_today,): (i64,) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM entities WHERE created_at >= date_trunc('day', now() AT TIME ZONE 'UTC'))
              + (SELECT count(*) FROM relationships WHERE created_at >= date_trunc('day', now() AT TIME ZONE 'UTC'))",
    )
    .fetch_one(&state.pg)
    .await?;

    let (embedded_docs,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM documents WHERE embedding_status = 'DONE'")
            .fetch_one(&state.pg)
            .await?;
    let (chunks,): (i64,) = sqlx::query_as("SELECT count(*) FROM embedding_chunks")
        .fetch_one(&state.pg)
        .await?;

    let (embed_tokens,): (f64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount),0) FROM cost_records
         WHERE kind='embedding_tokens' AND created_at >= date_trunc('day', now() AT TIME ZONE 'UTC')",
    )
    .fetch_one(&state.pg)
    .await?;

    // Per-agent activity + budget (§55, §60).
    let agents: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT agent_id, name FROM agents ORDER BY name")
            .fetch_all(&state.pg)
            .await?;
    let mut agent_items = Vec::new();
    for (agent_id, name) in agents {
        let (calls,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM tool_calls
             WHERE agent_id = $1 AND created_at >= date_trunc('day', now() AT TIME ZONE 'UTC')",
        )
        .bind(agent_id)
        .fetch_one(&state.pg)
        .await?;
        let identity = crate::types::AgentIdentity {
            agent_id,
            name: name.clone(),
            key_id: Uuid::nil(),
            admin: false,
        };
        let bs = crate::cost::budget_state(state, &identity).await?;
        agent_items.push(json!({
            "name": name, "calls_today": calls,
            "budget_state": bs.state.as_str(), "budget_ratio": bs.ratio,
        }));
    }

    // SP6 §26: native monitor signal layer status + fresh geo event count.
    let (geo_24h,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM geo_events WHERE ingested_at > now() - interval '24 hours'",
    )
    .fetch_one(&state.pg)
    .await?;
    // Per-source health cells written by the monitor scheduler (Redis hash).
    let health_map: Option<std::collections::HashMap<String, String>> = state
        .redis_timed(redis::cmd("HGETALL").arg("hub:monitor:health").clone(), 2000)
        .await;
    let monitor_meta: Value = match health_map {
        Some(map) if !map.is_empty() => {
            let mut sources: Vec<Value> = Vec::new();
            for (name, cell) in &map {
                let c: Value = serde_json::from_str(cell).unwrap_or_else(|_| json!({}));
                sources.push(json!({
                    "name": name,
                    "state": c.get("state").cloned().unwrap_or(json!("unknown")),
                    "ts": c.get("ts").cloned().unwrap_or(Value::Null),
                    "detail": c.get("detail").cloned().unwrap_or(Value::Null),
                    "last_new": c.get("last_new").cloned().unwrap_or(json!(0)),
                }));
            }
            sources.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            let ok = sources.iter().filter(|s| s["state"] == "ok").count();
            json!({
                "up": true,
                "sources_ok": ok,
                "sources_total": sources.len(),
                "sources": sources,
            })
        }
        _ => json!({ "up": false }),
    };

    Ok(json!({
        "ts": chrono::Utc::now(),
        "health": health,
        "investigations": { "active": active_inv, "items": inv_items },
        "alerts": { "open": open_alerts, "recent": recent_alerts },
        "agents": agent_items,
        "evidence": { "docs_today": docs_today, "docs_total": docs_total },
        "graph": { "entities": entities_total, "relationships": rels_total, "changes_today": graph_today },
        "memory": { "embedded_docs": embedded_docs, "chunks": chunks },
        "cloud": {
            "embedding_tokens_today": embed_tokens,
            "est_cost_usd": (embed_tokens * 0.02 / 1_000_000.0 * 10000.0).round() / 10000.0,
        },
        "radar": { "geo_events_24h": geo_24h, "monitor": monitor_meta },
    }))
}

// ---------- §67 unified search ----------

pub async fn unified_search(state: &AppState, q: &str, limit: i64) -> Result<Value> {
    let like = format!("%{}%", q.replace(['%', '_'], ""));
    let lim = limit.clamp(1, 25);

    // Exact / partial entity matches (PG canonical, §36).
    let entities: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String, String)>(
        "SELECT entity_id, kind, name, created_by FROM entities
         WHERE name ILIKE $1 ORDER BY (lower(name) = lower($2)) DESC, created_at DESC LIMIT $3",
    )
    .bind(&like)
    .bind(q)
    .bind(lim)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(id, kind, name, by)| json!({ "entity_id": id, "kind": kind, "name": name, "created_by": by }))
    .collect();

    // Documents: keyword channel + vector channel (best effort).
    let keyword = crate::store::keyword_search(&state.pg, q, lim).await?;
    let mut similar: Vec<Value> = Vec::new();
    if let Ok((qvec, _)) = crate::embed::query_embedding(state, q).await {
        if let Ok(hits) = crate::vector::search(state, &qvec, lim as u64).await {
            for (doc_id, score) in hits {
                if let Some((url, title)) = sqlx::query_as::<_, (String, Option<String>)>(
                    "SELECT url_canonical, title FROM documents WHERE document_id = $1",
                )
                .bind(doc_id)
                .fetch_optional(&state.pg)
                .await?
                {
                    similar.push(json!({ "document_id": doc_id, "score": score, "url": url, "title": title }));
                }
            }
        }
    }

    // Relationships touching matching entities.
    let relationships: Vec<Value> = sqlx::query_as::<_, (String, String, String, String, String)>(
        "SELECT f.kind, f.name, r.rel_type, t.kind, t.name FROM relationships r
         JOIN entities f ON f.entity_id = r.from_entity
         JOIN entities t ON t.entity_id = r.to_entity
         WHERE f.name ILIKE $1 OR t.name ILIKE $1 LIMIT $2",
    )
    .bind(&like)
    .bind(lim)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(fk, fn_, rel, tk, tn)| {
        json!({ "from": format!("{fk}:{fn_}"), "rel_type": rel, "to": format!("{tk}:{tn}") })
    })
    .collect();

    let findings: Vec<Value> = sqlx::query_as::<_, (Uuid, Uuid, String, String, String)>(
        "SELECT finding_id, investigation_id, title, claim_text, created_by FROM findings
         WHERE title ILIKE $1 OR claim_text ILIKE $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(&like)
    .bind(lim)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(id, inv, title, claim, by)| {
        json!({ "finding_id": id, "investigation_id": inv, "title": title,
                "claim_text": claim, "created_by": by })
    })
    .collect();

    let alerts: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String, String, String)>(
        "SELECT alert_id, severity, source, title, status FROM alerts
         WHERE title ILIKE $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(&like)
    .bind(lim)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(id, sev, src, title, st)| {
        json!({ "alert_id": id, "severity": sev, "source": src, "title": title, "status": st })
    })
    .collect();

    let investigations: Vec<Value> = sqlx::query_as::<_, (Uuid, String, Option<String>, String)>(
        "SELECT investigation_id, title, target, status FROM investigations
         WHERE title ILIKE $1 OR target ILIKE $1 OR question ILIKE $1
         ORDER BY created_at DESC LIMIT $2",
    )
    .bind(&like)
    .bind(lim)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(id, title, target, st)| {
        json!({ "investigation_id": id, "title": title, "target": target, "status": st })
    })
    .collect();

    Ok(json!({
        "query": q,
        "entities": entities,
        "documents": keyword,
        "similar_documents": similar,
        "relationships": relationships,
        "findings": findings,
        "alerts": alerts,
        "investigations": investigations,
    }))
}

// ---------- documents browser ----------

pub async fn list_documents(
    state: &AppState,
    q: Option<&str>,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Value> {
    let like = q.map(|s| format!("%{}%", s.replace(['%', '_'], "")));
    let items: Vec<Value> = sqlx::query_as::<_, (Uuid, String, Option<String>, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT document_id, url_canonical, title, content_hash, embedding_status, retrieved_at
         FROM documents
         WHERE ($1::text IS NULL OR url_canonical ILIKE $1 OR title ILIKE $1)
           AND ($2::text IS NULL OR embedding_status = $2)
         ORDER BY retrieved_at DESC LIMIT $3 OFFSET $4",
    )
    .bind(&like)
    .bind(status)
    .bind(limit.clamp(1, 100))
    .bind(offset.max(0))
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(id, url, title, hash, emb, ts)| json!({
        "document_id": id, "url": url, "title": title,
        "content_hash": &hash[..16.min(hash.len())],
        "embedding_status": emb, "retrieved_at": ts,
    }))
    .collect();
    let (total,): (i64,) = sqlx::query_as("SELECT count(*) FROM documents")
        .fetch_one(&state.pg)
        .await?;
    Ok(json!({ "total": total, "count": items.len(), "items": items }))
}

// ---------- entities browse ----------

pub async fn list_entities(state: &AppState, q: Option<&str>, kind: Option<&str>, limit: i64) -> Result<Value> {
    let like = q.map(|s| format!("%{}%", s.replace(['%', '_'], "")));
    let items: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT entity_id, kind, name, created_by, created_at FROM entities
         WHERE ($1::text IS NULL OR name ILIKE $1)
           AND ($2::text IS NULL OR kind = $2)
         ORDER BY created_at DESC LIMIT $3",
    )
    .bind(&like)
    .bind(kind)
    .bind(limit.clamp(1, 200))
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(id, kind, name, by, ts)| json!({
        "entity_id": id, "kind": kind, "name": name, "created_by": by, "created_at": ts,
    }))
    .collect();
    Ok(json!({ "count": items.len(), "items": items }))
}

pub async fn get_entity(state: &AppState, id: Uuid) -> Result<Value> {
    let ent: Option<(Uuid, String, String, Value, Value, String, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as(
            "SELECT entity_id, kind, name, aliases, attributes, created_by, created_at
             FROM entities WHERE entity_id = $1",
        )
        .bind(id)
        .fetch_optional(&state.pg)
        .await?;
    let Some((eid, kind, name, aliases, attrs, by, ts)) = ent else {
        return Err(crate::error::HubError::NotFound(format!("entity {id}")));
    };
    let relationships: Vec<Value> = sqlx::query_as::<_, (String, String, String, String, String)>(
        "SELECT f.kind, f.name, r.rel_type, t.kind, t.name FROM relationships r
         JOIN entities f ON f.entity_id = r.from_entity
         JOIN entities t ON t.entity_id = r.to_entity
         WHERE r.from_entity = $1 OR r.to_entity = $1 LIMIT 100",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(fk, fn_, rel, tk, tn)| json!({
        "from": format!("{fk}:{fn_}"), "rel_type": rel, "to": format!("{tk}:{tn}")
    }))
    .collect();
    let claims: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String, String)>(
        "SELECT c.claim_id, c.text, c.status, ce.role FROM claims c
         JOIN claim_entities ce ON ce.claim_id = c.claim_id
         WHERE ce.entity_id = $1 ORDER BY c.created_at DESC LIMIT 50",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(cid, text, st, role)| json!({ "claim_id": cid, "text": text, "status": st, "role": role }))
    .collect();
    Ok(json!({
        "entity_id": eid, "kind": kind, "name": name, "aliases": aliases,
        "attributes": attrs, "created_by": by, "created_at": ts,
        "relationships": relationships, "claims": claims,
    }))
}

// ---------- §55 agent activity ----------

pub async fn agents_activity(state: &AppState) -> Result<Value> {
    let agents: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT agent_id, name FROM agents ORDER BY name")
            .fetch_all(&state.pg)
            .await?;
    let mut out = Vec::new();
    for (agent_id, name) in agents {
        let (calls_today,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM tool_calls
             WHERE agent_id = $1 AND created_at >= date_trunc('day', now() AT TIME ZONE 'UTC')",
        )
        .bind(agent_id)
        .fetch_one(&state.pg)
        .await?;
        let recent: Vec<Value> = sqlx::query_as::<_, (String, String, i32, chrono::DateTime<chrono::Utc>)>(
            "SELECT tool, status, latency_ms, created_at FROM tool_calls
             WHERE agent_id = $1 ORDER BY created_at DESC LIMIT 15",
        )
        .bind(agent_id)
        .fetch_all(&state.pg)
        .await?
        .into_iter()
        .map(|(tool, st, lat, ts)| json!({ "tool": tool, "status": st, "latency_ms": lat, "ts": ts }))
        .collect();
        let costs: Vec<Value> = sqlx::query_as::<_, (String, f64)>(
            "SELECT kind, COALESCE(SUM(amount),0) FROM cost_records
             WHERE agent_id = $1 AND created_at >= date_trunc('day', now() AT TIME ZONE 'UTC')
             GROUP BY kind",
        )
        .bind(agent_id)
        .fetch_all(&state.pg)
        .await?
        .into_iter()
        .map(|(k, a)| json!({ "kind": k, "amount": a }))
        .collect();
        let (findings_n,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM findings WHERE agent_id = $1")
                .bind(agent_id)
                .fetch_one(&state.pg)
                .await?;
        let identity = crate::types::AgentIdentity {
            agent_id,
            name: name.clone(),
            key_id: Uuid::nil(),
            admin: false,
        };
        let bs = crate::cost::budget_state(state, &identity).await?;
        out.push(json!({
            "name": name, "calls_today": calls_today, "recent_calls": recent,
            "costs_today": costs, "findings_total": findings_n,
            "budget": { "state": bs.state.as_str(), "ratio": bs.ratio, "usage": bs.usage, "limits": bs.limits },
        }));
    }
    Ok(json!({ "agents": out }))
}

// ---------- §66 audit query ----------

pub async fn query_audit(
    state: &AppState,
    actor: Option<&str>,
    action: Option<&str>,
    result: Option<&str>,
    limit: i64,
) -> Result<Value> {
    let items: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String, Option<String>, Option<String>, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT audit_id, actor, action, object_type, object_id, result, created_at
         FROM audit_records
         WHERE ($1::text IS NULL OR actor = $1)
           AND ($2::text IS NULL OR action = $2)
           AND ($3::text IS NULL OR result = $3)
         ORDER BY created_at DESC LIMIT $4",
    )
    .bind(actor)
    .bind(action)
    .bind(result)
    .bind(limit.clamp(1, 200))
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(id, actor, action, ot, oid, res, ts)| json!({
        "audit_id": id, "actor": actor, "action": action,
        "object_type": ot, "object_id": oid, "result": res, "ts": ts,
    }))
    .collect();
    Ok(json!({ "count": items.len(), "items": items }))
}

// ---------- tasks list ----------

pub async fn list_tasks(state: &AppState, status: Option<&str>, limit: i64) -> Result<Value> {
    let items: Vec<Value> = sqlx::query_as::<_, (Uuid, Option<Uuid>, String, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT task_id, investigation_id, kind, status, created_by, created_at
         FROM tasks WHERE ($1::text IS NULL OR status = $1)
         ORDER BY created_at DESC LIMIT $2",
    )
    .bind(status)
    .bind(limit.clamp(1, 200))
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(id, inv, kind, st, by, ts)| json!({
        "task_id": id, "investigation_id": inv, "kind": kind,
        "status": st, "created_by": by, "created_at": ts,
    }))
    .collect();
    Ok(json!({ "count": items.len(), "items": items }))
}

// ---------- document detail with reverse references (§29 both directions) ----------

pub async fn document_detail(state: &AppState, id: Uuid) -> Result<Value> {
    let doc = crate::store::get_document(&state.pg, id).await?;
    let findings: Vec<Value> = sqlx::query_as::<_, (Uuid, Uuid, String, String, String)>(
        "SELECT f.finding_id, f.investigation_id, f.title, fe.relation, f.created_by
         FROM findings f JOIN finding_evidence fe ON fe.finding_id = f.finding_id
         WHERE fe.document_id = $1",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(fid, inv, title, rel, by)| json!({
        "finding_id": fid, "investigation_id": inv, "title": title, "relation": rel, "created_by": by
    }))
    .collect();
    let claims: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String, String)>(
        "SELECT c.claim_id, c.text, c.status, ce.relation FROM claims c
         JOIN claim_evidence ce ON ce.claim_id = c.claim_id WHERE ce.document_id = $1",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(cid, text, st, rel)| json!({ "claim_id": cid, "text": text, "status": st, "relation": rel }))
    .collect();
    let entities: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT DISTINCT e.entity_id, e.kind, e.name FROM entities e
         JOIN observations o ON o.entity_id = e.entity_id WHERE o.document_id = $1",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(eid, kind, name)| json!({ "entity_id": eid, "kind": kind, "name": name }))
    .collect();
    Ok(json!({
        "document": doc,
        "referenced_by_findings": findings,
        "referenced_by_claims": claims,
        "observed_entities": entities,
    }))
}

// ---------- §27 investigation workspace aggregate ----------

pub async fn investigation_workspace(state: &AppState, id: Uuid) -> Result<Value> {
    let investigation = crate::store::get_investigation(&state.pg, id).await?;
    let findings = crate::store::list_findings(&state.pg, id).await?;

    // Documents produced by this investigation's tasks.
    let documents: Vec<Value> = sqlx::query_as::<_, (Uuid, String, Option<String>, chrono::DateTime<chrono::Utc>)>(
        "SELECT DISTINCT d.document_id, d.url_canonical, d.title, d.retrieved_at FROM documents d
         WHERE d.parent_task IN (SELECT task_id FROM tasks WHERE investigation_id = $1)
            OR d.document_id IN (SELECT fe.document_id FROM finding_evidence fe
                                  JOIN findings f ON f.finding_id = fe.finding_id
                                  WHERE f.investigation_id = $1)
         ORDER BY d.retrieved_at DESC LIMIT 100",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(did, url, title, ts)| json!({ "document_id": did, "url": url, "title": title, "retrieved_at": ts }))
    .collect();

    // Entities observed on this investigation's documents.
    let entities: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT DISTINCT e.entity_id, e.kind, e.name FROM entities e
         JOIN observations o ON o.entity_id = e.entity_id
         JOIN documents d ON d.document_id = o.document_id
         JOIN tasks t ON t.task_id = d.parent_task
         WHERE t.investigation_id = $1 LIMIT 100",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(eid, kind, name)| json!({ "entity_id": eid, "kind": kind, "name": name }))
    .collect();

    let tasks: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT task_id, kind, status, created_by, created_at FROM tasks
         WHERE investigation_id = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(tid, kind, st, by, ts)| json!({
        "task_id": tid, "kind": kind, "status": st, "created_by": by, "created_at": ts
    }))
    .collect();

    let alerts: Vec<Value> = sqlx::query_as::<_, (Uuid, String, String, String, String)>(
        "SELECT alert_id, severity, source, title, status FROM alerts
         WHERE investigation_id = $1 ORDER BY created_at DESC LIMIT 50",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(aid, sev, src, title, st)| json!({
        "alert_id": aid, "severity": sev, "source": src, "title": title, "status": st
    }))
    .collect();

    let audit: Vec<Value> = sqlx::query_as::<_, (String, String, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT actor, action, result, created_at FROM audit_records
         WHERE object_id = $1 OR detail->>'investigation_id' = $1
         ORDER BY created_at DESC LIMIT 50",
    )
    .bind(id.to_string())
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|(actor, action, res, ts)| json!({ "actor": actor, "action": action, "result": res, "ts": ts }))
    .collect();

    Ok(json!({
        "investigation": investigation,
        "findings": findings,
        "documents": documents,
        "entities": entities,
        "tasks": tasks,
        "alerts": alerts,
        "audit": audit,
    }))
}

// ---------- SP4: radar + metrics (§26, §52) ----------

/// Radar events for the console map. All filters optional; ISO-8601 strings
/// for from/to. Bounded result set (limit capped at 2000 by the handler).
pub async fn radar_events(
    state: &AppState,
    from: Option<&str>,
    to: Option<&str>,
    severity: Option<&str>,
    source: Option<&str>,
    kind: Option<&str>,
    limit: i64,
) -> Result<Value> {
    let from_ts = from
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(7));
    let to_ts = to
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    let mut sql = String::from(
        "SELECT event_id, source, kind, title, lat, lon, severity, occurred_at, payload
         FROM geo_events
         WHERE lat IS NOT NULL AND occurred_at BETWEEN $1 AND $2",
    );
    let mut n = 2;
    let mut text_binds: Vec<(&str, String)> = Vec::new();
    for (clause, val) in [
        ("severity", severity),
        ("source", source),
        ("kind", kind),
    ] {
        if let Some(v) = val {
            n += 1;
            sql.push_str(&format!(" AND {clause} = ${n}"));
            text_binds.push((clause, v.to_string()));
        }
    }
    sql.push_str(" ORDER BY occurred_at DESC");
    n += 1;
    sql.push_str(&format!(" LIMIT ${n}"));

    let mut q = sqlx::query_as::<
        _,
        (Uuid, String, String, String, Option<f64>, Option<f64>, String, chrono::DateTime<chrono::Utc>, Value),
    >(&sql)
    .bind(from_ts)
    .bind(to_ts);
    for (_, v) in &text_binds {
        q = q.bind(v);
    }
    q = q.bind(limit);

    let rows = q.fetch_all(&state.pg).await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(event_id, source, kind, title, lat, lon, severity, occurred_at, payload)| {
            json!({
                "event_id": event_id, "source": source, "kind": kind, "title": title,
                "lat": lat, "lon": lon, "severity": severity,
                "occurred_at": occurred_at, "payload": payload,
            })
        })
        .collect();
    Ok(json!({ "items": items, "count": items.len(), "from": from_ts, "to": to_ts }))
}

/// §52 metrics summary aggregated from Prometheus instant queries.
/// Graceful degradation: `{"telemetry": "unavailable"}` when Prometheus is down.
pub async fn metrics_summary(state: &AppState) -> Result<Value> {
    let base = &state.config.prometheus_url;
    async fn prom(state: &AppState, base: &str, q: &str) -> Option<f64> {
        let url = format!("{base}/api/v1/query?query={q}");
        let r = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            state.http.get(&url).send(),
        )
        .await
        .ok()?
        .ok()?;
        let j: Value = r.json().await.ok()?;
        j.pointer("/data/result/0/value/1")?
            .as_str()?
            .parse::<f64>()
            .ok()
    }
    // All five instant queries concurrently — sequential 4s timeouts would
    // stack to 20s when Prometheus is down (§72: bounded degradation).
    let (cpu, ram, disk, net_rx, net_tx) = tokio::join!(
        prom(state, base, "100%20-%20(avg(rate(node_cpu_seconds_total%7Bmode%3D%22idle%22%7D%5B5m%5D))%20*%20100)"),
        prom(state, base, "(1%20-%20(node_memory_MemAvailable_bytes%20%2F%20node_memory_MemTotal_bytes))%20*%20100"),
        prom(state, base, "(1%20-%20(node_filesystem_avail_bytes%7Bmountpoint%3D%22%2F%22%2Cfstype!%3D%22tmpfs%22%7D%20%2F%20node_filesystem_size_bytes%7Bmountpoint%3D%22%2F%22%2Cfstype!%3D%22tmpfs%22%7D))%20*%20100"),
        prom(state, base, "sum(rate(node_network_receive_bytes_total%7Bdevice!~%22lo%7Cveth.*%7Cbr-.*%7Cdocker.*%22%7D%5B5m%5D))"),
        prom(state, base, "sum(rate(node_network_transmit_bytes_total%7Bdevice!~%22lo%7Cveth.*%7Cbr-.*%7Cdocker.*%22%7D%5B5m%5D))"),
    );

    if cpu.is_none() && ram.is_none() {
        return Ok(json!({ "telemetry": "unavailable" }));
    }
    Ok(json!({
        "telemetry": "ok",
        "ts": chrono::Utc::now(),
        "host": {
            "cpu_pct": cpu.map(|v| (v * 10.0).round() / 10.0),
            "ram_pct": ram.map(|v| (v * 10.0).round() / 10.0),
            "disk_pct": disk.map(|v| (v * 10.0).round() / 10.0),
            "net_rx_bps": net_rx.map(|v| v.round()),
            "net_tx_bps": net_tx.map(|v| v.round()),
        },
    }))
}
