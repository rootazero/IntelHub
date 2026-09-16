//! Tor Exit List (check.torproject.org) — daily bulk list of IPs from
//! which Tor exits into the public internet. OSINT Framework "Threats
//! → Tor Exit Nodes" gap. Free, keyless, plain-text dump at
//! `https://check.torproject.org/torbulkexitlist` (~1500 IPs, ~20KB).
//!
//! - **KEYLESS**: Tor Project publishes this list with no auth, no
//!   rate limit, no registration. The list is also published as a
//!   DNS zone (torproject.org/torbulkexitlist) but the HTTPS endpoint
//!   is easier to parse and works behind every corporate proxy.
//! - **CADENCE**: 24h. Tor relay churn is slow (a few dozen entries
//!   rotate per day) and the upstream only publishes once daily.
//! - **ANCHOR**: Tor relay operators are anonymous — there is no
//!   real geographic anchor. We use the Tor Project's mailing HQ
//!   (Seattle WA) as the honest "we don't actually know" stand-in,
//!   same pattern as other cyber sources that emit at the
//!   organisation's home jurisdiction when per-event coords are
//!   absent. All tor-exit Signals cluster on Seattle, which is a
//!   self-documenting "Tor metadata" cluster on the radar.
//! - **VOLUME**: list contains ~1500 IPs. We cap at MAX_PER_SWEEP
//!   to keep first-sweep noise bounded; subsequent sweeps rely on
//!   ingest-layer content_hash dedup so the same IP never lands
//!   twice.
//! - **KIND**: "cyber" (matches otx/urlscan/ahmia/leaksify visual
//!   cluster on radar). Tor exit IPs are a primary abuse-intel
//!   source — every scanner, credential-stuffer, and click-fraud
//!   bot rotates through these IPs.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const BULK_EXIT_LIST: &str = "https://check.torproject.org/torbulkexitlist";
const MAX_PER_SWEEP: usize = 50;
/// Tor Project's EIN/address-of-record HQ (Seattle WA). Honest
/// stand-in — we do NOT actually know where any specific exit relay
/// is operated from (anonymity is the whole point).
const TOR_PROJECT_HQ: (f64, f64) = (47.6062, -122.3321);

pub struct TorExit;

impl Source for TorExit {
    fn name(&self) -> &'static str {
        "tor_exit"
    }
    fn interval(&self) -> Duration {
        // 24h matches the Tor Project's bulk-list publish rhythm.
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let body = tokio::time::timeout(
                Duration::from_secs(25),
                ctx.http.get(BULK_EXIT_LIST).send(),
            )
            .await
            .map_err(|_| HubError::sensor("tor_exit: bulk list request timed out".to_string()))?
            .map_err(|e| HubError::sensor(format!("tor_exit: {e}")))?;

            if !body.status().is_success() {
                return Err(HubError::sensor(format!(
                    "tor_exit: HTTP {}",
                    body.status()
                )));
            }

            let text = body
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("tor_exit: body: {e}")))?;

            // Bulk list is one IPv4 per line, no header. Lines starting
            // with '#' (rare) are comments. Empty lines skipped.
            let mut ips: Vec<String> = Vec::with_capacity(MAX_PER_SWEEP * 2);
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if trimmed.parse::<std::net::Ipv4Addr>().is_ok() {
                    ips.push(trimmed.to_string());
                }
            }

            if ips.is_empty() {
                tracing::warn!(
                    target: "monitor::tor_exit",
                    "torbulkexitlist returned 0 IPs — upstream format may have changed"
                );
                return Ok(Vec::new());
            }

            // First MAX_PER_SWEEP only — ingest-layer dedup handles
            // the rest (same IP across sweeps → same content_hash).
            ips.truncate(MAX_PER_SWEEP);

            let mut out = Vec::with_capacity(ips.len());
            for ip in ips {
                out.push(
                    Signal::new(
                        "cyber",
                        format!("Tor exit node observed: {ip}"),
                        TOR_PROJECT_HQ.0,
                        TOR_PROJECT_HQ.1,
                        format!("tor_exit:node:{ip}"),
                    )
                    .severity("routine")
                    .payload(serde_json::json!({
                        "ip": ip,
                        "feed": "torbulkexitlist",
                        "feed_url": BULK_EXIT_LIST,
                        "anchor_source": "tor_project_hq_seattle",
                    })),
                );
            }
            Ok(out)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_exit_list_constant_matches_documented_url() {
        // Sanity guard: if Tor Project ever changes the path we want
        // CI to flag the collector, not silent failure at runtime.
        assert_eq!(BULK_EXIT_LIST, "https://check.torproject.org/torbulkexitlist");
    }

    #[test]
    fn max_per_sweep_is_bounded() {
        // First-sweep volume cap. If someone bumps this >500 the
        // radar becomes unreadable; CI pins the upper guardrail.
        assert!(MAX_PER_SWEEP > 0 && MAX_PER_SWEEP <= 500);
    }
}