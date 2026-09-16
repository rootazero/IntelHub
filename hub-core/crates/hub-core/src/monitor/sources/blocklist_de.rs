//! blocklist.de — fail2ban community IP blocklists aggregated from
//! thousands of sensors worldwide. Free, keyless, plain-text.
//! OSINT Framework "Threat Intel (per-IP, attacker types)" gap.
//! Endpoint: `https://lists.blocklist.de/lists/<list>.txt` — many
//! attack-type-specific lists (ssh, mail, apache, ftp, bots, ...).
//!
//! - **KEYLESS**: blocklist.de is a German fail2ban community
//!   project. No auth, no rate limit, no registration.
//! - **CADENCE**: 12h. blocklist.de updates hourly; 12h captures
//!   the bulk of meaningful churn without redundant fetches.
//! - **FEED SELECTION**: HUB_BLOCKLIST_DE_FEED env var. Default
//!   = `ssh` (SSH brute-force attackers, ~4700 IPs at any
//!   moment). Other useful feeds: `mail`, `apache`, `ftp`, `bots`,
//!   `strongips`, `all` (aggregated). See
//!   https://lists.blocklist.de/lists/ for the full directory.
//! - **ANCHOR**: blocklist.de operator is a German fail2ban
//!   community maintainer. We anchor at Berlin as the honest
//!   stand-in for "this IP was flagged by the German fail2ban
//!   community".
//! - **KIND**: "cyber" (matches the existing OSINT bridge
//!   cluster). blocklist.de is complementary to ipsum (per-IP
//!   aggregated blocklist) and shodan_internetdb (per-IP intel):
//// — blocklist.de is per-attack-type IP blocklist intel with
//!   fail2ban provenance.
//! - **VOLUME**: ~4700 IPs per feed (ssh). We cap at
//!   MAX_PER_SWEEP; ingest-layer content_hash dedup keeps repeat
//!   emissions bounded.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::time::Duration;

use crate::error::{HubError, Result};

use super::super::{Ctx, Signal, Source};

const BLOCKLIST_DE_BASE: &str = "https://lists.blocklist.de/lists";
const DEFAULT_FEED: &str = "ssh";
const MAX_PER_SWEEP: usize = 60;

/// blocklist.de operator (Berlin DE — German fail2ban community).
/// Honest stand-in for "this IP was flagged by the German
/// fail2ban community".
const BLOCKLIST_DE_HQ: (f64, f64) = (52.5200, 13.4050);

pub struct BlocklistDe;

impl Source for BlocklistDe {
    fn name(&self) -> &'static str {
        "blocklist_de"
    }
    fn interval(&self) -> Duration {
        // 12h matches blocklist.de's bulk-refresh rhythm.
        Duration::from_secs(12 * 3600)
    }
    fn fetch<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Vec<Signal>>> {
        async move {
            // Feed name from env (default `ssh`). Whitespace-
            // stripped + path-traversal guarded (no `..` /
            // slashes — blocklist.de would 404 but we don't want
            // the operator to be able to point us at arbitrary
            // URLs via env override).
            let feed = ctx
                .config
                .monitor_blocklist_de_feed
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(DEFAULT_FEED);
            if feed.contains('/') || feed.contains("..") {
                return Err(HubError::sensor(format!(
                    "blocklist_de: invalid feed name {feed:?} (no slashes / no '..')"
                )));
            }

            let url = format!("{BLOCKLIST_DE_BASE}/{feed}.txt");
            let body = tokio::time::timeout(
                Duration::from_secs(25),
                ctx.http.get(&url).send(),
            )
            .await
            .map_err(|_| HubError::sensor("blocklist_de: list request timed out".to_string()))?
            .map_err(|e| HubError::sensor(format!("blocklist_de: {e}")))?;

            if !body.status().is_success() {
                return Err(HubError::sensor(format!(
                    "blocklist_de: HTTP {}",
                    body.status()
                )));
            }

            let text = body
                .text()
                .await
                .map_err(|e| HubError::sensor(format!("blocklist_de: body: {e}")))?;

            // Format: one IPv4 per line, no header. Empty lines
            // and `#`-comments skipped.
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
                    target: "monitor::blocklist_de",
                    feed = feed,
                    "feed parsed 0 IPs — upstream format may have changed"
                );
                return Ok(Vec::new());
            }

            ips.truncate(MAX_PER_SWEEP);

            let mut out = Vec::with_capacity(ips.len());
            for ip in ips {
                out.push(
                    Signal::new(
                        "cyber",
                        format!("blocklist.de ({feed}): {ip}"),
                        BLOCKLIST_DE_HQ.0,
                        BLOCKLIST_DE_HQ.1,
                        format!("blocklist_de:{feed}:{ip}"),
                    )
                    .severity("priority")
                    .payload(serde_json::json!({
                        "ip": ip,
                        "feed": feed,
                        "feed_url": url,
                        "anchor_source": "blocklist_de_hq_berlin",
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
    fn blocklist_de_base_constant_matches_documented_url() {
        assert_eq!(BLOCKLIST_DE_BASE, "https://lists.blocklist.de/lists");
    }

    #[test]
    fn max_per_sweep_bounded() {
        assert!(MAX_PER_SWEEP > 0 && MAX_PER_SWEEP <= 500);
    }
}