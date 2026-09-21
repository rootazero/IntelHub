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
        // SP9 v2 change_log → Neo4j mirror. Reads graph_change_log rows
        // (written by graph_v2::compiler), translates to v1 op shapes,
        // enqueues into graph_sync_queue where the replay worker above
        // drains it. Cadence 15s.
        let s = (*state).clone();
        let t = ct.child_token();
        tokio::spawn(async move { crate::graphw::run_change_log_mirror(s, t).await });
    }
    {
        // 2026-09-14 phase 3: Neo4j ↔ PG reconciler. Detaches/deletes
        // Neo4j nodes + edges whose PG canonical rows no longer exist.
        // Defense-in-depth for the one-way change_log mirror: nothing in
        // production currently deletes graph tables, but admin tools /
        // future retract endpoints could. Runs once at startup + hourly.
        let s = (*state).clone();
        let t = ct.child_token();
        tokio::spawn(async move { crate::graphw::run_reconcile(s, t).await });
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
    {
        // SP6: native monitor — hub-core's own signal collectors (replaces
        // the Crucix container; spec 2026-09-10-intelhub-sp6-native-monitor).
        let s = (*state).clone();
        let t = ct.child_token();
        tokio::spawn(async move { crate::monitor::run_monitor(s, t).await });
    }
    {
        // SP5: SpiderFoot finished-scan → evidence bridge (§48).
        let s = (*state).clone();
        let t = ct.child_token();
        tokio::spawn(async move { crate::spiderfoot::run_spiderfoot_sync(s, t).await });
    }

    {
        // GEV P12 T1: adsbdb enrichment — load the disk cache and start the
        // 15s dirty flusher. Missing/corrupt cache starts empty, never fatal.
        let s = (*state).clone();
        tokio::spawn(async move {
            if let Err(e) = crate::gev_enrichment::start(s).await {
                tracing::warn!(error = %e, "adsbdb enrichment start failed");
            }
        });
    }

    // Qdrant collection provisioning (idempotent; failure is non-fatal —
    // the embedding worker will surface outages as alerts).
    if let Err(e) = crate::vector::ensure_collection(&state).await {
        tracing::warn!(error = %e, "qdrant collection init failed (non-fatal)");
    }

    // One-shot entity seeder: populate the graph from existing data +
    // curated roster. Idempotent via ON CONFLICT (kind, name). Fire-and-
    // forget (logged inside) — must not block the HTTP listener.
    {
        let s = (*state).clone();
        tokio::spawn(async move {
            if let Err(e) = crate::entity_seeder::seed_all(&s).await {
                tracing::warn!(error = %e, "entity seeder failed");
            }
        });
    }

    // SP9: async entity resolution worker — every HUB_KG_RESOLVE_ASYNC_TICK_SECS
    // (default 300s) drain entity_resolution_queue, classify pairs by JW score,
    // and call merge_entities for ≥0.95 (or defer to review / reject). Loops
    // forever by design (no cancellation token — task is aborted at process
    // exit); spawn before MCP service assembly so it's running before the
    // first /mcp request lands.
    {
        let pool = state.pg.clone();
        tokio::spawn(async move {
            crate::graph_v2::resolve_async::run_resolve_async_worker(pool).await;
        });
        tracing::info!(target: "hub.boot", "resolve_async worker started");
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
