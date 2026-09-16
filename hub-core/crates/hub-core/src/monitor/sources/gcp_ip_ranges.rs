//! GCP public IP-ranges feed.
//!
//! Free keyless endpoint at `https://www.gstatic.com/ipranges/
//! cloud.json` publishes the full set of Google Cloud
//! (GCP) IPv4 prefixes along with their scope (region) and
//! service attribution. Smaller than the AWS feed (~1100
//! prefixes vs AWS's ~3500) because GCP's network is more
//! consolidated.
//!
//! Use case: cloud-IP attribution — same pattern as
//! aws_ip_ranges.rs. A hostile IP attributed to a GCP
//! prefix lets you pivot to GCP's abuse contact and confirms
//! the threat actor is a GCP customer.
//!
//! One Signal per sweep. external_id embeds the sync_token,
//! so ingest-layer dedup means unchanged sync_token never
//! re-fires; a new sync_token produces a fresh Signal.
//!
//! Anchor: Google Mountain View HQ (37.4220, -122.0841).
//! Honest stand-in — GCP IP ranges have no per-prefix geo.

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Google Mountain View HQ anchor (Amphitheatre Parkway).
const GCP_LAT: f64 = 37.4220;
const GCP_LON: f64 = -122.0841;

#[derive(Deserialize)]
struct GCPRange {
    #[serde(rename = "syncToken")]
    sync_token: String,
    #[serde(rename = "creationTime")]
    creation_time: String,
    prefixes: Vec<GCPPrefix>,
}

#[derive(Deserialize)]
struct GCPPrefix {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    service: Option<String>,
}

impl Source for GCPIPRanges {
    fn name(&self) -> &'static str {
        "gcp_ip_ranges"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move { fetch(ctx).await.map(|opt| opt.into_iter().collect()) })
    }
}

pub struct GCPIPRanges;

async fn fetch(ctx: &Ctx) -> Result<Option<Signal>, HubError> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get("https://www.gstatic.com/ipranges/cloud.json")
            .header("Accept", "application/json")
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("gcp_ip_ranges: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("gcp_ip_ranges: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "gcp_ip_ranges: HTTP {s}: {snip}"
        )));
    }

    let body: GCPRange = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("gcp_ip_ranges: parse: {e}")))?;

    if body.prefixes.is_empty() {
        return Ok(None);
    }

    let mut scopes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut services: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for p in &body.prefixes {
        if let Some(s) = p.scope.as_deref() {
            scopes.insert(s.to_string());
        }
        if let Some(s) = p.service.as_deref() {
            services.insert(s.to_string());
        }
    }

    let payload = json!({
        "sync_token": body.sync_token,
        "creation_time": body.creation_time,
        "ipv4_prefix_count": body.prefixes.len(),
        "scopes": scopes.into_iter().collect::<Vec<_>>(),
        "services": services.into_iter().collect::<Vec<_>>(),
        "fetched_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("GCP IP-ranges snapshot: sync_token={}", body.sync_token),
            GCP_LAT,
            GCP_LON,
            format!("gcp_ip_ranges:{}", body.sync_token),
        )
        .payload(payload),
    ))
}