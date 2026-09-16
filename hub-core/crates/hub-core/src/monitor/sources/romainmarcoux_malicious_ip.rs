//! romainmarcoux malicious-ip aggregator (`full-40k.txt`).
//!
//! Free keyless GitHub raw feed, 40,000 most malicious IPs
//! (scanners + bruteforce), ordered by source-frequency.
//! https://github.com/romainmarcoux/malicious-ip
//!
//! metadata-pattern: emit ONE Signal per sweep with IP count
//! embedded in `external_id` so the ingest-layer content_hash
//! dedup means unchanged file never re-fires. Per-IP Signals
//! would flood the radar (40K × 24 sweeps/day = 960K/day).
//!
//! Upstream README: file updates hourly; 24h cadence is
//! sufficient for OSINT.
//!
//! Anchor: romainmarcoux is in Paris FR (honest stand-in —
//! GitHub maintainer location).

use std::time::Duration;

use chrono::Utc;
use serde_json::json;

use crate::monitor::{Ctx, Signal, Source};
use crate::HubError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

const ROMAINX_URL: &str =
    "https://raw.githubusercontent.com/romainmarcoux/malicious-ip/master/full-40k.txt";

/// romainmarcoux is in Paris, FR.
const ROMAINX_LAT: f64 = 48.85;
const ROMAINX_LON: f64 = 2.35;

pub struct RomainmarcouxMaliciousIp;

impl Source for RomainmarcouxMaliciousIp {
    fn name(&self) -> &'static str {
        "romainmarcoux_malicious_ip"
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

async fn fetch(ctx: &Ctx) -> Result<Option<Signal>, HubError> {
    let resp = tokio::time::timeout(REQUEST_TIMEOUT, ctx.http.get(ROMAINX_URL).send())
        .await
        .map_err(|_| {
            HubError::sensor("romainmarcoux_malicious_ip: request timed out".to_string())
        })?
        .map_err(|e| HubError::sensor(format!("romainmarcoux_malicious_ip: {e}")))?;

    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snip: String = body.chars().take(160).collect();
        return Err(HubError::sensor(format!(
            "romainmarcoux_malicious_ip: HTTP {s}: {snip}"
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| HubError::sensor(format!("romainmarcoux_malicious_ip: body: {e}")))?;

    let count = body.lines().filter(|l| !l.trim().is_empty()).count();
    if count == 0 {
        return Ok(None);
    }

    let payload = json!({
        "list_url": ROMAINX_URL,
        "ip_count": count,
        "source": "romainmarcoux/malicious-ip (full-40k.txt, ordered by source-frequency)",
        "whitelist": "Google Bot, Bing Bot removed per upstream README",
        "direction": "WAN > LAN/DMZ only — per upstream README",
        "fetched_at": Utc::now().to_rfc3339(),
    });

    Ok(Some(
        Signal::new(
            "cyber",
            format!("romainmarcoux malicious-ip {} IPs", count),
            ROMAINX_LAT,
            ROMAINX_LON,
            format!("romainmarcoux_malicious_ip:{}", count),
        )
        .payload(payload),
    ))
}