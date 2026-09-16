//! Spamhaus DROP (Don't Route Or Peer) List — the authoritative
//! free keyless list of netblocks operated by spammers, abusers,
//! and cybercrime. OSINT Framework "IP → Spamhaus" gap. Endpoint:
//! `https://www.spamhaus.org/drop/drop.txt` returns ~1700 CIDR
//! netblocks plain-text, format `1.10.16.0/20 ; SBL256894`.
//!
//! - **KEYLESS**: Spamhaus publishes DROP under no-restriction
//!   redistribution. No auth, no rate limit, no registration.
//! - **CADENCE**: 24h. Spamhaus updates DROP several times daily
//!   (timestamps in the body header); 24h sweep matches the
//!   bulk-refresh rhythm.
//! - **ANCHOR**: netblocks don't have a single geo (they span
//!   multiple jurisdictions). We anchor at Spamhaus HQ (Geneva
//!   CH — international anti-abuse org) as the honest stand-in
//!   for "this netblock was flagged by Spamhaus". Cyber-source
//!   cluster pattern: operator-HQ anchor.
//! - **KIND**: "cyber" (matches otx/urlscan/ahmia/leaksify/tor_exit
//!   / ipsum / crtsh / openphish / shodan_internetdb visual
//!   cluster). DROP is complementary to ipsum (top-N per IP) and
//!   shodan_internetdb (per-IP intel): DROP is upstream blocklist
//!   intel that catches entire netblocks spamhaus has convicted.
//! - **VOLUME**: ~1700 netblocks per fetch. We cap at MAX_PER_SWEEP
//!   so first-deploy doesn't flood the radar; content-hash
//!   dedup keyed on netblock + SBL ID keeps repeat emissions
//!   bounded.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const DROP_LIST: &str = "https://www.spamhaus.org/drop/drop.txt";
const MAX_PER_SWEEP: usize = 60;

/// Spamhaus Project HQ (Geneva CH) — international anti-abuse
/// organisation. Honest stand-in for "this netblock was flagged
/// by Spamhaus".
const SPAMHAUS_HQ: (f64, f64) = (46.2044, 6.1432);

pub struct SpamhausDrop;

impl Source for SpamhausDrop {
    fn name(&self) -> &'static str {
        "spamhaus_drop"
    }
    fn interval(&self) -> Duration {
        // 24h matches Spamhaus daily publish rhythm.
        Duration::from_secs(24 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            let body = tokio::time::timeout(
                Duration::from_secs(25),
                ctx.http.get(DROP_LIST).send(),
            )
            .await
            .map_err(|_| HubError::sensor("spamhaus_drop: list request timed out".to_string()))?
            .map_err(|e| HubError::sensor(format!("spamhaus_drop: {e}")))?;

            if !body.status().is_success() {
                return Err(HubError::sensor(format!(
                    "spamhaus_drop: HTTP {}",
                    body.status()
                )));
            }

            let text = body
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("spamhaus_drop: body: {e}")))?;

            // Format: `; comment`, blank lines, or
            // `<cidr> ; <SBLxxxxx>`. SBL = Spamhaus Block List ID.
            let mut netblocks: Vec<(String, String)> = Vec::with_capacity(MAX_PER_SWEEP * 2);
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with(';') {
                    continue;
                }
                // Split on the `;` separator — CIDR first, SBL ID
                // second. Both are whitespace-trimmed.
                let mut parts = trimmed.splitn(2, ';');
                let cidr = parts.next().unwrap_or("").trim().to_string();
                let sbl = parts
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_start_matches("SBL")
                    .to_string();
                if cidr.is_empty() || !cidr.contains('/') {
                    continue;
                }
                netblocks.push((cidr, sbl));
            }

            if netblocks.is_empty() {
                tracing::warn!(
                    target: "monitor::spamhaus_drop",
                    "DROP list parsed 0 netblocks — upstream format may have changed"
                );
                return Ok(Vec::new());
            }

            netblocks.truncate(MAX_PER_SWEEP);

            let mut out = Vec::with_capacity(netblocks.len());
            for (cidr, sbl) in netblocks {
                out.push(
                    Signal::new(
                        "cyber",
                        format!("Spamhaus DROP netblock: {cidr} (SBL{sbl})"),
                        SPAMHAUS_HQ.0,
                        SPAMHAUS_HQ.1,
                        format!("spamhaus_drop:netblock:{cidr}"),
                    )
                    .severity("priority")
                    .payload(serde_json::json!({
                        "cidr": cidr,
                        "sbl_id": sbl,
                        "feed": "spamhaus_drop",
                        "feed_url": DROP_LIST,
                        "anchor_source": "spamhaus_hq_geneva",
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
    fn drop_list_constant_matches_documented_url() {
        assert_eq!(DROP_LIST, "https://www.spamhaus.org/drop/drop.txt");
    }

    #[test]
    fn max_per_sweep_bounded() {
        assert!(MAX_PER_SWEEP > 0 && MAX_PER_SWEEP <= 500);
    }
}