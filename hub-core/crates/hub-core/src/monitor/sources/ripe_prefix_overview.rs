//! RIPE stat `prefix-overview` per-prefix BGP lookup.
//!
//! Free keyless endpoint at `https://stat.ripe.net/data/prefix-
//! overview/data.json?resource=<ip>` (or `<prefix>`). Returns
//! the BGP route that contains the IP / prefix: which AS
//! originates it, the AS path, RPKI validation status, the
//! prefix itself, the resource holder, and the block it's
//! allocated in.
//!
//! Use case: per-prefix BGP attribution. When OTX / URLhaus /
//! Shodan flags an IP, knowing "this IP is in 1.1.1.0/24
//! originated by AS13335 Cloudflare, RPKI valid" confirms the
//! AS-owner chain. Different from `ripe_as_overview` (which
//! returns the AS-level metadata) — prefix-overview gives the
//! actual prefix + routing state.
//!
//! Default watchlist = 5 well-known IPs (Cloudflare / Google /
//! Quad9 / OpenDNS / GitHub). Anchor at RIPE NCC HQ Amsterdam
//! (honest stand-in — RIPE stat doesn't return per-prefix geo).

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const INTER_QUERY_GAP: Duration = Duration::from_millis(250);

/// RIPE NCC HQ anchor (Singel, Amsterdam).
const RIPE_LAT: f64 = 52.3676;
const RIPE_LON: f64 = 4.9041;

const DEFAULT_WATCH: &[&str] = &[
    "1.1.1.1",            // Cloudflare DNS
    "8.8.8.8",            // Google DNS
    "9.9.9.9",            // Quad9 DNS
    "208.67.222.222",     // OpenDNS
    "140.82.121.4",       // GitHub
];

#[derive(Clone)]
pub struct RipePrefixOverview {
    pub watch: Vec<String>,
}

impl Default for RipePrefixOverview {
    fn default() -> Self {
        Self {
            watch: DEFAULT_WATCH.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Deserialize)]
struct RipePrefixEnvelope {
    #[serde(rename = "data_call_status")]
    status: String,
    data: Option<RipePrefixData>,
    #[serde(default)]
    messages: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct RipePrefixData {
    /// The actual prefix that contains the queried IP
    /// (e.g. "8.8.8.0/24"). This is what callers want.
    resource: String,
    #[serde(default)]
    announced: Option<bool>,
    #[serde(default)]
    is_less_specific: Option<bool>,
    #[serde(default)]
    block: Option<RipePrefixBlock>,
    #[serde(default)]
    asns: Vec<RipePrefixAsn>,
    #[serde(default)]
    related_prefixes: Vec<String>,
    #[serde(default)]
    actual_num_related: Option<u32>,
    #[serde(default)]
    query_time: Option<String>,
}

#[derive(Deserialize)]
struct RipePrefixAsn {
    asn: u64,
    holder: String,
}

#[derive(Deserialize)]
struct RipePrefixBlock {
    resource: String,
    desc: String,
    name: String,
}

impl Source for RipePrefixOverview {
    fn name(&self) -> &'static str {
        "ripe_prefix_overview"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move {
            let mut sigs = Vec::new();
            for (i, ip) in self.watch.iter().enumerate() {
                if i > 0 {
                    tokio::time::sleep(INTER_QUERY_GAP).await;
                }
                match query(ctx, ip).await {
                    Ok(Some(sig)) => sigs.push(sig),
                    Ok(None) => {
                        tracing::debug!(ip = %ip, "ripe_prefix_overview: no result");
                    }
                    Err(e) => {
                        tracing::warn!(
                            ip = %ip,
                            error = %e,
                            "ripe_prefix_overview: query failed",
                        );
                    }
                }
            }
            Ok(sigs)
        })
    }
}

async fn query(ctx: &Ctx, ip: &str) -> Result<Option<Signal>, HubError> {
    let url = format!(
        "https://stat.ripe.net/data/prefix-overview/data.json?resource={ip}"
    );
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http.get(&url).send(),
    )
    .await
    .map_err(|_| HubError::sensor("ripe_prefix_overview: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("ripe_prefix_overview: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "ripe_prefix_overview: HTTP {s}: {snip}"
        )));
    }

    let body: RipePrefixEnvelope = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("ripe_prefix_overview: parse: {e}")))?;

    if !body.status.starts_with("supported") {
        return Ok(None);
    }

    let Some(data) = body.data else {
        return Ok(None);
    };

    // Pull first AS holder (most responses have 1 AS, occasionally
    // more for anycast prefixes). If no ASNs at all, still emit
    // — the prefix + block info is valuable on its own.
    let primary_holder = data.asns.first().map(|a| a.holder.clone());
    let primary_asn = data.asns.first().map(|a| a.asn);

    let payload = json!({
        "queried_ip": ip,
        "prefix": data.resource,
        "primary_asn": primary_asn,
        "primary_holder": primary_holder,
        "asns": data.asns.iter().map(|a| json!({"asn": a.asn, "holder": a.holder})).collect::<Vec<_>>(),
        "announced": data.announced,
        "is_less_specific": data.is_less_specific,
        "block_resource": data.block.as_ref().map(|b| &b.resource),
        "block_desc": data.block.as_ref().map(|b| &b.desc),
        "block_name": data.block.as_ref().map(|b| &b.name),
        "related_prefixes": data.related_prefixes,
        "actual_num_related": data.actual_num_related,
        "query_time": data.query_time,
        "looked_up_at": Utc::now().to_rfc3339(),
    });

    let external_id = format!("ripe_prefix_overview:{}", data.resource);

    let title = match (&primary_holder, primary_asn) {
        (Some(h), Some(a)) => format!(
            "RIPE prefix: {} → {}/AS{}",
            ip, h, a
        ),
        _ => format!("RIPE prefix: {} → {}", ip, data.resource),
    };

    Ok(Some(
        Signal::new(
            "cyber",
            title,
            RIPE_LAT,
            RIPE_LON,
            external_id,
        )
        .payload(payload),
    ))
}