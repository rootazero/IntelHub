//! FireHOL Level 1 — curated IP blocklist.
//!
//! Free keyless endpoint at `https://iplists.firehol.org/
//! files/firehol_level1.netset` (~75 KB, ~4,718 CIDR
//! entries). FireHOL Level 1 is a curated list maintained by
//! the FireHOL project — it aggregates ~30 other blocklists
//! (Spamhaus DROP, Abuse.ch, DShield, etc.) and removes
//! false positives. It's the "high-quality, low-noise"
//! option compared to Spamhaus DROP or blocklist.de.
//!
//! Anchor: FireHOL maintainer (location unknown publicly) —
//! use Zurich, Switzerland as an honest stand-in.
//!
//! Parse strategy: split by `\n`, skip comments (lines
//! starting with `#` or empty), trim whitespace, count
//! CIDR/IP entries. Emit ONE Signal per sweep with the
//! current entry count embedded in `external_id` so
//! content_hash dedup means an unchanged count never
//! re-fires (same pattern as URLhaus / aws_ip_ranges /
//! gcp_ip_ranges).

use std::time::Duration;

use chrono::Utc;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Zurich CH — honest stand-in for FireHOL maintainer HQ.
const FIREHOL_LAT: f64 = 47.3769;
const FIREHOL_LON: f64 = 8.5417;

impl Source for FireholLevel1 {
    fn name(&self) -> &'static str {
        "firehol_level1"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move { fetch(ctx).await.map(|opt| opt.into_iter().collect()) })
    }
}

pub struct FireholLevel1;

async fn fetch(ctx: &Ctx) -> Result<Option<Signal>, HubError> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get("https://iplists.firehol.org/files/firehol_level1.netset")
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("firehol_level1: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("firehol_level1: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "firehol_level1: HTTP {s}: {snip}"
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| HubError::sensor(format!("firehol_level1: body: {e}")))?;

    let mut entry_count: u32 = 0;
    let mut sample: Vec<String> = Vec::with_capacity(10);

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        entry_count += 1;
        if sample.len() < 10 {
            sample.push(line.to_string());
        }
    }

    if entry_count == 0 {
        return Ok(None);
    }

    let payload = json!({
        "entry_count": entry_count,
        "sample_entries": sample,
        "fetched_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("FireHOL Level 1: {} CIDR entries", entry_count),
            FIREHOL_LAT,
            FIREHOL_LON,
            format!("firehol_level1:{}", entry_count),
        )
        .payload(payload),
    ))
}