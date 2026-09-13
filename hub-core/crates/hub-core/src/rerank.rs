//! Reranker stage (directive §A): cross-encoder over the top-K retrieval
//! candidates to refine ordering before returning. Runs after RRF fusion in
//! `hybrid_inner` and after vector search in `semantic_inner`. Graceful
//! degradation: any failure (timeout / 5xx / disabled) returns the
//! pre-rerank ranking unchanged and adds `"rerank": "skipped: <reason>"` to
//! the response so callers can tell.
//!
//! Default backend: T8star `/v1/rerank` (OpenAI-compatible, same relay that
//! hosts the embed endpoint). Default model: `BAAI/bge-reranker-v2-m3`
//! (multilingual, OSINT-friendly en+zh). Costs billed to `rerank_tokens`
//! with estimated tokens = sum(len(doc)/4) + len(query)/4 — T8star does
//! not return usage in the response, so we estimate like embed does.

use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::{HubError, Result};
use crate::state::AppState;

/// Provider-agnostic response shape. T8star today; future-local-ONNX will
/// translate to the same `results[].{index, relevance_score}` contract.
#[derive(Debug, Deserialize)]
struct RerankResponse {
    results: Vec<RerankHit>,
}

#[derive(Debug, Deserialize)]
struct RerankHit {
    index: usize,
    #[allow(dead_code)]
    #[serde(default)]
    document: Option<Value>,
    relevance_score: f64,
}

/// Pure: parse a rerank response JSON string and map hits back to the
/// caller's (doc_id, text) candidates by `index`. Sorts descending by
/// relevance_score. Skips out-of-range and duplicate indices gracefully.
pub fn parse_rerank_response(body: &str, candidates: &[(Uuid, String)]) -> Result<Vec<(Uuid, f64)>> {
    let parsed: RerankResponse = serde_json::from_str(body)
        .map_err(|e| HubError::sensor(format!("rerank: malformed JSON: {e}")))?;
    let mut scored: Vec<(Uuid, f64)> = Vec::with_capacity(parsed.results.len());
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for hit in parsed.results {
        if !seen.insert(hit.index) {
            continue; // degenerate duplicate index, keep first
        }
        if let Some((id, _)) = candidates.get(hit.index) {
            scored.push((*id, hit.relevance_score));
        }
        // out-of-range silently dropped (model returned more than we sent)
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored)
}

/// Estimate rerank token usage. T8star does not return `usage` in
/// `/v1/rerank` responses, so we approximate at ~4 chars/token, the same
/// heuristic `embed.rs` uses for `query_embedding`.
fn estimate_tokens(query: &str, documents: &[String]) -> u64 {
    let q = (query.len() / 4) as u64;
    let d: u64 = documents.iter().map(|s| (s.len() / 4) as u64).sum();
    q + d
}

/// Outcome of a rerank attempt (for callers that need to surface state).
#[derive(Debug, Clone)]
pub struct RerankOutcome {
    /// Reranked (id, score) sorted desc. Empty when disabled or failed.
    pub scored: Vec<(Uuid, f64)>,
    /// One of: "reranked" | "disabled" | "skipped: <reason>" | "no_candidates"
    pub status: String,
    /// Tokens billed for this call (0 when not called).
    pub tokens_billed: u64,
}

/// Call the configured rerank backend on up to `top_k` candidates and
/// return their new ordering. Candidates is (doc_id, text_excerpt).
pub async fn rerank_documents(
    state: &AppState,
    query: &str,
    candidates: &[(Uuid, String)],
    top_k: usize,
) -> RerankOutcome {
    if !state.config.rerank_enabled {
        return RerankOutcome { scored: Vec::new(), status: "disabled".into(), tokens_billed: 0 };
    }
    if candidates.is_empty() {
        return RerankOutcome { scored: Vec::new(), status: "no_candidates".into(), tokens_billed: 0 };
    }
    let slice: &[(Uuid, String)] = if candidates.len() > top_k { &candidates[..top_k] } else { candidates };
    let docs: Vec<String> = slice.iter().map(|(_, t)| t.clone()).collect();
    let tokens_estimate = estimate_tokens(query, &docs);

    let url = format!("{}/rerank", state.config.embedding_base_url);
    let key = match state.config.embedding_api_key.as_deref() {
        Some(k) => k,
        None => {
            return RerankOutcome {
                scored: Vec::new(),
                status: "skipped: EMBEDDING_API_KEY not set".into(),
                tokens_billed: 0,
            };
        }
    };
    let body = json!({
        "model": state.config.rerank_model,
        "query": query,
        "documents": docs,
    });
    let resp = match state
        .http
        .post(&url)
        .bearer_auth(key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(state.config.rerank_timeout_secs))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return RerankOutcome {
                scored: Vec::new(),
                status: format!("skipped: transport: {e}"),
                tokens_billed: 0,
            };
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return RerankOutcome {
            scored: Vec::new(),
            status: format!("skipped: HTTP {status}: {}", body.chars().take(200).collect::<String>()),
            tokens_billed: 0,
        };
    }
    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            return RerankOutcome {
                scored: Vec::new(),
                status: format!("skipped: read body: {e}"),
                tokens_billed: 0,
            };
        }
    };
    match parse_rerank_response(&body, slice) {
        Ok(scored) => RerankOutcome {
            scored,
            status: "reranked".into(),
            tokens_billed: tokens_estimate,
        },
        Err(e) => RerankOutcome {
            scored: Vec::new(),
            status: format!("skipped: parse: {e}"),
            tokens_billed: 0,
        },
    }
}
