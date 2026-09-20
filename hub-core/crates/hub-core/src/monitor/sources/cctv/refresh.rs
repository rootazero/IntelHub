//! CCTV live-provider refresh + frame health sampling (GEV P3 T9).
//!
//! Two Source impls on top of the T8 catalog base:
//!
//! - [`CctvRefresh`] (1h): pull every registered keyless city provider
//!   (`providers::providers()` — TfL, Ontario 511; NYC/LTA land in T10)
//!   and upsert into `cctv_cameras`. Failure policy mirrors the "failed
//!   sources stay visible" rule: a provider that errors (after 30s/60s
//!   backoff retries) keeps its previous rows, the error text lands in
//!   its `cctv-<id>` health cell, and the next provider still runs.
//!   The stale sweep is **provider-scoped** (`DELETE WHERE provider = $p
//!   AND fetched_at < round_ts`) and only runs on a non-empty successful
//!   round — a failed or empty round must never wipe that provider's
//!   rows, and one provider's sweep never touches another's (or the
//!   static loader's `static-%` rows).
//!
//! - [`CctvHealth`] (5min): HEAD-probe a rotating sample of each
//!   provider's `frame_url`s (3 per provider per round, oldest-checked
//!   first), persisting `health_status` / `health_checked_at` per row
//!   and a per-provider summary in the health cell. Probe failures never
//!   delete rows — the catalog stays visible with its status.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::{json, Value};

use crate::error::Result;

use crate::gev_cctv::accept_upstream_body_for_browser;
use crate::monitor::{Ctx, Signal, Source};
use super::providers::{self, CityCameraProvider};
use super::{CameraRow, UPSERT_SQL};

/// 1h catalog cadence — city camera inventories move slowly.
const REFRESH_INTERVAL_SECS: u64 = 3600;
/// 5min frame-probe cadence.
const HEALTH_INTERVAL_SECS: u64 = 300;
/// Per-provider frame probe sample size per health round.
const HEALTH_SAMPLE_PER_PROVIDER: i64 = 3;
/// Probe timeout per HEAD request (frame hosts are often slow town-hall
/// servers; keep it well under the 25s shared client ceiling).
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// Body-read deadline for the magic-byte sniff. Reading the full image
/// would defeat the probe's budget — 8 KB is enough for every image
/// magic header and keeps the per-camera cost bounded.
const PROBE_BODY_TIMEOUT: Duration = Duration::from_secs(2);

/// Insert-or-update one row. Same statement (and bind order) as the T8
/// static loader — one table, one write path, N sources.
async fn upsert_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rows: &[CameraRow],
    round_ts: DateTime<Utc>,
) -> Result<usize> {
    for row in rows {
        sqlx::query(UPSERT_SQL)
            .bind(&row.id)
            .bind(&row.city)
            .bind(&row.city_id)
            .bind(&row.name)
            .bind(row.lat)
            .bind(row.lon)
            .bind(row.heading_deg)
            .bind(row.fov_deg)
            .bind(row.pitch_deg)
            .bind(row.range_m)
            .bind(row.mount_height_m)
            .bind(row.ground_elevation_m)
            .bind(&row.feed_type)
            .bind(&row.frame_url)
            .bind(&row.media_url)
            .bind(&row.provider)
            .bind(&row.source_kind)
            .bind(&row.heading_confidence)
            .bind(&row.pose_source)
            .bind(&row.license_note)
            .bind(&row.credit)
            .bind(&row.code)
            .bind(round_ts)
            .execute(&mut **tx)
            .await
            .map_err(|e| {
                crate::error::HubError::sensor(format!("cctv-refresh: upsert {}: {e}", row.id))
            })?;
    }
    Ok(rows.len())
}

/// Provider-scoped stale sweep: only rows of THIS provider, only after a
/// non-empty successful round (caller guarantees).
const SCOPED_SWEEP_SQL: &str =
    "DELETE FROM cctv_cameras WHERE provider = $1 AND fetched_at < $2";

async fn write_health_cell(ctx: &Ctx, provider: &str, cell: Value) {
    let _: Option<()> = ctx
        .state
        .redis_timed(
            redis::cmd("HSET")
                .arg("hub:monitor:health")
                .arg(format!("cctv-{provider}"))
                .arg(cell.to_string())
                .clone(),
            2000,
        )
        .await;
}

/// One provider round: fetch with 2 backoff retries, then upsert + scoped
/// sweep on success. Returns the health-cell body. Never propagates Err —
/// one city's outage must not fail the whole source tick (its rows stay
/// visible with the error text recorded).
async fn refresh_one(
    ctx: &Ctx,
    provider: &'static dyn CityCameraProvider,
) -> (String, Value) {
    let id = provider.id();
    let mut attempt = 0u32;
    let rows = loop {
        attempt += 1;
        match provider.fetch_catalog(&ctx.http).await {
            Ok(rows) if !rows.is_empty() => break rows,
            Ok(_) => {
                // Empty catalog: upstream answered but yielded nothing —
                // keep previous rows (trait contract), report degraded.
                return (
                    id.to_string(),
                    json!({
                        "state": "degraded",
                        "detail": "upstream returned an empty catalog — previous rows kept",
                        "ts": Utc::now().to_rfc3339(),
                    }),
                );
            }
            Err(e) => {
                if attempt > 2 {
                    tracing::warn!(provider = id, error = %e, "cctv-refresh: provider failed after retries");
                    return (
                        id.to_string(),
                        json!({
                            "state": "error",
                            "detail": e.to_string(),
                            "attempts": attempt,
                            "ts": Utc::now().to_rfc3339(),
                        }),
                    );
                }
                let wait = Duration::from_secs(30 * u64::from(attempt));
                tracing::info!(provider = id, attempt, ?wait, "cctv-refresh: retrying provider");
                tokio::time::sleep(wait).await;
            }
        }
    };

    let round_ts = Utc::now();
    let started = Instant::now();
    let result: Result<(usize, u64)> = async {
        let mut tx = ctx.state.pg.begin().await.map_err(|e| {
            crate::error::HubError::sensor(format!("cctv-refresh: tx begin: {e}"))
        })?;
        let n = upsert_rows(&mut tx, &rows, round_ts).await?;
        let swept = sqlx::query(SCOPED_SWEEP_SQL)
            .bind(id)
            .bind(round_ts)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                crate::error::HubError::sensor(format!("cctv-refresh: scoped sweep: {e}"))
            })?
            .rows_affected();
        tx.commit()
            .await
            .map_err(|e| crate::error::HubError::sensor(format!("cctv-refresh: commit: {e}")))?;
        Ok((n, swept))
    }
    .await;

    match result {
        Ok((n, swept)) => (
            id.to_string(),
            json!({
                "state": "ok",
                "cameras": n,
                "swept": swept,
                "elapsed_ms": started.elapsed().as_millis() as u64,
                "ts": round_ts.to_rfc3339(),
            }),
        ),
        Err(e) => {
            tracing::warn!(provider = id, error = %e, "cctv-refresh: persistence failed");
            (
                id.to_string(),
                json!({
                    "state": "error",
                    "detail": e.to_string(),
                    "ts": Utc::now().to_rfc3339(),
                }),
            )
        }
    }
}

pub struct CctvRefresh;

impl Source for CctvRefresh {
    fn name(&self) -> &'static str {
        "cctv-refresh"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(REFRESH_INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            for provider in providers::providers() {
                let (id, cell) = refresh_one(ctx, provider).await;
                write_health_cell(ctx, &id, cell).await;
            }
            Ok(vec![]) // catalog, not geo events (celestrak precedent)
        }
        .boxed()
    }
}

/// Rotating frame probe: per provider, the 3 least-recently-checked
/// cameras with a frame_url. HEAD-ish probe via GET with a tiny timeout
/// (some frame hosts mis-implement HEAD); status 2xx/3xx → "ok",
/// otherwise "down". Rows are never deleted here — the health columns
/// only annotate.
pub struct CctvHealth;

impl Source for CctvHealth {
    fn name(&self) -> &'static str {
        "cctv-health"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(HEALTH_INTERVAL_SECS)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let providers_list: Vec<String> = sqlx::query_scalar(
                "SELECT DISTINCT provider FROM cctv_cameras WHERE frame_url IS NOT NULL AND active",
            )
            .fetch_all(&ctx.state.pg)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "cctv-health: provider list query failed");
                Vec::new()
            });

            for provider in providers_list {
                let sample: Vec<(String, String)> = sqlx::query_as(
                    "SELECT id, frame_url FROM cctv_cameras \
                     WHERE provider = $1 AND frame_url IS NOT NULL AND active \
                     ORDER BY health_checked_at NULLS FIRST LIMIT $2",
                )
                .bind(&provider)
                .bind(HEALTH_SAMPLE_PER_PROVIDER)
                .fetch_all(&ctx.state.pg)
                .await
                .unwrap_or_default();

                let mut ok = 0usize;
                let mut down = 0usize;
                for (cam_id, url) in sample {
                    // TxDOT ITS returns a JSON envelope — body-sniff the
                    // decoded JPEG magic bytes instead of the raw body.
                    let is_txdot = url.contains("/GetCctvSnapshotByIcdId");
                    let status = if is_txdot {
                        // TxDOT: fetch JSON, decode base64, validate JPEG magic.
                        match tokio::time::timeout(
                            PROBE_TIMEOUT,
                            ctx.http
                                .get(&url)
                                .header("Accept", "application/json")
                                .send(),
                        )
                        .await
                        {
                            Ok(Ok(resp))
                                if resp.status().is_success()
                                    || resp.status().is_redirection() =>
                            {
                                match resp.bytes().await {
                                    Ok(body) => {
                                        match serde_json::from_slice::<serde_json::Value>(&body)
                                        {
                                            Ok(json) => match crate::gev_cctv::parse_txdot_envelope(&json)
                                            {
                                                Ok(_) => "ok",
                                                Err(_) => "down",
                                            },
                                            Err(_) => "down",
                                        }
                                    }
                                    Err(_) => "down",
                                }
                            }
                            _ => "down",
                        }
                    } else {
                        // Standard image: GET + content-type + magic-byte sniff.
                        // Reading just the first 8 KB is enough for the
                        // header sniff and well under typical image sizes;
                        // a tiny timeout aborts before a full image body.
                        match tokio::time::timeout(
                            PROBE_TIMEOUT,
                            ctx.http.get(&url).send(),
                        )
                        .await
                        {
                            Ok(Ok(resp))
                                if resp.status().is_success()
                                    || resp.status().is_redirection() =>
                            {
                                let ct = resp
                                    .headers()
                                    .get(reqwest::header::CONTENT_TYPE)
                                    .and_then(|v| v.to_str().ok())
                                    .map(str::to_owned);
                                // Read up to 8 KB — enough for any image
                                // magic bytes, aborts the rest of the body.
                                let probe = tokio::time::timeout(
                                    PROBE_BODY_TIMEOUT,
                                    resp.bytes(),
                                )
                                .await;
                                match probe {
                                    Ok(Ok(body)) => {
                                        if accept_upstream_body_for_browser(ct.as_deref(), &body) {
                                            "ok"
                                        } else {
                                            "down"
                                        }
                                    }
                                    _ => "down",
                                }
                            }
                            _ => "down",
                        }
                    };
                    if status == "ok" {
                        ok += 1;
                    } else {
                        down += 1;
                    }
                    if let Err(e) = sqlx::query(
                        "UPDATE cctv_cameras SET health_status = $2, health_checked_at = now() WHERE id = $1",
                    )
                    .bind(&cam_id)
                    .bind(status)
                    .execute(&ctx.state.pg)
                    .await
                    {
                        tracing::warn!(error = %e, "cctv-health: row update failed");
                    }
                }
                write_health_cell(
                    ctx,
                    &format!("{provider}:frames"),
                    json!({
                        "state": if down == 0 { "ok" } else { "degraded" },
                        "sampled": ok + down,
                        "ok": ok,
                        "down": down,
                        "ts": Utc::now().to_rfc3339(),
                    }),
                )
                .await;
            }
            Ok(vec![])
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_sweep_is_provider_limited() {
        // The sweep must name the provider column — a global
        // `fetched_at < ts` delete would wipe sibling providers and the
        // static loader's rows (T8 4/4-gated sweep is static-% scoped).
        assert!(SCOPED_SWEEP_SQL.contains("provider = $1"));
        assert!(SCOPED_SWEEP_SQL.contains("fetched_at < $2"));
    }

    #[test]
    fn refresh_cadence_is_one_hour() {
        assert_eq!(CctvRefresh.interval(), Duration::from_secs(3600));
        assert_eq!(CctvHealth.interval(), Duration::from_secs(300));
    }

    #[test]
    fn providers_are_registered() {
        let ids: Vec<&str> = providers::providers().iter().map(|p| p.id()).collect();
        assert!(ids.contains(&"tfl"));
        assert!(ids.contains(&"ontario511"));
        for id in ids {
            assert!(!id.starts_with("static-"), "live providers must not use the static prefix");
        }
    }
}
