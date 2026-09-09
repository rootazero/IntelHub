//! Event bus: Redis Stream `hub.events` is the durable bus; an in-process
//! tokio broadcast channel feeds SSE; every event is persisted to Postgres
//! `events`. MCP is never used as the internal message bus (directive §64).

use crate::state::AppState;
use crate::types::BusEvent;

pub const STREAM: &str = "hub.events";

/// Publish an event: broadcast to SSE subscribers, persist to PG, append to
/// the Redis stream (best-effort ordering: broadcast first so live consumers
/// see it even if PG/Redis hiccup).
pub async fn publish(state: &AppState, ev: BusEvent) {
    let _ = state.event_tx.send(ev.clone());

    if let Err(e) = crate::store::persist_event(&state.pg, &ev).await {
        tracing::warn!(error = %e, "event persist failed");
    }

    let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".to_string());
    let res: Option<String> = state
        .redis_timed(
            redis::cmd("XADD")
                .arg(STREAM)
                .arg("MAXLEN").arg("~").arg(10000)
                .arg("*")
                .arg("data").arg(payload)
                .clone(),
            2000,
        )
        .await;
    if res.is_none() {
        tracing::warn!("event XADD failed or timed out (redis mid-failure)");
    }
}
