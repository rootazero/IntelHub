//! MISP warninglist: dynamic DNS providers.
//!
//! Free keyless GitHub raw feed at
//! `https://raw.githubusercontent.com/MISP/misp-warninglists/
//! main/lists/dynamic-dns/list.json` (~1 MB, ~45,543 domain
//! suffixes maintained by the MISP project). These are domains
//! served by dynamic-DNS providers (DuckDNS, No-IP, freenom,
//! afraid.org, etc.) — domain names that appear in threat
//! intel should be weighted DOWN because attackers commonly
//! rotate through these providers. Crossing a dynamic-dns
//! domain against OTX / URLhaus / OpenPhish feeds is a
//! standard incident-response triage step.
//!
//! Emits ONE Signal per sweep. `external_id` embeds the
//! MISP `version` field, so the ingest-layer content_hash
//! dedup means an unchanged version never re-fires; a new
//! version produces a fresh Signal with the updated list
//! count + timestamp. Same pattern as aws_ip_ranges /
//! gcp_ip_ranges.
//!
//! Anchor: CIRCL (Computer Incident Response Center
//! Luxembourg) HQ — they maintain MISP. 49.6116, 6.1319.

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

/// CIRCL (MISP maintainer) HQ anchor — Luxembourg.
const CIRCL_LAT: f64 = 49.6116;
const CIRCL_LON: f64 = 6.1319;

#[derive(Deserialize)]
struct MispList {
    #[serde(default)]
    description: Option<String>,
    /// MISP version field — INTEGER in the JSON (e.g. 20260908),
    /// not a string. Using `u32` so we don't silently fail
    /// deserialization when the maintainer changes the type.
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    list_type: Option<String>,
    #[serde(default)]
    matching_attributes: Vec<String>,
    list: Vec<String>,
}

impl Source for MispDynamicDns {
    fn name(&self) -> &'static str {
        "misp_dynamic_dns"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move { fetch(ctx).await.map(|opt| opt.into_iter().collect()) })
    }
}

pub struct MispDynamicDns;

async fn fetch(ctx: &Ctx) -> Result<Option<Signal>, HubError> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(
                "https://raw.githubusercontent.com/MISP/misp-warninglists/main/lists/dynamic-dns/list.json",
            )
            .header("Accept", "application/json")
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("misp_dynamic_dns: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("misp_dynamic_dns: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "misp_dynamic_dns: HTTP {s}: {snip}"
        )));
    }

    let body: MispList = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("misp_dynamic_dns: parse: {e}")))?;

    if body.list.is_empty() {
        return Ok(None);
    }

    let version = body.version.unwrap_or(0);

    // Sample first 10 entries for payload (full list would be
    // ~1 MB and bloat the Signal). The version + count is the
    // change-detection mechanism.
    let sample: Vec<String> = body.list.iter().take(10).cloned().collect();

    let payload = json!({
        "version": version,
        "list_count": body.list.len(),
        "description": body.description,
        "name": body.name,
        "list_type": body.list_type,
        "matching_attributes": body.matching_attributes,
        "sample_entries": sample,
        "fetched_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("MISP dynamic-dns list v{}: {} entries", version, body.list.len()),
            CIRCL_LAT,
            CIRCL_LON,
            format!("misp_dynamic_dns:v{}", version),
        )
        .payload(payload),
    ))
}