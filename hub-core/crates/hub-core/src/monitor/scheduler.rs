//! Per-source task supervision: independent cadence, error isolation,
//! exponential backoff while failing, first-run stagger, plus the daily
//! geo_events retention purge.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::state::AppState;

use super::{Ctx, Source};

pub async fn run(state: AppState, sources: Vec<Box<dyn Source>>, ct: CancellationToken) {
    let enabled = state.config.monitor_sources.clone();
    let run_all = enabled.iter().any(|e| e == "all");
    let ctx = match Ctx::new(state.config.clone()) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            tracing::error!(error = %e, "monitor: failed to build HTTP client; monitor offline");
            return;
        }
    };

    {
        let s = state.clone();
        let t = ct.child_token();
        tokio::spawn(async move { retention_loop(s, t).await });
    }

    let mut started = 0usize;
    for (idx, src) in sources.into_iter().enumerate() {
        if !run_all && !enabled.iter().any(|e| e == src.name()) {
            tracing::info!(source = src.name(), "monitor source disabled by HUB_MONITOR_SOURCES");
            continue;
        }
        let s = state.clone();
        let t = ct.child_token();
        let c = ctx.clone();
        let stagger = Duration::from_secs((idx as u64 % 10) * 3);
        tokio::spawn(async move { source_loop(s, c, src, t, stagger).await });
        started += 1;
    }
    tracing::info!(sources = started, "monitor scheduler started");
}

async fn source_loop(
    state: AppState,
    ctx: Arc<Ctx>,
    src: Box<dyn Source>,
    ct: CancellationToken,
    stagger: Duration,
) {
    let name = src.name();
    tokio::select! {
        _ = ct.cancelled() => return,
        _ = tokio::time::sleep(stagger) => {}
    }
    let mut failures = 0u32;
    loop {
        let started = Instant::now();
        match src.fetch(&ctx).await {
            Ok(signals) => {
                let fetched = signals.len();
                match super::geo::persist_signals(&state, name, signals).await {
                    Ok(new) => {
                        super::geo::report_health(&state, name, true, "ok", new).await;
                        if failures > 0 {
                            tracing::info!(source = name, "monitor source recovered");
                        }
                        failures = 0;
                        tracing::info!(
                            source = name,
                            fetched,
                            new,
                            ms = started.elapsed().as_millis() as u64,
                            "monitor sweep"
                        );
                    }
                    Err(e) => {
                        failures += 1;
                        super::geo::report_health(&state, name, false, &e.to_string(), 0).await;
                        tracing::warn!(source = name, error = %e, "monitor persist failed");
                    }
                }
            }
            Err(e) => {
                failures += 1;
                super::geo::report_health(&state, name, false, &e.to_string(), 0).await;
                tracing::warn!(source = name, failures, error = %e, "monitor fetch failed");
            }
        }
        // While failing: exponential backoff 30s→60s→…→cap 10min (faster than
        // the normal interval so a flaky upstream recovers quickly; never so
        // hot that we hammer a rate-limited host).
        let wait = if failures > 0 {
            Duration::from_secs((30u64 << failures.min(5)).min(600))
        } else {
            src.interval()
        };
        tokio::select! {
            _ = ct.cancelled() => { tracing::info!(source = name, "monitor source stopping"); return; }
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

/// Daily retention purge (spec §2): geo_events older than the configured
/// window, except the static chokepoint reference layer.
async fn retention_loop(state: AppState, ct: CancellationToken) {
    let mut tick = tokio::time::interval(Duration::from_secs(24 * 3600));
    tick.tick().await; // first tick fires immediately — skip it
    loop {
        tokio::select! {
            _ = ct.cancelled() => return,
            _ = tick.tick() => {
                let days = state.config.monitor_geo_retention_days as i32;
                match sqlx::query(
                    "DELETE FROM geo_events WHERE source <> 'monitor:chokepoint'
                     AND ingested_at < now() - make_interval(days => $1)",
                )
                .bind(days)
                .execute(&state.pg)
                .await
                {
                    Ok(r) if r.rows_affected() > 0 => {
                        tracing::info!(deleted = r.rows_affected(), "geo_events retention purge");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "geo_events retention purge failed"),
                }
            }
        }
    }
}
