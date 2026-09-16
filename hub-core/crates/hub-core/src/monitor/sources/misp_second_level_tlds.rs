//! MISP warninglist: Second Level TLDs as known by Mozilla Foundation.
//!
//! Free keyless GitHub raw feed at
//! `https://raw.githubusercontent.com/MISP/misp-warninglists/
//! main/lists/second-level-tlds/list.json` (~218 KB, ~10,315
//! entries). These are real public-suffix 2nd-level domains
//! maintained by Mozilla (the PSL is the canonical list of
//! where cookies can be set, where domain registrars can
//! delegate, etc.).
//!
//! Sentinel for OSINT false-positive suppression: if OTX /
//! URLhaus / OpenPhish flag a hostname that is itself a 2nd-
//! level TLD (e.g. `0.bg`, `001.test.code-builder-stg.platform
//! .salesforce.com`, `0am.jp`), the indicator may be invalid —
//! these names are not attackable in the typical sense
//! (attackers usually target sub-domains under them).
//!
//! No overlap with MISP rfc6761 (RFC reserved TLDs): this
//! list is real public suffixes (e.g. `co.uk`, `com.au`)
//! while rfc6761 is special-use reserved (e.g. `local`,
//! `onion`, `test`).
//!
//! Anchor: CIRCL (MISP maintainer) HQ — Luxembourg.
//!
//! Emits ONE Signal per sweep. `external_id` embeds the MISP
//! `version` field so ingest-layer content_hash dedup
//! handles repeat (same pattern as misp_dynamic_dns /
//! misp_rfc5735 / misp_rfc6761).

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
    /// INTEGER (e.g. 20260908), not string.
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    name: Option<String>,
    list: Vec<String>,
}

impl Source for MispSecondLevelTlds {
    fn name(&self) -> &'static str {
        "misp_second_level_tlds"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(
        &'a self,
        ctx: &'a Ctx,
    ) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move { fetch(ctx).await.map(|opt| opt.into_iter().collect()) })
    }
}

pub struct MispSecondLevelTlds;

async fn fetch(ctx: &Ctx) -> Result<Option<Signal>, HubError> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get(
                "https://raw.githubusercontent.com/MISP/misp-warninglists/main/lists/second-level-tlds/list.json",
            )
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("misp_second_level_tlds: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("misp_second_level_tlds: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "misp_second_level_tlds: HTTP {s}: {snip}"
        )));
    }

    let body: MispList = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("misp_second_level_tlds: parse: {e}")))?;

    if body.list.is_empty() {
        return Ok(None);
    }

    let version = body.version.unwrap_or(0);
    let payload = json!({
        "version": version,
        "list_count": body.list.len(),
        "description": body.description,
        "name": body.name,
        "domains_sample": body.list.iter().take(10).collect::<Vec<_>>(),
        "fetched_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!(
                "MISP 2nd-level TLDs v{}: {} entries (Mozilla PSL)",
                version,
                body.list.len()
            ),
            CIRCL_LAT,
            CIRCL_LON,
            format!("misp_second_level_tlds:v{}", version),
        )
        .payload(payload),
    ))
}