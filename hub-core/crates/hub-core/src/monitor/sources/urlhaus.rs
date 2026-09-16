//! URLhaus — abuse.ch plain-text malware-distribution URL list.
//!
//! Free keyless endpoint at `https://urlhaus.abuse.ch/downloads/
//! text/` (~2.8 MB, ~50,000-100,000 active URLs that are
//! currently serving malware or being used for phishing). The
//! feed updates continuously — each entry is a single URL
//! per line, with comments prefixed by `#`.
//!
//! This complements openphish (phishing URLs) and crtsh
//! (recently-issued TLS certs for lookalike domains) by
//! covering the live malware-distribution URL space.
//!
//! Emits ONE Signal per sweep. `external_id` embeds the
//! current URL count, so the ingest-layer content_hash dedup
//! means an unchanged count never re-fires; a new count
//! (URL added or removed) produces a fresh Signal. Same
//! pattern as aws_ip_ranges.
//!
//! Anchor: abuse.ch HQ — Bern, Switzerland. Honest stand-in
//! (the feed has no per-URL geo).
//!
//! Parse strategy: split by `\n`, skip lines starting with `#`,
//! trim whitespace, count non-empty entries. We DO NOT emit a
//! per-URL Signal — that would flood the radar with thousands
//! of duplicates per day. The metadata pattern keeps the
//! feed "alive" without spam.

use std::time::Duration;

use chrono::Utc;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

/// abuse.ch HQ anchor — Bern, Switzerland.
const ABUSE_CH_LAT: f64 = 46.9485;
const ABUSE_CH_LON: f64 = 7.4521;

impl Source for Urlhaus {
    fn name(&self) -> &'static str {
        "urlhaus"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(4 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> futures::future::BoxFuture<'a, Result<Vec<Signal>, HubError>> {
        Box::pin(async move { fetch(ctx).await.map(|opt| opt.into_iter().collect()) })
    }
}

pub struct Urlhaus;

async fn fetch(ctx: &Ctx) -> Result<Option<Signal>, HubError> {
    let resp = tokio::time::timeout(
        REQUEST_TIMEOUT,
        ctx.http
            .get("https://urlhaus.abuse.ch/downloads/text/")
            .send(),
    )
    .await
    .map_err(|_| HubError::sensor("urlhaus: request timed out".to_string()))?
    .map_err(|e| HubError::sensor(format!("urlhaus: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "urlhaus: HTTP {s}: {snip}"
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| HubError::sensor(format!("urlhaus: body: {e}")))?;

    // Parse header (comments) and URL entries. The header
    // includes a "Last updated" line we surface in the payload.
    let mut url_count: u32 = 0;
    let mut last_updated: Option<String> = None;
    let mut sample: Vec<String> = Vec::with_capacity(10);

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# Last updated:") {
            last_updated = Some(rest.trim().to_string());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        url_count += 1;
        if sample.len() < 10 {
            sample.push(line.to_string());
        }
    }

    if url_count == 0 {
        return Ok(None);
    }

    let payload = json!({
        "url_count": url_count,
        "last_updated": last_updated,
        "sample_urls": sample,
        "fetched_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("URLhaus malware URLs: {} active entries", url_count),
            ABUSE_CH_LAT,
            ABUSE_CH_LON,
            format!("urlhaus:{}", url_count),
        )
        .payload(payload),
    ))
}