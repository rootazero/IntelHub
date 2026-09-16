//! MISP warninglist: RFC 5735 Special-Use IPv4 Addresses.
//!
//! Free keyless GitHub raw feed at
//! `https://raw.githubusercontent.com/MISP/misp-warninglists/
//! main/lists/rfc5735/list.json` (~644 bytes, ~15 CIDR
//! entries). These are the Special-Use IPv4 address blocks
//! defined by IETF (RFC 5735): 0.0.0.0/8, 10.0.0.0/8
//! (private), 127.0.0.0/8 (loopback), 169.254.0.0/16
//! (link-local), 172.16.0.0/12 (private), 192.0.0.0/24,
//! 192.0.2.0/24 (TEST-NET-1), 192.88.99.0/24, 192.168.0.0/16
//! (private), 198.18.0.0/15 (benchmarking), 198.51.100.0/24
//! (TEST-NET-2), 203.0.113.0/24 (TEST-NET-3), 224.0.0.0/4
//! (multicast), 240.0.0.0/4 (reserved), 255.255.255.255/32.
//!
//! Sentinel for OSINT false-positive suppression: if OTX /
//! URLhaus / Shodan flag an IP that is in RFC 5735 reserved
//! space, the indicator is invalid — these addresses
//! should NEVER appear in external threat intel.
//!
//! Anchor: CIRCL (MISP maintainer) HQ — Luxembourg.
//!
//! Emits ONE Signal per sweep. `external_id` embeds the MISP
//! `version` field so ingest-layer content_hash dedup
//! handles repeat (same pattern as misp_dynamic_dns).

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

const CIRCL_LAT: f64 = 49.6116;
const CIRCL_LON: f64 = 6.1319;

#[derive(Deserialize)]
struct MispList {
    #[serde(default)]
    description: Option<String>,
    /// INTEGER (e.g. 20240118), not string.
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    name: Option<String>,
    list: Vec<String>,
}

impl Source for MispRfc5735 {
    fn name(&self) -> &'static str {
        "misp_rfc5735"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move { fetch(ctx).await.map(|opt| opt.into_iter().collect()) })
    }
}

pub struct MispRfc5735;

async fn fetch(ctx: &Ctx) -> Result<Option<Signal>, HubError> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(
                "https://raw.githubusercontent.com/MISP/misp-warninglists/main/lists/rfc5735/list.json",
            )
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("misp_rfc5735: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("misp_rfc5735: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "misp_rfc5735: HTTP {s}: {snip}"
        )));
    }

    let body: MispList = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("misp_rfc5735: parse: {e}")))?;

    if body.list.is_empty() {
        return Ok(None);
    }

    let version = body.version.unwrap_or(0);
    let payload = json!({
        "version": version,
        "list_count": body.list.len(),
        "description": body.description,
        "name": body.name,
        "cidrs": body.list,
        "fetched_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("MISP RFC 5735 special-use IPv4 v{}: {} CIDRs", version, body.list.len()),
            CIRCL_LAT,
            CIRCL_LON,
            format!("misp_rfc5735:v{}", version),
        )
        .payload(payload),
    ))
}