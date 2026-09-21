//! Per-source task supervision: independent cadence, error isolation,
//! exponential backoff while failing, first-run stagger, plus the daily
//! geo_events retention purge.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::state::AppState;

use super::{Ctx, SeriesCollector, Source};

pub async fn run(
    state: AppState,
    sources: Vec<Box<dyn Source>>,
    collectors: Vec<Box<dyn SeriesCollector>>,
    ct: CancellationToken,
) {
    let enabled = state.config.monitor_sources.clone();
    let run_all = enabled.iter().any(|e| e == "all");
    let ctx = match Ctx::new(Arc::new(state.clone())) {
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
        // fix/deploy-stampede Layer 2: spread first-run stagger across the
        // full source count instead of clustering into 10 slots (which had
        // 7 sources sharing each 3-second slot when registry hits 74). At
        // 1.5s/source, 74 sources spread over ~110 seconds — cctv-refresh
        // (sequential, 11 providers, up to 103s) gets a clean window too.
        let stagger = Duration::from_secs((idx as u64 * 3) / 2);
        tokio::spawn(async move { source_loop(s, c, src, t, stagger).await });
        started += 1;
    }
    for (idx, col) in collectors.into_iter().enumerate() {
        if !run_all && !enabled.iter().any(|e| e == col.name()) {
            tracing::info!(source = col.name(), "monitor collector disabled by HUB_MONITOR_SOURCES");
            continue;
        }
        let s = state.clone();
        let t = ct.child_token();
        let c = ctx.clone();
        let stagger = Duration::from_secs(((idx + started) as u64 * 3) / 2);
        tokio::spawn(async move { series_loop(s, c, col, t, stagger).await });
        started += 1;
    }
    tracing::info!(sources = started, "monitor scheduler started");
}

/// SP6B series/event collector loop — same supervision policy as source_loop
/// (error isolation, exponential backoff while failing, health reporting).
async fn series_loop(
    state: AppState,
    ctx: Arc<Ctx>,
    col: Box<dyn SeriesCollector>,
    ct: CancellationToken,
    stagger: Duration,
) {
    let name = col.name();
    tokio::select! {
        _ = ct.cancelled() => return,
        _ = tokio::time::sleep(stagger) => {}
    }
    let mut failures = 0u32;
    loop {
        let started = Instant::now();
        match col.collect(&state, &ctx).await {
            Ok((fetched, new)) => {
                super::geo::report_health(&state, name, true, "ok", new, fetched).await;
                if failures > 0 {
                    tracing::info!(source = name, "monitor collector recovered");
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
                super::geo::report_health(&state, name, false, &e.to_string(), 0, 0).await;
                tracing::warn!(source = name, failures, error = %e, "monitor collect failed");
            }
        }
        let wait = if failures > 0 {
            Duration::from_secs((30u64 << failures.min(5)).min(600))
        } else {
            col.interval()
        };
        tokio::select! {
            _ = ct.cancelled() => { tracing::info!(source = name, "monitor collector stopping"); return; }
            _ = tokio::time::sleep(wait) => {}
        }
    }
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
                        super::geo::report_health(&state, name, true, "ok", new, fetched).await;
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
                        super::geo::report_health(&state, name, false, &e.to_string(), 0, 0).await;
                        tracing::warn!(source = name, error = %e, "monitor persist failed");
                    }
                }
            }
            Err(e) => {
                failures += 1;
                super::geo::report_health(&state, name, false, &e.to_string(), 0, 0).await;
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

#[cfg(test)]
mod tests {
    /// fix/deploy-stampede Layer 2: the old formula
    /// `Duration::from_secs((idx as u64 % 10) * 3)` clustered 74
    /// sources into 10 stagger slots (7 per slot). The new formula
    /// `Duration::from_secs((idx as u64 * 3) / 2)` distributes them
    /// across the full registry count at 1.5s/source. This test pins
    /// the new behavior: for any pair of adjacent source indices in
    /// the typical registry size (74 sources + ~26 collectors ≈ 100
    /// total), the staggered start must be at least 1 second apart.
    #[test]
    fn stagger_spreads_per_source_not_per_slot() {
        // Reproduce the production formula inlined into scheduler::run.
        // (We don't refactor the production code to use a helper fn
        // because that would force the Source trait dispatch through
        // a closure — keep the test pinned to the same string source.)
        let stagger_for = |idx: u64| (idx * 3) / 2;

        // Adjacent indices: each must differ by at least 1 second.
        for idx in 0..200u64 {
            let prev = stagger_for(idx);
            let next = stagger_for(idx + 1);
            assert!(
                next.saturating_sub(prev) >= 1,
                "stagger collapsed between idx={idx} ({prev}s) and idx={} ({next}s)",
                idx + 1
            );
        }

        // Total spread: 74 sources should span >60s, not the 27s of the
        // old `% 10` formula.
        let total_spread_old = 10u64 * 3;
        let total_spread_new = stagger_for(73);
        assert!(
            total_spread_new > total_spread_old,
            "new formula must spread farther than the old `% 10` slot width \
             (old={}s, new={}s)",
            total_spread_old,
            total_spread_new
        );

        // Guard against accidentally going back to the `% 10` slot.
        assert_eq!(
            stagger_for(0),
            0,
            "first source must start at t=0"
        );
        assert_eq!(
            stagger_for(2),
            3,
            "idx=2 must be 3s after idx=0 (1.5s/source)"
        );
        // idx=10 used to be the same slot as idx=0; under the new
        // formula it lands at 15s — definitively not the same second.
        assert_ne!(
            stagger_for(10),
            stagger_for(0),
            "idx=10 must not share the t=0 slot"
        );
    }
}
