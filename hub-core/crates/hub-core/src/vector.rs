//! Qdrant client (REST). SP2A: collection provisioning + health only.
//! Qdrant never generates embeddings (directive §39); the embedding pipeline
//! lands in SP2B and will write points into the collection created here.

use crate::error::Result;
use crate::state::AppState;

/// Idempotently create the evidence collection (dim from config, cosine).
pub async fn ensure_collection(state: &AppState) -> Result<()> {
    let url = format!("{}/collections/{}", state.config.qdrant_url, state.config.qdrant_collection);
    let resp = state
        .http
        .put(&url)
        .json(&serde_json::json!({
            "vectors": { "size": state.config.embedding_dims, "distance": "Cosine" }
        }))
        .send()
        .await?;
    // 200 = created, 400 with "already exists" = fine
    if resp.status().is_success() {
        tracing::info!(collection = %state.config.qdrant_collection, "qdrant collection created");
        return Ok(());
    }
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if text.contains("already exists") || text.contains("exists") {
        return Ok(());
    }
    Err(crate::error::HubError::internal(format!(
        "qdrant collection init failed: {status}: {text}"
    )))
}

pub async fn healthy(state: &AppState) -> bool {
    let url = format!("{}/healthz", state.config.qdrant_url);
    matches!(state.http.get(&url).send().await, Ok(r) if r.status().is_success())
}

/// Upsert embedded chunks (SP2B). Point ids are deterministic (uuid v5), so
/// re-embedding identical content overwrites rather than duplicates.
pub async fn upsert_points(
    state: &AppState,
    points: Vec<(uuid::Uuid, Vec<f32>, serde_json::Value)>,
) -> Result<()> {
    if points.is_empty() {
        return Ok(());
    }
    let url = format!(
        "{}/collections/{}/points?wait=true",
        state.config.qdrant_url, state.config.qdrant_collection
    );
    let body = serde_json::json!({
        "points": points.into_iter().map(|(id, vector, payload)| {
            serde_json::json!({ "id": id, "vector": vector, "payload": payload })
        }).collect::<Vec<_>>()
    });
    let resp = state.http.put(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(crate::error::HubError::sensor(format!(
            "qdrant upsert {status}: {}",
            text.chars().take(300).collect::<String>()
        )));
    }
    Ok(())
}

/// Vector search → (document_id, best score) merged across chunks.
pub async fn search(
    state: &AppState,
    vector: &[f32],
    limit: u64,
) -> Result<Vec<(uuid::Uuid, f32)>> {
    let url = format!(
        "{}/collections/{}/points/search",
        state.config.qdrant_url, state.config.qdrant_collection
    );
    let resp = state
        .http
        .post(&url)
        .json(&serde_json::json!({
            "vector": vector,
            "limit": limit,
            "with_payload": true,
        }))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(crate::error::HubError::sensor(format!(
            "qdrant search {status}: {}",
            text.chars().take(300).collect::<String>()
        )));
    }
    let body: serde_json::Value = resp.json().await?;
    let mut best: std::collections::HashMap<uuid::Uuid, f32> = std::collections::HashMap::new();
    if let Some(results) = body.get("result").and_then(|r| r.as_array()) {
        for hit in results {
            let doc = hit
                .get("payload")
                .and_then(|p| p.get("document_id"))
                .and_then(|d| d.as_str())
                .and_then(|s| uuid::Uuid::parse_str(s).ok());
            let score = hit.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0) as f32;
            if let Some(doc) = doc {
                let entry = best.entry(doc).or_insert(score);
                if score > *entry {
                    *entry = score;
                }
            }
        }
    }
    let mut out: Vec<(uuid::Uuid, f32)> = best.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}
