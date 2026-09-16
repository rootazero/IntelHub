//! RIPE stat `as-overview` per-ASN lookup.
//!
//! Free keyless endpoint at `https://stat.ripe.net/data/as-
//! overview/data.json?resource=AS<num>` returns the
//! authoritative holder name + allocation block for a given
//! Autonomous System Number. Complements the AWS / GCP IP-
//! ranges feeds (which give you the prefix cloud attribution)
//! by surfacing the AS-owner attribution.
//!
//! Use case: AS attribution. When OTX / URLhaus / Shodan tags
//! an IP as hostile, knowing the AS holder name + RIPE
//! allocation block lets you pivot to abuse contacts and
//! ownership chains.
//!
//! Default watchlist = 5 well-known ASes (Cloudflare /
//! Google / DigitalOcean / Amazon / GitHub). Each lookup
//! emits ONE Signal anchored at the RIPE NCC HQ in
//! Amsterdam (52.3676, 4.9041). Honest stand-in — RIPE stat
//! doesn't return AS HQ coordinates.
//!
//! Rate limit: 250ms between queries (RIPE stat is public +
//! shared, not aggressive). 5 queries = ~1.25s sweep.

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const INTER_QUERY_GAP: Duration = Duration::from_millis(250);

/// RIPE NCC HQ anchor (Singel, Amsterdam).
const RIPE_LAT: f64 = 52.3676;
const RIPE_LON: f64 = 4.9041;

const DEFAULT_WATCH: &[&str] = &[
    "13335",  // Cloudflare
    "15169",  // Google
    "14061",  // DigitalOcean
    "16509",  // Amazon AWS
    "36459",  // GitHub
];

#[derive(Clone)]
pub struct RipeAsOverview {
    pub watch: Vec<String>,
}

impl Default for RipeAsOverview {
    fn default() -> Self {
        Self {
            watch: DEFAULT_WATCH.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Deserialize)]
struct RipeAsEnvelope {
    #[serde(rename = "data_call_status")]
    status: String,
    data: Option<RipeAsData>,
}

#[derive(Deserialize)]
struct RipeAsData {
    holder: String,
    block: RipeAsBlock,
    announced: bool,
    #[serde(default)]
    query_starttime: Option<String>,
    #[serde(default)]
    query_endtime: Option<String>,
}

#[derive(Deserialize)]
struct RipeAsBlock {
    resource: String,
    desc: String,
    name: String,
}

impl Source for RipeAsOverview {
    fn name(&self) -> &'static str {
        "ripe_as_overview"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move {
            let mut sigs = Vec::new();
            for (i, asn) in self.watch.iter().enumerate() {
                if i > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query(ctx, asn).await {
                    Ok(Some(sig)) => sigs.push(sig),
                    Ok(None) => {
                        tracing::debug!(asn = %asn, "ripe_as_overview: no result");
                    }
                    Err(e) => {
                        tracing::warn!(
                            asn = %asn,
                            error = %e,
                            "ripe_as_overview: query failed",
                        );
                    }
                }
            }
            Ok(sigs)
        })
    }
}

async fn query(ctx: &Ctx, asn: &str) -> Result<Option<Signal>, HubError> {
    let url = format!(
        "https://stat.ripe.net/data/as-overview/data.json?resource=AS{asn}"
    );
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http.get(&url).send(),
    )
    .await
    .map_err(|_| HubError::sensor("ripe_as_overview: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("ripe_as_overview: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "ripe_as_overview: HTTP {s}: {snip}"
        )));
    }

    let body: RipeAsEnvelope = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("ripe_as_overview: parse: {e}")))?;

    if !body.status.starts_with("supported") {
        return Ok(None);
    }

    let Some(data) = body.data else {
        return Ok(None);
    };

    let payload = json!({
        "asn": asn,
        "holder": data.holder,
        "block_resource": data.block.resource,
        "block_desc": data.block.desc,
        "block_name": data.block.name,
        "announced": data.announced,
        "query_starttime": data.query_starttime,
        "query_endtime": data.query_endtime,
        "looked_up_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("AS{asn} holder: {}", data.holder),
            RIPE_LAT,
            RIPE_LON,
            format!("ripe_as_overview:{asn}"),
        )
        .payload(payload),
    ))
}