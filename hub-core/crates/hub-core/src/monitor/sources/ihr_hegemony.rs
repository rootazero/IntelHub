//! IHR (Internet Health Report) Hegemony API — IIJ Lab Tokyo.
//!
//! Free keyless Django REST API. For a given origin AS,
//! returns the dependent ASes that route traffic through it,
//! weighted by a BGP-update-derived "hegemony score" (0-1).
//! https://ihr.iijlab.net/ihr/api/hegemony/
//!
//! Use case: tracks INTERNET TOPOLOGY CHANGES. When a major
//! transit provider (Cogent, NTT, GTT) suddenly loses reach
//! to 30% of the internet due to a peering dispute, IHR's
//! hegemony scores shift within minutes. Useful for OSINT
//! monitoring of infrastructure-level incidents.
//!
//! Default watchlist = 2 well-known CDNs with 100+ dependents:
//!   13335 (Cloudflare, 172 dependents)
//!   20940 (Akamai,    135 dependents)
//! Override via env `HUB_IHR_HEGEMONY_ASNS=13335,20940,<more>`
//!
//! Per AS, emit TOP-20 dependents by hege score (40 Signals
//! per sweep max). Anchor: IIJ Lab HQ Tokyo.
//! Cadence: 24h (hegemony scores shift slowly; Cloudflare
//! CDN caches the response for 4h anyway).
//! Severity: priority if hege > 0.10 (significant reach),
//! info otherwise.
//!
//! NOTE: omit `af=4` filter — IHR hegemony data is IPv6-
//! dominant (count drops to 0 with af=4). Default mixed v4+v6
//! gives the full result set.

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const INTER_QUERY_GAP: Duration = Duration::from_millis(300);

const IHR_URL: &str = "https://ihr.iijlab.net/ihr/api/hegemony/";

/// IIJ Lab HQ anchor (Tokyo, Japan).
const IHR_LAT: f64 = 35.6814;
const IHR_LON: f64 = 139.7670;

const TOP_PER_ASN: usize = 20;

/// Default watchlist: 2 CDNs with 100+ transitive dependents.
pub const DEFAULT_WATCH: &[&str] = &[
    "13335", // Cloudflare (172 dependents)
    "20940", // Akamai    (135 dependents)
];

#[derive(Clone)]
pub struct IhrHegemony {
    pub watch: Vec<String>,
}

impl Default for IhrHegemony {
    fn default() -> Self {
        Self {
            watch: DEFAULT_WATCH.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Deserialize)]
struct IhrResponse {
    #[allow(dead_code)]
    count: u32,
    results: Vec<IhrRow>,
}

#[derive(Deserialize)]
struct IhrRow {
    #[allow(dead_code)]
    timebin: String,
    originasn: u32,
    asn: u32,
    hege: f64,
    #[serde(default)]
    af: Option<u32>,
    asn_name: String,
    #[serde(default)]
    originasn_name: Option<String>,
}

impl Source for IhrHegemony {
    fn name(&self) -> &'static str {
        "ihr_hegemony"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(
        &'a self,
        ctx: &'a Ctx,
    ) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move { fetch(ctx, &self.watch).await })
    }
}

async fn fetch(ctx: &Ctx, watch: &[String]) -> Result<Vec<Signal>, HubError> {
    // Honor user-configured watchlist; fall back to defaults if empty.
    let watch: Vec<String> = if watch.is_empty() {
        DEFAULT_WATCH.iter().map(|s| s.to_string()).collect()
    } else {
        watch.to_vec()
    };

    let mut sigs = Vec::new();
    for (i, origin_asn_str) in watch.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(INTER_QUERY_GAP).await;
        }
        match query(ctx, origin_asn_str).await {
            Ok(Some(sig)) => sigs.push(sig),
            Ok(None) => {
                tracing::debug!(
                    origin_asn = %origin_asn_str,
                    "ihr_hegemony: no result (empty dependents)",
                );
            }
            Err(e) => {
                tracing::warn!(
                    origin_asn = %origin_asn_str,
                    error = %e,
                    "ihr_hegemony: query failed",
                );
            }
        }
    }
    Ok(sigs)
}

async fn query(ctx: &Ctx, origin_asn: &str) -> Result<Option<Signal>, HubError> {
    let url = format!(
        "{}?originasn={}&ordering=-hege",
        IHR_URL, origin_asn
    );
    let resp = tokio::time::timeout(REQUEST_TIMEOUT, ctx.http.get(&url).send())
        .await
        .map_err(|_| HubError::sensor(format!("ihr_hegemony: {origin_asn} request timed out")))?
        .map_err(|e| HubError::sensor(format!("ihr_hegemony: {origin_asn}: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "ihr_hegemony: {origin_asn} HTTP {s}: {snip}"
        )));
    }

    let body: IhrResponse = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("ihr_hegemony: {origin_asn} parse: {e}")))?;

    if body.results.is_empty() {
        return Ok(None);
    }

    let now = Utc::now();
    let dependents = body.results.into_iter().take(TOP_PER_ASN).collect::<Vec<_>>();
    let origin_name = dependents
        .first()
        .and_then(|r| r.originasn_name.clone())
        .unwrap_or_else(|| format!("AS{}", origin_asn));

    // Average hege across top-N (metadata)
    let top_heges: Vec<f64> = dependents.iter().map(|r| r.hege).collect();
    let max_hege = top_heges.iter().cloned().fold(0.0_f64, f64::max);
    let avg_hege = if top_heges.is_empty() {
        0.0
    } else {
        top_heges.iter().sum::<f64>() / top_heges.len() as f64
    };

    // Pick the top-1 dependent as the "headline" signal. Emit ONE Signal
    // per origin ASN — keeps radar clean. Per-dependent Signals would
    // multiply into 40 Signals per sweep.
    let top = dependents
        .first()
        .expect("results non-empty checked above");

    let payload = json!({
        "origin_asn": top.originasn,
        "origin_asn_name": origin_name,
        "top_dependent_asn": top.asn,
        "top_dependent_name": top.asn_name,
        "top_dependent_hege": top.hege,
        "max_hege": max_hege,
        "avg_hege_top20": avg_hege,
        "dependent_count": top_heges.len(),
        "af": top.af,
        "timebin": top.timebin,
        "fetched_at": now.to_rfc3339(),
    });

    let severity = if max_hege > 0.10 { "priority" } else { "info" };

    Ok(Some(
        Signal::new(
            "cyber",
            format!(
                "IHR hegemony AS{} ({}): {} dependents, max hege {:.4}",
                top.originasn,
                origin_name.split(" - ").next().unwrap_or(&origin_name),
                top_heges.len(),
                max_hege,
            ),
            IHR_LAT,
            IHR_LON,
            format!("ihr_hegemony:AS{}", top.originasn),
        )
        .severity(severity)
        .payload(payload),
    ))
}