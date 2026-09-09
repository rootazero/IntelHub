//! Server assembly: MCP service + REST router + auth middleware + ingest
//! worker, one process (single binary per directive §20).

use std::sync::Arc;

use axum::middleware;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

pub async fn serve(state: Arc<AppState>) -> anyhow::Result<()> {
    let ct = CancellationToken::new();

    // Ingest worker (queued path) in the same process.
    {
        let worker_state = (*state).clone();
        let wct = ct.child_token();
        tokio::spawn(async move { crate::ingest::run_worker(worker_state, wct).await });
    }

    // Redis supervisor: PING with hard timeout every 15s; replace the shared
    // connection when it silently dies (half-open bridge TCP wedged every
    // command path in SP3 acceptance — never again).
    {
        let s = (*state).clone();
        let t = ct.child_token();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                tokio::select! {
                    _ = t.cancelled() => break,
                    _ = tick.tick() => {
                        let mut conn = s.redis().await;
                        let ok = tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            redis::cmd("PING").query_async::<String>(&mut conn),
                        )
                        .await
                        .map(|r| r.is_ok())
                        .unwrap_or(false);
                        if !ok {
                            s.redis_reconnect().await;
                        }
                    }
                }
            }
        });
    }

    // SP2B workers: embedding pipeline, graph-sync replay, alert webhook
    // dispatcher, sensor flap watcher, component update watcher.
    {
        let s = (*state).clone();
        let t = ct.child_token();
        tokio::spawn(async move { crate::embed::run_worker(s, t).await });
    }
    {
        let s = (*state).clone();
        let t = ct.child_token();
        tokio::spawn(async move { crate::graphw::run_replay(s, t).await });
    }
    {
        let s = (*state).clone();
        let t = ct.child_token();
        tokio::spawn(async move { crate::alerts::run_dispatcher(s, t).await });
    }
    {
        let s = (*state).clone();
        let t = ct.child_token();
        tokio::spawn(async move { crate::alerts::run_sensor_watcher(s, t).await });
    }
    {
        let s = (*state).clone();
        let t = ct.child_token();
        tokio::spawn(async move { crate::components::run_update_watcher(s, t).await });
    }

    // Qdrant collection provisioning (idempotent; failure is non-fatal —
    // the embedding worker will surface outages as alerts).
    if let Err(e) = crate::vector::ensure_collection(&state).await {
        tracing::warn!(error = %e, "qdrant collection init failed (non-fatal)");
    }

    let mcp_state = state.clone();
    let mcp_config = StreamableHttpServerConfig::default()
        .with_cancellation_token(ct.child_token())
        // DNS-rebinding protection: allow our LAN address/hostname (default
        // allowlist is localhost-only, which 403s LAN clients).
        .with_allowed_hosts(state.config.mcp_allowed_hosts.clone());
    let mcp_service = StreamableHttpService::new(
        move || Ok(crate::mcp::HubMcp::new(mcp_state.clone())),
        LocalSessionManager::default().into(),
        mcp_config,
    );

    let app = crate::api::router()
        .nest_service("/mcp", mcp_service)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::auth_middleware,
        ))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(&state.config.listen_addr).await?;
    tracing::info!(addr = %state.config.listen_addr, "intelhub-core listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            ct.cancel();
        })
        .await?;
    Ok(())
}
