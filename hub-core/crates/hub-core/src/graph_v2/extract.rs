//! graph_v2 extractor endpoints — first real callers of the v2 audit helper.
//!
//! Previously v2 only had `dispatch_*` functions in compiler.rs that ran
//! inside investigation tasks. This module exposes a thin REST surface
//! so external extractors (e.g. an LLM pipeline that pulls claims out of
//! crawled documents) can write claims with the proper audit shape
//! without going through the full investigation workflow.
//!
//! Design notes:
//!   * Endpoint shape mirrors v1 MCP `create_claim` (text + entities)
//!     but routes through the v2 audit helper so the change_log → Neo4j
//!     mirror path is uniform.
//!   * Transactions per claim: claims + claim_entities + audit row are
//!     inserted in a single tx. If any claim fails the whole batch rolls
//!     back — easier to reason about than partial successes.
//!   * No policy::preflight() — extract_claims is a "write side" endpoint
//!     for system-level actors (LLM extractor, ingest pipeline). MCP
//!     create_claim goes through preflight for user/agent roles.
//!   * One POST = N claims (no per-claim POST) to amortise auth/db cost.

use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::graph_v2::audit::emit_claim_audit;
use crate::state::AppState;
use crate::types::AgentIdentity;
use axum::{extract::State, Extension, Json};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct EntityRef {
    pub kind: String,
    pub name: String,
    /// subject | object | mentioned (default: mentioned)
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimInput {
    pub text: String,
    #[serde(default)]
    pub entity_refs: Vec<EntityRef>,
}

#[derive(Debug, Deserialize)]
pub struct ExtractClaimsBody {
    pub claims: Vec<ClaimInput>,
    /// Optional task_id for traceability. If absent the audit row's
    /// task_id stays NULL — fine for ad-hoc extractor runs.
    #[serde(default)]
    pub task_id: Option<Uuid>,
}

#[derive(Debug, serde::Serialize)]
pub struct ExtractClaimsResponse {
    pub claim_ids: Vec<Uuid>,
}

/// POST /api/v1/v2/extract_claims — batch insert claims via v2 audit path.
///
/// Returns the generated claim_ids in input order. The mirror worker
/// drains the change_log audit rows within ~15s and produces :Claim
/// nodes + :ABOUT edges in Neo4j.
pub async fn extract_claims(
    State(state): State<Arc<AppState>>,
    Extension(agent): Extension<AgentIdentity>,
    Json(body): Json<ExtractClaimsBody>,
) -> Result<Json<ExtractClaimsResponse>, (axum::http::StatusCode, String)> {
    if body.claims.is_empty() {
        return Err((axum::http::StatusCode::BAD_REQUEST, "claims must be non-empty".into()));
    }
    if body.claims.len() > 200 {
        return Err((axum::http::StatusCode::BAD_REQUEST, "claims capped at 200 per call".into()));
    }
    let actor = format!("agent:{}", agent.name);
    let mut claim_ids = Vec::with_capacity(body.claims.len());
    for c in &body.claims {
        let cid = insert_one_claim(&state, &actor, c, body.task_id).await.map_err(|e| {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("{e}"))
        })?;
        claim_ids.push(cid);
    }
    Ok(Json(ExtractClaimsResponse { claim_ids }))
}

async fn insert_one_claim(
    state: &AppState,
    actor: &str,
    c: &ClaimInput,
    task: Option<Uuid>,
) -> Result<Uuid, sqlx::Error> {
    let claim_id = Uuid::new_v4();
    let mut tx: Transaction<'_, Postgres> = state.pg.begin().await?;

    // 1. Canonical claim row
    sqlx::query("INSERT INTO claims (claim_id, text, created_by) VALUES ($1,$2,$3)")
        .bind(claim_id)
        .bind(&c.text)
        .bind(actor)
        .execute(&mut *tx)
        .await?;

    // 2. Resolve/create entities + link them. Reuses the v1
    //    create_entity ON CONFLICT (kind, name) DO UPDATE so the same
    //    row is found/created idempotently regardless of who arrives
    //    first. entity_refs is an "anchor" by (kind, name) — the
    //    entity_id comes back from the upsert.
    let mut audit_entities: Vec<Value> = Vec::new();
    for er in &c.entity_refs {
        let role = er.role.as_deref().unwrap_or("mentioned");
        if !matches!(role, "subject" | "object" | "mentioned") {
            // Roll back; the caller must fix the bad role.
            tx.rollback().await?;
            return Err(sqlx::Error::Protocol(format!(
                "invalid entity role '{role}' (allowed: subject|object|mentioned)"
            )));
        }
        let kind = crate::graphw::valid_kind(&er.kind).map_err(|e| {
            sqlx::Error::Protocol(format!("invalid entity kind: {e}"))
        })?;
        let name = crate::graphw::normalize_name(&er.name).map_err(|e| {
            sqlx::Error::Protocol(format!("invalid entity name: {e}"))
        })?;
        let (entity_id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO entities (entity_id, kind, name, created_by)
             VALUES ($1,$2,$3,$4)
             ON CONFLICT (kind, name) DO UPDATE SET aliases = entities.aliases
             RETURNING entity_id",
        )
        .bind(Uuid::new_v4())
        .bind(&kind)
        .bind(&name)
        .bind(actor)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO claim_entities (claim_id, entity_id, role) VALUES ($1,$2,$3)
             ON CONFLICT DO NOTHING",
        )
        .bind(claim_id)
        .bind(entity_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
        audit_entities.push(json!({ "kind": kind, "name": name }));
    }

    // 3. Audit row via the v2 helper (the point of this whole module:
    //    verify the helper works against real callers).
    emit_claim_audit(
        &state.pg,
        claim_id,
        &c.text,
        Some(&audit_entities),
        actor,
        task,
    )
    .await?;

    tx.commit().await?;
    Ok(claim_id)
}
