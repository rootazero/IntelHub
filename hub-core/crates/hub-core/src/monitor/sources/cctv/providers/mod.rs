//! City camera providers (GEV P3 T9): one impl per keyless upstream.
//!
//! The `CityCameraProvider` trait is the seam between the periodic
//! refresh source (`cctv::refresh::CctvRefresh`) and each upstream's
//! fetch+parse. Every provider returns normalized `CameraRow`s (the T8
//! `cctv::mod` write shape) so the refresh source can upsert them with
//! the SAME `UPSERT_SQL` the static loader uses — one table, one write
//! path, N sources.
//!
//! async fn in trait: native `async fn` is NOT `dyn`-object-safe, and
//! this crate follows the zero-new-crates rule (no async_trait dep), so
//! each method is desugared to `Pin<Box<dyn Future + Send + 'a>>`
//! (PlannerLlm precedent, planner_llm.rs — "照抄" the existing style).

mod ontario511;
mod tfl;

use std::future::Future;
use std::pin::Pin;

use crate::error::Result;

use super::CameraRow;

pub trait CityCameraProvider: Send + Sync {
    /// Stable provider id — also the `provider` column value AND the
    /// `cctv-<id>` health-cell suffix. Must NOT start with `static-`
    /// (that prefix is reserved for the T8 static-catalog rows; the
    /// refresh-due probe selects `provider NOT LIKE 'static-%'`).
    fn id(&self) -> &'static str;
    /// Display city for the catalog (contracts.md §3 `city`).
    fn city(&self) -> &'static str;
    /// Fetch + parse the full catalog. `Err` = transport / HTTP / JSON
    /// failure (caller retries with backoff); `Ok(vec![])` = upstream
    /// answered but yielded zero cameras (caller keeps previous rows —
    /// an empty catalog must never be read as "upstream removed all").
    fn fetch_catalog<'a>(
        &'a self,
        client: &'a reqwest::Client,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CameraRow>>> + Send + 'a>>;
}

/// The live keyless providers. NYC 511NY + Singapore LTA are T10 — add
/// them here and both the refresh loop and the health-sampling loop pick
/// them up with zero other changes (scoped sweep + health cell are keyed
/// off `provider.id()`).
pub fn providers() -> Vec<&'static dyn CityCameraProvider> {
    vec![&tfl::Tfl, &ontario511::Ontario511]
}
