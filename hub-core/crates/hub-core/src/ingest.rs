//! osint-ingest (directive §50): the single normalization entry for all
//! content before any AI analysis. Inline path (crawl tool) and queued path
//! (Redis Stream `hub.ingest`) share the same pipeline.

use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::Result;
use crate::state::AppState;
use crate::store;
use crate::types::{BusEvent, EvidenceEvent};

pub const INGEST_STREAM: &str = "hub.ingest";

/// Tracking/noise query params stripped during canonicalization.
const JUNK_PARAMS: &[&str] = &[
    "utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content",
    "utm_id", "fbclid", "gclid", "dclid", "msclkid", "mc_cid", "mc_eid",
    "igshid", "ref", "ref_src", "spm", "si",
];

/// Canonicalize a URL: lowercase scheme+host, drop default port, drop
/// fragment, strip tracking params, sort remaining query params.
pub fn canonicalize_url(raw: &str) -> String {
    let Ok(mut u) = url::Url::parse(raw) else {
        return raw.trim().to_string();
    };
    u.set_fragment(None);
    let mut kept: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| !JUNK_PARAMS.contains(&k.to_ascii_lowercase().as_str()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    kept.sort();
    u.query_pairs_mut().clear().extend_pairs(kept);
    let mut s = u.to_string();
    if s.ends_with('/') && u.path() == "/" && u.query().is_none() {
        s.pop();
    }
    s.to_lowercase()
}

/// sha256 hex of normalized text (whitespace-collapsed).
pub fn content_hash(text: &str) -> String {
    let normalized: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut h = Sha256::new();
    h.update(normalized.as_bytes());
    crate::auth::hex_lower(&h.finalize())
}

/// 64-bit simhash over word tokens.
pub fn simhash64(text: &str) -> i64 {
    let mut v = [0i32; 64];
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
    {
        let mut h = Sha256::new();
        h.update(token.to_lowercase().as_bytes());
        let d = h.finalize();
        let x = u64::from_be_bytes(d[0..8].try_into().unwrap());
        for i in 0..64 {
            if x & (1u64 << i) != 0 {
                v[i] += 1;
            } else {
                v[i] -= 1;
            }
        }
    }
    let mut out = 0u64;
    for (i, &c) in v.iter().enumerate() {
        if c > 0 {
            out |= 1u64 << i;
        }
    }
    out as i64
}

pub fn hamming(a: i64, b: i64) -> u32 {
    (a ^ b).count_ones()
}

pub struct IngestOutcome {
    pub document_id: Uuid,
    pub content_hash: String,
    pub duplicate: bool,
    pub near_duplicate_of: Option<Uuid>,
}

/// Full pipeline for one evidence item. Idempotent by content_hash.
#[allow(clippy::too_many_arguments)]
pub async fn ingest_content(
    state: &AppState,
    origin: &str,
    url: &str,
    title: Option<&str>,
    content: &str,
    published_at: Option<chrono::DateTime<Utc>>,
    metadata: serde_json::Value,
    provenance: serde_json::Value,
    parent_task: Option<Uuid>,
    embed_mode: &str, // "auto"|"force"|"skip" (SP2B §40)
) -> Result<IngestOutcome> {
    let canonical = canonicalize_url(url);
    let hash = content_hash(content);
    let shash = simhash64(content);

    // Near-dup check over recent window (exact dup handled by UNIQUE).
    let mut near_dup_of = None;
    if let Ok(recent) = store::recent_simhashes(&state.pg, 2000).await {
        for (doc_id, other) in recent {
            if hamming(shash, other) <= state.config.simhash_max_hamming {
                near_dup_of = Some(doc_id);
                break;
            }
        }
    }

    // Raw capture to disk (skip rewrite for exact dups — path is hash-stable).
    let raw_path = format!("{}/{}.txt", state.config.raw_dir, hash);
    if !tokio::fs::try_exists(&raw_path).await.unwrap_or(false) {
        tokio::fs::create_dir_all(&state.config.raw_dir).await?;
        tokio::fs::write(&raw_path, content).await?;
    }

    let ev = EvidenceEvent {
        event_id: Uuid::new_v4(),
        source: origin.to_string(),
        url: url.to_string(),
        retrieved_at: Utc::now(),
        published_at,
        content_hash: hash.clone(),
        content: content.to_string(),
        metadata,
        provenance,
    };
    let source_id = store::get_or_create_source(&state.pg, origin, None).await?;
    let inserted = store::insert_document(
        &state.pg,
        &ev,
        source_id,
        &canonical,
        title,
        shash,
        &raw_path,
        parent_task,
    )
    .await?;

    if !inserted.duplicate {
        match embed_mode {
            "skip" => {
                let _ = sqlx::query(
                    "UPDATE documents SET embedding_status='SKIPPED' WHERE document_id=$1",
                )
                .bind(inserted.document_id)
                .execute(&state.pg)
                .await;
            }
            mode => {
                store::enqueue_embedding_job(
                    &state.pg,
                    inserted.document_id,
                    &state.config.embedding_model,
                    mode == "force",
                )
                .await?;
            }
        }
        let ev_out = BusEvent::new(
            "DOCUMENT_INGESTED",
            "system:ingest",
            serde_json::json!({
                "document_id": inserted.document_id,
                "url": canonical,
                "content_hash": hash,
                "origin": origin,
                "near_duplicate": near_dup_of.is_some(),
            }),
        );
        crate::events::publish(state, ev_out).await;
    }

    Ok(IngestOutcome {
        document_id: inserted.document_id,
        content_hash: inserted.content_hash,
        duplicate: inserted.duplicate,
        near_duplicate_of: near_dup_of,
    })
}

/// Queued-path worker: consumes EvidenceEvents from `hub.ingest`
/// (XREAD BLOCK on a DEDICATED connection — Redis is FIFO per connection, so
/// a blocking command on the shared multiplexed connection head-of-line-blocks
/// every other command (caused false supervisor reconnect churn). Reconnects
/// itself on error; replay-safe because ingest is idempotent).
pub async fn run_worker(state: AppState, ct: tokio_util::sync::CancellationToken) {
    const LAST_ID_KEY: &str = "hub:ingest:lastid";
    let mut last_id: String = state
        .redis_timed::<Option<String>>(redis::cmd("GET").arg(LAST_ID_KEY).clone(), 2000)
        .await
        .flatten()
        .unwrap_or_else(|| "0".to_string());
    tracing::info!(last_id = %last_id, "ingest worker started (dedicated blocking connection)");

    // Dedicated connection for the blocking XREAD (never shares the slot).
    let mut conn = match state.redis_client.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "ingest worker: initial redis connection failed");
            return;
        }
    };

    loop {
        if ct.is_cancelled() {
            tracing::info!("ingest worker shutting down");
            return;
        }
        let res: redis::RedisResult<redis::Value> = tokio::time::timeout(
            // BLOCK 5s + margin — a wedged connection must not hang the worker.
            std::time::Duration::from_secs(10),
            redis::cmd("XREAD")
                .arg("BLOCK").arg(5000)
                .arg("COUNT").arg(10)
                .arg("STREAMS").arg(INGEST_STREAM).arg(&last_id)
                .query_async(&mut conn),
        )
        .await
        .unwrap_or_else(|_| {
            Err(redis::RedisError::from((redis::ErrorKind::IoError, "xread timeout")))
        });

        let Ok(redis::Value::Array(streams)) = res else {
            // Error or timeout: if the error is real (not a block timeout),
            // reconnect the dedicated connection before retrying.
            if res.is_err() {
                if let Ok(c) = state.redis_client.get_multiplexed_async_connection().await {
                    conn = c;
                }
            }
            continue;
        };
        for stream in streams {
            let redis::Value::Array(kv) = stream else { continue };
            let Some(redis::Value::Array(entries)) = kv.get(1).cloned() else { continue };
            for entry in entries {
                let redis::Value::Array(e) = entry else { continue };
                let Some(redis::Value::BulkString(id_bytes)) = e.first() else { continue };
                let id = String::from_utf8_lossy(id_bytes).to_string();
                if let Some(redis::Value::BulkString(data)) = e.get(1).and_then(|f| match f {
                    redis::Value::Array(pairs) => pairs.iter().skip_while(|p| !matches!(p, redis::Value::BulkString(k) if k == b"data")).nth(1).cloned(),
                    _ => None,
                }) {
                    let text = String::from_utf8_lossy(&data).to_string();
                    if let Ok(ev) = serde_json::from_str::<EvidenceEvent>(&text) {
                        if let Err(err) = ingest_content(
                            &state,
                            &ev.source,
                            &ev.url,
                            None,
                            &ev.content,
                            ev.published_at,
                            ev.metadata.clone(),
                            ev.provenance.clone(),
                            None,
                            "auto",
                        )
                        .await
                        {
                            tracing::warn!(error = %err, "queued ingest failed");
                        }
                    }
                }
                last_id = id.clone();
                let _: Option<()> = state
                    .redis_timed(redis::cmd("SET").arg(LAST_ID_KEY).arg(&id).clone(), 2000)
                    .await;
            }
        }
    }
}
