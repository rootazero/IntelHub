//! MISP warninglist: RFC 6761 Special-Use Domain Names.
//!
//! Free keyless GitHub raw feed at
//! `https://raw.githubusercontent.com/MISP/misp-warninglists/
//! main/lists/rfc6761/list.json` (~897 bytes, ~25 entries).
//! These are the Special-Use Domain Names defined by IETF
//! (RFC 6761): in-addr.arpa, ip6.arpa, home.arpa, local,
//! onion, test, example, invalid, localhost, and the
//! reverse-DNS subzones for RFC 1918 private address space
//! (10.in-addr.arpa, 16.172.in-addr.arpa, etc.).
//!
//! Sentinel for OSINT false-positive suppression: if OTX /
//! URLhaus / OpenPhish flag a hostname that is in RFC 6761
//! reserved space (e.g. *.local, *.test, *.example,
//! *.onion, *.home.arpa), the indicator is invalid — these
//! names should NEVER appear in external threat intel.
//!
//! Anchor: CIRCL (MISP maintainer) HQ — Luxembourg.
//!
//! Emits ONE Signal per sweep. `external_id` embeds the MISP
//! `version` field so ingest-layer content_hash dedup
//! handles repeat (same pattern as misp_dynamic_dns and
//! misp_rfc5735).

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

impl Source for MispRfc6761 {
    fn name(&self) -> &'static str {
        "misp_rfc6761"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move { fetch(ctx).await.map(|opt| opt.into_iter().collect()) })
    }
}

pub struct MispRfc6761;

async fn fetch(ctx: &Ctx) -> Result<Option<Signal>, HubError> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(
                "https://raw.githubusercontent.com/MISP/misp-warninglists/main/lists/rfc6761/list.json",
            )
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("misp_rfc6761: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("misp_rfc6761: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "misp_rfc6761: HTTP {s}: {snip}"
        )));
    }

    let body: MispList = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("misp_rfc6761: parse: {e}")))?;

    if body.list.is_empty() {
        return Ok(None);
    }

    let version = body.version.unwrap_or(0);
    let payload = json!({
        "version": version,
        "list_count": body.list.len(),
        "description": body.description,
        "name": body.name,
        "domains": body.list,
        "fetched_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("MISP RFC 6761 special-use TLD v{}: {} entries", version, body.list.len()),
            CIRCL_LAT,
            CIRCL_LON,
            format!("misp_rfc6761:v{}", version),
        )
        .payload(payload),
    ))
}