//! SpiderFoot evidence bridge (SP5, directive §48: sensor output passes the
//! normalization/ingest layer before any storage). Polls the SpiderFoot API
//! for FINISHED scans and ingests each scan's JSON export as ONE evidence
//! document (dedupe via redis ingested-set + content-hash). Scan rows are
//! intentionally NOT exploded into entities — that would flood the graph;
//! evidence-level search is the right granularity (SP5 Q2=B).

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

const DONE_SET: &str = "hub:spiderfoot:done";
const MAX_EXPORT_BYTES: usize = 1024 * 1024; // 1MB cap per scan export

fn find_hex_id(row: &Value) -> Option<String> {
    let arr = row.as_array()?;
    for v in arr {
        if let Some(s) = v.as_str() {
            // SpiderFoot scan instance ids are 8-char hex (accept up to 32)
            if (8..=32).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn find_status(row: &Value) -> Option<String> {
    let arr = row.as_array()?;
    for v in arr {
        if let Some(s) = v.as_str() {
            match s {
                "FINISHED" | "RUNNING" | "STARTING" | "ABORTED" | "FAILED" | "ABORT-REQUESTED" | "CREATED" => {
                    return Some(s.to_string())
                }
                _ => {}
            }
        }
    }
    None
}

pub async fn run_spiderfoot_sync(state: AppState, ct: CancellationToken) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(120));
    tracing::info!(url = %state.config.spiderfoot_url, "spiderfoot-sync worker started");
    loop {
        tokio::select! {
            _ = ct.cancelled() => { tracing::info!("spiderfoot-sync shutting down"); return; }
            _ = tick.tick() => {}
        }
        if let Err(e) = sync_once(&state).await {
            tracing::debug!(error = %e, "spiderfoot-sync tick skipped"); // sensor off = normal
        }
    }
}

async fn sync_once(state: &AppState) -> crate::error::Result<()> {
    let base = &state.config.spiderfoot_url;
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        state.http.get(format!("{base}/scanlist")).send(),
    )
    .await
    .map_err(|_| crate::error::HubError::internal("spiderfoot scanlist timeout"))?
    .map_err(|e| crate::error::HubError::internal(e.to_string()))?;
    let scans: Vec<Value> = resp.json().await.unwrap_or_default();

    for scan in &scans {
        let Some(id) = find_hex_id(scan) else { continue };
        if find_status(scan).as_deref() != Some("FINISHED") {
            continue;
        }
        // already ingested?
        let done: Option<bool> = state
            .redis_timed(redis::cmd("SISMEMBER").arg(DONE_SET).arg(&id).clone(), 2000)
            .await;
        if done.unwrap_or(false) {
            continue;
        }

        // export scan results as JSON (endpoint verified against sfwebui.py)
        let export = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            state
                .http
                .get(format!("{base}/scaneventresultexportmulti?ids={id}&filetype=json"))
                .send(),
        )
        .await;
        let Ok(Ok(r)) = export else { continue };
        if !r.status().is_success() {
            tracing::warn!(scan = %id, status = %r.status(), "spiderfoot export failed");
            continue;
        }
        let mut text = r.text().await.unwrap_or_default();
        if text.len() > MAX_EXPORT_BYTES {
            text.truncate(MAX_EXPORT_BYTES);
            text.push_str("\n…[truncated at 1MB]");
        }
        if text.trim().is_empty() || text.trim() == "[]" {
            let _: Option<i64> = state
                .redis_timed(redis::cmd("SADD").arg(DONE_SET).arg(&id).clone(), 2000)
                .await;
            continue; // empty scan — mark done, no evidence value
        }

        let name = scan
            .as_array()
            .and_then(|a| a.iter().find_map(|v| v.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| id.clone());
        let outcome = crate::ingest::ingest_content(
            state,
            "spiderfoot",
            &format!("{base}/scaninfo?id={id}"),
            Some(&format!("SpiderFoot scan: {name}")),
            &text,
            None,
            serde_json::json!({ "scan_id": id, "scan_row": scan }),
            serde_json::json!({ "ingested_by": "hub:spiderfoot-sync" }),
            None,
            "auto", None,
        )
        .await?;
        let _: Option<i64> = state
            .redis_timed(redis::cmd("SADD").arg(DONE_SET).arg(&id).clone(), 2000)
            .await;
        tracing::info!(scan = %id, document = %outcome.document_id, "spiderfoot scan ingested");
        crate::events::publish(
            state,
            crate::types::BusEvent::new(
                "spiderfoot_scan_ingested",
                "hub:spiderfoot-sync",
                serde_json::json!({ "scan_id": id, "document_id": outcome.document_id }),
            ),
        )
        .await;
    }
    Ok(())
}
