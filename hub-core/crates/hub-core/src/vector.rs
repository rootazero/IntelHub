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
