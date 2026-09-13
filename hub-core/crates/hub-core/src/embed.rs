//! Embedding pipeline (directive §39–42) — the hub's ONLY cloud call.
//!
//! Worker consumes embedding_jobs (FOR UPDATE SKIP LOCKED). Decision rules
//! (§40): ≥HUB_EMBED_MIN_WORDS words, no simhash near-dup, not a duplicate —
//! violations become SKIPPED with a recorded reason; `force` bypasses the word
//! threshold (agent-requested). Cache key (§42): content_hash +
//! chunk_algorithm_version + model — chunks already present for that triple are
//! NEVER re-embedded. Failures retry ×3 with backoff → FAILED + alert (§72).

use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::{HubError, Result};
use crate::state::AppState;

pub const CHUNK_VERSION: &str = "fixed-800-80-v1";
const CHUNK_CHARS: usize = 3200; // ≈800 tokens @ ~4 chars/token
const OVERLAP_CHARS: usize = 320; // ≈80 tokens
const BATCH: usize = 16;

/// Deterministic Qdrant point id for a chunk (idempotent re-embed = same id).
fn point_id(document_id: Uuid, chunk_ix: usize, model: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{document_id}:{chunk_ix}:{CHUNK_VERSION}:{model}").as_bytes(),
    )
}

/// Split into ~800-token windows with ~80-token overlap (char approximation).
pub fn chunk_text(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= CHUNK_CHARS {
        return vec![text.trim().to_string()];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + CHUNK_CHARS).min(chars.len());
        let chunk: String = chars[start..end].iter().collect();
        let chunk = chunk.trim().to_string();
        if !chunk.is_empty() {
            out.push(chunk);
        }
        if end == chars.len() {
            break;
        }
        start = end - OVERLAP_CHARS;
    }
    out
}

/// Call the OpenAI-compatible embeddings endpoint (T8star relay).
/// Returns (vectors, total_tokens_billed).
pub async fn embed_batch(state: &AppState, texts: &[String]) -> Result<(Vec<Vec<f32>>, u64)> {
    let key = state
        .config
        .embedding_api_key
        .as_deref()
        .ok_or_else(|| HubError::internal("EMBEDDING_API_KEY not configured"))?;
    let url = format!("{}/embeddings", state.config.embedding_base_url);
    let resp = state
        .http
        .post(&url)
        .bearer_auth(key)
        .json(&json!({ "model": state.config.embedding_model, "input": texts }))
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(HubError::sensor(format!(
            "embedding API {status}: {}",
            body.chars().take(300).collect::<String>()
        )));
    }
    let body: Value = resp.json().await?;
    let mut vectors: Vec<(usize, Vec<f32>)> = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let ix = item.get("index")?.as_u64()? as usize;
                    let v: Vec<f32> = item
                        .get("embedding")?
                        .as_array()?
                        .iter()
                        .filter_map(|x| x.as_f64().map(|f| f as f32))
                        .collect();
                    Some((ix, v))
                })
                .collect()
        })
        .unwrap_or_default();
    vectors.sort_by_key(|(ix, _)| *ix);
    let vectors: Vec<Vec<f32>> = vectors.into_iter().map(|(_, v)| v).collect();
    if vectors.len() != texts.len() {
        return Err(HubError::sensor(format!(
            "embedding API returned {} vectors for {} inputs",
            vectors.len(),
            texts.len()
        )));
    }
    let tokens = body
        .get("usage")
        .and_then(|u| u.get("total_tokens"))
        .and_then(|t| t.as_u64())
        .unwrap_or_else(|| texts.iter().map(|t| (t.len() / 4) as u64).sum());
    Ok((vectors, tokens))
}

/// Embed a single query text with a 24h Redis cache (§42 spirit for queries).
pub async fn query_embedding(state: &AppState, text: &str) -> Result<(Vec<f32>, u64)> {
    let cache_key = format!(
        "hub:qemb:{}",
        crate::auth::hash_key(&format!("{}:{}", state.config.embedding_model, text))
    );
    if let Some(cached) = state
        .redis_timed::<Option<String>>(redis::cmd("GET").arg(&cache_key).clone(), 2000)
        .await
        .flatten()
    {
        if let Ok(v) = serde_json::from_str::<Vec<f32>>(&cached) {
            return Ok((v, 0)); // cache hit: zero billed tokens
        }
    }
    let (vectors, tokens) = embed_batch(state, &[text.to_string()]).await?;
    let v = vectors
        .into_iter()
        .next()
        .ok_or_else(|| HubError::sensor("empty embedding response"))?;
    if let Ok(s) = serde_json::to_string(&v) {
        let _: Option<()> = state
            .redis_timed(redis::cmd("SET").arg(&cache_key).arg(s).arg("EX").arg(86400).clone(), 2000)
            .await;
    }
    Ok((v, tokens))
}

// ---------- worker ----------

pub async fn run_worker(state: AppState, ct: tokio_util::sync::CancellationToken) {
    if !state.config.embed_enabled {
        tracing::info!("embedding worker disabled (HUB_EMBED_WORKER_ENABLED != true)");
        return;
    }
    tracing::info!("embedding worker started");
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = ct.cancelled() => break,
            _ = tick.tick() => {
                if let Err(e) = process_next(&state).await {
                    tracing::warn!(error = %e, "embedding worker tick failed");
                }
            }
        }
    }
}

/// Claim one pending job (SKIP LOCKED) and process it.
async fn process_next(state: &AppState) -> Result<()> {
    let claimed: Option<(Uuid, Uuid, bool, i32)> = sqlx::query_as(
        "UPDATE embedding_jobs SET status='RUNNING', updated_at=now()
         WHERE job_id = (
           SELECT job_id FROM embedding_jobs WHERE status='PENDING'
           ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED
         ) RETURNING job_id, document_id, force, attempts",
    )
    .fetch_optional(&state.pg)
    .await?;
    let Some((job_id, document_id, force, attempts)) = claimed else {
        return Ok(());
    };

    // §60 YELLOW: pause non-forced embedding while any budget ≥70%.
    if !force && crate::cost::embeddings_paused(state).await {
        sqlx::query(
            "UPDATE embedding_jobs SET status='PENDING', reason='budget YELLOW: auto-embedding paused', updated_at=now()
             WHERE job_id=$1",
        )
        .bind(job_id)
        .execute(&state.pg)
        .await?;
        // Avoid a tight re-claim loop while paused.
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        return Ok(());
    }

    match process_document(state, document_id, force).await {
        Ok(outcome) => {
            sqlx::query(
                "UPDATE embedding_jobs SET status=$2, reason=$3, updated_at=now() WHERE job_id=$1",
            )
            .bind(job_id)
            .bind(outcome.0)
            .bind(outcome.1)
            .execute(&state.pg)
            .await?;
        }
        Err(e) => {
            if attempts >= 2 {
                sqlx::query(
                    "UPDATE embedding_jobs SET status='FAILED', attempts=attempts+1, reason=$2, updated_at=now()
                     WHERE job_id=$1",
                )
                .bind(job_id)
                .bind(e.to_string())
                .execute(&state.pg)
                .await?;
                let _ = sqlx::query(
                    "UPDATE documents SET embedding_status='FAILED' WHERE document_id=$1",
                )
                .bind(document_id)
                .execute(&state.pg)
                .await;
                let _ = crate::alerts::raise(
                    state,
                    crate::alerts::NewAlert {
                        severity: "warning",
                        source: "infra",
                        title: "Embedding job failed permanently (3 attempts)",
                        body: Some(&e.to_string()),
                        task_id: None,
                        investigation_id: None,
                        entity_name: None,
                        evidence_id: Some(document_id),
                        recommended_action: Some("Check embedding relay reachability / API key"),
                        dedupe_key: Some("embedding:failed"),
                    },
                )
                .await;
            } else {
                sqlx::query(
                    "UPDATE embedding_jobs SET status='PENDING', attempts=attempts+1, reason=$2, updated_at=now()
                     WHERE job_id=$1",
                )
                .bind(job_id)
                .bind(e.to_string())
                .execute(&state.pg)
                .await?;
            }
        }
    }
    Ok(())
}

/// Returns (final_status, reason).
async fn process_document(
    state: &AppState,
    document_id: Uuid,
    force: bool,
) -> Result<(&'static str, Option<String>)> {
    let doc: Option<(String, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT content_text, simhash, created_by FROM documents d
         LEFT JOIN LATERAL (SELECT NULL::text AS created_by) x ON true
         WHERE d.document_id = $1",
    )
    .bind(document_id)
    .fetch_optional(&state.pg)
    .await?;
    let Some((text, simhash, _)) = doc else {
        return Ok(("SKIPPED", Some("document not found".into())));
    };

    if !force {
        // §40 rule 1: boilerplate / too short
        let words = text.split_whitespace().count() as u32;
        if words < state.config.embed_min_words {
            let _ = sqlx::query(
                "UPDATE documents SET embedding_status='SKIPPED' WHERE document_id=$1",
            )
            .bind(document_id)
            .execute(&state.pg)
            .await;
            return Ok((
                "SKIPPED",
                Some(format!("{} words < min {}", words, state.config.embed_min_words)),
            ));
        }
        // §40 rule 2: simhash near-dup of an already-embedded document
        if let Some(sh) = simhash {
            let near: Option<(Uuid,)> = sqlx::query_as(
                "SELECT document_id FROM documents
                 WHERE embedding_status='DONE' AND simhash IS NOT NULL AND document_id <> $1
                   AND bit_count((simhash # $2)::bit(64)) <= $3
                 LIMIT 1",
            )
            .bind(document_id)
            .bind(sh)
            .bind(state.config.simhash_max_hamming as i32)
            .fetch_optional(&state.pg)
            .await?;
            if let Some((other,)) = near {
                let _ = sqlx::query(
                    "UPDATE documents SET embedding_status='SKIPPED' WHERE document_id=$1",
                )
                .bind(document_id)
                .execute(&state.pg)
                .await;
                return Ok(("SKIPPED", Some(format!("near-duplicate of {other}"))));
            }
        }
    }

    let model = state.config.embedding_model.clone();
    let chunks = chunk_text(&text);

    // §42 cache proof: chunks already stored for (doc, version, model) are reused.
    let cached: Vec<(i32,)> = sqlx::query_as(
        "SELECT chunk_ix FROM embedding_chunks
         WHERE document_id=$1 AND chunk_version=$2 AND model=$3",
    )
    .bind(document_id)
    .bind(CHUNK_VERSION)
    .bind(&model)
    .fetch_all(&state.pg)
    .await?;
    let cached_ix: std::collections::HashSet<i32> = cached.into_iter().map(|(i,)| i).collect();
    let todo: Vec<(usize, &String)> = chunks
        .iter()
        .enumerate()
        .filter(|(ix, _)| !cached_ix.contains(&(*ix as i32)))
        .collect();

    let mut total_tokens: u64 = 0;
    for batch in todo.chunks(BATCH) {
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.to_string()).collect();
        let (vectors, tokens) = embed_batch(state, &texts).await?;
        total_tokens += tokens;
        let points: Vec<(Uuid, Vec<f32>, Value)> = batch
            .iter()
            .zip(vectors)
            .map(|((ix, text), vec)| {
                (
                    point_id(document_id, *ix, &model),
                    vec,
                    json!({
                        "document_id": document_id,
                        "chunk_ix": ix,
                        "chunk_version": CHUNK_VERSION,
                    }),
                )
            })
            .collect();
        for ((ix, text), (pid, _, _)) in batch.iter().zip(points.iter()) {
            sqlx::query(
                "INSERT INTO embedding_chunks
                   (chunk_id, document_id, chunk_ix, chunk_version, model, content_hash, token_count, qdrant_point)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                 ON CONFLICT (document_id, chunk_ix, chunk_version, model) DO NOTHING",
            )
            .bind(Uuid::new_v4())
            .bind(document_id)
            .bind(*ix as i32)
            .bind(CHUNK_VERSION)
            .bind(&model)
            .bind(crate::ingest::content_hash(text))
            .bind((text.len() / 4) as i32)
            .bind(pid)
            .execute(&state.pg)
            .await?;
        }
        crate::vector::upsert_points(state, points).await?;
    }

    if total_tokens > 0 {
        // Attribute cost to the agent that owns the parent task when known.
        let agent: Option<(Uuid,)> = sqlx::query_as(
            "SELECT a.agent_id FROM documents d
             JOIN tasks t ON t.task_id = d.parent_task
             JOIN agents a ON a.name = replace(t.created_by, 'agent:', '')
             WHERE d.document_id = $1",
        )
        .bind(document_id)
        .fetch_optional(&state.pg)
        .await?;
        crate::cost::record_cost(
            state,
            agent.map(|(id,)| id),
            None,
            None,
            "embedding_tokens",
            total_tokens as f64,
            "token",
            json!({ "document_id": document_id, "model": model }),
        )
        .await?;
    }

    sqlx::query("UPDATE documents SET embedding_status='DONE' WHERE document_id=$1")
        .bind(document_id)
        .execute(&state.pg)
        .await?;
    let reused = chunks.len() - todo.len();
    Ok((
        "DONE",
        Some(format!(
            "{} chunks ({} reused from §42 cache), {} tokens",
            chunks.len(),
            reused,
            total_tokens
        )),
    ))
}
