//! AWS public IP-ranges feed.
//!
//! Free keyless endpoint at `https://ip-ranges.amazonaws.com/
//! ip-ranges.json` publishes the full set of AWS-owned IP
//! prefixes (EC2, S3, CloudFront, Route53, etc.) along with
//! their region + service attribution. Updated daily by AWS;
//! the response includes a `syncToken` (Unix epoch seconds) and
//! a `createDate` (YYYY-MM-DD-HH-MM-SS) that change only when
//! prefixes are added or removed.
//!
//! Use case: cloud-IP attribution. When OSINT intel (OTX,
//! URLhaus, PhishTank, etc.) flags an IP as hostile, knowing
//! "this IP is an AWS EC2 instance in eu-west-1" is a key
//! attribution primitive — the threat actor is a customer of
//! AWS, not AWS itself.
//!
//! We emit ONE Signal per sweep. The `external_id` includes
//! the syncToken, so ingest-layer content_hash dedup means a
//! Signal with unchanged syncToken never re-fires; a new
//! syncToken produces a fresh Signal with the new prefix
//! counts, regions, and services. This avoids needing Redis
//! state while still surfacing every meaningful change.
//!
//! Anchor: AWS Seattle HQ (47.6062, -122.3321). Honest
//! stand-in — AWS IP ranges have no per-prefix geo.

use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// AWS Seattle HQ anchor (Doppler / Virginia St HQ).
const AWS_LAT: f64 = 47.6062;
const AWS_LON: f64 = -122.3321;

#[derive(Deserialize)]
struct AWSRange {
    #[serde(rename = "syncToken")]
    sync_token: String,
    #[serde(rename = "createDate")]
    create_date: String,
    prefixes: Vec<AWSPrefix>,
    #[serde(rename = "ipv6_prefixes")]
    ipv6_prefixes: Vec<AWSPrefix>,
}

#[derive(Deserialize)]
struct AWSPrefix {
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    service: Option<String>,
    #[serde(default)]
    network_border_group: Option<String>,
}

impl Source for AWSIpRanges {
    fn name(&self) -> &'static str {
        "aws_ip_ranges"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move { fetch(ctx).await.map(|opt| opt.into_iter().collect()) })
    }
}

pub struct AWSIpRanges;

async fn fetch(ctx: &Ctx) -> Result<Option<Signal>, HubError> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get("https://ip-ranges.amazonaws.com/ip-ranges.json")
            .header("Accept", "application/json")
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("aws_ip_ranges: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("aws_ip_ranges: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "aws_ip_ranges: HTTP {s}: {snip}"
        )));
    }

    let body: AWSRange = resp
        .json()
        .await
        .map_err(|e| HubError::sensor(format!("aws_ip_ranges: parse: {e}")))?;

    let ipv4_count = body.prefixes.len();
    let ipv6_count = body.ipv6_prefixes.len();
    if ipv4_count == 0 && ipv6_count == 0 {
        return Ok(None);
    }

    // Aggregate unique regions and services across both prefix
    // lists. Cheap: O(N) set inserts.
    let mut regions: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut services: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for p in body.prefixes.iter().chain(body.ipv6_prefixes.iter()) {
        if let Some(r) = p.region.as_deref() {
            regions.insert(r.to_string());
        }
        if let Some(s) = p.service.as_deref() {
            services.insert(s.to_string());
        }
    }

    let payload = json!({
        "sync_token": body.sync_token,
        "create_date": body.create_date,
        "ipv4_prefix_count": ipv4_count,
        "ipv6_prefix_count": ipv6_count,
        "regions": regions.into_iter().collect::<Vec<_>>(),
        "services": services.into_iter().collect::<Vec<_>>(),
        "fetched_at": Utc::now().to_rfc3339(),
    });

    // external_id embeds sync_token → ingest-layer content_hash
    // dedup means unchanged sync_token never re-fires; new
    // sync_token produces a fresh Signal.
    Ok(Some(
        Signal::new(
            "cyber",
            format!("AWS IP-ranges snapshot: sync_token={}", body.sync_token),
            AWS_LAT,
            AWS_LON,
            format!("aws_ip_ranges:{}", body.sync_token),
        )
        .payload(payload),
    ))
}